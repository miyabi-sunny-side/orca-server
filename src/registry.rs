use crate::{
    database::{Database, Device, Settings},
    plates::{Error, Result, Store},
    printer::{Config, Printer},
    profiles::{PRINTER, Profiles},
    queue,
    slicer::Slicer,
};
use axum::{
    Json, Router,
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
struct Registry {
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

/// Open the persistent printer registry and reuse the Bambu LAN print paths per device.
/// # Errors
/// Rejects unreadable storage, unsupported schema versions or invalid initial environment settings.
pub fn router(
    _root: &FsPath,
    store: Store,
    slicer: Option<Slicer>,
    source: Option<crate::scad::Source>,
) -> Result<Router> {
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
    Ok(Router::new()
        .route("/api/printers", get(list).post(create))
        .route("/api/printers/profiles", get(machines))
        .route("/api/printers/{id}", get(read).put(update).delete(delete))
        .route("/api/filaments", get(materials).post(create_material))
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
        .with_state(Arc::new(registry)))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selected {
    printer_id: Option<String>,
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
        let changed = comparison != old.device.settings;
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
        settings.validate()?;
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
    queue.read().await.map(Json)
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

async fn materials(
    State(registry): State<Arc<Registry>>,
) -> Result<Json<Vec<crate::filament::Filament>>> {
    let db = registry.db.clone();
    Ok(Json(
        crate::plate_api::blocking(move || db.filaments()).await?,
    ))
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
    Ok(Json(json!({"filament":f,"settings":settings})))
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
    let profiles = registry
        .profiles
        .as_ref()
        .ok_or(Error::Unavailable("OrcaSlicer profiles unavailable"))?;
    let choices = profiles.choices_for(&query.machine)?;
    let compatible: Vec<_> = choices["filaments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|key| {
            let key = key.as_str()?;
            let setting = crate::filament::SettingData {
                machine_profile_key: query.machine.clone(),
                base_profile_key: key.into(),
                overrides_json: crate::filament::Overrides::default(),
            };
            let p = profiles.resolve_filament(&setting, &f.data.material).ok()?;
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
    let db = registry.db.clone();
    let details = crate::plate_api::blocking(move || {
        let mut details = BTreeMap::new();
        for f in db.filaments()? {
            let settings = db.filament_settings(&f.id)?;
            details.insert(f.id.clone(), (f, settings));
        }
        Ok(details)
    })
    .await?;
    let slots: Vec<_> = slots
        .into_iter()
        .map(|slot| {
            let mut value = json!(slot);
            value["current"] = json!(current && slot.reported.present.is_some());
            value["nozzle_fit"] = json!("unknown");
            value["setting"] = Value::Null;
            if let Some((f, settings)) = slot.filament_id.as_ref().and_then(|id| details.get(id)) {
                value["filament"] = json!(f);
                if let Some(s) = settings.iter().find(|s| {
                    s.data.machine_profile_key == entry.device.settings.machine_profile_key
                }) {
                    value["setting"] = json!(s);
                    if let Some(p) = &registry.profiles
                        && let Ok(resolved) = p.resolve_filament(&s.data, &f.data.material)
                    {
                        value["setting"]["resolved"] = temperature_view(&resolved);
                        let diameter = p.machine(&s.data.machine_profile_key).ok().and_then(|m| {
                            m.get("nozzle_diameter")?
                                .get(0)?
                                .as_str()
                                .map(str::to_owned)
                        });
                        let hrc = resolved
                            .get("required_nozzle_HRC")
                            .and_then(|v| v.get(0))
                            .and_then(Value::as_str)
                            .and_then(|v| v.parse().ok());
                        value["nozzle_fit"] = json!(crate::filament::nozzle_fit(
                            &f.data.material,
                            diameter.as_deref().unwrap_or(""),
                            &entry.device.settings.nozzle_material,
                            hrc
                        ));
                    }
                }
            } else {
                value["filament"] = Value::Null;
            }
            value
        })
        .collect();
    Ok(Json(
        json!({"printer_id":id,"current":current,"slots":slots}),
    ))
}
async fn map_inventory(
    State(registry): State<Arc<Registry>>,
    Path((id, slot)): Path<(String, String)>,
    Json(mapping): Json<crate::ams::Mapping>,
) -> Result<StatusCode> {
    let entries = registry.entries.lock().await;
    let entry = entries.get(&id).ok_or(Error::NotFound)?;
    entry.printer.ams_inventory(Some((slot, mapping))).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
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
