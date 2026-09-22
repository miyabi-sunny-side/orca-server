use crate::{
    database::{Database, Device, Settings},
    plates::{Error, Result, Store},
    printer::{Config, Printer},
    profiles::{PRINTER, Profiles},
    queue,
    slicer::Slicer,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path as FsPath, sync::Arc};
use tokio::sync::Mutex;

struct Entry {
    device: Device,
    printer: Printer,
    queue: Arc<queue::Service>,
    error: Option<&'static str>,
}
pub(crate) struct Registry {
    db: Database,
    // ponytail: one lock protects registry changes and print/queue admission for a handful of printers.
    entries: Mutex<BTreeMap<String, Entry>>,
    fallback: Entry,
    store: Store,
    profiles: Option<Arc<Profiles>>,
    slicer: Option<Slicer>,
    source: Option<crate::scad::Source>,
}

fn select_id<'a>(ids: &'a [String], requested: Option<&str>) -> Result<Option<&'a str>> {
    if let Some(id) = requested {
        return ids
            .iter()
            .find(|v| v.as_str() == id)
            .map(|v| Some(v.as_str()))
            .ok_or(Error::NotFound);
    }
    match ids {
        [] => Ok(None),
        [id] => Ok(Some(id)),
        _ => Err(Error::Conflict("Select a printer explicitly")),
    }
}
fn check_edit(in_use: bool, configuration_changed: bool, deleting: bool) -> Result<()> {
    if in_use && (configuration_changed || deleting) {
        return Err(Error::Conflict(
            "Printer is in use; finish or remove its jobs before changing configuration",
        ));
    }
    Ok(())
}

impl Registry {
    fn selected<'a>(
        &'a self,
        entries: &'a BTreeMap<String, Entry>,
        query: &Selected,
    ) -> Result<&'a Entry> {
        let ids: Vec<_> = entries.keys().cloned().collect();
        Ok(match select_id(&ids, query.printer_id.as_deref())? {
            Some(id) => entries.get(id).expect("selected existing printer"),
            None => &self.fallback,
        })
    }
    fn config(&self, settings: &Settings) -> Result<Config> {
        settings.validate()?;
        let diameter = if let Some(profiles) = &self.profiles {
            profiles.validate_process(
                &settings.machine_profile_key,
                &settings.default_process_profile_key,
            )?;
            profiles.machine(&settings.machine_profile_key)?["nozzle_diameter"][0]
                .as_str()
                .ok_or(Error::Invalid("Invalid nozzle diameter in machine profile"))?
                .to_owned()
        } else if settings.machine_profile_key == PRINTER
            && settings.default_process_profile_key == crate::profiles::Selection::default().process
        {
            // Preserve explicitly configured legacy P1S monitoring without a CLI installation.
            "0.4".into()
        } else {
            return Err(Error::Unavailable(
                "OrcaSlicer profiles are required to resolve this machine",
            ));
        };
        Config::for_settings(settings, &diameter)
    }
    fn entry(&self, device: Device, strict: bool) -> Result<Entry> {
        let (config, error) = match self.config(&device.settings) {
            Ok(config) => (Some(config), None),
            Err(Error::Invalid(message) | Error::Unavailable(message)) if !strict => {
                (None, Some(message))
            }
            Err(error) => return Err(error),
        };
        let printer = Printer::new(config, Some((self.db.clone(), device.clone())))
            .map_err(Error::Invalid)?;
        let queue = Arc::new(queue::Service::new(
            self.store.clone(),
            device.clone(),
            printer.clone(),
            self.slicer.clone(),
            self.source.clone(),
        ));
        queue.cleanup()?;
        Ok(Entry {
            device,
            printer,
            queue,
            error,
        })
    }
    async fn view(&self, entry: &Entry) -> Value {
        let mut value = serde_json::to_value(&entry.device).expect("registry serializes");
        value["status"] = json!(entry.printer.status().await);
        value["configuration_error"] = json!(entry.error);
        value["machine"]=self.profiles.as_ref().and_then(|profiles| profiles.machine(&entry.device.settings.machine_profile_key).ok())
            .map_or(Value::Null, |p| json!({"model":p.get("printer_model"),"nozzle_diameter":p.get("nozzle_diameter").and_then(|v|v.get(0))}));
        value
    }
}
impl Entry {
    async fn in_use(&self) -> bool {
        let status = self.printer.status().await;
        self.queue.in_use()
            || status
                .start
                .as_ref()
                .is_some_and(crate::print_start::Attempt::blocks_start)
            || status
                .print
                .state
                .as_deref()
                .is_some_and(|state| !matches!(state, "IDLE" | "FINISH"))
    }
    fn usable(&self) -> Result<()> {
        if let Some(error) = self.error {
            return Err(Error::Unavailable(error));
        }
        Ok(())
    }
}

/// Build the application with its persistent registry and per-device Bambu LAN paths.
/// # Errors
/// Rejects unreadable storage, unsupported schema versions or invalid initial environment settings.
pub fn router(
    _root: &FsPath,
    store: Store,
    slicer: Option<Slicer>,
    source: Option<crate::scad::Source>,
) -> Result<Router> {
    let api = crate::app_with_slicer(store.clone(), source.clone(), slicer.clone());
    let db = store.db.clone();
    let disabled = Printer::new(None, None).map_err(Error::Invalid)?;
    let fallback_device = Device {
        id: String::new(),
        settings: Settings {
            name: String::new(),
            host: String::new(),
            serial: String::new(),
            access_code: String::new(),
            tls_certificate: String::new(),
            machine_profile_key: PRINTER.into(),
            default_process_profile_key: crate::profiles::Selection::default().process,
            bed_type: crate::profiles::BEDS[0].into(),
            nozzle_material: "unknown".into(),
            mqtt_port: 8883,
            ftps_port: 990,
            start_timeout_secs: 600,
        },
    };
    let fallback = Entry {
        queue: Arc::new(queue::Service::new(
            store.clone(),
            fallback_device.clone(),
            disabled.clone(),
            slicer.clone(),
            source.clone(),
        )),
        device: fallback_device,
        printer: disabled,
        error: None,
    };
    let mut registry = Registry {
        db,
        entries: Mutex::new(BTreeMap::new()),
        fallback,
        store,
        profiles: slicer.as_ref().map(|s| s.profiles.clone()),
        slicer,
        source,
    };
    for device in registry.db.list()? {
        let entry = registry.entry(device, false)?;
        registry
            .entries
            .get_mut()
            .insert(entry.device.id.clone(), entry);
    }
    crate::estimates::start(
        registry.store.clone(),
        registry.slicer.clone(),
        registry.source.clone(),
    )?;
    let registry = Arc::new(registry);
    let routes = Router::new()
        .route(
            "/api/default-settings",
            get(read_defaults).put(update_defaults),
        )
        .route("/api/printers", get(list).post(create))
        .route("/api/printers/profiles", get(machines))
        .route("/api/printers/{id}", get(read).put(update).delete(delete))
        .route("/api/filaments", get(materials).post(create_material))
        .route("/api/plate-filaments", get(plate_materials))
        .merge(product_routes())
        .route(
            "/api/filaments/{id}",
            get(material).put(update_material).delete(delete_material),
        )
        .route("/api/filaments/{id}/profiles", get(material_profiles))
        .route("/api/filaments/{id}/settings", post(create_setting))
        .route(
            "/api/filaments/{id}/settings/{sid}",
            axum::routing::put(update_setting).delete(delete_setting),
        )
        .route("/api/printers/{id}/ams", get(inventory))
        .route("/api/printers/{id}/ams/resolve", get(resolve_inventory))
        .route(
            "/api/printers/{id}/ams/priority",
            axum::routing::put(prioritize_inventory),
        )
        .route(
            "/api/printers/{id}/ams/auto-refill",
            axum::routing::put(set_auto_refill),
        )
        .route(
            "/api/printers/{id}/ams/{slot}",
            axum::routing::put(map_inventory),
        )
        .route("/api/printer/status", get(status))
        .route(
            "/api/queue",
            get(read_queue)
                .post(act_queue)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .layer(DefaultBodyLimit::max(80 * 1024))
        .layer(axum::middleware::from_fn(crate::plate_api::same_origin))
        .with_state(registry.clone());
    Ok(api.merge(routes).layer(Extension(registry)))
}
#[derive(serde::Serialize)]
struct Defaults {
    default_printer_id: Option<String>,
    conditions: crate::plates::Conditions,
    reason: Option<&'static str>,
    infill_patterns: &'static [&'static str],
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultQuery {
    machine: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultChoice {
    default_printer_id: Option<String>,
    #[serde(flatten)]
    strength: crate::strength::Strength,
}
async fn read_defaults(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<DefaultQuery>,
) -> Result<Json<Defaults>> {
    registry.defaults(query.machine.as_deref()).await.map(Json)
}
async fn update_defaults(
    State(registry): State<Arc<Registry>>,
    Json(choice): Json<DefaultChoice>,
) -> Result<StatusCode> {
    let _entries = registry.entries.lock().await;
    let db = registry.db.clone();
    crate::plate_api::blocking(move || {
        db.set_defaults(choice.default_printer_id.as_deref(), &choice.strength)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
impl Registry {
    async fn defaults(&self, machine: Option<&str>) -> Result<Defaults> {
        let entries = self.entries.lock().await;
        let mut result = Defaults {
            default_printer_id: self.db.default_printer()?,
            conditions: crate::plates::Conditions {
                strength: self.db.default_strength()?,
                ..Default::default()
            },
            reason: None,
            infill_patterns: crate::strength::PATTERNS,
        };
        let Some(entry) = result
            .default_printer_id
            .as_ref()
            .and_then(|id| entries.get(id))
        else {
            result.reason = Some(if entries.is_empty() {
                "printer"
            } else {
                "printer_selection"
            });
            return Ok(result);
        };
        let settings = &entry.device.settings;
        let machine = machine.unwrap_or(&settings.machine_profile_key);
        let Some(profiles) = self
            .profiles
            .as_ref()
            .filter(|p| p.machine(machine).is_ok())
        else {
            result.reason = Some("profiles");
            return Ok(result);
        };
        result.conditions.required_machine_profile_key = Some(machine.into());
        result.conditions.bed_type = Some(settings.bed_type.clone());
        if profiles
            .validate_process(machine, &settings.default_process_profile_key)
            .is_ok()
        {
            result.conditions.process_profile_key =
                Some(settings.default_process_profile_key.clone());
        }
        let (current, slots) = entry.printer.ams_inventory(None).await?;
        if !current {
            result.reason = Some("ams_sync");
            return Ok(result);
        }
        let filaments = self.db.filaments()?;
        for slot in slots {
            if slot.reported.present != Some(true) || slot.load_order.is_none() {
                continue;
            }
            let Some(filament) = slot
                .filament_id
                .and_then(|id| filaments.iter().find(|f| f.id == id))
            else {
                continue;
            };
            if self.db.filament_settings(&filament.id)?.iter().any(|s| {
                s.data.machine_profile_key == machine
                    && profiles
                        .resolve_filament(&s.data, &filament.data.material)
                        .is_ok()
            }) {
                result.conditions.filament_id = Some(filament.id.clone());
                break;
            }
        }
        result.reason = if result.conditions.filament_id.is_none() {
            Some("material")
        } else if result.conditions.process_profile_key.is_none() {
            Some("process")
        } else {
            None
        };
        Ok(result)
    }
    pub(crate) async fn fill_creation(&self, value: &mut crate::plates::Conditions) -> Result<()> {
        let defaults = self
            .defaults(value.required_machine_profile_key.as_deref())
            .await?
            .conditions;
        value.required_machine_profile_key = value
            .required_machine_profile_key
            .take()
            .or(defaults.required_machine_profile_key);
        value.filament_id = value.filament_id.take().or(defaults.filament_id);
        value.process_profile_key = value
            .process_profile_key
            .take()
            .or(defaults.process_profile_key);
        value.bed_type = value.bed_type.take().or(defaults.bed_type);
        value.strength.sparse_infill_pattern = value
            .strength
            .sparse_infill_pattern
            .take()
            .or(defaults.strength.sparse_infill_pattern);
        value.strength.sparse_infill_density = value
            .strength
            .sparse_infill_density
            .or(defaults.strength.sparse_infill_density);
        value.strength.wall_loops = value.strength.wall_loops.or(defaults.strength.wall_loops);
        Ok(())
    }
}
fn product_routes() -> Router<Arc<Registry>> {
    Router::new()
        .route("/api/filament-products", get(products).post(create_product))
        .route(
            "/api/filament-products/{id}",
            get(product).put(update_product).delete(delete_product),
        )
        .route(
            "/api/filament-products/{id}/colors",
            axum::routing::post(create_color),
        )
        .route(
            "/api/filament-products/{id}/adopt",
            axum::routing::post(adopt_color),
        )
        .route(
            "/api/filament-products/{id}/colors/{fid}",
            axum::routing::put(update_color).delete(delete_color),
        )
        .route(
            "/api/filament-products/{id}/profiles",
            get(product_profiles),
        )
        .route(
            "/api/filament-products/{id}/settings",
            axum::routing::post(create_product_setting),
        )
        .route(
            "/api/filament-products/{id}/settings/{sid}",
            axum::routing::put(update_product_setting).delete(delete_product_setting),
        )
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selected {
    printer_id: Option<String>,
    plate_id: Option<String>,
}
async fn list(State(registry): State<Arc<Registry>>) -> Json<Vec<Value>> {
    let entries = registry.entries.lock().await;
    let mut values = Vec::new();
    for entry in entries.values() {
        values.push(registry.view(entry).await);
    }
    Json(values)
}
async fn machines(State(registry): State<Arc<Registry>>) -> Result<Json<Value>> {
    Ok(Json(
        registry
            .profiles
            .as_ref()
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
            .machines(),
    ))
}
async fn read(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let entries = registry.entries.lock().await;
    Ok(Json(
        registry
            .view(entries.get(&id).ok_or(Error::NotFound)?)
            .await,
    ))
}
async fn create(
    State(registry): State<Arc<Registry>>,
    Json(settings): Json<Settings>,
) -> Result<(StatusCode, Json<Value>)> {
    tokio::spawn(async move {
        let mut entries = registry.entries.lock().await;
        if registry.fallback.queue.in_use() {
            return Err(Error::Conflict(
                "Remove unassigned queue jobs before registering a printer",
            ));
        }
        let device = Device {
            id: uuid::Uuid::new_v4().to_string(),
            settings,
        };
        let entry = registry.entry(device.clone(), true)?;
        let db = registry.db.clone();
        crate::plate_api::blocking(move || db.save(&device)).await?;
        let response = registry.view(&entry).await;
        entries.insert(entry.device.id.clone(), entry);
        Ok((StatusCode::CREATED, Json(response)))
    })
    .await
    .map_err(|_| Error::Unavailable("Registry request interrupted; refresh the printer list"))?
}
async fn update(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(mut settings): Json<Settings>,
) -> Result<Json<Value>> {
    tokio::spawn(async move {
        let mut entries = registry.entries.lock().await;
        let old = entries.get(&id).ok_or(Error::NotFound)?;
        if settings.access_code.is_empty() {
            settings
                .access_code
                .clone_from(&old.device.settings.access_code);
        }
        if settings.tls_certificate.is_empty() {
            settings
                .tls_certificate
                .clone_from(&old.device.settings.tls_certificate);
        }
        let mut comparison = settings.clone();
        comparison.name.clone_from(&old.device.settings.name);
        comparison
            .default_process_profile_key
            .clone_from(&old.device.settings.default_process_profile_key);
        comparison
            .bed_type
            .clone_from(&old.device.settings.bed_type);
        let changed = comparison != old.device.settings || old.error.is_some();
        let queue = old.queue.clone();
        let _queue_guard = queue.lock.lock().await;
        let status = old.printer.status().await;
        let active = queue.active()
            || status
                .print
                .state
                .as_deref()
                .is_some_and(|s| !matches!(s, "IDLE" | "FINISH"));
        check_edit(active, changed, false)?;
        registry.config(&settings)?;
        let device = Device {
            id: id.clone(),
            settings,
        };
        let replacement = if changed {
            Some(registry.entry(device.clone(), true)?)
        } else {
            None
        };
        let saved = device.clone();
        let db = registry.db.clone();
        crate::plate_api::blocking(move || db.save(&saved)).await?;
        if let Some(entry) = replacement {
            entries.insert(id.clone(), entry);
        } else {
            entries.get_mut(&id).expect("existing printer").device = device;
        }
        Ok(Json(registry.view(&entries[&id]).await))
    })
    .await
    .map_err(|_| Error::Unavailable("Registry request interrupted; refresh the printer list"))?
}
async fn delete(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    tokio::spawn(async move {
        let mut entries = registry.entries.lock().await;
        let queue = entries.get(&id).ok_or(Error::NotFound)?.queue.clone();
        let _queue_guard = queue.lock.lock().await;
        check_edit(
            entries.get(&id).ok_or(Error::NotFound)?.in_use().await,
            false,
            true,
        )?;
        let db = registry.db.clone();
        let deleted = id.clone();
        crate::plate_api::blocking(move || db.delete(&deleted)).await?;
        entries.remove(&id);
        Ok(StatusCode::NO_CONTENT)
    })
    .await
    .map_err(|_| Error::Unavailable("Registry request interrupted; refresh the printer list"))?
}
async fn status(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
) -> Result<Json<crate::printer_state::Status>> {
    let entries = registry.entries.lock().await;
    let entry = registry.selected(&entries, &query)?;
    entry.usable()?;
    Ok(Json(entry.printer.status().await))
}
async fn read_queue(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
) -> Result<Json<Value>> {
    let queue = {
        let entries = registry.entries.lock().await;
        let entry = registry.selected(&entries, &query)?;
        entry.usable()?;
        entry.queue.clone()
    };
    queue.read(query.plate_id.as_deref()).await.map(Json)
}
async fn act_queue(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
    Json(request): Json<queue::Command>,
) -> Result<Json<Value>> {
    let queue = {
        let entries = registry.entries.lock().await;
        let entry = registry.selected(&entries, &query)?;
        entry.usable()?;
        entry.queue.clone()
    };
    tokio::spawn(async move { queue.apply(request).await.map(Json) })
        .await
        .map_err(|_| Error::Unavailable("Queue request interrupted; refresh the queue"))?
}

#[derive(Default, Deserialize)]
struct MaterialSearch {
    #[serde(default)]
    q: String,
}
async fn materials(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<MaterialSearch>,
) -> Result<Json<Vec<crate::filament::Filament>>> {
    let db = registry.db.clone();
    let materials = crate::plate_api::blocking(move || db.filaments()).await?;
    Ok(Json(crate::plate_filaments::rank(materials, &query.q)))
}

async fn plate_materials(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<crate::plate_filaments::Query>,
) -> Result<Json<crate::plate_filaments::Candidates>> {
    let entries = registry.entries.lock().await;
    let mut inventories = Vec::new();
    for entry in entries.values().filter(|e| {
        query
            .machine
            .as_ref()
            .is_none_or(|m| m == &e.device.settings.machine_profile_key)
    }) {
        let (current, slots) = match entry.printer.ams_inventory(None).await {
            Ok((current, slots)) => (Some(current), slots),
            Err(error) => {
                tracing::warn!(printer_id=%entry.device.id, %error, "Could not read plate material inventory");
                (None, vec![])
            }
        };
        inventories.push(crate::plate_filaments::Inventory {
            id: entry.device.id.clone(),
            name: entry.device.settings.name.clone(),
            machine: entry.device.settings.machine_profile_key.clone(),
            current,
            slots,
        });
    }
    let db = registry.db.clone();
    let materials = crate::plate_api::blocking(move || db.filaments()).await?;
    Ok(Json(crate::plate_filaments::candidates(
        materials,
        inventories,
        &query,
    )))
}

async fn products(
    State(registry): State<Arc<Registry>>,
) -> Result<Json<Vec<crate::products::Product>>> {
    let db = registry.db.clone();
    Ok(Json(
        crate::plate_api::blocking(move || db.products()).await?,
    ))
}
async fn product(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let db = registry.db.clone();
    let p = crate::plate_api::blocking(move || db.product(&id)).await?;
    let mut value = json!(p);
    for (s, v) in p
        .settings
        .iter()
        .zip(value["settings"].as_array_mut().expect("settings array"))
    {
        if let Ok(profile) = registry
            .profiles
            .as_ref()
            .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))
            .and_then(|profiles| profiles.resolve_filament(&s.data, &p.data.material))
        {
            v["resolved"] = temperature_view(&profile);
            v["error"] = Value::Null;
        } else {
            v["resolved"] = Value::Null;
            v["error"] = json!("Base profile is unavailable or incompatible; select it again");
        }
    }
    Ok(Json(value))
}
async fn create_product(
    State(registry): State<Arc<Registry>>,
    Json(data): Json<crate::products::ProductData>,
) -> Result<(StatusCode, Json<crate::products::Product>)> {
    let db = registry.db.clone();
    let p = crate::plate_api::blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        db.save_product(&id, &data)?;
        db.product(&id)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(p)))
}
async fn update_product(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<crate::products::ProductData>,
) -> Result<Json<crate::products::Product>> {
    let _entries = registry.entries.lock().await;
    let db = registry.db.clone();
    let profiles = registry.profiles.clone();
    Ok(Json(
        crate::plate_api::blocking(move || {
            let old = db.product(&id)?;
            if old.data.material != data.material {
                for setting in old.settings {
                    profiles
                        .as_ref()
                        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?
                        .resolve_filament(&setting.data, &data.material)?;
                }
            }
            db.save_product(&id, &data)?;
            db.product(&id)
        })
        .await?,
    ))
}
async fn delete_product(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || db.delete_product(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn create_color(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<crate::products::ColorData>,
) -> Result<(StatusCode, Json<crate::products::Color>)> {
    let db = registry.db.clone();
    let color = crate::plate_api::blocking(move || {
        db.product(&id)?;
        let fid = uuid::Uuid::new_v4().to_string();
        db.save_color(&id, &fid, &data)?;
        db.product(&id)?
            .colors
            .into_iter()
            .find(|c| c.id == fid)
            .ok_or(Error::NotFound)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(color)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptColor {
    filament_id: String,
}
async fn adopt_color(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<AdoptColor>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || db.adopt_color(&id, &data.filament_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn update_color(
    State(registry): State<Arc<Registry>>,
    Path((id, fid)): Path<(String, String)>,
    Json(data): Json<crate::products::ColorData>,
) -> Result<Json<crate::products::Color>> {
    let db = registry.db.clone();
    Ok(Json(
        crate::plate_api::blocking(move || {
            if !db.product(&id)?.colors.iter().any(|c| c.id == fid) {
                return Err(Error::NotFound);
            }
            db.save_color(&id, &fid, &data)?;
            db.product(&id)?
                .colors
                .into_iter()
                .find(|c| c.id == fid)
                .ok_or(Error::NotFound)
        })
        .await?,
    ))
}
async fn delete_color(
    State(registry): State<Arc<Registry>>,
    Path((id, fid)): Path<(String, String)>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || {
        let c = db.connection()?;
        if c.execute(
            "DELETE FROM filaments WHERE id=?1 AND product_id=?2",
            rusqlite::params![fid, id],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn product_profiles(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Query(query): Query<MaterialProfiles>,
) -> Result<Json<Value>> {
    let db = registry.db.clone();
    let p = crate::plate_api::blocking(move || db.product(&id)).await?;
    profiles_for_material(&registry, &query.machine, &p.data.material)
}
async fn create_product_setting(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<crate::filament::SettingData>,
) -> Result<(StatusCode, Json<crate::products::ProductSetting>)> {
    Ok((
        StatusCode::CREATED,
        Json(
            save_product_setting(&registry, id, uuid::Uuid::new_v4().to_string(), data, false)
                .await?,
        ),
    ))
}
async fn update_product_setting(
    State(registry): State<Arc<Registry>>,
    Path((id, sid)): Path<(String, String)>,
    Json(data): Json<crate::filament::SettingData>,
) -> Result<Json<crate::products::ProductSetting>> {
    Ok(Json(
        save_product_setting(&registry, id, sid, data, true).await?,
    ))
}
async fn save_product_setting(
    registry: &Registry,
    id: String,
    sid: String,
    data: crate::filament::SettingData,
    exists: bool,
) -> Result<crate::products::ProductSetting> {
    let _entries = registry.entries.lock().await;
    let db = registry.db.clone();
    let profiles = registry
        .profiles
        .clone()
        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?;
    crate::plate_api::blocking(move || {
        let p = db.product(&id)?;
        if exists && !p.settings.iter().any(|s| s.id == sid) {
            return Err(Error::NotFound);
        }
        profiles.resolve_filament(&data, &p.data.material)?;
        db.save_product_setting(&id, &sid, &data)?;
        Ok(crate::products::ProductSetting { id: sid, data })
    })
    .await
}
async fn delete_product_setting(
    State(registry): State<Arc<Registry>>,
    Path((id, sid)): Path<(String, String)>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || db.delete_product_setting(&id, &sid)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn material(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let db = registry.db.clone();
    let (f, settings) = crate::plate_api::blocking(move || {
        let f = db
            .filaments()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or(Error::NotFound)?;
        Ok((f, db.filament_settings(&id)?))
    })
    .await?;
    let settings: Vec<_> = settings
        .into_iter()
        .map(|s| {
            let resolved = registry
                .profiles
                .as_ref()
                .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))
                .and_then(|p| p.resolve_filament(&s.data, &f.data.material));
            let mut value = json!(s);
            if let Ok(profile) = resolved {
                value["resolved"] = temperature_view(&profile);
                value["error"] = Value::Null;
            } else {
                value["resolved"] = Value::Null;
                value["error"] =
                    json!("Base profile is unavailable or incompatible; select it again");
            }
            value
        })
        .collect();
    let c = registry.db.connection()?;
    let product_id = crate::products::product_id(&c, &f.id)?;
    Ok(Json(
        json!({"filament":f,"settings":settings,"product_id":product_id}),
    ))
}
async fn create_material(
    State(registry): State<Arc<Registry>>,
    Json(data): Json<crate::filament::FilamentData>,
) -> Result<(StatusCode, Json<crate::filament::Filament>)> {
    let db = registry.db.clone();
    let saved = crate::plate_api::blocking(move || {
        let f = crate::filament::Filament {
            id: uuid::Uuid::new_v4().to_string(),
            data,
        };
        db.save_filament(&f)?;
        Ok(f)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(saved)))
}
async fn update_material(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<crate::filament::FilamentData>,
) -> Result<Json<crate::filament::Filament>> {
    // Serialize edits with configuration changes, including settings validation.
    let _entries = registry.entries.lock().await;
    let db = registry.db.clone();
    let profiles = registry.profiles.clone();
    Ok(Json(
        crate::plate_api::blocking(move || {
            let old = db
                .filaments()?
                .into_iter()
                .find(|f| f.id == id)
                .ok_or(Error::NotFound)?;
            if old.data.material != data.material {
                let settings = db.filament_settings(&id)?;
                for s in settings {
                    profiles
                        .as_ref()
                        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?
                        .resolve_filament(&s.data, &data.material)?;
                }
            }
            let f = crate::filament::Filament { id, data };
            db.save_filament(&f)?;
            Ok(f)
        })
        .await?,
    ))
}
async fn delete_material(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || db.delete_filament(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
fn temperature_view(profile: &serde_json::Map<String, Value>) -> Value {
    json!({"nozzle_temperature_initial_layer":profile.get("nozzle_temperature_initial_layer").and_then(|v|v.get(0)),
        "nozzle_temperature":profile.get("nozzle_temperature").and_then(|v|v.get(0)),
        "required_nozzle_hrc":profile.get("required_nozzle_HRC").and_then(|v|v.get(0))})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialProfiles {
    machine: String,
}
async fn material_profiles(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Query(query): Query<MaterialProfiles>,
) -> Result<Json<Value>> {
    let db = registry.db.clone();
    let f = crate::plate_api::blocking(move || {
        db.filaments()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or(Error::NotFound)
    })
    .await?;
    profiles_for_material(&registry, &query.machine, &f.data.material)
}
fn profiles_for_material(
    registry: &Registry,
    machine: &str,
    material: &str,
) -> Result<Json<Value>> {
    let profiles = registry
        .profiles
        .as_ref()
        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?;
    let choices = profiles.choices_for(machine)?;
    let compatible: Vec<_> = choices["filaments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|key| {
            let key = key.as_str()?;
            let setting = crate::filament::SettingData {
                machine_profile_key: machine.into(),
                base_profile_key: key.into(),
                overrides_json: crate::filament::Overrides::default(),
            };
            let p = profiles.resolve_filament(&setting, material).ok()?;
            Some(json!({"key":key,"resolved":temperature_view(&p)}))
        })
        .collect();
    Ok(Json(json!(compatible)))
}
async fn create_setting(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(data): Json<crate::filament::SettingData>,
) -> Result<(StatusCode, Json<crate::filament::Setting>)> {
    let saved = save_setting(&registry, id, uuid::Uuid::new_v4().to_string(), data, false).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}
async fn update_setting(
    State(registry): State<Arc<Registry>>,
    Path((id, sid)): Path<(String, String)>,
    Json(data): Json<crate::filament::SettingData>,
) -> Result<Json<crate::filament::Setting>> {
    Ok(Json(save_setting(&registry, id, sid, data, true).await?))
}
async fn save_setting(
    registry: &Registry,
    id: String,
    sid: String,
    data: crate::filament::SettingData,
    exists: bool,
) -> Result<crate::filament::Setting> {
    let _entries = registry.entries.lock().await;
    let db = registry.db.clone();
    let profiles = registry
        .profiles
        .clone()
        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?;
    crate::plate_api::blocking(move || {
        let f = db
            .filaments()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or(Error::NotFound)?;
        if exists && !db.filament_settings(&id)?.iter().any(|s| s.id == sid) {
            return Err(Error::NotFound);
        }
        profiles.resolve_filament(&data, &f.data.material)?;
        let s = crate::filament::Setting {
            id: sid,
            filament_id: id,
            data,
        };
        db.save_setting(&s)?;
        Ok(s)
    })
    .await
}
async fn delete_setting(
    State(registry): State<Arc<Registry>>,
    Path((id, sid)): Path<(String, String)>,
) -> Result<StatusCode> {
    let db = registry.db.clone();
    crate::plate_api::blocking(move || db.delete_setting(&id, &sid)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn inventory(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    let (current, slots) = entry.printer.ams_inventory(None).await?;
    let refill = entry.printer.status().await.auto_refill;
    let db = registry.db.clone();
    let printer_id = id.clone();
    let machine = entry.device.settings.machine_profile_key.clone();
    let details = crate::plate_api::blocking(move || {
        let mut details = BTreeMap::new();
        for f in db.filaments()? {
            let settings = db.filament_settings(&f.id)?;
            let group = if current {
                db.resolve_slots(&printer_id, &f.id, &machine)?
            } else {
                vec![]
            };
            details.insert(f.id.clone(), (f, settings, group));
        }
        Ok(details)
    })
    .await?;
    let slots: Vec<_> = slots
        .into_iter()
        .map(|slot| {
            let mut value = json!(slot);
            value["current"] = json!(current && slot.reported.present.is_some());
            value["setting"] = Value::Null;
            value["priority_group"] = json!([]);
            value["backup_peers"] = json!(
                slot.ams_id
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(slot.slot_index))
                    .and_then(|n| refill.peers(n))
            );
            if let Some((f, settings, group)) =
                slot.filament_id.as_ref().and_then(|id| details.get(id))
            {
                value["filament"] = json!(f);
                if let Some(s) = settings.iter().find(|s| {
                    s.data.machine_profile_key == entry.device.settings.machine_profile_key
                }) {
                    value["setting"] = json!(s);
                    if let Some(p) = &registry.profiles
                        && let Ok(resolved) = p.resolve_filament(&s.data, &f.data.material)
                    {
                        value["setting"]["resolved"] = temperature_view(&resolved);
                        value["priority_group"] = json!(
                            group
                                .iter()
                                .map(|s| json!({"id":s.id,"revision":s.revision}))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            } else {
                value["filament"] = Value::Null;
            }
            value
        })
        .collect();
    Ok(Json(
        json!({"printer_id":id,"current":current,"slots":slots,"auto_refill":refill}),
    ))
}
async fn map_inventory(
    State(registry): State<Arc<Registry>>,
    Path((id, slot)): Path<(String, String)>,
    Json(mapping): Json<crate::ams::Mapping>,
) -> Result<StatusCode> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    entry
        .printer
        .ams_inventory(Some(crate::ams::Change::Mapping(slot, mapping)))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RequestedMaterial {
    filament_id: String,
}
fn usable_material(registry: &Registry, device: &Device, filament: &str) -> Result<()> {
    let f = registry
        .db
        .filaments()?
        .into_iter()
        .find(|f| f.id == filament)
        .ok_or(Error::NotFound)?;
    let setting = registry
        .db
        .filament_settings(filament)?
        .into_iter()
        .find(|s| s.data.machine_profile_key == device.settings.machine_profile_key)
        .ok_or(Error::Conflict("Material has no settings for this machine"))?;
    registry
        .profiles
        .as_ref()
        .ok_or(Error::Unavailable("Profiles are unavailable"))?
        .resolve_filament(&setting.data, &f.data.material)?;
    Ok(())
}
async fn resolve_inventory(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Query(q): Query<RequestedMaterial>,
) -> Result<Json<Value>> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    usable_material(&registry, &entry.device, &q.filament_id)?;
    let candidates = entry.printer.resolve_material(q.filament_id).await?;
    Ok(Json(
        json!({"preferred_slot":candidates.first(),"candidates":candidates}),
    ))
}
async fn prioritize_inventory(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(priority): Json<crate::ams::Priority>,
) -> Result<StatusCode> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    usable_material(&registry, &entry.device, &priority.filament_id)?;
    entry
        .printer
        .ams_inventory(Some(crate::ams::Change::Priority(priority)))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefillSetting {
    enabled: bool,
}
async fn set_auto_refill(
    State(registry): State<Arc<Registry>>,
    Path(id): Path<String>,
    Json(setting): Json<RefillSetting>,
) -> Result<(StatusCode, Json<Value>)> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    entry.printer.set_auto_refill(setting.enabled).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"status":"requested"}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    async fn call(app: &Router, method: &str, path: &str, body: Value, expected: u16) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            expected,
            "{path}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        }
    }
    #[tokio::test]
    async fn material_search_uses_fuzzy_product_vendor_type_and_color() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let app = router(root.path(), store, None, None).unwrap();
        for (name, vendor, material, color) in [
            ("PLA Matte 黒", "Bambu Lab", "PLA", "000000FF"),
            ("PETG 白", "Other", "PETG", "FFFFFFFF"),
        ] {
            call(&app,"POST","/api/filaments",json!({"name":name,"vendor":vendor,"material":material,"color":color,"bambu_filament_id":null}),201).await;
        }
        for q in ["bmb", "mt", "PLA", "0000"] {
            let found = call(
                &app,
                "GET",
                &format!("/api/filaments?q={q}"),
                Value::Null,
                200,
            )
            .await;
            assert_eq!(found.as_array().unwrap().len(), 1);
            assert_eq!(found[0]["name"], "PLA Matte 黒");
        }
        assert_eq!(
            call(&app, "GET", "/api/filaments?q=unfindable", Value::Null, 200).await,
            json!([])
        );
    }

    #[tokio::test]
    async fn products_share_settings_across_colors_and_preserve_exact_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let app = router(root.path(), store.clone(), None, None).unwrap();
        let data = json!({"name":"PLA Matte","vendor":"Bambu Lab","material":"PLA","bambu_filament_id":"GFA01"});
        let product = call(&app, "POST", "/api/filament-products", data.clone(), 201).await;
        let pid = product["id"].as_str().unwrap();
        let path = format!("/api/filament-products/{pid}");
        let mut colors = vec![];
        for (name, color) in [("黒", "000000FF"), ("白", "FFFFFFFF"), ("黄", "FFFF00FF")] {
            let value = call(
                &app,
                "POST",
                &format!("{path}/colors"),
                json!({"name":name,"color":color}),
                201,
            )
            .await;
            colors.push(value["id"].as_str().unwrap().to_owned());
        }
        let mut setting = crate::filament::Setting {
            id: "shared-setting".into(),
            filament_id: colors[0].clone(),
            data: crate::filament::SettingData {
                machine_profile_key: "machine".into(),
                base_profile_key: "base".into(),
                overrides_json: crate::filament::Overrides {
                    nozzle_temperature: Some(215),
                    ..Default::default()
                },
            },
        };
        store.db.save_setting(&setting).unwrap();
        setting.filament_id = colors[1].clone();
        setting.data.overrides_json.nozzle_temperature = Some(220);
        store.db.save_setting(&setting).unwrap();
        for id in &colors {
            assert_eq!(
                store.db.filament_settings(id).unwrap()[0]
                    .data
                    .overrides_json
                    .nozzle_temperature,
                Some(220)
            );
        }
        let mut tray = crate::printer_state::Tray {
            present: Some(true),
            tag_uid: Some("TAG".into()),
            profile_id: Some("GFA01".into()),
            material: Some("PLA".into()),
            color: Some("000000FF".into()),
            ..Default::default()
        };
        assert_eq!(
            crate::filament::candidates(&tray, &store.db.filaments().unwrap()),
            vec![colors[0].clone()]
        );
        tray.color = Some("161616FF".into());
        assert!(crate::filament::candidates(&tray, &store.db.filaments().unwrap()).is_empty());
        call(
            &app,
            "POST",
            &format!("{path}/colors"),
            json!({"name":"別の黒","color":"000000FF"}),
            201,
        )
        .await;
        tray.color = Some("000000FF".into());
        assert_eq!(
            crate::filament::candidates(&tray, &store.db.filaments().unwrap()).len(),
            2
        );
        call(
            &app,
            "POST",
            &format!("{path}/colors"),
            json!({"name":"bad","color":"red;url(x)"}),
            400,
        )
        .await;
        let mut edited = data;
        edited["vendor"] = json!("Bambu");
        call(&app, "PUT", &path, edited, 200).await;
        assert!(
            store
                .db
                .filaments()
                .unwrap()
                .iter()
                .all(|f| f.data.vendor == "Bambu")
        );
        let saved = call(&app, "GET", &path, Value::Null, 200).await;
        assert_eq!(saved["colors"].as_array().unwrap().len(), 4);
        assert_eq!(
            saved["settings"][0]["overrides_json"]["nozzle_temperature"],
            220
        );
        assert_legacy_color_adoption(&app, &store, &path, &setting).await;
    }
    async fn assert_legacy_color_adoption(
        app: &Router,
        store: &Store,
        path: &str,
        setting: &crate::filament::Setting,
    ) {
        let legacy=call(app,"POST","/api/filaments",json!({"name":"Matte 白（旧登録）","vendor":"Bambu","material":"PLA","color":"FFFFFFFF","bambu_filament_id":"GFA01"}),201).await;
        let fid = legacy["id"].as_str().unwrap();
        let mut other = setting.clone();
        other.id = "old-setting".into();
        other.filament_id = fid.into();
        other.data.overrides_json.nozzle_temperature = Some(225);
        store.db.save_setting(&other).unwrap();
        call(
            app,
            "POST",
            &format!("{path}/adopt"),
            json!({"filament_id":fid}),
            409,
        )
        .await;
        other.data.overrides_json.nozzle_temperature = Some(220);
        store.db.save_setting(&other).unwrap();
        call(
            app,
            "POST",
            &format!("{path}/adopt"),
            json!({"filament_id":fid}),
            204,
        )
        .await;
        assert_eq!(
            store.db.filament_settings(fid).unwrap()[0].id,
            "shared-setting"
        );
        assert!(store.db.filaments().unwrap().iter().any(|f| f.id == fid));
        call(
            app,
            "POST",
            &format!("{path}/adopt"),
            json!({"filament_id":fid}),
            204,
        )
        .await;
    }
    #[test]
    fn explicit_printer_selection_never_falls_back_to_another_device() {
        let keys = vec!["one".to_owned(), "two".to_owned()];
        assert_eq!(select_id(&keys, Some("two")).unwrap(), Some("two"));
        assert!(select_id(&keys, Some("missing")).is_err());
        assert!(select_id(&keys, None).is_err());
        assert_eq!(select_id(&keys[..1], None).unwrap(), Some("one"));
        assert_eq!(select_id(&[], None).unwrap(), None);
        assert!(select_id(&[], Some("one")).is_err());
    }
    #[test]
    fn editing_rules_distinguish_names_from_configuration_and_in_use_deletion() {
        assert!(check_edit(false, true, true).is_ok());
        assert!(check_edit(true, true, false).is_err());
        assert!(check_edit(true, false, true).is_err());
        assert!(check_edit(true, false, false).is_ok());
    }
}
