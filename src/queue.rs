use crate::{
    database::{Database, Device},
    plates::{Error, Result, Store},
    print_start::{Attempt, Phase},
    printer::Printer,
    printer_state::Status,
    profiles::{Profiles, Selection},
    scad::Source,
    slicer::Slicer,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Specification {
    pub ams_slot_id: String,
    pub filament_id: String,
    pub required_machine_profile_key: String,
    pub process_profile_key: String,
    pub bed_type: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Job {
    pub id: String,
    pub plate_id: String,
    pub name: String,
    #[serde(flatten)]
    pub specification: Specification,
    pub state: String,
    pub attempt_id: Option<String>,
    pub artifact_path: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    epoch: String,
    generation: i64,
    request_id: String,
    action: Action,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Add {
        plate_id: String,
        plate_version: i64,
    },
    Reestimate {
        job_id: String,
    },
    Move {
        job_id: String,
        index: usize,
    },
    Remove {
        job_id: String,
    },
    Next {
        expected_job: String,
        removed_job: Option<String>,
        cleared: bool,
    },
    Retry {
        expected_job: String,
        cleared: bool,
    },
    Discard {
        expected_job: String,
        cleared: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Execution {
    pub plate: crate::plates::Plate,
    // Only an explicit recovery may reuse this observed FAILED attempt during preparation/transfer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_attempt: Option<String>,
    #[serde(flatten)]
    pub settings: Resolved,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Binding {
    pub filament: crate::filament::Filament,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting: Option<crate::filament::SettingData>,
    #[serde(default)]
    pub ams_slot_id: String,
    pub slot_revision: i64,
    pub ams_slot: u8,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Resolved {
    pub selection: Selection,
    pub profiles: BTreeMap<String, serde_json::Map<String, Value>>,
    #[serde(flatten)]
    pub main: Binding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<Binding>,
}
impl Resolved {
    pub(crate) fn filament_profiles(&self) -> Vec<serde_json::Map<String, Value>> {
        ["filament.json", "interface.json"]
            .into_iter()
            .filter_map(|key| self.profiles.get(key).cloned())
            .collect()
    }
    fn check_bindings(
        &mut self,
        c: &Connection,
        device: &Device,
        status: &Status,
        refresh: bool,
    ) -> Result<()> {
        let machine: String = c.query_row(
            "SELECT machine_profile_key FROM printers WHERE id=?1",
            [&device.id],
            |r| r.get(0),
        )?;
        if !status.synchronized
            || machine != self.selection.machine
            || machine != device.settings.machine_profile_key
        {
            return Err(Error::Conflict(
                "Printer or nozzle changed during preparation",
            ));
        }
        let diameter = self.profiles["printer.json"]["nozzle_diameter"][0]
            .as_str()
            .ok_or(Error::Invalid("Invalid nozzle profile"))?;
        crate::print_start::check_nozzle(status, diameter, &device.settings.nozzle_material)?;
        let filaments = crate::database::load_filaments(c)?;
        for binding in std::iter::once(&mut self.main).chain(self.interface.iter_mut()) {
            let slot = crate::ams::resolve(c, &device.id, &binding.filament.id, &machine)?
                .into_iter()
                .find(|s| s.id == binding.ams_slot_id && s.ams_id < 4)
                .ok_or(Error::Conflict(
                    "Selected AMS slot does not contain the planned material",
                ))?;
            let number = slot.ams_id * 4 + slot.slot_index;
            if number != binding.ams_slot
                || (!refresh && slot.revision != binding.slot_revision)
                || filaments.iter().find(|f| f.id == binding.filament.id) != Some(&binding.filament)
                || binding.setting.as_ref().is_some_and(|setting| {
                    crate::products::load_setting(c, &binding.filament.id, &machine)
                        .as_ref()
                        .ok()
                        != Some(setting)
                })
            {
                return Err(Error::Conflict(
                    "AMS assignment or material settings changed during preparation",
                ));
            }
            let tray = status
                .ams
                .as_ref()
                .and_then(|a| a.units.iter().find(|u| u.id == number / 4))
                .and_then(|u| u.trays.iter().find(|t| t.id == number % 4));
            if tray.is_none_or(|t| t.present != Some(true)) {
                return Err(Error::Conflict(
                    "Selected AMS slot is not confirmed present",
                ));
            }
            if refresh {
                binding.slot_revision = slot.revision;
            }
        }
        Ok(())
    }
}
fn binding(
    c: &Connection,
    pid: &str,
    filament_id: &str,
    machine: &str,
    slot_id: Option<&str>,
) -> Result<Binding> {
    let slot = crate::ams::resolve(c, pid, filament_id, machine)?
        .into_iter()
        .find(|s| s.ams_id < 4 && slot_id.is_none_or(|id| id == s.id))
        .ok_or(Error::Conflict(
            "No confirmed AMS slot contains the selected material",
        ))?;
    let filament = crate::database::load_filaments(c)?
        .into_iter()
        .find(|f| f.id == filament_id)
        .ok_or(Error::NotFound)?;
    let setting = Some(crate::products::load_setting(c, filament_id, machine)?);
    Ok(Binding {
        filament,
        setting,
        ams_slot_id: slot.id,
        slot_revision: slot.revision,
        ams_slot: slot.ams_id * 4 + slot.slot_index,
    })
}
fn recovery_state(c: &Connection, job: &Job, status: &Status) -> Result<Option<String>> {
    let has_identity = [&status.print.name, &status.print.file]
        .into_iter()
        .any(|s| s.as_ref().is_some_and(|v| !v.is_empty()));
    if status.ready_to_print && !has_identity {
        return Ok(None);
    }
    let id = job.attempt_id.as_deref().unwrap_or("");
    let mut target = id.to_owned();
    if !status.matches_attempt(id) {
        let (execution, attempt): (Option<String>, Option<String>) = c.query_row(
            "SELECT execution_json,attempt_json FROM print_jobs WHERE id=?1",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let attempt: Option<Attempt> = attempt
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(std::io::Error::other)?;
        let execution: Option<Execution> = execution
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(std::io::Error::other)?;
        // An unsent or explicitly rejected attempt can retain the previous stop; an uncertain send cannot.
        if attempt
            .as_ref()
            .is_none_or(|a| !a.was_sent() || a.phase == Phase::Rejected)
        {
            match execution.and_then(|e| e.stopped_attempt) {
                Some(previous) if status.matches_attempt(&previous) => target = previous,
                // An unsent/rejected start can leave the preceding completed job's name on the printer.
                None if status.ready_to_print => return Ok(None),
                _ => {}
            }
        }
    }
    if status.ready_to_print {
        if !status.matches_attempt(&target) {
            return Err(Error::Conflict(
                "Printer report does not match the recovery target",
            ));
        }
        return Ok(None);
    }
    status.check_stopped(&target)?;
    Ok(Some(target))
}

fn retry_execution(
    c: &Connection,
    device: &Device,
    job: &Job,
    status: &Status,
) -> Result<Execution> {
    let stopped_attempt = recovery_state(c, job, status)?;
    let raw: String = c.query_row(
        "SELECT execution_json FROM print_jobs WHERE id=?1",
        [&job.id],
        |r| r.get(0),
    )?;
    let mut execution: Execution = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
    execution.stopped_attempt = stopped_attempt;
    if execution.settings.main.ams_slot_id.is_empty() {
        execution
            .settings
            .main
            .ams_slot_id
            .clone_from(&job.specification.ams_slot_id);
    }
    // Explicit retry refreshes revisions of the same slots; material IDs and slice inputs remain frozen.
    execution.settings.check_bindings(c, device, status, true)?;
    Ok(execution)
}
struct Preparation {
    job: Job,
    execution: Execution,
    originals: Vec<Option<Vec<u8>>>,
}

fn jobs(c: &Connection, pid: &str) -> Result<Vec<Job>> {
    Ok(c.prepare("SELECT id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,attempt_id,artifact_path,last_error FROM print_jobs WHERE printer_id=?1 AND state NOT IN ('completed','cancelled') ORDER BY position,id")?
        .query_map([pid],|r|Ok(Job{id:r.get(0)?,plate_id:r.get(1)?,name:r.get(2)?,specification:Specification{ams_slot_id:r.get(3)?,filament_id:r.get(4)?,required_machine_profile_key:r.get(5)?,process_profile_key:r.get(6)?,bed_type:r.get(7)?},state:r.get(8)?,attempt_id:r.get(9)?,artifact_path:r.get(10)?,last_error:r.get(11)?}))?
        .collect::<std::result::Result<_,_>>()?)
}
fn generation(c: &Connection, pid: &str) -> Result<i64> {
    Ok(c.query_row(
        "SELECT queue_generation FROM printers WHERE id=?1",
        [pid],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}
fn check_command(
    epoch: &str,
    generation: i64,
    last: Option<&Command>,
    request: &Command,
) -> Result<bool> {
    if request.epoch != epoch {
        return Err(Error::Conflict(
            "Server restarted or printer settings changed; reload and inspect the queue",
        ));
    }
    if uuid::Uuid::parse_str(&request.request_id).is_err() {
        return Err(Error::Invalid("request_id must be a UUID"));
    }
    if last.is_some_and(|l| l.request_id == request.request_id) {
        return if last == Some(request) {
            Ok(true)
        } else {
            Err(Error::Conflict(
                "Request ID was already used with different input",
            ))
        };
    }
    if request.generation != generation {
        return Err(Error::Conflict("Queue changed; reload before operating"));
    }
    Ok(false)
}
fn record(c: &Connection, pid: &str, command: &Command) -> Result<()> {
    c.execute(
        "UPDATE printers SET queue_generation=queue_generation+1,queue_request=?1 WHERE id=?2",
        params![
            serde_json::to_string(command).map_err(std::io::Error::other)?,
            pid
        ],
    )?;
    Ok(())
}
pub(crate) fn resolve(
    c: &Connection,
    pid: &str,
    s: &Specification,
    profiles: &Profiles,
    plate: &crate::plates::Plate,
) -> Result<Resolved> {
    plate.ensure_printable()?;
    let main = binding(
        c,
        pid,
        &s.filament_id,
        &s.required_machine_profile_key,
        Some(&s.ams_slot_id),
    )?;
    let setting = main.setting.as_ref().expect("resolved setting");
    let selection = Selection {
        machine: s.required_machine_profile_key.clone(),
        process: s.process_profile_key.clone(),
        filament: setting.base_profile_key.clone(),
        bed: s.bed_type.clone(),
    };
    let mut resolved = profiles.resolved(&selection)?;
    let primary = crate::support::material_profile(
        profiles.resolve_filament(setting, &main.filament.data.material)?,
        &main.filament,
        None,
    )?;
    crate::profiles::validate_bed(&primary, &selection.bed)?;
    let interface = if let Some(id) = plate
        .conditions
        .interface_id()
        .filter(|id| *id != main.filament.id)
    {
        let secondary =
            binding(c, pid, id, &selection.machine, None).map_err(|error| match error {
                Error::Conflict(_) => {
                    Error::Conflict("No confirmed AMS slot contains the support interface material")
                }
                other => other,
            })?;
        let profile = profiles.resolve_filament(
            secondary.setting.as_ref().expect("resolved setting"),
            &secondary.filament.data.material,
        )?;
        resolved.insert(
            "interface.json".into(),
            crate::support::material_profile(profile, &secondary.filament, Some(&primary))?,
        );
        Some(secondary)
    } else {
        None
    };
    resolved.insert("filament.json".into(), primary);
    plate.conditions.apply(
        resolved
            .get_mut("process.json")
            .ok_or(Error::Invalid("Resolved process is missing"))?,
    )?;
    Ok(Resolved {
        selection,
        profiles: resolved,
        main,
        interface,
    })
}
fn available(
    c: &Connection,
    device: &Device,
    s: &Specification,
    plate: &crate::plates::Plate,
    status: &Status,
    profiles: &Profiles,
) -> Result<Resolved> {
    let mut resolved = resolve(c, &device.id, s, profiles, plate)?;
    resolved.check_bindings(c, device, status, false)?;
    Ok(resolved)
}
fn ready(
    c: &Connection,
    device: &Device,
    s: &Specification,
    plate: &crate::plates::Plate,
    status: &Status,
    profiles: &Profiles,
) -> Result<Resolved> {
    if !status.ready_to_print {
        return Err(Error::Conflict("Wait for a current, ready printer report"));
    }
    available(c, device, s, plate, status, profiles)
}
pub(crate) fn planned(
    c: &Connection,
    pid: &str,
    plate: &crate::plates::Plate,
) -> Result<Specification> {
    plate.ensure_printable()?;
    let condition = &plate.conditions;
    let missing =
        || Error::Conflict("Complete the plate machine, material, process and bed conditions");
    let machine = condition
        .required_machine_profile_key
        .as_ref()
        .ok_or_else(missing)?;
    let filament = condition.filament_id.as_ref().ok_or_else(missing)?;
    let process = condition.process_profile_key.as_ref().ok_or_else(missing)?;
    let bed = condition.bed_type.as_ref().ok_or_else(missing)?;
    let registered: String = c.query_row(
        "SELECT machine_profile_key FROM printers WHERE id=?1",
        [pid],
        |r| r.get(0),
    )?;
    if machine != &registered {
        return Err(Error::Conflict(
            "Required machine or nozzle differs from the registered configuration",
        ));
    }
    let slot = crate::ams::resolve(c, pid, filament, machine)?
        .into_iter()
        .find(|s| s.ams_id < 4)
        .ok_or(Error::Conflict(
            "No confirmed AMS slot contains the plate material",
        ))?;
    Ok(Specification {
        ams_slot_id: slot.id,
        filament_id: filament.clone(),
        required_machine_profile_key: machine.clone(),
        process_profile_key: process.clone(),
        bed_type: bed.clone(),
    })
}
pub(crate) fn message(error: &Error) -> String {
    match error {
        Error::Slicer(message) => message.clone(),
        Error::Invalid(s) | Error::Unavailable(s) | Error::Conflict(s) | Error::Upstream(s) => {
            (*s).into()
        }
        Error::Timeout => "Slicing timed out".into(),
        _ => "Preparation failed; check source data and server storage".into(),
    }
}
fn directory(store: &Store, job: &str, attempt: &str) -> Result<PathBuf> {
    for id in [job, attempt] {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(Error::Invalid("Invalid execution ID"));
        }
    }
    Ok(store.root.join("jobs").join(job).join(attempt))
}

pub(crate) fn originals(
    c: &Connection,
    plate: &crate::plates::Plate,
    cached: Option<&std::path::Path>,
) -> Result<Vec<Option<Vec<u8>>>> {
    let mut originals = Vec::new();
    let mut index = 0;
    for model in &plate.models {
        let cached = cached
            .map(|path| path.join(format!("{index}.stl")))
            .filter(|path| path.is_file())
            .map(std::fs::read)
            .transpose()?;
        let original = if cached.is_some() {
            cached
        } else if model.source.is_none() {
            c.query_row(
                "SELECT original FROM plate_items WHERE plate_id=?1 AND id=?2",
                params![plate.id, model.id],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten()
            .ok_or(Error::Conflict(
                "Original model for this attempt is unavailable",
            ))?
            .into()
        } else {
            None
        };
        originals.push(original);
        index += usize::from(model.quantity);
    }
    Ok(originals)
}

pub(crate) async fn write_inputs(
    path: &std::path::Path,
    plate: &crate::plates::Plate,
    settings: &Resolved,
    originals: Vec<Option<Vec<u8>>>,
    provider: Option<&Source>,
) -> Result<usize> {
    let mut count = 0;
    let mut remaining = crate::plates::MAX_UPLOAD;
    for (model, original) in plate.models.iter().zip(originals) {
        let bytes = if let Some(bytes) = original {
            bytes
        } else if let Some(source) = &model.source {
            provider
                .ok_or(Error::Unavailable("SCAD_LIVE_URL is not configured"))?
                .model(source, remaining)
                .await?
        } else {
            return Err(Error::Unavailable("Uploaded original is unavailable"));
        };
        let size = bytes
            .len()
            .checked_mul(usize::from(model.quantity))
            .ok_or(Error::Invalid("Models exceed 64 MiB"))?;
        remaining = remaining
            .checked_sub(size)
            .ok_or(Error::Invalid("Models exceed 64 MiB"))?;
        for _ in 0..model.quantity {
            std::fs::write(path.join(format!("{count}.stl")), &bytes)?;
            count += 1;
        }
    }
    for (name, profile) in &settings.profiles {
        serde_json::to_writer(std::fs::File::create(path.join(name))?, profile)
            .map_err(std::io::Error::other)?;
    }
    Ok(count)
}

pub(crate) fn sync_files(path: &std::path::Path) -> Result<()> {
    // Flush immutable inputs and outputs before persisting the upload/start attempt.
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::File::open(entry.path())?.sync_all()?;
        }
    }
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

pub(crate) struct Service {
    pub(crate) lock: Mutex<()>,
    epoch: String,
    store: Store,
    device: Device,
    printer: Printer,
    slicer: Option<Slicer>,
    source: Option<Source>,
}
impl Service {
    pub(crate) fn new(
        store: Store,
        device: Device,
        printer: Printer,
        slicer: Option<Slicer>,
        source: Option<Source>,
    ) -> Self {
        Self {
            lock: Mutex::new(()),
            epoch: uuid::Uuid::new_v4().to_string(),
            store,
            device,
            printer,
            slicer,
            source,
        }
    }
    pub(crate) fn in_use(&self) -> bool {
        self.store
            .db
            .connection()
            .and_then(|c| jobs(&c, &self.device.id))
            .map_or(true, |j| !j.is_empty())
    }
    pub(crate) fn active(&self) -> bool {
        self.store
            .db
            .connection()
            .and_then(|c| jobs(&c, &self.device.id))
            .map_or(true, |j| j.iter().any(|j| j.state != "queued"))
    }
    pub(crate) async fn read(&self, plate_id: Option<&str>) -> Result<Value> {
        let (_observation, status) = self.printer.observed_status().await?;
        let c = self.store.db.connection()?;
        let list = jobs(&c, &self.device.id)?;
        let current = list.iter().find(|j| j.state != "queued");
        let mut waiting = Vec::new();
        for job in list.iter().filter(|j| j.state == "queued") {
            let mut value = json!(job);
            value["estimate"] =
                crate::estimates::view(&c, job, self.slicer.as_ref().map(|s| s.profiles.as_ref()))?;
            let plate = crate::plates::load(&c, &job.plate_id)?;
            value["name"] = json!(plate.name);
            value["plate_version"] = json!(plate.version);
            value["plate_deleted"] = json!(crate::plates::is_deleted(&c, &job.plate_id)?);
            for key in [
                "required_machine_profile_key",
                "filament_id",
                "process_profile_key",
                "bed_type",
            ] {
                value[key] = json!(plate.conditions)[key].clone();
            }
            value["ams_slot_id"] = Value::Null;
            let hold = self.plan(&c, &plate, &status).map(|spec| {
                value["ams_slot_id"] = json!(spec.ams_slot_id);
            });
            value["hold_reason"] = hold.err().map_or(Value::Null, |e| json!(message(&e)));
            waiting.push(value);
        }
        let idle = status.ready_to_print;
        let next = idle
            && current.is_none_or(|j| j.state == "awaiting_removal")
            && waiting.first().is_some_and(|j| j["hold_reason"].is_null());
        let retry = current
            .filter(|j| j.state == "needs_attention")
            .map(|j| retry_execution(&c, &self.device, j, &status));
        let discard = current
            .filter(|j| matches!(j.state.as_str(), "needs_attention" | "awaiting_removal"))
            .map(|j| recovery_state(&c, j, &status));
        let recovery = json!({
            "retry_reason": retry.as_ref().and_then(|r| r.as_ref().err()).map(message),
            "discard_reason": discard.as_ref().and_then(|r| r.as_ref().err()).map(message),
        });
        let retry = retry.is_some_and(|r| r.is_ok());
        let discard = discard.is_some_and(|r| r.is_ok());
        let admission = plate_id.map(|id| -> Result<Value> {
            let plate = crate::plates::load(&c, id)?;
            let result = self.admission(&c, &plate, &status);
            Ok(json!({"plate_version":plate.version,"allowed":result.is_ok(),"reason":result.err().map(|e|message(&e))}))
        }).transpose()?;
        let current_view = current
            .map(|job| -> Result<Value> {
                let mut value = json!(job);
                value["plate_deleted"] = json!(crate::plates::is_deleted(&c, &job.plate_id)?);
                value["estimate"] = crate::estimates::view(
                    &c,
                    job,
                    self.slicer.as_ref().map(|s| s.profiles.as_ref()),
                )?;
                value["actual_ams_slot"] = if job.state == "printing" && status.synchronized {
                    json!(
                        status
                            .ams
                            .as_ref()
                            .and_then(|ams| ams.current_tray)
                            .filter(|n| *n < 16)
                    )
                } else {
                    Value::Null
                };
                Ok(value)
            })
            .transpose()?;
        Ok(
            json!({"epoch":self.epoch,"generation":generation(&c,&self.device.id)?,"request_id":uuid::Uuid::new_v4().to_string(),"current":current_view,"admission":admission,"waiting":waiting,"printer":status,"recovery":recovery,"allowed":{"next":next,"retry":retry,"discard":discard}}),
        )
    }
    pub(crate) async fn apply(self: &Arc<Self>, command: Command) -> Result<Value> {
        let _guard = self.lock.lock().await;
        let preparation = {
            let (_observation, status) = self.printer.observed_status().await?;
            self.mutate(&command, &status)?
        };
        self.printer.forget_retired().await?;
        if let Some(preparation) = preparation {
            let service = self.clone();
            tokio::spawn(async move {
                let id = preparation.job.id.clone();
                let attempt = preparation
                    .job
                    .attempt_id
                    .clone()
                    .expect("reserved attempt");
                if let Err(error) = service.prepare(preparation).await {
                    let _ = service.store.db.fail_job(&id, &attempt, &message(&error));
                }
            });
        }
        self.cleanup()?;
        self.read(None).await
    }
    fn check_request(&self, c: &Connection, command: &Command) -> Result<bool> {
        // A replaced registry entry must not admit work with old machine/nozzle settings.
        let current_machine: String = c
            .query_row(
                "SELECT machine_profile_key FROM printers WHERE id=?1",
                [&self.device.id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if current_machine != self.device.settings.machine_profile_key {
            return Err(Error::Conflict(
                "Printer settings changed; reload the queue",
            ));
        }
        let last: Option<String> = c.query_row(
            "SELECT queue_request FROM printers WHERE id=?1",
            [&self.device.id],
            |r| r.get(0),
        )?;
        let last = last
            .map(|s| serde_json::from_str::<Command>(&s))
            .transpose()
            .map_err(std::io::Error::other)?;
        if check_command(
            &self.epoch,
            generation(c, &self.device.id)?,
            last.as_ref(),
            command,
        )? {
            return Ok(true);
        }
        Ok(false)
    }
    fn mutate(&self, command: &Command, status: &Status) -> Result<Option<Preparation>> {
        let mut c = self.store.db.connection()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if self.check_request(&tx, command)? {
            return Ok(None);
        }
        let all = jobs(&tx, &self.device.id)?;
        let waiting: Vec<_> = all.iter().filter(|j| j.state == "queued").collect();
        let current = all.iter().find(|j| j.state != "queued");
        if self.change_waiting(&tx, &command.action, &waiting, status)? {
            record(&tx, &self.device.id, command)?;
            tx.commit()?;
            return Ok(None);
        }
        let mut preparation = None;
        match &command.action {
            Action::Next {
                expected_job,
                removed_job,
                cleared,
            } => {
                if !cleared
                    || !status.ready_to_print
                    || current.is_some_and(|j| j.state != "awaiting_removal")
                    || current.map(|j| &j.id) != removed_job.as_ref()
                    || waiting.first().map(|j| &j.id) != Some(expected_job)
                {
                    return Err(Error::Conflict(
                        "Queue or removal target changed; inspect the printer and confirm again",
                    ));
                }
                // Validate before completing the previous job: a held next job stays editable.
                let job = waiting[0];
                let prep = self.reserve(&tx, job, status, current.map(|j| j.id.as_str()))?;
                preparation = Some(prep);
            }
            Action::Retry {
                expected_job,
                cleared,
            } => {
                let job = current
                    .filter(|j| j.id == *expected_job && j.state == "needs_attention")
                    .ok_or(Error::Conflict("Recovery target changed"))?;
                if !cleared {
                    return Err(Error::Conflict(
                        "Inspect the printer and empty plate before retrying",
                    ));
                }
                preparation = Some(self.reserve(&tx, job, status, None)?);
            }
            Action::Discard {
                expected_job,
                cleared,
            } => {
                let job = current
                    .filter(|j| {
                        j.id == *expected_job
                            && matches!(j.state.as_str(), "awaiting_removal" | "needs_attention")
                    })
                    .ok_or(Error::Conflict("Removal target changed"))?;
                if !cleared {
                    return Err(Error::Conflict(
                        "Inspect the printer and empty plate before clearing this job",
                    ));
                }
                recovery_state(&tx, job, status)?;
                tx.execute(
                    "UPDATE print_jobs SET state=?1 WHERE id=?2",
                    params![
                        if job.state == "awaiting_removal" {
                            "completed"
                        } else {
                            "cancelled"
                        },
                        job.id
                    ],
                )?;
            }
            _ => unreachable!("waiting actions handled above"),
        }
        record(&tx, &self.device.id, command)?;
        tx.commit()?;
        Ok(preparation)
    }
    fn change_waiting(
        &self,
        tx: &Connection,
        action: &Action,
        waiting: &[&Job],
        status: &Status,
    ) -> Result<bool> {
        let selected = |id: &str| {
            waiting
                .iter()
                .copied()
                .find(|j| j.id == id)
                .ok_or(Error::Conflict("Waiting job changed; reload the queue"))
        };
        match action {
            Action::Add {
                plate_id,
                plate_version,
            } => {
                let plate = crate::plates::load(tx, plate_id)?;
                if plate.version != *plate_version {
                    return Err(Error::Conflict("Plate changed; reload before adding"));
                }
                let s = self.admission(tx, &plate, status)?;
                tx.execute("INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',coalesce((SELECT max(position)+1 FROM print_jobs WHERE printer_id=?2),0))",params![uuid::Uuid::new_v4().to_string(),self.device.id,plate_id,plate.name,s.ams_slot_id,s.filament_id,s.required_machine_profile_key,s.process_profile_key,s.bed_type])?;
            }
            Action::Reestimate { job_id } => {
                selected(job_id)?;
                crate::estimates::retry(tx, &self.store, job_id)?;
            }
            Action::Move { job_id, index } => {
                selected(job_id)?;
                if *index >= waiting.len() {
                    return Err(Error::Invalid("Queue index is out of range"));
                }
                let mut ids: Vec<_> = waiting
                    .iter()
                    .map(|j| j.id.as_str())
                    .filter(|id| *id != job_id)
                    .collect();
                ids.insert(*index, job_id);
                for (position, id) in ids.iter().enumerate() {
                    tx.execute(
                        "UPDATE print_jobs SET position=?1 WHERE id=?2",
                        params![i64::try_from(position).expect("100 jobs"), id],
                    )?;
                }
            }
            Action::Remove { job_id } => {
                selected(job_id)?;
                tx.execute(
                    "UPDATE print_jobs SET state='cancelled' WHERE id=?1",
                    [job_id],
                )?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn admission(
        &self,
        c: &Connection,
        plate: &crate::plates::Plate,
        status: &Status,
    ) -> Result<Specification> {
        if crate::plates::is_deleted(c, &plate.id)? {
            return Err(Error::Conflict("Plate has been deleted"));
        }
        let count: i64 = c.query_row(
            "SELECT count(*) FROM print_jobs WHERE printer_id=?1 AND state='queued'",
            [&self.device.id],
            |r| r.get(0),
        )?;
        if count >= 100 {
            return Err(Error::Conflict("Queue holds at most 100 waiting jobs"));
        }
        self.plan(c, plate, status)
    }
    fn plan(
        &self,
        c: &Connection,
        plate: &crate::plates::Plate,
        status: &Status,
    ) -> Result<Specification> {
        let slicer = self
            .slicer
            .as_ref()
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?;
        let specification = planned(c, &self.device.id, plate)?;
        available(
            c,
            &self.device,
            &specification,
            plate,
            status,
            &slicer.profiles,
        )?;
        Ok(specification)
    }
    fn reserve(
        &self,
        c: &Connection,
        job: &Job,
        status: &Status,
        completed: Option<&str>,
    ) -> Result<Preparation> {
        let slicer = self
            .slicer
            .as_ref()
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?;
        let mut job = job.clone();
        let execution = if job.state == "queued" {
            let plate = crate::plates::load(c, &job.plate_id)?;
            job.specification = planned(c, &self.device.id, &plate)?;
            let settings = ready(
                c,
                &self.device,
                &job.specification,
                &plate,
                status,
                &slicer.profiles,
            )?;
            Execution {
                plate,
                settings,
                stopped_attempt: None,
            }
        } else {
            retry_execution(c, &self.device, &job, status)?
        };
        let cached = job
            .attempt_id
            .as_ref()
            .map(|id| directory(&self.store, &job.id, id))
            .transpose()?;
        let originals = originals(c, &execution.plate, cached.as_deref())?;
        job.state = "preparing".into();
        job.name.clone_from(&execution.plate.name);
        let attempt = uuid::Uuid::new_v4().to_string();
        job.attempt_id = Some(attempt.clone());
        job.artifact_path = Some(format!("jobs/{}/{attempt}", job.id));
        job.last_error = None;
        if let Some(id) = completed {
            c.execute("UPDATE print_jobs SET state='completed' WHERE id=?1", [id])?;
        }
        c.execute("UPDATE print_jobs SET name=?1,state='preparing',attempt_id=?2,artifact_path=?3,execution_json=?4,attempt_json=NULL,last_error=NULL,ams_slot_id=?6,filament_id=?7,required_machine_profile_key=?8,process_profile_key=?9,bed_type=?10 WHERE id=?5",params![job.name,attempt,job.artifact_path,serde_json::to_string(&execution).map_err(std::io::Error::other)?,job.id,job.specification.ams_slot_id,job.specification.filament_id,job.specification.required_machine_profile_key,job.specification.process_profile_key,job.specification.bed_type])?;
        Ok(Preparation {
            job,
            execution,
            originals,
        })
    }
    async fn prepare(&self, preparation: Preparation) -> Result<()> {
        let Preparation {
            job,
            execution,
            originals,
        } = preparation;
        let attempt_id = job.attempt_id.as_deref().expect("reserved attempt");
        let path = directory(&self.store, &job.id, attempt_id)?;
        std::fs::create_dir_all(&path)?;
        let count = write_inputs(
            &path,
            &execution.plate,
            &execution.settings,
            originals,
            self.source.as_ref(),
        )
        .await?;
        if !crate::estimates::reuse_for(
            &self.store,
            &job.id,
            &execution.plate,
            &execution.settings,
            &path,
            count,
        ) {
            self.slicer
                .as_ref()
                .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
                .slice(
                    path.clone(),
                    count,
                    execution.settings.selection.clone(),
                    execution.settings.filament_profiles(),
                    &job.id,
                )
                .await?;
        }
        crate::estimates::actual(
            &self.store,
            &job,
            &execution.plate,
            &execution.settings,
            &path,
        )?;
        let bytes = std::fs::read(path.join("print.gcode.3mf"))?;
        let materials = crate::print_start::materials_for(
            &bytes,
            &execution.settings.selection.machine,
            &execution.settings.filament_profiles(),
        )?;
        sync_files(&path)?;
        let mut attempt = Attempt::new(
            job.plate_id.clone(),
            job.id.clone(),
            execution.settings.main.ams_slot,
            materials[0].clone(),
        );
        attempt.interface =
            execution
                .settings
                .interface
                .as_ref()
                .map(|i| crate::print_start::MaterialSlot {
                    ams_slot: i.ams_slot,
                    material: materials[1].clone(),
                });
        attempt.id = attempt_id.into();
        self.printer.start(attempt, bytes).await?;
        Ok(())
    }
    pub(crate) fn cleanup(&self) -> Result<()> {
        let c = self.store.db.connection()?;
        let terminal=c.prepare("SELECT id FROM print_jobs WHERE printer_id=?1 AND state IN ('completed','cancelled')")?.query_map([&self.device.id],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for id in terminal {
            if uuid::Uuid::parse_str(&id).is_err() {
                return Err(Error::Invalid("Invalid job ID"));
            }
            let path = self.store.root.join("jobs").join(&id);
            match std::fs::remove_dir_all(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => continue,
            }
            c.execute(
                "DELETE FROM print_jobs WHERE id=?1 AND state IN ('completed','cancelled')",
                [id],
            )?;
        }
        Ok(())
    }
}

impl Database {
    pub(crate) fn fail_job(&self, id: &str, attempt: &str, message: &str) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        tx.execute("UPDATE printers SET queue_generation=queue_generation+1 WHERE id IN (SELECT printer_id FROM print_jobs WHERE id=?1 AND attempt_id=?2 AND state='preparing')",params![id,attempt])?;
        tx.execute("UPDATE print_jobs SET state='needs_attention',last_error=?1 WHERE id=?2 AND attempt_id=?3 AND state='preparing'",params![message,id,attempt])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn restore_attempt(&self, pid: &str) -> Result<Option<Attempt>> {
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let row:Option<(String,String,Option<String>)>=tx.query_row("SELECT id,state,attempt_json FROM print_jobs WHERE printer_id=?1 AND state NOT IN ('queued','completed','cancelled')",[pid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let mut restored = None;
        if let Some((id, state, raw)) = row {
            restored = raw
                .map(|s| serde_json::from_str::<Attempt>(&s))
                .transpose()
                .map_err(std::io::Error::other)?;
            if !matches!(state.as_str(), "awaiting_removal" | "printing") {
                if let Some(a) = &mut restored {
                    a.fail(
                        Phase::Unknown,
                        "Server restarted; inspect the printer before another start",
                    );
                }
                tx.execute("UPDATE print_jobs SET state='needs_attention',last_error='Server restarted; inspect the printer before another start' WHERE id=?1",[id])?;
                tx.execute(
                    "UPDATE printers SET queue_generation=queue_generation+1 WHERE id=?1",
                    [pid],
                )?;
            }
        }
        tx.commit()?;
        Ok(restored)
    }
    pub(crate) fn check_attempt(
        &self,
        device: &Device,
        attempt: &Attempt,
        status: &Status,
    ) -> Result<()> {
        let c = self.connection()?;
        let (raw,slot_id,filament_id,machine):(String,String,String,String)=c.query_row("SELECT execution_json,ams_slot_id,filament_id,required_machine_profile_key FROM print_jobs WHERE printer_id=?1 AND id=?2 AND attempt_id=?3 AND state IN ('preparing','printing','needs_attention')",params![device.id,attempt.job_id,attempt.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or(Error::Conflict("Execution is no longer active"))?;
        let mut execution: Execution = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        if let Some(stopped) = &execution.stopped_attempt {
            status.check_stopped(stopped)?;
        } else if !status.ready_to_print {
            return Err(Error::Conflict("Wait for a current, ready printer report"));
        }
        if machine != device.settings.machine_profile_key
            || execution.settings.main.filament.id != filament_id
        {
            return Err(Error::Conflict(
                "Printer or nozzle changed during preparation",
            ));
        }
        if execution.settings.main.ams_slot_id.is_empty() {
            execution.settings.main.ams_slot_id.clone_from(&slot_id);
        }
        let interface_matches = match (&attempt.interface, &execution.settings.interface) {
            (None, None) => true,
            (Some(actual), Some(expected)) => {
                actual.ams_slot == expected.ams_slot
                    && execution
                        .settings
                        .profiles
                        .get("interface.json")
                        .and_then(|p| p.get("filament_type"))
                        .and_then(|v| v.get(0))
                        .and_then(Value::as_str)
                        == Some(actual.material.as_str())
            }
            _ => false,
        };
        if attempt.plate_id != execution.plate.id
            || execution.settings.main.ams_slot_id != slot_id
            || attempt.ams_slot != execution.settings.main.ams_slot
            || execution.settings.profiles["filament.json"]["filament_type"][0] != attempt.material
            || !interface_matches
        {
            return Err(Error::Conflict(
                "Print material order or AMS mapping differs from the frozen execution",
            ));
        }
        execution
            .settings
            .check_bindings(&c, device, status, false)?;
        Ok(())
    }
    pub(crate) fn persist_attempt(&self, pid: &str, attempt: &Attempt) -> Result<()> {
        let state = match attempt.phase {
            Phase::Uploading | Phase::AwaitingConfirmation | Phase::Accepted => "preparing",
            Phase::Printing => "printing",
            Phase::Finished => "awaiting_removal",
            _ => "needs_attention",
        };
        let raw = serde_json::to_string(attempt).map_err(std::io::Error::other)?;
        let mut c = self.connection()?;
        let tx = c.transaction()?;
        let old:Option<(String,Option<String>,Option<String>)>=tx.query_row("SELECT state,last_error,attempt_json FROM print_jobs WHERE printer_id=?1 AND id=?2 AND attempt_id=?3 AND state NOT IN ('completed','cancelled')",params![pid,attempt.job_id,attempt.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (previous, error, previous_attempt) =
            old.ok_or(Error::Conflict("Execution is no longer active"))?;
        if previous_attempt.as_deref() == Some(&raw) {
            return Ok(());
        }
        if previous != state || error != attempt.message {
            tx.execute(
                "UPDATE printers SET queue_generation=queue_generation+1 WHERE id=?1",
                [pid],
            )?;
        }
        tx.execute("UPDATE print_jobs SET state=?1,attempt_json=?2,last_error=?3 WHERE id=?4 AND attempt_id=?5",params![state,raw,attempt.message,attempt.job_id,attempt.id])?;
        if state == "awaiting_removal"
            && previous != state
            && self
                .notifications_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            tx.execute("INSERT INTO print_notifications(attempt_id,job_id,printer_id,printer_name,plate_name) SELECT ?1,j.id,j.printer_id,p.name,j.name FROM print_jobs j JOIN printers p ON p.id=j.printer_id WHERE j.id=?2 AND j.attempt_id=?1 ON CONFLICT(attempt_id) DO NOTHING",params![attempt.id,attempt.job_id])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MACHINE: &str = crate::profiles::PRINTER;
    fn status() -> Status {
        let mut state = crate::printer_state::State::new(true);
        state.connected();
        state.apply(br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"1","ams":[{"id":"0","tray":[{"id":"0","tray_type":"PLA","tray_color":"FFFFFFFF"}]}]}}}"#,1);
        state.status(1)
    }
    async fn service(root: &std::path::Path, id: &str) -> Service {
        use std::os::unix::fs::PermissionsExt;
        let mut store = Store::open(root.join("data")).unwrap();
        let device = Device {
            id: id.into(),
            settings: crate::database::Settings {
                name: id.into(),
                host: "127.0.0.1".into(),
                serial: id.into(),
                access_code: "fixture".into(),
                tls_certificate: "fixture".into(),
                machine_profile_key: MACHINE.into(),
                default_process_profile_key: Selection::default().process,
                bed_type: crate::profiles::BEDS[0].into(),
                nozzle_material: "stainless_steel".into(),
                mqtt_port: 8883,
                ftps_port: 990,
                start_timeout_secs: 60,
            },
        };
        store.db.save(&device).unwrap();
        let f = crate::filament::Filament {
            id: "pla".into(),
            data: crate::filament::FilamentData {
                name: "white".into(),
                vendor: "fixture".into(),
                material: "PLA".into(),
                color: "FFFFFFFF".into(),
                bambu_filament_id: None,
            },
        };
        store.db.save_filament(&f).unwrap();
        store
            .db
            .save_setting(&crate::filament::Setting {
                id: "pla-setting".into(),
                filament_id: "pla".into(),
                data: crate::filament::SettingData {
                    machine_profile_key: MACHINE.into(),
                    base_profile_key: Selection::default().filament,
                    overrides_json: crate::filament::Overrides {
                        nozzle_temperature: Some(215),
                        ..Default::default()
                    },
                },
            })
            .unwrap();
        store.db.observe_ams(&device, &status()).unwrap();
        let slot = store.db.ams_slots(id).unwrap().remove(0);
        store
            .db
            .map_slot(id, &slot.id, slot.revision, Some("pla"))
            .unwrap();
        let app = root.join("app");
        let defaults = Selection::default();
        for (category, name) in [
            ("machine", MACHINE),
            ("process", defaults.process.as_str()),
            ("filament", defaults.filament.as_str()),
        ] {
            let directory = app.join("resources/profiles/BBL").join(category);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("profile.json"),json!({"name":name,"instantiation":"true","nozzle_diameter":["0.4"],"filament_type":["PLA"],"cool_plate_temp":["35"],"cool_plate_temp_initial_layer":["35"],"eng_plate_temp":["55"],"eng_plate_temp_initial_layer":["55"],"hot_plate_temp":["55"],"hot_plate_temp_initial_layer":["55"],"textured_plate_temp":["55"],"textured_plate_temp_initial_layer":["55"],"required_nozzle_HRC":["0"],"compatible_printers":[MACHINE]}).to_string()).unwrap();
        }
        let binary = app.join("AppRun");
        std::fs::write(&binary, "#!/bin/sh\nprintf 'OrcaSlicer-2.4.2:\\n'\n").unwrap();
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let slicer = Slicer::new(app, std::time::Duration::from_secs(1))
            .await
            .unwrap();
        store.profiles = Some(slicer.profiles.clone());
        Service::new(
            store,
            device,
            Printer::new(None, None).unwrap(),
            Some(slicer),
            None,
        )
    }
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One isolated two-material preparation and compatibility scenario.
    async fn support_resolves_two_materials_and_freezes_both_slots_and_settings() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let job_id = add(&s);
        let j = jobs(&s.store.db.connection().unwrap(), "one")
            .unwrap()
            .remove(0);
        // An unassigned interface must prevent admission, even when its PLA profile equals the main's.
        let mut f = s.store.db.filaments().unwrap().remove(0);
        f.id = "pla-black".into();
        f.data.name = "black".into();
        f.data.color = "000000FF".into();
        s.store.db.save_filament(&f).unwrap();
        s.store
            .db
            .save_setting(&crate::filament::Setting {
                id: "black-settings".into(),
                filament_id: f.id.clone(),
                data: crate::filament::SettingData {
                    machine_profile_key: MACHINE.into(),
                    base_profile_key: Selection::default().filament,
                    overrides_json: crate::filament::Overrides {
                        nozzle_temperature: Some(225),
                        ..Default::default()
                    },
                },
            })
            .unwrap();
        let mut conditions = json!(s.store.get(&j.plate_id).unwrap().conditions);
        conditions["support_enabled"] = json!(true);
        conditions["support_interface_filament_id"] = json!(f.id);
        let plate = edit_conditions(&s, &j.plate_id, conditions.clone());
        assert!(
            s.plan(&s.store.db.connection().unwrap(), &plate, &status())
                .is_err()
        );
        let mut state = crate::printer_state::State::new(true);
        state.connected();
        state.apply(br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"5","ams":[{"id":"0","tray":[{"id":"0","tray_type":"PLA","tray_color":"FFFFFFFF"},{"id":"2","tray_type":"PLA","tray_color":"000000FF"}]}]}}}"#,1);
        let report = state.status(1);
        s.store.db.observe_ams(&s.device, &report).unwrap();
        let slot = s
            .store
            .db
            .ams_slots("one")
            .unwrap()
            .into_iter()
            .find(|a| a.slot_index == 2)
            .unwrap();
        s.store
            .db
            .map_slot("one", &slot.id, slot.revision, Some(&f.id))
            .unwrap();
        let prep = s
            .reserve(&s.store.db.connection().unwrap(), &j, &report, None)
            .unwrap();
        let mut attempt = Attempt::new(
            prep.job.plate_id.clone(),
            prep.job.id.clone(),
            0,
            "PLA".into(),
        );
        attempt.id = prep.job.attempt_id.clone().unwrap();
        attempt.interface = Some(crate::print_start::MaterialSlot {
            ams_slot: 2,
            material: "PLA".into(),
        });
        s.store
            .db
            .check_attempt(&s.device, &attempt, &report)
            .unwrap();
        let mut swapped = attempt.clone();
        swapped.ams_slot = 2;
        swapped.interface.as_mut().unwrap().ams_slot = 0;
        assert!(
            s.store
                .db
                .check_attempt(&s.device, &swapped, &report)
                .is_err()
        );
        let mut missing = attempt.clone();
        missing.interface = None;
        assert!(
            s.store
                .db
                .check_attempt(&s.device, &missing, &report)
                .is_err()
        );
        let raw = json!(prep.execution);
        assert_eq!(
            raw["profiles"]["process.json"]["support_interface_filament"],
            "2"
        );
        assert_eq!(
            raw["profiles"]["interface.json"]["nozzle_temperature"],
            json!(["225"])
        );
        assert_eq!(raw["interface"]["ams_slot"], 2);
        assert_eq!(raw["interface"]["filament"]["id"], "pla-black");
        assert_eq!(raw["ams_slot"], 0);
        let restored: Execution = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(json!(restored), raw);
        assert_eq!(prep.job.id, job_id);
    }

    fn command(s: &Service, action: Action) -> Command {
        Command {
            epoch: s.epoch.clone(),
            generation: generation(&s.store.db.connection().unwrap(), &s.device.id).unwrap(),
            request_id: uuid::Uuid::new_v4().to_string(),
            action,
        }
    }
    fn add(s: &Service) -> String {
        let plate = s
            .store
            .edit(
                None,
                crate::plates::Edit {
                    conditions: crate::plates::Conditions {
                        required_machine_profile_key: Some(MACHINE.into()),
                        filament_id: Some("pla".into()),
                        process_profile_key: Some(Selection::default().process),
                        bed_type: Some(crate::profiles::BEDS[0].into()),
                        strength: crate::strength::Strength::default(),
                        brim_enabled: false,
                        ..Default::default()
                    },
                    name: "parts".into(),
                    version: None,
                    models: vec![crate::plates::ItemEdit {
                        id: None,
                        name: "latest.stl".into(),
                        source: Some("parts/latest.stl".into()),
                        quantity: 2,
                    }],
                },
            )
            .unwrap();
        let command = command(
            s,
            Action::Add {
                plate_id: plate.id,
                plate_version: plate.version,
            },
        );
        s.mutate(&command, &status()).unwrap();
        jobs(&s.store.db.connection().unwrap(), &s.device.id)
            .unwrap()
            .last()
            .unwrap()
            .id
            .clone()
    }
    fn edit_conditions(s: &Service, id: &str, conditions: Value) -> crate::plates::Plate {
        let mut edit = json!(s.store.get(id).unwrap());
        edit.as_object_mut().unwrap().remove("id");
        edit["conditions"] = conditions;
        s.store
            .edit(Some(id), serde_json::from_value(edit).unwrap())
            .unwrap()
    }
    #[tokio::test]
    async fn plate_conditions_gate_addition_and_freeze_only_at_preparation() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let raw =
            json!({"name":"front","models":[{"name":"part.stl","source":"part.stl","quantity":2}]});
        let plate = s
            .store
            .edit(None, serde_json::from_value(raw).unwrap())
            .unwrap();
        let add = || {
            command(&s, serde_json::from_value(json!({"type":"add","plate_id":plate.id,"plate_version":s.store.get(&plate.id).unwrap().version})).unwrap())
        };
        assert!(s.mutate(&add(), &status()).is_err());
        let configured = json!({"required_machine_profile_key":MACHINE,"filament_id":"pla","process_profile_key":Selection::default().process,"bed_type":crate::profiles::BEDS[0]});
        for missing in [
            "required_machine_profile_key",
            "filament_id",
            "bed_type",
            "process_profile_key",
        ] {
            let mut conditions = configured.clone();
            conditions[missing] = Value::Null;
            if missing == "required_machine_profile_key" {
                conditions["process_profile_key"] = Value::Null;
            }
            edit_conditions(&s, &plate.id, conditions);
            assert!(s.mutate(&add(), &status()).is_err(), "{missing}");
        }
        let set_conditions = |bed: &str| {
            let mut value = configured.clone();
            value["bed_type"] = json!(bed);
            edit_conditions(&s, &plate.id, value)
        };
        set_conditions(crate::profiles::BEDS[0]);
        let mut stale = status();
        stale.synchronized = false;
        assert!(s.mutate(&add(), &stale).is_err());
        let slot = s.store.db.ams_slots("one").unwrap().remove(0);
        s.store
            .db
            .map_slot("one", &slot.id, slot.revision, None)
            .unwrap();
        assert!(s.mutate(&add(), &status()).is_err());
        let slot = s.store.db.ams_slots("one").unwrap().remove(0);
        s.store
            .db
            .map_slot("one", &slot.id, slot.revision, Some("pla"))
            .unwrap();
        let mut busy = status();
        busy.ready_to_print = false;
        let request = add();
        s.mutate(&request, &busy).unwrap();
        s.mutate(&request, &busy).unwrap();
        let old = jobs(&s.store.db.connection().unwrap(), "one").unwrap();
        assert_eq!(old.len(), 1);
        let before = command(
            &s,
            Action::Next {
                expected_job: old[0].id.clone(),
                removed_job: None,
                cleared: true,
            },
        );
        let changed = set_conditions(crate::profiles::BEDS[3]);
        assert!(s.mutate(&before, &status()).is_err());
        let prep = s
            .mutate(&command(&s, before.action.clone()), &status())
            .unwrap()
            .unwrap();
        assert_eq!(prep.execution.plate.version, changed.version);
        assert_eq!(
            prep.execution.settings.selection.bed,
            crate::profiles::BEDS[3]
        );
        assert_eq!(prep.job.specification.bed_type, crate::profiles::BEDS[3]);
        let snapshot = || {
            s.store
                .db
                .connection()
                .unwrap()
                .query_row(
                    "SELECT execution_json FROM print_jobs WHERE id=?1",
                    [&old[0].id],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
        };
        let frozen = snapshot();
        set_conditions(crate::profiles::BEDS[0]);
        assert_eq!(snapshot(), frozen);
        s.mutate(&add(), &busy).unwrap();
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one")
                .unwrap()
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn addition_limit_matches_admission_and_recovers_after_removal() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let mut last = String::new();
        for _ in 0..99 {
            last = add(&s);
        }
        let job = jobs(&s.store.db.connection().unwrap(), "one")
            .unwrap()
            .into_iter()
            .find(|j| j.id == last)
            .unwrap();
        let plate = s.store.get(&job.plate_id).unwrap();
        assert!(
            s.admission(&s.store.db.connection().unwrap(), &plate, &status())
                .is_ok()
        );
        add(&s);
        assert!(
            s.admission(&s.store.db.connection().unwrap(), &plate, &status())
                .is_err()
        );
        let rejected = command(
            &s,
            Action::Add {
                plate_id: plate.id.clone(),
                plate_version: plate.version,
            },
        );
        assert!(s.mutate(&rejected, &status()).is_err());
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one")
                .unwrap()
                .len(),
            100
        );
        s.mutate(&command(&s, Action::Remove { job_id: last }), &status())
            .unwrap();
        assert!(
            s.admission(&s.store.db.connection().unwrap(), &plate, &status())
                .is_ok()
        );
    }
    #[test]
    fn command_fences_reject_stale_reordered_modified_and_restored_requests() {
        let action = Action::Remove {
            job_id: "one".into(),
        };
        let first = Command {
            epoch: "running".into(),
            generation: 4,
            request_id: uuid::Uuid::new_v4().to_string(),
            action,
        };
        assert!(!check_command("running", 4, None, &first).unwrap());
        assert!(check_command("running", 5, Some(&first), &first).unwrap());
        let mut altered = first.clone();
        altered.action = Action::Remove {
            job_id: "two".into(),
        };
        assert!(check_command("running", 5, Some(&first), &altered).is_err());
        altered.request_id = uuid::Uuid::new_v4().to_string();
        assert!(check_command("running", 5, Some(&first), &altered).is_err());
        assert!(check_command("restored", 4, Some(&first), &first).is_err());
    }
    #[tokio::test]
    async fn waiting_intent_updates_and_reservations_are_atomic_per_printer() {
        let root = tempfile::tempdir().unwrap();
        let first = service(root.path(), "one").await;
        let second = service(root.path(), "two").await;
        let id = add(&first);
        let next = add(&first);
        let other = add(&second);
        let stale = command(&first, Action::Remove { job_id: id.clone() });
        first
            .mutate(
                &command(
                    &first,
                    Action::Move {
                        job_id: next.clone(),
                        index: 0,
                    },
                ),
                &status(),
            )
            .unwrap();
        assert!(first.mutate(&stale, &status()).is_err());
        let start = command(
            &first,
            Action::Next {
                expected_job: next.clone(),
                removed_job: None,
                cleared: true,
            },
        );
        let mut cold = status();
        cold.synchronized = false;
        cold.ready_to_print = false;
        assert!(first.mutate(&start, &cold).is_err());
        assert!(
            jobs(&first.store.db.connection().unwrap(), "one")
                .unwrap()
                .iter()
                .all(|j| j.state == "queued")
        );
        let prep = first.mutate(&start, &status()).unwrap().unwrap();
        assert_eq!(
            prep.execution.settings.profiles["filament.json"]["nozzle_temperature"],
            json!(["215"])
        );
        assert_eq!(prep.execution.plate.models[0].quantity, 2);
        assert!(first.mutate(&start, &status()).unwrap().is_none());
        assert!(
            first
                .mutate(
                    &command(
                        &first,
                        Action::Next {
                            expected_job: id,
                            removed_job: None,
                            cleared: true
                        }
                    ),
                    &status()
                )
                .is_err()
        );
        assert!(
            second
                .mutate(
                    &command(
                        &second,
                        Action::Next {
                            expected_job: other,
                            removed_job: None,
                            cleared: true
                        }
                    ),
                    &status()
                )
                .unwrap()
                .is_some()
        );
    }
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn stopped_recovery_is_bound_to_the_observed_attempt_through_preparation() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let id = add(&s);
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Next {
                        expected_job: id.clone(),
                        removed_job: None,
                        cleared: true,
                    },
                ),
                &status(),
            )
            .unwrap()
            .unwrap();
        let mut original = Attempt::new(prep.job.plate_id.clone(), id.clone(), 0, "PLA".into());
        original.id = prep.job.attempt_id.unwrap();
        original.sent(1);
        original.phase = Phase::Unknown;
        s.store.db.persist_attempt("one", &original).unwrap();
        let stopped = || {
            let mut report = status();
            report.ready_to_print = false;
            report.print.state = Some("FAILED".into());
            report.print.name = Some(original.name());
            report.print.file = Some(original.filename());
            report
        };
        let mut invalid = Vec::new();
        for state in ["RUNNING", "PREPARE", "PAUSE", "UNKNOWN"] {
            let mut report = stopped();
            report.print.state = Some(state.into());
            invalid.push(report);
        }
        for connection in ["disconnected", "stale", "synchronizing"] {
            let mut report = stopped();
            report.connection = connection;
            report.synchronized = false;
            invalid.push(report);
        }
        for error in [None, Some(12)] {
            let mut report = stopped();
            report.print.error = error;
            invalid.push(report);
        }
        let mut other = stopped();
        other.print.file = Some("another-job.gcode.3mf".into());
        invalid.push(other);
        let mut unidentified = stopped();
        unidentified.print.file = None;
        unidentified.print.name = None;
        invalid.push(unidentified);
        for report in &invalid {
            for action in [
                Action::Retry {
                    expected_job: id.clone(),
                    cleared: true,
                },
                Action::Discard {
                    expected_job: id.clone(),
                    cleared: true,
                },
            ] {
                assert!(s.mutate(&command(&s, action), report).is_err());
            }
        }
        for action in [
            Action::Retry {
                expected_job: id.clone(),
                cleared: false,
            },
            Action::Discard {
                expected_job: id.clone(),
                cleared: false,
            },
        ] {
            assert!(s.mutate(&command(&s, action), &stopped()).is_err());
        }
        let request = command(
            &s,
            Action::Retry {
                expected_job: id.clone(),
                cleared: true,
            },
        );
        let prep = s.mutate(&request, &stopped()).unwrap().unwrap();
        assert_ne!(prep.job.attempt_id.as_deref(), Some(original.id.as_str()));
        assert!(s.mutate(&request, &stopped()).unwrap().is_none());
        let mut attempt = Attempt::new(prep.job.plate_id, id.clone(), 0, "PLA".into());
        attempt.id = prep.job.attempt_id.unwrap();
        s.store
            .db
            .check_attempt(&s.device, &attempt, &stopped())
            .unwrap();
        // A different final observation cannot consume the stopped-print authorization.
        invalid.push(status());
        for report in &invalid {
            assert!(
                s.store
                    .db
                    .check_attempt(&s.device, &attempt, report)
                    .is_err()
            );
        }
        // A preparation failure before sending may retry the same observed stop.
        s.store
            .db
            .fail_job(&id, &attempt.id, "Preparation failed")
            .unwrap();
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Retry {
                        expected_job: id.clone(),
                        cleared: true,
                    },
                ),
                &stopped(),
            )
            .unwrap()
            .unwrap();
        attempt.id = prep.job.attempt_id.unwrap();
        attempt.sent(2);
        attempt.phase = Phase::Unknown;
        s.store.db.persist_attempt("one", &attempt).unwrap();
        // After a send, a report for the older attempt is no proof that this attempt stopped.
        assert!(
            s.mutate(
                &command(
                    &s,
                    Action::Retry {
                        expected_job: id,
                        cleared: true
                    }
                ),
                &stopped()
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn unsent_or_rejected_start_can_retry_while_previous_finished_name_remains() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let id = add(&s);
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Next {
                        expected_job: id.clone(),
                        removed_job: None,
                        cleared: true,
                    },
                ),
                &status(),
            )
            .unwrap()
            .unwrap();
        s.store
            .db
            .fail_job(&id, prep.job.attempt_id.as_ref().unwrap(), "Upload failed")
            .unwrap();
        let mut ready = status();
        ready.print.state = Some("FINISH".into());
        ready.print.name = Some("orca-previous-finished-attempt".into());
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Retry {
                        expected_job: id.clone(),
                        cleared: true,
                    },
                ),
                &ready,
            )
            .unwrap()
            .unwrap();
        let mut rejected = Attempt::new(prep.job.plate_id, id.clone(), 0, "PLA".into());
        rejected.id = prep.job.attempt_id.unwrap();
        rejected.sent(1);
        rejected.phase = Phase::Rejected;
        s.store.db.persist_attempt("one", &rejected).unwrap();
        assert!(
            s.mutate(
                &command(
                    &s,
                    Action::Retry {
                        expected_job: id,
                        cleared: true
                    }
                ),
                &ready
            )
            .unwrap()
            .is_some()
        );
    }

    #[tokio::test]
    async fn prepared_attempt_rejects_a_reassigned_slot() {
        let root = tempfile::tempdir().unwrap();
        let first = service(root.path(), "one").await;
        let id = add(&first);
        let prep = first
            .mutate(
                &command(
                    &first,
                    Action::Next {
                        expected_job: id,
                        removed_job: None,
                        cleared: true,
                    },
                ),
                &status(),
            )
            .unwrap()
            .unwrap();
        let mut attempt = Attempt::new(
            prep.job.plate_id,
            prep.job.id,
            prep.execution.settings.main.ams_slot,
            "PLA".into(),
        );
        attempt.id = prep.job.attempt_id.unwrap();
        assert!(
            first
                .store
                .db
                .check_attempt(&first.device, &attempt, &status())
                .is_ok()
        );
        let slot = first.store.db.ams_slots("one").unwrap().remove(0);
        first
            .store
            .db
            .map_slot("one", &slot.id, slot.revision, None)
            .unwrap();
        assert!(
            first
                .store
                .db
                .check_attempt(&first.device, &attempt, &status())
                .is_err()
        );
    }
    #[tokio::test]
    async fn restart_preserves_printing_and_finished_jobs_without_authorizing_a_resend() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let id = add(&s);
        let start = command(
            &s,
            Action::Next {
                expected_job: id,
                removed_job: None,
                cleared: true,
            },
        );
        let prep = s.mutate(&start, &status()).unwrap().unwrap();
        let mut a = Attempt::new(prep.job.plate_id, prep.job.id, 0, "PLA".into());
        a.id = prep.job.attempt_id.unwrap();
        a.sent(1);
        s.store.db.persist_attempt("one", &a).unwrap();
        let uncertain = s.store.db.restore_attempt("one").unwrap().unwrap();
        assert_eq!(uncertain.phase, Phase::Unknown);
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one").unwrap()[0].state,
            "needs_attention"
        );
        a.phase = Phase::Printing;
        s.store.db.persist_attempt("one", &a).unwrap();
        let restored = s.store.db.restore_attempt("one").unwrap().unwrap();
        assert_eq!(restored.phase, Phase::Printing);
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one").unwrap()[0].state,
            "printing"
        );
        a.phase = Phase::Finished;
        s.store.db.persist_attempt("one", &a).unwrap();
        assert_eq!(
            s.store.db.restore_attempt("one").unwrap().unwrap().phase,
            Phase::Finished
        );
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one").unwrap()[0].state,
            "awaiting_removal"
        );
    }
    #[tokio::test]
    async fn completion_intent_commits_with_finish_and_survives_job_removal() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        s.store
            .db
            .notifications_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let id = add(&s);
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Next {
                        expected_job: id,
                        removed_job: None,
                        cleared: true,
                    },
                ),
                &status(),
            )
            .unwrap()
            .unwrap();
        let mut a = Attempt::new(prep.job.plate_id, prep.job.id, 0, "PLA".into());
        a.id = prep.job.attempt_id.unwrap();
        a.phase = Phase::Accepted;
        s.store.db.persist_attempt("one", &a).unwrap();
        a.phase = Phase::Unknown;
        s.store.db.persist_attempt("one", &a).unwrap();
        assert_eq!(
            s.store
                .db
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM print_notifications", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        a.phase = Phase::Printing;
        s.store.db.persist_attempt("one", &a).unwrap();
        s.store.db.connection().unwrap().execute_batch("CREATE TRIGGER reject_notification BEFORE INSERT ON print_notifications BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        a.phase = Phase::Finished;
        assert!(s.store.db.persist_attempt("one", &a).is_err());
        assert_eq!(
            jobs(&s.store.db.connection().unwrap(), "one").unwrap()[0].state,
            "printing"
        );
        s.store
            .db
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_notification;")
            .unwrap();
        s.store.db.persist_attempt("one", &a).unwrap();
        s.store.db.persist_attempt("one", &a).unwrap();
        a.message = Some("same completion".into());
        s.store.db.persist_attempt("one", &a).unwrap();
        s.mutate(
            &command(
                &s,
                Action::Discard {
                    expected_job: a.job_id.clone(),
                    cleared: true,
                },
            ),
            &status(),
        )
        .unwrap();
        s.cleanup().unwrap();
        let c = s.store.db.connection().unwrap();
        assert!(jobs(&c, "one").unwrap().is_empty());
        let row: (String, String, String) = c
            .query_row(
                "SELECT attempt_id,job_id,printer_name FROM print_notifications",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (a.id, a.job_id, "one".into()));
        assert_eq!(
            c.query_row("SELECT count(*) FROM print_notifications", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn enabling_notifications_does_not_replay_old_finished_jobs() {
        let root = tempfile::tempdir().unwrap();
        let s = service(root.path(), "one").await;
        let id = add(&s);
        let prep = s
            .mutate(
                &command(
                    &s,
                    Action::Next {
                        expected_job: id,
                        removed_job: None,
                        cleared: true,
                    },
                ),
                &status(),
            )
            .unwrap()
            .unwrap();
        let mut a = Attempt::new(prep.job.plate_id, prep.job.id, 0, "PLA".into());
        a.id = prep.job.attempt_id.unwrap();
        a.phase = Phase::Finished;
        s.store.db.persist_attempt("one", &a).unwrap();
        s.store
            .db
            .notifications_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut restored = s.store.db.restore_attempt("one").unwrap().unwrap();
        restored.message = Some("old finished report".into());
        s.store.db.persist_attempt("one", &restored).unwrap();
        assert_eq!(
            s.store
                .db
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM print_notifications", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
