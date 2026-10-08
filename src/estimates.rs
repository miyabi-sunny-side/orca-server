use crate::{
    artifacts,
    plates::{Error, Plate, Result, Store},
    profiles::Profiles,
    queue::{self, Execution, Job, Resolved},
    scad::Source,
    slicer::Slicer,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

// Change this only when the generated input/CLI contract becomes incompatible.
const GENERATOR: &str = "orca-2.4.2/plate-slice-1";
const RECHECK_SECONDS: i64 = 30;

#[derive(Serialize, Deserialize)]
struct Record {
    input: Value,
    state: String,
    seconds: Option<u64>,
    error: Option<String>,
    id: Option<String>,
    #[serde(default)]
    attempt_id: Option<String>,
    #[serde(default)]
    output_key: Option<String>,
    /// Why this printer cannot print the plate: `unfit` or `material_setting`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}
/// The two reasons a printer cannot print a plate that the page names; other failures keep
/// their short message only.
fn reason(error: &Error) -> Option<&'static str> {
    match error {
        Error::Invalid("Models must fit together on one plate") => Some("unfit"),
        Error::Slicer(message) if message.contains("Model is outside the printable area") => {
            Some("unfit")
        }
        Error::Conflict(crate::products::MISSING_SETTING) => Some("material_setting"),
        _ => None,
    }
}
fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write;
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").expect("write to String");
            out
        })
}
fn plan(plate: &Plate, settings: &Resolved, originals: &[Option<Vec<u8>>]) -> Value {
    json!({"generator":GENERATOR,"models":plate.models.iter().zip(originals).map(|(m,bytes)|
        json!({"source":m.source,"quantity":m.quantity,"roles":m.roles,"content":bytes.as_ref().map(|b|digest(b))})
    ).collect::<Vec<_>>(),"selection":settings.selection,"profiles":settings.profiles,"roles":settings.roles})
}
fn input_key(path: &Path) -> Result<String> {
    let mut files = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file()
            && !matches!(name.as_str(), "project.3mf" | "print.gcode.3mf")
        {
            files.insert(
                name,
                digest(&crate::plates::read_limited(
                    &entry.path(),
                    crate::plates::MAX_UPLOAD,
                )?),
            );
        }
    }
    Ok(digest(
        &serde_json::to_vec(&json!({"generator":GENERATOR,"files":files}))
            .map_err(std::io::Error::other)?,
    ))
}
fn presentation(record: Option<&Record>) -> Value {
    record.map_or_else(
        || json!({"state":"pending","seconds":null,"error":null,"reason":null}),
        |r| json!({"state":r.state,"seconds":r.seconds,"error":r.error,"reason":r.reason}),
    )
}
fn load(c: &Connection, plate: &str, printer: &str) -> Result<Option<(Value, Record)>> {
    let row: Option<(String, String)> = c
        .query_row(
            "SELECT plan_json,record_json FROM plate_slices WHERE plate_id=?1 AND printer_id=?2",
            [plate, printer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // Cache metadata is disposable; an unreadable record must return to computation.
    Ok(row.and_then(|(plan, record)| {
        Some((
            serde_json::from_str(&plan).ok()?,
            serde_json::from_str(&record).ok()?,
        ))
    }))
}
fn saved_plan(c: &Connection, plate: &Plate, printer: &str, profiles: &Profiles) -> Result<Value> {
    let settings = queue::slice_settings(c, profiles, plate, printer)?;
    Ok(plan(plate, &settings, &queue::originals(c, plate)?))
}
/// The current result of one plate on one printer.
pub(crate) fn pair_view(
    c: &Connection,
    plate: &Plate,
    printer: &str,
    profiles: Option<&Profiles>,
) -> Result<Value> {
    let planned = profiles
        .ok_or(Error::Unavailable("OrcaSlicer is not configured"))
        .and_then(|p| saved_plan(c, plate, printer, p));
    let planned = match planned {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(
                json!({"state":"failed","seconds":null,"error":queue::message(&error),"reason":reason(&error)}),
            );
        }
    };
    let record = load(c, &plate.id, printer)?;
    Ok(presentation(
        record
            .as_ref()
            .filter(|(p, _)| p == &planned)
            .map(|(_, r)| r),
    ))
}
/// Every registered printer's result for a plate, in the printer list's order.
pub(crate) fn plate_view(
    c: &Connection,
    plate: &Plate,
    profiles: Option<&Profiles>,
) -> Result<Value> {
    let printers = c
        .prepare("SELECT id,name,machine_profile_key FROM printers ORDER BY name,id")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut views = Vec::new();
    for (id, name, machine) in printers {
        let mut view = pair_view(c, plate, &id, profiles)?;
        view["printer_id"] = json!(id);
        view["printer_name"] = json!(name);
        view["machine_profile_key"] = json!(machine);
        views.push(view);
    }
    Ok(json!({"printers":views}))
}
/// The printer cannot place the plate's models on its bed.
pub(crate) fn check_fit(
    c: &Connection,
    plate: &Plate,
    printer: &str,
    profiles: &Profiles,
) -> Result<()> {
    if pair_view(c, plate, printer, Some(profiles))?["reason"] == "unfit" {
        return Err(Error::Conflict(
            "The models do not fit on this printer's build plate",
        ));
    }
    Ok(())
}
pub(crate) fn view(
    c: &Connection,
    job: &Job,
    printer: &str,
    profiles: Option<&Profiles>,
) -> Result<Value> {
    if job.state == "queued" {
        return pair_view(
            c,
            &crate::plates::load(c, &job.plate_id)?,
            printer,
            profiles,
        );
    }
    // The active attempt owns its duration; later plate edits cannot replace it.
    let raw: Option<String> = c.query_row(
        "SELECT estimate_json FROM print_executions WHERE id=(SELECT attempt_id FROM print_jobs WHERE id=?1)",
        [&job.id],
        |r| r.get(0),
    )?;
    let record: Option<Record> = raw
        .map(|s| serde_json::from_str(&s).map_err(std::io::Error::other))
        .transpose()?;
    if job.state == "preparing"
        && record
            .as_ref()
            .is_none_or(|r| r.attempt_id != job.attempt_id)
    {
        return Ok(json!({"state":"calculating","seconds":null,"error":null}));
    }
    Ok(presentation(record.as_ref()))
}
pub(crate) fn retry_plate(c: &Connection, plate: &str) -> Result<()> {
    c.execute("UPDATE plate_slices SET record_json=json_set(record_json,'$.state','pending','$.seconds',NULL,'$.error',NULL),checked_at=0 WHERE plate_id=?1", [plate])?;
    Ok(())
}
pub(crate) fn retry(c: &Connection, _store: &Store, job: &str) -> Result<()> {
    let plate: String = c.query_row(
        "SELECT plate_id FROM print_jobs WHERE id=?1 AND state='queued'",
        [job],
        |r| r.get(0),
    )?;
    retry_plate(c, &plate)
}
fn save_record(
    c: &Connection,
    plate: &str,
    printer: &str,
    planned: &Value,
    record: &Record,
) -> Result<()> {
    // One current generation per plate and printer. Executions already have their own frozen files.
    c.execute("INSERT INTO plate_slices(plate_id,printer_id,plan_json,record_json) SELECT ?1,?4,?2,?3 WHERE EXISTS(SELECT 1 FROM plates WHERE id=?1 AND deleted=0) AND EXISTS(SELECT 1 FROM printers WHERE id=?4)
        ON CONFLICT(plate_id,printer_id) DO UPDATE SET plan_json=excluded.plan_json,record_json=excluded.record_json,checked_at=unixepoch()",
        params![plate,planned.to_string(),serde_json::to_string(record).map_err(std::io::Error::other)?,printer])?;
    Ok(())
}
fn record(plate: &Plate, settings: &Resolved, state: &str) -> Record {
    Record {
        input: json!({"plate":plate,"settings":settings,"generator":GENERATOR}),
        state: state.into(),
        seconds: None,
        error: None,
        id: Some(uuid::Uuid::new_v4().to_string()),
        attempt_id: None,
        output_key: None,
        reason: None,
    }
}
fn current(
    c: &Connection,
    plate: &Plate,
    printer: &str,
    planned: &Value,
    profiles: &Profiles,
) -> bool {
    crate::plates::is_deleted(c, &plate.id).is_ok_and(|deleted| !deleted)
        && crate::plates::load(c, &plate.id)
            .and_then(|p| saved_plan(c, &p, printer, profiles))
            .is_ok_and(|p| &p == planned)
}
fn reuse(
    store: &Store,
    plate: &str,
    printer: &str,
    key: &str,
    path: &Path,
    count: usize,
    settings: &Resolved,
) -> Result<Option<Record>> {
    let cached: Option<(String, Vec<u8>, Vec<u8>)> = store
        .db
        .connection()?
        .query_row(
            "SELECT record_json,project,gcode FROM plate_slices WHERE plate_id=?1 AND printer_id=?3 AND input_key=?2 AND project IS NOT NULL AND gcode IS NOT NULL",
            params![plate, key, printer],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((raw, project, gcode)) = cached else {
        return Ok(None);
    };
    let Ok(mut record) = serde_json::from_str::<Record>(&raw) else {
        return Ok(None);
    };
    if record.output_key.as_deref() != Some(&digest(&gcode)) {
        return Ok(None);
    }
    for (name, bytes, sliced) in [
        ("project.3mf", project, false),
        ("print.gcode.3mf", gcode, true),
    ] {
        std::fs::write(path.join(name), bytes)?;
        if artifacts::validate(
            &path.join(name),
            count,
            &settings.selection,
            &settings.filament_profiles(),
            sliced,
        )
        .is_err()
        {
            return Ok(None);
        }
    }
    // A retry clears seconds but keeps the cache; the restored G-code owns the prediction.
    record.seconds = Some(artifacts::estimated_seconds(&path.join("print.gcode.3mf"))?);
    Ok(Some(record))
}

// Called under the shared lock by background computation and every print preparation.
async fn compute(
    store: &Store,
    slicer: &Slicer,
    source: Option<&Source>,
    execution: &Execution,
    printer: &str,
    originals: Vec<Option<Vec<u8>>>,
    path: &Path,
) -> Result<()> {
    let plate = &execution.plate;
    let settings = &execution.settings;
    let planned = plan(plate, settings, &originals);
    let mut result = record(plate, settings, "calculating");
    let calculation = async {
        let count = queue::write_inputs(path, plate, settings, originals, source).await?;
        let key = input_key(path)?;
        if let Some(mut cached) = reuse(store, &plate.id, printer, &key, path, count, settings)? {
            cached.state = "ready".into();
            cached.error = None;
            result = cached;
            return Ok((key, false));
        }
        {
            let c = store.db.connection()?;
            if current(&c, plate, printer, &planned, &slicer.profiles) {
                save_record(&c, &plate.id, printer, &planned, &result)?;
            }
        }
        slicer
            .slice(
                path.to_path_buf(),
                count,
                settings.selection.clone(),
                settings.filament_profiles(),
                &plate.id,
            )
            .await?;
        result.seconds = Some(artifacts::estimated_seconds(&path.join("print.gcode.3mf"))?);
        result.output_key = Some(digest(&std::fs::read(path.join("print.gcode.3mf"))?));
        result.state = "ready".into();
        Ok::<_, Error>((key, true))
    }
    .await;
    let (key, generated) = match calculation {
        Ok(value) => value,
        Err(error) => {
            result.state = "failed".into();
            result.error = Some(queue::message(&error));
            result.reason = reason(&error).map(Into::into);
            let c = store.db.connection()?;
            if current(&c, plate, printer, &planned, &slicer.profiles) {
                save_record(&c, &plate.id, printer, &planned, &result)?;
            }
            return Err(error);
        }
    };
    // Source content can change while Orca is running. Never publish that old result as current.
    let still_current = {
        let c = store.db.connection()?;
        current(&c, plate, printer, &planned, &slicer.profiles)
    };
    if !still_current {
        return Ok(());
    }
    if generated && plate.models.iter().any(|m| m.source.is_some()) {
        let scratch = tempfile::tempdir_in(store.root.join("slices-work"))?;
        let originals = queue::originals(&*store.db.connection()?, plate)?;
        let verification = queue::write_inputs(scratch.path(), plate, settings, originals, source)
            .await
            .and_then(|_| input_key(scratch.path()));
        if !verification.as_ref().is_ok_and(|latest| latest == &key) {
            let c = store.db.connection()?;
            if current(&c, plate, printer, &planned, &slicer.profiles) {
                result.state = if verification.is_err() {
                    "failed"
                } else {
                    "pending"
                }
                .into();
                result.seconds = None;
                result.error = verification.err().map(|e| queue::message(&e));
                save_record(&c, &plate.id, printer, &planned, &result)?;
            }
            return Ok(());
        }
    }
    let project = std::fs::read(path.join("project.3mf"))?;
    let gcode = std::fs::read(path.join("print.gcode.3mf"))?;
    let mut c = store.db.connection()?;
    let tx = c.transaction()?;
    if current(&tx, plate, printer, &planned, &slicer.profiles) {
        save_record(&tx, &plate.id, printer, &planned, &result)?;
        tx.execute("UPDATE plate_slices SET input_key=?1,project=?2,gcode=?3,checked_at=unixepoch(),generated_at=CASE WHEN ?4 OR generated_at IS NULL THEN unixepoch() ELSE generated_at END WHERE plate_id=?5 AND printer_id=?6",params![key,project,gcode,generated,plate.id,printer])?;
    }
    tx.commit()?;
    Ok(())
}
pub(crate) async fn prepare(
    store: &Store,
    slicer: &Slicer,
    source: Option<&Source>,
    execution: &Execution,
    printer: &str,
    originals: Vec<Option<Vec<u8>>>,
    path: &Path,
) -> Result<()> {
    let _lock = store.slice_lock.lock().await;
    compute(store, slicer, source, execution, printer, originals, path).await
}
/// Every plate on every registered printer whose result is missing, stale or due a recheck.
fn pending(store: &Store, slicer: &Slicer) -> Result<Vec<(String, String)>> {
    let c = store.db.connection()?;
    let pairs = c
        .prepare("SELECT p.id,r.id FROM plates p CROSS JOIN printers r WHERE p.deleted=0 ORDER BY p.id,r.id")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut pending = Vec::new();
    for (id, printer) in pairs {
        let plate = crate::plates::load(&c, &id)?;
        let planned = saved_plan(&c, &plate, &printer, &slicer.profiles);
        let planned = match planned {
            Ok(p) => p,
            Err(error) => {
                let record = Record {
                    input: Value::Null,
                    state: "failed".into(),
                    seconds: None,
                    error: Some(queue::message(&error)),
                    id: None,
                    attempt_id: None,
                    output_key: None,
                    reason: reason(&error).map(Into::into),
                };
                if load(&c, &id, &printer)?
                    .as_ref()
                    .is_none_or(|(_, previous)| {
                        previous.state != record.state || previous.error != record.error
                    })
                {
                    save_record(&c, &id, &printer, &Value::Null, &record)?;
                }
                continue;
            }
        };
        let previous = load(&c, &id, &printer)?;
        let due = c
            .query_row(
                "SELECT checked_at<=unixepoch()-?2 FROM plate_slices WHERE plate_id=?1 AND printer_id=?3",
                params![id, RECHECK_SECONDS, printer],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(true);
        if previous
            .as_ref()
            .is_none_or(|(p, r)| p != &planned || r.state == "pending")
            || due
        {
            pending.push((id, printer));
        }
    }
    Ok(pending)
}
pub(crate) fn start(store: Store, slicer: Option<Slicer>, source: Option<Source>) -> Result<()> {
    let Some(slicer) = slicer else { return Ok(()) };
    store.db.connection()?.execute("UPDATE plate_slices SET checked_at=0,record_json=CASE WHEN json_extract(record_json,'$.state')='calculating' THEN json_set(record_json,'$.state','pending') ELSE record_json END",[])?;
    let workspace = store.root.join("slices-work");
    if workspace.exists() {
        std::fs::remove_dir_all(&workspace)?;
    }
    std::fs::create_dir_all(&workspace)?;
    tokio::spawn(async move {
        // ponytail: one CLI slot and one cache worker; use per-plate locks if parallel slicing is introduced.
        loop {
            if let Ok(ids) = pending(&store, &slicer) {
                for (id, printer) in ids {
                    let _lock = store.slice_lock.lock().await;
                    let work = (|| -> Result<_> {
                        let c = store.db.connection()?;
                        if crate::plates::is_deleted(&c, &id)? {
                            return Err(Error::NotFound);
                        }
                        let plate = crate::plates::load(&c, &id)?;
                        let settings =
                            queue::slice_settings(&c, &slicer.profiles, &plate, &printer)?;
                        let originals = queue::originals(&c, &plate)?;
                        Ok((
                            Execution {
                                plate,
                                settings,
                                stopped_attempt: None,
                            },
                            originals,
                        ))
                    })();
                    if let Ok((execution, originals)) = work {
                        if let Ok(path) = tempfile::tempdir_in(&workspace) {
                            let _ = compute(
                                &store,
                                &slicer,
                                source.as_ref(),
                                &execution,
                                &printer,
                                originals,
                                path.path(),
                            )
                            .await;
                        } else {
                            tracing::warn!("Plate slice workspace unavailable; will retry");
                        }
                    }
                }
            } else {
                tracing::warn!("Plate slice storage unavailable; will retry");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    Ok(())
}
pub(crate) fn actual(
    store: &Store,
    job: &Job,
    plate: &Plate,
    settings: &Resolved,
    path: &Path,
) -> Result<()> {
    let mut record = record(plate, settings, "ready");
    record.attempt_id.clone_from(&job.attempt_id);
    record.seconds = Some(artifacts::estimated_seconds(&path.join("print.gcode.3mf"))?);
    let updated=store.db.connection()?.execute("UPDATE print_executions SET estimate_json=?1 WHERE id=?3 AND EXISTS(SELECT 1 FROM print_jobs WHERE id=?2 AND attempt_id=?3 AND state='preparing')",params![serde_json::to_string(&record).map_err(std::io::Error::other)?,job.id,job.attempt_id])?;
    if updated != 1 {
        return Err(Error::Conflict("Execution is no longer active"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_placement_and_missing_material_settings_name_a_reason() {
        for (error, expected) in [
            (Error::Invalid("Models must fit together on one plate"), Some("unfit")),
            (
                Error::Slicer("OrcaSlicer slice: Model is outside the printable area (exit status: 1; reference p)".into()),
                Some("unfit"),
            ),
            (Error::Conflict(crate::products::MISSING_SETTING), Some("material_setting")),
            (Error::Slicer("OrcaSlicer arrange: CLI exited abnormally (exit status: 1; reference p)".into()), None),
            (Error::Invalid("OrcaSlicer did not slice every model on the plate"), None),
            (Error::Upstream("Model source is unavailable"), None),
            (Error::Timeout, None),
        ] {
            assert_eq!(reason(&error), expected, "{error:?}");
        }
    }
}
