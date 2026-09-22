use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::plates::{Error, Input, MAX_UPLOAD, ModelInput, Plate, Result, Store};

// Browsers may send multipart POSTs across origins without a CORS preflight.
// This is a browser-origin check; native API clients need no credential/header.
pub async fn same_origin(request: axum::extract::Request, next: Next) -> Response {
    if !request.method().is_safe()
        && let Some(origin) = request.headers().get(header::ORIGIN)
    {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        let allowed = origin
            .to_str()
            .ok()
            .and_then(|value| value.parse::<axum::http::Uri>().ok())
            .is_some_and(|origin| {
                matches!(origin.scheme_str(), Some("http" | "https"))
                    && origin.path() == "/"
                    && origin.query().is_none()
                    && origin.authority().is_some_and(|authority| {
                        host.is_some_and(|host| authority.as_str().eq_ignore_ascii_case(host))
                    })
            });
        if !allowed {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error":"Cross-origin writes are not allowed"})),
            )
                .into_response();
        }
    }
    next.run(request).await
}

pub fn router(store: Store) -> Router {
    Router::new()
        .route("/api/plates", get(list).post(create))
        .route("/api/plates/{id}", get(read).delete(remove))
        .route("/api/plates/{id}/duplicate", post(duplicate))
        .route("/api/plates/{id}/files/{*path}", get(file))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(store)
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Slicer(message) => {
                return (StatusCode::BAD_GATEWAY, Json(json!({"error":message}))).into_response();
            }
            Self::Invalid(message) => (StatusCode::BAD_REQUEST, message),
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, "OrcaSlicer timed out"),
            Self::NotFound => (StatusCode::NOT_FOUND, "Requested item not found"),
            Self::Io(error) => {
                tracing::error!(%error, "plate storage operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Plate storage operation failed",
                )
            }
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|e| Error::Io(std::io::Error::other(e)))?
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
}

async fn list(State(store): State<Store>, Query(query): Query<Search>) -> Result<Json<Vec<Plate>>> {
    if query.q.len() > 1024 {
        return Err(Error::Invalid("Search query is too long"));
    }
    blocking(move || store.list(&query.q)).await.map(Json)
}

async fn read(State(store): State<Store>, Path(id): Path<String>) -> Result<Json<Plate>> {
    blocking(move || store.get(&id)).await.map(Json)
}

async fn remove(State(store): State<Store>, Path(id): Path<String>) -> Result<StatusCode> {
    blocking(move || store.delete(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Duplicate {
    name: String,
}

async fn duplicate(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(input): Json<Duplicate>,
) -> Result<(StatusCode, Json<Plate>)> {
    let copy = blocking(move || store.duplicate(&id, &input.name)).await?;
    Ok((StatusCode::CREATED, Json(copy)))
}

async fn create(
    State(store): State<Store>,
    registry: Option<Extension<std::sync::Arc<crate::registry::Registry>>>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Plate>)> {
    let mut input = form(multipart).await?;
    if let Some(Extension(registry)) = registry {
        registry.fill_creation(&mut input.conditions).await?;
    }
    let plate = blocking(move || store.save(input)).await?;
    Ok((StatusCode::CREATED, Json(plate)))
}

async fn file(
    State(store): State<Store>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response> {
    let bytes = blocking(move || store.read_file(&id, &path)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

async fn form(mut multipart: Multipart) -> Result<Input> {
    let mut name = None;
    let mut conditions = None;
    let mut models = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| Error::Invalid("Invalid multipart upload"))?
    {
        match field.name().unwrap_or("") {
            "name" if name.is_none() => {
                name = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| Error::Invalid("Invalid name"))?,
                );
            }
            "conditions" if conditions.is_none() => {
                conditions = Some(
                    serde_json::from_str::<crate::plates::Conditions>(
                        &field
                            .text()
                            .await
                            .map_err(|_| Error::Invalid("Invalid conditions"))?,
                    )
                    .map_err(|_| Error::Invalid("Invalid conditions"))?,
                );
            }
            "models" if models.len() < 64 => {
                let filename = field
                    .file_name()
                    .ok_or(Error::Invalid("Missing STL filename"))?
                    .to_owned();
                let data = field
                    .bytes()
                    .await
                    .map_err(|_| Error::Invalid("Invalid STL upload"))?
                    .to_vec();
                models.push(ModelInput {
                    name: filename,
                    data,
                    source: None,
                });
            }
            _ => return Err(Error::Invalid("Unexpected or duplicate form field")),
        }
    }
    Ok(Input {
        conditions: conditions.unwrap_or_default(),
        name: name.ok_or(Error::Invalid("Missing plate name"))?,
        models,
    })
}
