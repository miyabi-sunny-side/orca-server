use crate::plates::{Error, Input, MAX_UPLOAD, ModelInput, Result, valid_model_name};
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
struct ImportState {
    store: crate::plates::Store,
    source: Option<Source>,
}

pub(crate) fn router(store: crate::plates::Store, source: Option<Source>) -> Router {
    Router::new()
        .route("/api/scad/models", get(list_models))
        .route("/api/plates/import", post(import))
        .with_state(ImportState { store, source })
}

fn require_source(source: Option<Source>) -> Result<Source> {
    source.ok_or(Error::Unavailable("SCAD_LIVE_URL is not configured"))
}

async fn list_models(State(state): State<ImportState>) -> Result<Json<Vec<String>>> {
    require_source(state.source)?.models().await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRequest {
    name: String,
    models: Vec<String>,
    plate_id: Option<String>,
    #[serde(default)]
    settings: serde_json::Map<String, serde_json::Value>,
}

async fn import(
    State(state): State<ImportState>,
    payload: std::result::Result<Json<ImportRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::plates::Plate>)> {
    let Json(payload) = payload.map_err(|_| Error::Invalid("Invalid import request"))?;
    let source = require_source(state.source)?;
    if let Some(id) = payload.plate_id.clone() {
        let store = state.store.clone();
        crate::plate_api::blocking(move || store.get(&id)).await?;
    }
    let input = source
        .input(payload.name, payload.models, payload.settings.into())
        .await?;
    let status = if payload.plate_id.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let plate =
        crate::plate_api::blocking(move || state.store.save(payload.plate_id.as_deref(), input))
            .await?;
    Ok((status, Json(plate)))
}

#[derive(Clone)]
pub struct Source {
    base: Url,
    client: Client,
}

impl Source {
    /// Configures a trusted scad-live HTTP endpoint, optionally under a path prefix.
    ///
    /// # Errors
    /// Rejects non-HTTP URLs, credentials, queries, fragments, or client initialization errors.
    pub fn new(base: &str) -> Result<Self> {
        let mut base = Url::parse(base).map_err(|_| Error::Invalid("Invalid SCAD_LIVE_URL"))?;
        if base.scheme() != "http"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(Error::Invalid(
                "SCAD_LIVE_URL must be HTTP without credentials, query, or fragment",
            ));
        }
        if !base.path().ends_with('/') {
            base.path_segments_mut()
                .map_err(|()| Error::Invalid("Invalid SCAD_LIVE_URL"))?
                .push("");
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Invalid("Cannot initialize scad-live client"))?;
        Ok(Self { base, client })
    }

    /// Lists the relative STL paths supplied by scad-live.
    ///
    /// # Errors
    /// Rejects network failures, oversized/invalid JSON, or invalid model paths.
    pub async fn models(&self) -> Result<Vec<String>> {
        let url = self
            .base
            .join("api/models")
            .map_err(|_| Error::Invalid("Invalid SCAD_LIVE_URL"))?;
        let body = self.fetch(url, 1024 * 1024).await?;
        let mut paths: Vec<String> = serde_json::from_slice(&body)
            .map_err(|_| Error::Upstream("Invalid scad-live model list"))?;
        if paths.iter().any(|path| !valid_model_name(path)) {
            return Err(Error::Upstream("Invalid scad-live model path"));
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Copies the selected model bytes. No background synchronization is started.
    ///
    /// # Errors
    /// Rejects invalid selections, transfer failures, and payloads above the plate limit.
    pub async fn input(
        &self,
        name: String,
        models: Vec<String>,
        settings: serde_json::Value,
    ) -> Result<Input> {
        crate::plates::validate_metadata(&name, &settings)?;
        if models.is_empty() || models.len() > 64 {
            return Err(Error::Invalid("Select 1–64 STL models"));
        }
        let urls: Vec<_> = models
            .iter()
            .map(|path| model_url(&self.base, path))
            .collect::<Result<_>>()?;
        let mut remaining = MAX_UPLOAD;
        let mut imported = Vec::new();
        for (path, url) in models.into_iter().zip(urls) {
            let data = self.fetch(url, remaining).await?;
            remaining -= data.len();
            imported.push(ModelInput {
                name: path.clone(),
                source: Some(path),
                data,
            });
        }
        Ok(Input {
            name,
            models: imported,
            settings,
        })
    }

    async fn fetch(&self, url: Url, limit: usize) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| Error::Upstream("scad-live request failed"))?;
        if !response.status().is_success() {
            return Err(Error::Upstream(
                "scad-live returned an unsuccessful response",
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > u64::try_from(limit).unwrap_or(u64::MAX))
        {
            return Err(Error::Upstream("scad-live response exceeds the size limit"));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Upstream("scad-live transfer failed"))?
        {
            if data.len().saturating_add(chunk.len()) > limit {
                return Err(Error::Upstream("scad-live response exceeds the size limit"));
            }
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    }
}

fn model_url(base: &Url, path: &str) -> Result<Url> {
    if !valid_model_name(path) {
        return Err(Error::Invalid("Invalid STL model path"));
    }
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| Error::Invalid("Invalid SCAD_LIVE_URL"))?
        .pop_if_empty()
        .push("models")
        .extend(path.split('/'));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::Body,
        extract::{Path, State},
        http::{Request, StatusCode},
        response::{IntoResponse, Redirect, Response},
        routing::get,
    };
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tower::ServiceExt;

    #[test]
    fn accepts_http_base_and_encodes_each_path_segment() {
        assert!(Source::new("http://localhost:5003/scad/").is_ok());
        for url in [
            "file:///tmp",
            "https://example.test",
            "http://user:secret@example.test",
            "http://example.test/?token=x",
            "http://example.test/#fragment",
        ] {
            assert!(Source::new(url).is_err());
        }
        let base = Url::parse("http://example.test/scad/").unwrap();
        assert_eq!(
            model_url(&base, "sub a/100% #?.stl").unwrap().as_str(),
            "http://example.test/scad/models/sub%20a/100%25%20%23%3F.stl"
        );
        for path in [
            "../bad.stl",
            "/bad.stl",
            "http://else/bad.stl",
            "a/../bad.stl",
            "x\\y.stl",
            "foo.txt",
        ] {
            assert!(model_url(&base, path).is_err());
        }
    }

    #[derive(Clone, Default)]
    struct Fixture {
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        redirected: Arc<AtomicUsize>,
    }

    async fn file(State(fixture): State<Fixture>, Path(path): Path<String>) -> Response {
        if path == "redirect.stl" {
            return Redirect::temporary("/redirect-target").into_response();
        }
        fixture.files.lock().unwrap().get(&path).map_or_else(
            || StatusCode::NOT_FOUND.into_response(),
            |data| data.clone().into_response(),
        )
    }

    struct Server {
        base: String,
        handle: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn serve(router: Router) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Server { base, handle }
    }

    async fn fixture_server(fixture: Fixture) -> Server {
        serve(
            Router::new()
                .route(
                    "/api/models",
                    get(|State(f): State<Fixture>| async move {
                        Json(f.files.lock().unwrap().keys().cloned().collect::<Vec<_>>())
                    }),
                )
                .route("/models/{*path}", get(file))
                .route(
                    "/redirect-target",
                    get(|State(f): State<Fixture>| async move {
                        f.redirected.fetch_add(1, Ordering::SeqCst);
                        "unexpected"
                    }),
                )
                .with_state(fixture),
        )
        .await
    }

    #[tokio::test]
    async fn selected_http_models_are_snapshots_until_explicit_reimport() {
        let fixture = Fixture::default();
        let stl = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        for name in ["box one.stl", "parts/箱%#?.stl"] {
            fixture
                .files
                .lock()
                .unwrap()
                .insert(name.into(), stl.clone());
        }
        let server = fixture_server(fixture.clone()).await;
        let source = Source::new(&server.base).unwrap();
        let models = source.models().await.unwrap();
        assert_eq!(models, vec!["box one.stl", "parts/箱%#?.stl"]);
        let root = tempfile::tempdir().unwrap();
        let store = crate::plates::Store::open(root.path()).unwrap();
        let input = source
            .input("Imported".into(), models.clone(), serde_json::json!({}))
            .await
            .unwrap();
        let saved = store.save(None, input).unwrap();
        assert_eq!(saved.models.len(), 2);
        assert_eq!(saved.models[0].source.as_deref(), Some("box one.stl"));
        let changed = String::from_utf8(stl.clone())
            .unwrap()
            .replace("vertex 1 0 0", "vertex 2 0 0")
            .into_bytes();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("box one.stl".into(), changed.clone());
        assert_eq!(
            store.read_file(&saved.id, &saved.models[0].path).unwrap(),
            stl
        );
        let input = source
            .input("Updated".into(), models.clone(), serde_json::json!({}))
            .await
            .unwrap();
        let updated = store.save(Some(&saved.id), input).unwrap();
        assert_eq!(
            store
                .read_file(&updated.id, &updated.models[0].path)
                .unwrap(),
            changed
        );
        fixture.files.lock().unwrap().remove("parts/箱%#?.stl");
        assert!(
            source
                .input("Failure".into(), models.clone(), serde_json::json!({}))
                .await
                .is_err()
        );
        assert_eq!(store.get(&updated.id).unwrap(), updated);
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/箱%#?.stl".into(), b"broken STL".to_vec());
        let broken = source
            .input("Broken".into(), models, serde_json::json!({}))
            .await
            .unwrap();
        assert!(store.save(Some(&updated.id), broken).is_err());
        assert_eq!(store.get(&updated.id).unwrap(), updated);
    }

    #[tokio::test]
    async fn invalid_upstream_lists_redirects_and_bad_selections_are_errors() {
        let fixture = Fixture::default();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("../escape.stl".into(), vec![]);
        let server = fixture_server(fixture.clone()).await;
        let source = Source::new(&server.base).unwrap();
        assert!(source.models().await.is_err());
        fixture
            .files
            .lock()
            .unwrap()
            .insert("large.stl".into(), vec![0; 32]);
        assert!(matches!(
            source
                .fetch(model_url(&source.base, "large.stl").unwrap(), 4)
                .await,
            Err(Error::Upstream("scad-live response exceeds the size limit"))
        ));
        assert!(
            source
                .input(
                    "Redirect".into(),
                    vec!["redirect.stl".into()],
                    serde_json::json!({})
                )
                .await
                .is_err()
        );
        assert_eq!(fixture.redirected.load(Ordering::SeqCst), 0);
        for selection in [vec![], vec!["../bad.stl".into()], vec!["x.stl".into(); 65]] {
            assert!(
                source
                    .input("Invalid".into(), selection, serde_json::json!({}))
                    .await
                    .is_err()
            );
        }
        let html =
            serve(Router::new().route("/api/models", get(|| async { "<html>not JSON</html>" })))
                .await;
        assert!(Source::new(&html.base).unwrap().models().await.is_err());
    }

    #[tokio::test]
    async fn disabled_source_is_an_explicit_service_error() {
        let root = tempfile::tempdir().unwrap();
        let response = crate::app_with_store(crate::plates::Store::open(root.path()).unwrap())
            .oneshot(
                Request::builder()
                    .uri("/api/scad/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn import_api_lists_creates_reimports_and_retains_data_on_failure() {
        let fixture = Fixture::default();
        fixture.files.lock().unwrap().insert(
            "box one.stl".into(),
            include_bytes!("../tests/fixtures/triangle.stl").to_vec(),
        );
        let server = fixture_server(fixture.clone()).await;
        let root = tempfile::tempdir().unwrap();
        let store = crate::plates::Store::open(root.path()).unwrap();
        let app = crate::app_with_source(store.clone(), Some(Source::new(&server.base).unwrap()));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/scad/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&bytes).unwrap(),
            ["box one.stl"]
        );
        let request = |value: serde_json::Value| {
            Request::builder()
                .method("POST")
                .uri("/api/plates/import")
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap()
        };
        let response = app
            .clone()
            .oneshot(request(
                serde_json::json!({"name":"First", "models":["box one.stl"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let saved: crate::plates::Plate = serde_json::from_slice(&bytes).unwrap();
        let response = app
            .clone()
            .oneshot(request(
                serde_json::json!({"plate_id":saved.id,"name":"Updated", "models":["box one.stl"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let updated = store.get(&saved.id).unwrap();
        assert_ne!(updated.revision, saved.revision);
        fixture.files.lock().unwrap().clear();
        let response = app
            .clone()
            .oneshot(request(
                serde_json::json!({"plate_id":saved.id,"name":"Failure", "models":["box one.stl"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(store.get(&saved.id).unwrap(), updated);
        let response = app
            .clone()
            .oneshot(request(
                serde_json::json!({"name":"Failure", "models":["../x.stl"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let mut outside = request(serde_json::json!({"name":"Injected", "models":["box one.stl"]}));
        outside
            .headers_mut()
            .insert("origin", "http://elsewhere.test".parse().unwrap());
        outside
            .headers_mut()
            .insert("host", "orca.local".parse().unwrap());
        assert_eq!(
            app.oneshot(outside).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
