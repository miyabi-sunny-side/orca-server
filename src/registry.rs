use crate::{
    database::{Database, Device, Settings},
    plates::{Error, Result, Store},
    printer::{Config, Printer, StartRequest},
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
    queue: queue::Service,
    error: Option<&'static str>,
}
struct Registry {
    db: Database,
    // ponytail: one lock protects registry changes and print/queue admission for a handful of printers.
    entries: Mutex<BTreeMap<String, Entry>>,
    fallback: Entry,
    store: Store,
    profiles: Option<Arc<Profiles>>,
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
        let printer = Printer::new(config).map_err(Error::Invalid)?;
        let queue = queue::Service::new(
            self.store.clone(),
            printer.clone(),
            device.settings.machine_profile_key.clone(),
        )?;
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
        self.queue.in_use().await
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
pub fn router(root: &FsPath, store: Store, slicer: Option<Slicer>) -> Result<Router> {
    let db = Database::open(root, || {
        Config::from_env()
            .map_err(Error::Invalid)?
            .map(|config| {
                config.import().map(|settings| Device {
                    id: "p1".into(),
                    settings,
                })
            })
            .transpose()
    })?;
    let disabled = Printer::new(None).map_err(Error::Invalid)?;
    let fallback = Entry {
        device: Device {
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
        },
        queue: queue::Service::new(store.clone(), disabled.clone(), PRINTER.into())?,
        printer: disabled,
        error: None,
    };
    let mut registry = Registry {
        db,
        entries: Mutex::new(BTreeMap::new()),
        fallback,
        store,
        profiles: slicer.map(|s| s.profiles),
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
        .route("/api/printer/status", get(status))
        .route(
            "/api/plates/{id}/print",
            post(start).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/printer/start/{id}/resolve",
            post(resolve).layer(DefaultBodyLimit::max(4096)),
        )
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
        if registry.fallback.queue.in_use().await {
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
        check_edit(old.in_use().await, changed, false)?;
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
async fn start(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
    Path(id): Path<String>,
    Json(request): Json<StartRequest>,
) -> Result<(StatusCode, Json<crate::print_start::Attempt>)> {
    let entries = registry.entries.lock().await;
    let entry = registry.selected(&entries, &query)?;
    entry.usable()?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            entry
                .printer
                .start(registry.store.clone(), id, request)
                .await?,
        ),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolution {
    checked_printer: bool,
}
async fn resolve(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
    Path(id): Path<String>,
    Json(request): Json<Resolution>,
) -> Result<Json<crate::print_start::Attempt>> {
    let entries = registry.entries.lock().await;
    Ok(Json(
        registry
            .selected(&entries, &query)?
            .printer
            .resolve(&id, request.checked_printer)
            .await?,
    ))
}
async fn read_queue(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
) -> Result<Json<Value>> {
    let entries = registry.entries.lock().await;
    let entry = registry.selected(&entries, &query)?;
    entry.usable()?;
    Ok(Json(entry.queue.read().await))
}
async fn act_queue(
    State(registry): State<Arc<Registry>>,
    Query(query): Query<Selected>,
    Json(request): Json<queue::Command>,
) -> Result<Json<Value>> {
    tokio::spawn(async move {
        let entries = registry.entries.lock().await;
        let entry = registry.selected(&entries, &query)?;
        entry.usable()?;
        entry.queue.apply(request).await.map(Json)
    })
    .await
    .map_err(|_| Error::Unavailable("Queue request interrupted; refresh the queue"))?
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
