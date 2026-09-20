use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
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
        .route("/api/plates/{id}", get(read).put(replace))
        .route("/api/plates/{id}/files/{*path}", get(file))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(store)
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Invalid(message) => (StatusCode::BAD_REQUEST, message),
            Self::NotFound => (StatusCode::NOT_FOUND, "Plate or file not found"),
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

async fn blocking<T: Send + 'static>(
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

async fn create(
    State(store): State<Store>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Plate>)> {
    let input = form(multipart).await?;
    let plate = blocking(move || store.save(None, input)).await?;
    Ok((StatusCode::CREATED, Json(plate)))
}

async fn replace(
    State(store): State<Store>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> Result<Json<Plate>> {
    let input = form(multipart).await?;
    blocking(move || store.save(Some(&id), input))
        .await
        .map(Json)
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
    let mut settings = None;
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
            "settings" if settings.is_none() => {
                let text = field
                    .text()
                    .await
                    .map_err(|_| Error::Invalid("Invalid settings"))?;
                if text.len() > 16 * 1024 {
                    return Err(Error::Invalid("Settings exceed 16 KiB"));
                }
                settings = Some(
                    serde_json::from_str(&text)
                        .map_err(|_| Error::Invalid("Settings must be JSON"))?,
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
        name: name.ok_or(Error::Invalid("Missing plate name"))?,
        models,
        settings: settings.unwrap_or_else(|| json!({})),
    })
}
