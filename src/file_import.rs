use crate::{
    model_import::{Package, Selection},
    plate_api::blocking,
    plates::{Conditions, Error, Input, MAX_UPLOAD, ModelInput, Plate, Result, Store},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};

pub(crate) struct Original {
    pub file_name: String,
    pub data: Vec<u8>,
    pub selection: Selection,
}

pub(crate) fn router(store: Store) -> Router {
    Router::new()
        .route("/api/plates/file-info", post(info))
        .route("/api/plates/file-preview", post(preview))
        .route("/api/plates/files", post(save))
        .route("/api/plates/{id}/original", get(original))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD + 256 * 1024))
        .with_state(store)
}
#[derive(Default)]
struct Upload {
    name: Option<String>,
    conditions: Option<Conditions>,
    quantities: Option<Vec<u16>>,
    plate: Option<usize>,
    models: Vec<ModelInput>,
}
impl Upload {
    async fn read(mut multipart: Multipart) -> Result<Self> {
        let mut upload = Self::default();
        let mut total = 0;
        while let Some(field) = multipart.next_field().await.map_err(|_| bad_form())? {
            let key = field.name().unwrap_or("").to_owned();
            if key == "models" && upload.models.len() < 64 {
                let name = field.file_name().ok_or_else(bad_form)?.to_owned();
                let data = field.bytes().await.map_err(|_| bad_form())?.to_vec();
                total += data.len();
                if total > MAX_UPLOAD {
                    return Err(Error::Invalid("ファイルの合計を64 MiB以内にしてください。"));
                }
                upload.models.push(ModelInput {
                    name,
                    data,
                    source: None,
                });
            } else {
                let text = field.text().await.map_err(|_| bad_form())?;
                if text.len() > 256 * 1024 {
                    return Err(bad_form());
                }
                match key.as_str() {
                    "name" if upload.name.is_none() => upload.name = Some(text),
                    "conditions" if upload.conditions.is_none() => {
                        upload.conditions =
                            Some(serde_json::from_str(&text).map_err(|_| bad_form())?);
                    }
                    "quantities" if upload.quantities.is_none() => {
                        upload.quantities =
                            Some(serde_json::from_str(&text).map_err(|_| bad_form())?);
                    }
                    "plate" if upload.plate.is_none() => {
                        upload.plate = Some(text.parse().map_err(|_| bad_form())?);
                    }
                    _ => return Err(bad_form()),
                }
            }
        }
        if upload.models.is_empty() {
            return Err(bad_form());
        }
        Ok(upload)
    }
    fn package(&self) -> Result<Package> {
        if self.models.len() != 1 || !is_3mf(&self.models[0].name) {
            return Err(Error::Invalid(
                "3MFは1ファイルずつ選んでください。STLとは同時に取り込めません。",
            ));
        }
        Package::read(&self.models[0].data)
    }
    fn selected(&self, package: &Package) -> Result<usize> {
        let index = self
            .plate
            .or_else(|| (package.plates.len() == 1).then_some(0))
            .ok_or(Error::Invalid("取り込むプレートを選んでください。"))?;
        if index >= package.plates.len() {
            return Err(Error::Invalid("プレートの選択を確認してください。"));
        }
        Ok(index)
    }
    fn into_input(mut self) -> Result<(Input, Vec<u16>, Option<Original>)> {
        let original = if self.models.iter().any(|m| is_3mf(&m.name)) {
            let package = self.package()?;
            let index = self.selected(&package)?;
            let derived = package.mesh(index)?;
            let source = self.models.pop().expect("one 3MF");
            let role_model = !package.plates[index].roles.is_empty();
            let name = if role_model {
                source.name.clone()
            } else {
                format!("{}.stl", &source.name[..source.name.len() - 4])
            };
            if !crate::plates::valid_model_name(&name) {
                return Err(bad_form());
            }
            self.models.push(ModelInput {
                name,
                data: if role_model {
                    source.data.clone()
                } else {
                    derived
                },
                source: None,
            });
            Some(Original {
                file_name: source.name,
                data: source.data,
                selection: package.plates[index].clone(),
            })
        } else {
            if self.plate.is_some()
                || self
                    .models
                    .iter()
                    .any(|m| !crate::plates::valid_model_name(&m.name))
            {
                return Err(Error::Invalid(
                    "STL（複数可）またはモデルを含む3MFを選んでください。",
                ));
            }
            None
        };
        let quantities = self
            .quantities
            .unwrap_or_else(|| vec![1; self.models.len()]);
        Ok((
            Input {
                name: self.name.ok_or_else(bad_form)?,
                conditions: self.conditions.unwrap_or_default(),
                models: self.models,
            },
            quantities,
            original,
        ))
    }
}
fn is_3mf(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".3mf")
}
fn bad_form() -> Error {
    Error::Invalid("ファイル・名前・個数を確認して再試行してください。")
}
async fn info(multipart: Multipart) -> Result<Json<Vec<Selection>>> {
    let upload = Upload::read(multipart).await?;
    blocking(move || Ok(Json(upload.package()?.plates))).await
}
async fn preview(multipart: Multipart) -> Result<Response> {
    let upload = Upload::read(multipart).await?;
    let bytes = blocking(move || {
        let package = upload.package()?;
        package.mesh(upload.selected(&package)?)
    })
    .await?;
    Ok(binary(bytes, "model/stl"))
}
async fn save(
    State(store): State<Store>,
    registry: Option<Extension<std::sync::Arc<crate::registry::Registry>>>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Plate>)> {
    let upload = Upload::read(multipart).await?;
    let (mut input, quantities, original) = blocking(move || upload.into_input()).await?;
    if let Some(Extension(registry)) = registry {
        registry.fill_creation(&mut input.conditions).await?;
    }
    let plate = blocking(move || store.save_upload(input, &quantities, original)).await?;
    Ok((StatusCode::CREATED, Json(plate)))
}
async fn original(State(store): State<Store>, Path(id): Path<String>) -> Result<Response> {
    let bytes = blocking(move || store.read_import(&id)).await?;
    Ok(binary(bytes, "model/3mf"))
}
fn binary(bytes: Vec<u8>, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        bytes,
    )
        .into_response()
}
