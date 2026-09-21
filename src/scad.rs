use crate::plates::{Error, Result, valid_model_name};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
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
        .route("/api/scad/model", get(public_preview))
        .route("/api/plates/import", post(import))
        .route("/api/plates/{id}", put(replace))
        .route("/api/plates/{id}/models/{model_id}", get(preview))
        .with_state(ImportState { store, source })
}

fn require_source(source: Option<Source>) -> Result<Source> {
    source.ok_or(Error::Unavailable("SCAD_LIVE_URL is not configured"))
}

async fn preview(
    State(state): State<ImportState>,
    Path((id, model_id)): Path<(String, String)>,
) -> Result<Response> {
    let store = state.store;
    let (model, store, id) = crate::plate_api::blocking(move || {
        let model = store
            .get(&id)?
            .models
            .into_iter()
            .find(|model| model.id == model_id)
            .ok_or(Error::NotFound)?;
        Ok((model, store, id))
    })
    .await?;
    let bytes = if let Some(path) = model.source {
        require_source(state.source)?
            .model(&path, crate::plates::MAX_UPLOAD)
            .await?
    } else {
        crate::plate_api::blocking(move || store.read_file(&id, &model.id)).await?
    };
    Ok(stl_response(bytes))
}

fn stl_response(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Deserialize)]
struct ModelPath {
    path: String,
}

async fn public_preview(
    State(state): State<ImportState>,
    Query(query): Query<ModelPath>,
) -> Result<Response> {
    if !valid_model_name(&query.path) {
        return Err(Error::Invalid("Invalid STL model path"));
    }
    let source = require_source(state.source)?;
    if source.models().await?.binary_search(&query.path).is_err() {
        return Err(Error::NotFound);
    }
    source
        .model(&query.path, crate::plates::MAX_UPLOAD)
        .await
        .map(stl_response)
}

#[derive(Deserialize)]
struct ModelSearch {
    #[serde(default)]
    q: String,
}

async fn list_models(
    State(state): State<ImportState>,
    Query(query): Query<ModelSearch>,
) -> Result<Json<Vec<String>>> {
    if query.q.len() > 1024 {
        return Err(Error::Invalid("Search query is too long"));
    }
    Ok(Json(crate::search::rank(
        &query.q,
        require_source(state.source)?.models().await?,
    )))
}

async fn import(
    State(state): State<ImportState>,
    payload: std::result::Result<Json<crate::plates::Edit>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::plates::Plate>)> {
    let Json(payload) = payload.map_err(|_| Error::Invalid("Invalid composition"))?;
    let plate = save_composition(state, None, payload).await?;
    Ok((StatusCode::CREATED, Json(plate)))
}

async fn replace(
    State(state): State<ImportState>,
    Path(id): Path<String>,
    Json(edit): Json<crate::plates::Edit>,
) -> Result<Json<crate::plates::Plate>> {
    save_composition(state, Some(id), edit).await.map(Json)
}

async fn save_composition(
    state: ImportState,
    id: Option<String>,
    edit: crate::plates::Edit,
) -> Result<crate::plates::Plate> {
    if edit.models.iter().any(|model| model.source.is_some()) {
        let known = require_source(state.source)?.models().await?;
        if edit
            .models
            .iter()
            .filter_map(|model| model.source.as_ref())
            .any(|source| known.binary_search(source).is_err())
        {
            return Err(Error::Invalid(
                "Unknown SCAD model; list the available models again",
            ));
        }
    }
    crate::plate_api::blocking(move || state.store.edit(id.as_deref(), edit)).await
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

    /// Fetch the current STL for one reference; no cached copy is used.
    /// # Errors
    /// Rejects invalid paths, missing/oversized models and malformed STL.
    pub async fn model(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        let bytes = self.fetch(model_url(&self.base, path)?, limit).await?;
        crate::plates::validate_stl(&bytes)?;
        Ok(bytes)
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
    async fn each_preparation_fetches_current_stl_and_never_falls_back() {
        let fixture = Fixture::default();
        let first = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("box.stl".into(), first.clone());
        let server = fixture_server(fixture.clone()).await;
        let source = Source::new(&server.base).unwrap();
        assert_eq!(
            source
                .model("box.stl", crate::plates::MAX_UPLOAD)
                .await
                .unwrap(),
            first
        );
        let changed = String::from_utf8(first)
            .unwrap()
            .replace("vertex 1 0 0", "vertex 2 0 0")
            .into_bytes();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("box.stl".into(), changed.clone());
        assert_eq!(
            source
                .model("box.stl", crate::plates::MAX_UPLOAD)
                .await
                .unwrap(),
            changed
        );
        fixture.files.lock().unwrap().clear();
        assert!(
            source
                .model("box.stl", crate::plates::MAX_UPLOAD)
                .await
                .is_err()
        );
        fixture
            .files
            .lock()
            .unwrap()
            .insert("box.stl".into(), b"broken".to_vec());
        assert!(
            source
                .model("box.stl", crate::plates::MAX_UPLOAD)
                .await
                .is_err()
        );
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
        assert!(source.model("redirect.stl", 1024).await.is_err());
        assert_eq!(fixture.redirected.load(Ordering::SeqCst), 0);
        assert!(source.model("../bad.stl", 1024).await.is_err());
        let html =
            serve(Router::new().route("/api/models", get(|| async { "<html>not JSON</html>" })))
                .await;
        assert!(Source::new(&html.base).unwrap().models().await.is_err());
    }

    #[tokio::test]
    async fn unsaved_preview_reads_only_current_published_models_without_saving() {
        use axum::body::to_bytes;
        let root = tempfile::tempdir().unwrap();
        let store = crate::plates::Store::open(root.path()).unwrap();
        let fixture = Fixture::default();
        let triangle = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        fixture.files.lock().unwrap().extend([
            ("parts/public.stl".into(), triangle.clone()),
            ("private.stl".into(), triangle.clone()),
        ]);
        let server = serve(
            Router::new()
                .route(
                    "/api/models",
                    get(|| async { Json(vec!["parts/public.stl"]) }),
                )
                .route("/models/{*path}", get(file))
                .with_state(fixture.clone()),
        )
        .await;
        let app = crate::app_with_source(store.clone(), Some(Source::new(&server.base).unwrap()));
        let request = |path: &str| {
            Request::builder()
                .uri(format!("/api/scad/model?path={path}"))
                .body(Body::empty())
                .unwrap()
        };
        let get = async |path: &str| app.clone().oneshot(request(path)).await.unwrap();
        let response = get("parts%2Fpublic.stl").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            triangle
        );
        let cube = include_bytes!("../tests/fixtures/cube.stl").to_vec();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/public.stl".into(), cube.clone());
        let response = get("parts%2Fpublic.stl").await;
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            cube
        );
        for path in ["private.stl", "missing.stl"] {
            assert_eq!(get(path).await.status(), StatusCode::NOT_FOUND);
        }
        for path in [
            "..%2Fprivate.stl",
            "%2Fprivate.stl",
            "http%3A%2F%2Fevil%2Fx.stl",
            "x.txt",
        ] {
            assert_eq!(get(path).await.status(), StatusCode::BAD_REQUEST);
        }
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/public.stl".into(), b"broken".to_vec());
        assert_eq!(
            get("parts%2Fpublic.stl").await.status(),
            StatusCode::BAD_REQUEST
        );
        fixture.files.lock().unwrap().clear();
        assert_eq!(
            get("parts%2Fpublic.stl").await.status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            crate::app_with_source(store.clone(), None)
                .oneshot(request("parts%2Fpublic.stl"))
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(store.list("").unwrap().is_empty());
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
    async fn composition_api_saves_references_without_downloading_them() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::plates::Store::open(root.path()).unwrap();
        let fixture = Fixture::default();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/part.stl".into(), b"not downloaded at save".to_vec());
        let server = fixture_server(fixture).await;
        let app = crate::app_with_source(store.clone(), Some(Source::new(&server.base).unwrap()));
        let request = |source: &str| {
            Request::builder().method("POST").uri("/api/plates/import")
            .header("content-type","application/json").body(Body::from(serde_json::json!({
                "name":"Reference", "models":[{"name":"part.stl","source":source,"quantity":2}]
            }).to_string())).unwrap()
        };
        let response = app
            .clone()
            .oneshot(request("parts/part.stl"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let saved = store.list("").unwrap().remove(0);
        assert_eq!(saved.models[0].quantity, 2);
        assert!(store.read_file(&saved.id, &saved.models[0].id).is_err());
        assert_eq!(
            app.clone()
                .oneshot(request("missing.stl"))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/plates/{}", saved.id))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name":"Rejected", "version":saved.version,
                "models":[{"name":"missing.stl","source":"missing.stl","quantity":10}]})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(store.get(&saved.id).unwrap(), saved);
        assert_eq!(store.list("").unwrap().len(), 1);
        assert_eq!(
            app.clone()
                .oneshot(request("../bad.stl"))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let mut outside = request("part.stl");
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
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Keep ownership, freshness and failures on the same stored models.
    async fn preview_reads_only_owned_models_and_current_reference_bytes() {
        use crate::plates::{Conditions, Edit, Input, ItemEdit, ModelInput, Store};
        use axum::body::to_bytes;
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let original = include_bytes!("../tests/fixtures/triangle.stl").to_vec();
        let uploaded = store
            .save(Input {
                name: "Uploaded".into(),
                conditions: Conditions::default(),
                models: vec![ModelInput {
                    name: "saved.stl".into(),
                    data: original.clone(),
                    source: None,
                }],
            })
            .unwrap();
        let referenced = store
            .edit(
                None,
                Edit {
                    name: "Reference".into(),
                    version: None,
                    conditions: Conditions::default(),
                    models: vec![ItemEdit {
                        id: None,
                        name: "current.stl".into(),
                        source: Some("parts/current.stl".into()),
                        quantity: 1,
                    }],
                },
            )
            .unwrap();
        let fixture = Fixture::default();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/current.stl".into(), original.clone());
        let server = fixture_server(fixture.clone()).await;
        let app = crate::app_with_source(store.clone(), Some(Source::new(&server.base).unwrap()));
        let request = |plate: &str, model: &str| {
            Request::builder()
                .uri(format!("/api/plates/{plate}/models/{model}"))
                .body(Body::empty())
                .unwrap()
        };
        for plate in [&uploaded, &referenced] {
            let response = app
                .clone()
                .oneshot(request(&plate.id, &plate.models[0].id))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                original
            );
        }
        for (plate, model) in [
            (&uploaded.id, &referenced.models[0].id),
            (&referenced.id, &uploaded.models[0].id),
            (&uploaded.id, &"missing".to_owned()),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(request(plate, model))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        let changed = include_bytes!("../tests/fixtures/cube.stl").to_vec();
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/current.stl".into(), changed.clone());
        let response = app
            .clone()
            .oneshot(request(&referenced.id, &referenced.models[0].id))
            .await
            .unwrap();
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            changed
        );
        fixture.files.lock().unwrap().clear();
        assert_eq!(
            app.clone()
                .oneshot(request(&referenced.id, &referenced.models[0].id))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_GATEWAY
        );
        fixture
            .files
            .lock()
            .unwrap()
            .insert("parts/current.stl".into(), b"broken".to_vec());
        assert_eq!(
            app.clone()
                .oneshot(request(&referenced.id, &referenced.models[0].id))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let offline = crate::app_with_store(store.clone());
        assert_eq!(
            offline
                .clone()
                .oneshot(request(&referenced.id, &referenced.models[0].id))
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            offline
                .oneshot(request(&uploaded.id, &uploaded.models[0].id))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(store.get(&referenced.id).unwrap(), referenced);
        assert_eq!(store.get(&uploaded.id).unwrap(), uploaded);
    }
}
