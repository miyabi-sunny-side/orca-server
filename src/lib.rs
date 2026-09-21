mod ams;
mod artifacts;
mod database;
mod estimates;
mod filament;
mod mcp;
pub mod notifications;
mod plate_api;
pub mod plates;
mod print_start;
pub mod printer;
mod printer_state;
mod products;
mod profiles;
pub mod queue;
pub mod registry;
pub mod scad;
mod search;
pub mod slicer;

use axum::{
    Json, Router,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    response::Response,
    routing::get,
};
use serde::Serialize;
use tower_http::trace::TraceLayer;

static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/client/dist");

pub fn app_with_source(store: plates::Store, source: Option<scad::Source>) -> Router {
    app_with_slicer(store, source, None)
}

pub fn app_with_slicer(
    mut store: plates::Store,
    source: Option<scad::Source>,
    slicer: Option<slicer::Slicer>,
) -> Router {
    store.profiles = slicer.as_ref().map(|s| s.profiles.clone());
    app()
        .merge(plate_api::router(store.clone()))
        .merge(scad::router(store, source))
        .merge(slicer::router(slicer))
        .layer(axum::middleware::from_fn(plate_api::same_origin))
}

pub fn app_with_store(store: plates::Store) -> Router {
    app_with_source(store, None)
}

/// Add trusted-network MCP tools to the fully composed application API.
pub fn with_mcp(api: Router) -> Router {
    mcp::mount(api)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn app() -> Router {
    let api = Router::new()
        .route("/health", get(api_health))
        .route(
            "/about",
            get(|| async {
                Json(serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "source_url": option_env!("ORCA_SOURCE_URL").filter(|url| !url.is_empty()),
                }))
            }),
        )
        .fallback(api_not_found);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/LICENSE", get(|| async { include_str!("../LICENSE") }))
        .route(
            "/THIRD_PARTY_NOTICES",
            get(|| async { include_str!("../THIRD_PARTY_NOTICES") }),
        )
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/", axum::routing::any(api_not_found))
        .nest("/api", api)
        .fallback_service(get(ui))
        .layer(TraceLayer::new_for_http())
}

async fn ui(uri: Uri) -> Response {
    let file = UI
        .get_file(uri.path().trim_start_matches('/'))
        .unwrap_or_else(|| {
            UI.get_file("index.html")
                .expect("build requires index.html")
        });
    let content_type = mime_guess::from_path(file.path()).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, content_type.as_ref())],
        file.contents(),
    )
        .into_response()
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn api_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found\n")
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::app;

    async fn get(uri: &str) -> axum::response::Response {
        app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn ui_is_available_without_a_static_directory() {
        let response = get("/").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<!doctype html>"));
        assert!(std::str::from_utf8(&body).unwrap().contains("/assets/"));
    }

    #[tokio::test]
    async fn compiled_assets_are_served_with_their_content_types() {
        let response = get("/").await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        for (attribute, extension, content_type) in [
            ("src=\"", ".js", "text/javascript"),
            ("href=\"", ".css", "text/css"),
        ] {
            let asset = html
                .split(attribute)
                .skip(1)
                .filter_map(|part| part.split('"').next())
                .find(|path| path.ends_with(extension))
                .expect("compiled HTML references its asset");
            let response = get(asset).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], content_type);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(!body.is_empty());
            assert!(!body.starts_with(b"<!doctype html>"));

            let response = app()
                .oneshot(
                    Request::builder()
                        .method("HEAD")
                        .uri(asset)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], content_type);
            assert!(
                to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn ui_rejects_mutating_requests() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/projects/example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn liveness_is_lightweight_plain_text() {
        let response = get("/healthz").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn api_health_returns_stable_json() {
        let response = get("/api/health").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            r#"{"status":"ok"}"#
        );
    }

    #[tokio::test]
    async fn demo_routes_are_not_part_of_the_service() {
        for uri in ["/api/items", "/api/items/theme"] {
            assert_eq!(get(uri).await.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn unknown_api_routes_do_not_fall_back_to_the_spa() {
        for uri in ["/api", "/api/", "/api/missing"] {
            let response = get(uri).await;

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn unknown_client_routes_return_the_spa_with_success() {
        let response = get("/projects/example").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-type").unwrap(), "text/html");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<!doctype html>"));
    }
}
