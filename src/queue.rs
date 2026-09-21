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
        specification: Specification,
    },
    Edit {
        job_id: String,
        specification: Specification,
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
    #[serde(flatten)]
    pub settings: Resolved,
    pub slot_revision: i64,
    pub ams_slot: u8,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Resolved {
    pub selection: Selection,
    pub profiles: BTreeMap<String, serde_json::Map<String, Value>>,
    pub filament: crate::filament::Filament,
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
fn load_setting(c: &Connection, s: &Specification) -> Result<crate::filament::SettingData> {
    let (base,raw):(String,String)=c.query_row("SELECT s.base_profile_key,s.overrides_json FROM filament_settings s JOIN filaments f ON f.product_id=s.product_id WHERE f.id=?1 AND s.machine_profile_key=?2",params![s.filament_id,s.required_machine_profile_key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Conflict("Configure this material for the required machine and nozzle first"))?;
    Ok(crate::filament::SettingData {
        machine_profile_key: s.required_machine_profile_key.clone(),
        base_profile_key: base,
        overrides_json: serde_json::from_str(&raw).map_err(std::io::Error::other)?,
    })
}
fn resolve(c: &Connection, pid: &str, s: &Specification, profiles: &Profiles) -> Result<Resolved> {
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM ams_slots WHERE id=?1 AND printer_id=?2)",
        params![s.ams_slot_id, pid],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(Error::Invalid(
            "Select an AMS slot belonging to this printer",
        ));
    }
    let setting = load_setting(c, s)?;
    let filament = crate::database::load_filaments(c)?
        .into_iter()
        .find(|f| f.id == s.filament_id)
        .ok_or(Error::NotFound)?;
    let selection = Selection {
        machine: s.required_machine_profile_key.clone(),
        process: s.process_profile_key.clone(),
        filament: setting.base_profile_key.clone(),
        bed: s.bed_type.clone(),
    };
    let mut resolved = profiles.resolved(&selection)?;
    resolved.insert(
        "filament.json".into(),
        profiles.resolve_filament(&setting, &filament.data.material)?,
    );
    Ok(Resolved {
        selection,
        profiles: resolved,
        filament,
    })
}
fn ready(
    c: &Connection,
    device: &Device,
    s: &Specification,
    status: &Status,
    profiles: &Profiles,
) -> Result<(Resolved, i64, u8)> {
    if !status.ready_to_print || !status.synchronized {
        return Err(Error::Conflict("Wait for a current, ready printer report"));
    }
    let current: String = c.query_row(
        "SELECT machine_profile_key FROM printers WHERE id=?1",
        [&device.id],
        |r| r.get(0),
    )?;
    if current != s.required_machine_profile_key || current != device.settings.machine_profile_key {
        return Err(Error::Conflict(
            "Required machine or nozzle differs from the registered configuration",
        ));
    }
    let resolved = resolve(c, &device.id, s, profiles)?;
    let diameter = resolved.profiles["printer.json"]["nozzle_diameter"][0]
        .as_str()
        .ok_or(Error::Invalid("Invalid nozzle profile"))?;
    crate::print_start::check_nozzle(status, diameter, &device.settings.nozzle_material)?;
    let hrc = resolved.profiles["filament.json"]
        .get("required_nozzle_HRC")
        .and_then(|v| v.get(0))
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok());
    if crate::filament::nozzle_fit(
        &resolved.filament.data.material,
        diameter,
        &device.settings.nozzle_material,
        hrc,
    ) == "unsupported"
    {
        return Err(Error::Conflict(
            "Material is incompatible with the registered nozzle",
        ));
    }
    let (ams,slot,assigned,present,revision):(u16,u8,Option<String>,Option<bool>,i64)=c.query_row("SELECT ams_id,slot_index,filament_id,present,revision FROM ams_slots WHERE printer_id=?1 AND id=?2",params![device.id,s.ams_slot_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if assigned.as_deref() != Some(&s.filament_id) || present != Some(true) || ams >= 4 {
        return Err(Error::Conflict(
            "Selected AMS slot does not contain the planned material",
        ));
    }
    let number = u8::try_from(ams * 4 + u16::from(slot)).expect("validated slot");
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
    Ok((resolved, revision, number))
}
fn message(error: &Error) -> String {
    match error {
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
    pub(crate) async fn read(&self) -> Result<Value> {
        let status = self.printer.status().await;
        if !self.device.id.is_empty() {
            self.printer.ams_inventory(None).await?;
        }
        let c = self.store.db.connection()?;
        let list = jobs(&c, &self.device.id)?;
        let current = list.iter().find(|j| j.state != "queued");
        let mut waiting = Vec::new();
        for job in list.iter().filter(|j| j.state == "queued") {
            let mut value = json!(job);
            let hold = self
                .slicer
                .as_ref()
                .ok_or(Error::Unavailable("OrcaSlicer is not configured"))
                .and_then(|s| ready(&c, &self.device, &job.specification, &status, &s.profiles));
            value["hold_reason"] = hold.err().map_or(Value::Null, |e| json!(message(&e)));
            waiting.push(value);
        }
        let idle = status.ready_to_print;
        let next = idle
            && current.is_none_or(|j| j.state == "awaiting_removal")
            && waiting.first().is_some_and(|j| j["hold_reason"].is_null());
        let retry = idle
            && current.is_some_and(|j| j.state == "needs_attention")
            && current.is_some_and(|j| {
                self.slicer.as_ref().is_some_and(|s| {
                    ready(&c, &self.device, &j.specification, &status, &s.profiles).is_ok()
                })
            });
        let discard = idle
            && current.is_some_and(|j| {
                matches!(j.state.as_str(), "needs_attention" | "awaiting_removal")
            });
        Ok(
            json!({"epoch":self.epoch,"generation":generation(&c,&self.device.id)?,"request_id":uuid::Uuid::new_v4().to_string(),"current":current,"waiting":waiting,"printer":status,"allowed":{"next":next,"retry":retry,"discard":discard}}),
        )
    }
    pub(crate) async fn apply(self: &Arc<Self>, command: Command) -> Result<Value> {
        let _guard = self.lock.lock().await;
        let status = self.printer.status().await;
        self.printer.ams_inventory(None).await?;
        let preparation = self.mutate(&command, &status)?;
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
        self.read().await
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
        if self.change_waiting(&tx, &command.action, &waiting)? {
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
                if !cleared || !status.ready_to_print {
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
                if !cleared || !status.ready_to_print {
                    return Err(Error::Conflict(
                        "Inspect the printer and empty plate before clearing this job",
                    ));
                }
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
    fn change_waiting(&self, tx: &Connection, action: &Action, waiting: &[&Job]) -> Result<bool> {
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
                specification,
            } => {
                let slicer = self
                    .slicer
                    .as_ref()
                    .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?;
                resolve(tx, &self.device.id, specification, &slicer.profiles)?;
                let plate = crate::plates::load(tx, plate_id)?;
                if waiting.len() >= 100 {
                    return Err(Error::Conflict("Queue holds at most 100 waiting jobs"));
                }
                let s = specification;
                tx.execute("INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',coalesce((SELECT max(position)+1 FROM print_jobs WHERE printer_id=?2),0))",params![uuid::Uuid::new_v4().to_string(),self.device.id,plate_id,plate.name,s.ams_slot_id,s.filament_id,s.required_machine_profile_key,s.process_profile_key,s.bed_type])?;
            }
            Action::Edit {
                job_id,
                specification,
            } => {
                selected(job_id)?;
                let slicer = self
                    .slicer
                    .as_ref()
                    .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?;
                resolve(tx, &self.device.id, specification, &slicer.profiles)?;
                let s = specification;
                tx.execute("UPDATE print_jobs SET ams_slot_id=?1,filament_id=?2,required_machine_profile_key=?3,process_profile_key=?4,bed_type=?5,last_error=NULL WHERE id=?6",params![s.ams_slot_id,s.filament_id,s.required_machine_profile_key,s.process_profile_key,s.bed_type,job_id])?;
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
        let (settings, slot_revision, ams_slot) = ready(
            c,
            &self.device,
            &job.specification,
            status,
            &slicer.profiles,
        )?;
        let execution = Execution {
            plate: crate::plates::load(c, &job.plate_id)?,
            settings,
            slot_revision,
            ams_slot,
        };
        let originals = c
            .prepare("SELECT original FROM plate_items WHERE plate_id=?1 ORDER BY position")?
            .query_map([&job.plate_id], |r| r.get::<_, Option<Vec<u8>>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut job = job.clone();
        job.state = "preparing".into();
        job.name.clone_from(&execution.plate.name);
        let attempt = uuid::Uuid::new_v4().to_string();
        job.attempt_id = Some(attempt.clone());
        job.artifact_path = Some(format!("jobs/{}/{attempt}", job.id));
        job.last_error = None;
        if let Some(id) = completed {
            c.execute("UPDATE print_jobs SET state='completed' WHERE id=?1", [id])?;
        }
        c.execute("UPDATE print_jobs SET name=?1,state='preparing',attempt_id=?2,artifact_path=?3,execution_json=?4,attempt_json=NULL,last_error=NULL WHERE id=?5",params![job.name,attempt,job.artifact_path,serde_json::to_string(&execution).map_err(std::io::Error::other)?,job.id])?;
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
        let mut count = 0;
        let mut remaining = crate::plates::MAX_UPLOAD;
        for (model, original) in execution.plate.models.iter().zip(originals) {
            let bytes = if let Some(source) = &model.source {
                self.source
                    .as_ref()
                    .ok_or(Error::Unavailable("SCAD_LIVE_URL is not configured"))?
                    .model(source, remaining)
                    .await?
            } else {
                original.ok_or(Error::Unavailable("Uploaded original is unavailable"))?
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
        for (name, profile) in &execution.settings.profiles {
            serde_json::to_writer(std::fs::File::create(path.join(name))?, profile)
                .map_err(std::io::Error::other)?;
        }
        self.slicer
            .as_ref()
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
            .slice(
                path.clone(),
                count,
                execution.settings.selection.clone(),
                &job.id,
            )
            .await?;
        let bytes = std::fs::read(path.join("print.gcode.3mf"))?;
        let material =
            crate::print_start::material_for(&bytes, &execution.settings.selection.machine)?;
        let expected = execution.settings.profiles["filament.json"]["filament_type"][0]
            .as_str()
            .ok_or(Error::Invalid("Material profile is invalid"))?;
        if material != expected {
            return Err(Error::Invalid(
                "Sliced material differs from the execution profile",
            ));
        }
        // Flush immutable inputs and outputs before persisting the upload/start attempt.
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::File::open(entry.path())?.sync_all()?;
            }
        }
        std::fs::File::open(&path)?.sync_all()?;
        let mut attempt = Attempt::new(
            job.plate_id.clone(),
            job.id.clone(),
            execution.ams_slot,
            material,
        );
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
        let execution: Execution = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        if !status.ready_to_print
            || !status.synchronized
            || machine != device.settings.machine_profile_key
        {
            return Err(Error::Conflict(
                "Printer or nozzle changed during preparation",
            ));
        }
        let (assigned, revision, present): (Option<String>, i64, Option<bool>) = c.query_row(
            "SELECT filament_id,revision,present FROM ams_slots WHERE id=?1 AND printer_id=?2",
            params![slot_id, device.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let material = crate::database::load_filaments(&c)?
            .into_iter()
            .find(|f| f.id == filament_id)
            .ok_or(Error::NotFound)?;
        if assigned.as_deref() != Some(&filament_id)
            || revision != execution.slot_revision
            || present != Some(true)
            || serde_json::to_value(material).ok()
                != serde_json::to_value(execution.settings.filament).ok()
        {
            return Err(Error::Conflict(
                "AMS assignment or material changed during preparation",
            ));
        }
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
        let store = Store::open(root.join("data")).unwrap();
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
            std::fs::write(directory.join("profile.json"),json!({"name":name,"instantiation":"true","nozzle_diameter":["0.4"],"filament_type":["PLA"],"required_nozzle_HRC":["0"],"compatible_printers":[MACHINE]}).to_string()).unwrap();
        }
        let binary = app.join("AppRun");
        std::fs::write(&binary, "#!/bin/sh\nprintf 'OrcaSlicer-2.4.2:\\n'\n").unwrap();
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let slicer = Slicer::new(app, std::time::Duration::from_secs(1))
            .await
            .unwrap();
        Service::new(
            store,
            device,
            Printer::new(None, None).unwrap(),
            Some(slicer),
            None,
        )
    }
    fn specification(service: &Service) -> Specification {
        Specification {
            ams_slot_id: service.store.db.ams_slots(&service.device.id).unwrap()[0]
                .id
                .clone(),
            filament_id: "pla".into(),
            required_machine_profile_key: MACHINE.into(),
            process_profile_key: Selection::default().process,
            bed_type: crate::profiles::BEDS[0].into(),
        }
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
                specification: specification(s),
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
        let mut spec = specification(&first);
        spec.ams_slot_id = specification(&second).ams_slot_id;
        let edit = command(
            &first,
            Action::Edit {
                job_id: id.clone(),
                specification: spec,
            },
        );
        assert!(first.mutate(&edit, &status()).is_err());
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
            prep.execution.ams_slot,
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
}
