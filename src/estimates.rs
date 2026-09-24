use crate::{
    artifacts,
    plates::{Error, Plate, Result, Store},
    profiles::Profiles,
    queue::{self, Job, Resolved},
    scad::Source,
    slicer::Slicer,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Record {
    input: Value,
    state: String,
    seconds: Option<u64>,
    error: Option<String>,
    id: Option<String>,
    #[serde(default)]
    attempt_id: Option<String>,
}
fn identity(plate: &Plate, settings: &Resolved) -> Value {
    json!({"plate":plate,"settings":settings,"slicer":"2.4.2","server":env!("CARGO_PKG_VERSION")})
}
fn presentation(record: Option<&Record>, input: &Value, preparing: bool) -> Value {
    if preparing {
        return json!({"state":"calculating","seconds":null,"error":null});
    }
    match record.filter(|r| &r.input == input) {
        Some(r) => json!({"state":r.state,"seconds":r.seconds,"error":r.error}),
        None => json!({"state":"pending","seconds":null,"error":null}),
    }
}
fn same_inputs(a: &Path, b: &Path, count: usize) -> bool {
    if ["stl", "3mf"].iter().any(|ext| {
        a.join(format!("{count}.{ext}")).exists() || b.join(format!("{count}.{ext}")).exists()
    }) {
        return false;
    }
    let names = (0..count)
        .map(|i| {
            let ext = if a.join(format!("{i}.3mf")).exists() || b.join(format!("{i}.3mf")).exists()
            {
                "3mf"
            } else {
                "stl"
            };
            format!("{i}.{ext}")
        })
        .chain(["printer.json", "process.json", "filament.json"].map(str::to_owned))
        .chain(
            ["interface.json", "secondary.json", "assemblies.json"]
                .iter()
                .filter(|name| a.join(name).exists() || b.join(name).exists())
                .map(|s| (*s).to_owned()),
        );
    names.into_iter().all(|name| {
        match (
            crate::plates::read_limited(&a.join(&name), crate::plates::MAX_UPLOAD),
            crate::plates::read_limited(&b.join(name), crate::plates::MAX_UPLOAD),
        ) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    })
}
fn directory(store: &Store, job: &str, id: &str) -> Result<PathBuf> {
    if uuid::Uuid::parse_str(job).is_err() || uuid::Uuid::parse_str(id).is_err() {
        return Err(Error::Invalid("Invalid estimate ID"));
    }
    Ok(store
        .root
        .join("jobs")
        .join(job)
        .join(format!("estimate-{id}")))
}
fn load(c: &Connection, job: &str) -> Result<Option<Record>> {
    let raw: Option<String> = c
        .query_row(
            "SELECT estimate_json FROM print_jobs WHERE id=?1",
            [job],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    raw.map(|s| {
        serde_json::from_str(&s)
            .map_err(std::io::Error::other)
            .map_err(Error::from)
    })
    .transpose()
}
fn save(c: &Connection, job: &str, record: &Record) -> Result<()> {
    c.execute(
        "UPDATE print_jobs SET estimate_json=?1 WHERE id=?2",
        params![
            serde_json::to_string(record).map_err(std::io::Error::other)?,
            job
        ],
    )?;
    Ok(())
}
fn plan(c: &Connection, job: &str, profiles: &Profiles) -> Result<(Plate, Resolved)> {
    let (plate_id, pid): (String, String) = c.query_row(
        "SELECT plate_id,printer_id FROM print_jobs WHERE id=?1",
        [job],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let plate = crate::plates::load(c, &plate_id)?;
    let specification = queue::planned(c, &pid, &plate)?;
    // Resolving slice settings does not require an idle printer or perform an AMS switch.
    let settings = queue::resolve(c, &pid, &specification, profiles, &plate)?;
    Ok((plate, settings))
}
pub(crate) fn view(c: &Connection, job: &Job, profiles: Option<&Profiles>) -> Result<Value> {
    let record = load(c, &job.id)?;
    if job.state != "queued" {
        return Ok(presentation(
            record.as_ref(),
            record.as_ref().map_or(&Value::Null, |r| &r.input),
            job.state == "preparing"
                && record
                    .as_ref()
                    .is_none_or(|r| r.attempt_id.as_ref() != job.attempt_id.as_ref()),
        ));
    }
    let input = profiles
        .ok_or(Error::Unavailable("OrcaSlicer is not configured"))
        .and_then(|p| plan(c, &job.id, p));
    Ok(match input {
        Ok((plate, settings)) => presentation(record.as_ref(), &identity(&plate, &settings), false),
        Err(error) => json!({"state":"failed","seconds":null,"error":queue::message(&error)}),
    })
}
pub(crate) fn retry(c: &Connection, store: &Store, job: &str) -> Result<()> {
    if let Some(record) = load(c, job)?
        && let Some(id) = record.id
    {
        let _ = std::fs::remove_dir_all(directory(store, job, &id)?);
    }
    c.execute(
        "UPDATE print_jobs SET estimate_json=NULL WHERE id=?1 AND state='queued'",
        [job],
    )?;
    Ok(())
}
struct Work {
    job: String,
    record: Record,
    plate: Plate,
    settings: Resolved,
    originals: Vec<Option<Vec<u8>>>,
    path: PathBuf,
}
fn reserve(store: &Store, slicer: &Slicer) -> Result<Option<Work>> {
    let c = store.db.connection()?;
    let jobs = c
        .prepare("SELECT id FROM print_jobs WHERE state='queued' ORDER BY position,id")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for job in jobs {
        let planned = plan(&c, &job, &slicer.profiles);
        let input = match &planned {
            Ok((p, s)) => identity(p, s),
            Err(e) => json!({"invalid":queue::message(e)}),
        };
        let previous = load(&c, &job)?;
        if previous
            .as_ref()
            .is_some_and(|r| r.input == input && matches!(r.state.as_str(), "ready" | "failed"))
        {
            continue;
        }
        if let Some(old) = previous.and_then(|r| r.id) {
            let _ = std::fs::remove_dir_all(directory(store, &job, &old)?);
        }
        let mut record = Record {
            input,
            state: "calculating".into(),
            seconds: None,
            error: None,
            id: None,
            attempt_id: None,
        };
        let (plate, settings) = match planned {
            Ok(input) => input,
            Err(error) => {
                record.state = "failed".into();
                record.error = Some(queue::message(&error));
                save(&c, &job, &record)?;
                continue;
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory(store, &job, &id)?;
        // Create before releasing the DB lock. Cancellation may remove it; never recreate it later.
        let setup = || -> Result<_> {
            let originals = queue::originals(&c, &plate, None)?;
            std::fs::create_dir_all(&path)?;
            Ok(originals)
        };
        let originals = match setup() {
            Ok(originals) => originals,
            Err(error) => {
                record.state = "failed".into();
                record.error = Some(queue::message(&error));
                save(&c, &job, &record)?;
                continue;
            }
        };
        record.id = Some(id);
        save(&c, &job, &record)?;
        return Ok(Some(Work {
            job,
            record,
            plate,
            settings,
            originals,
            path,
        }));
    }
    Ok(None)
}
async fn calculate(
    store: &Store,
    slicer: &Slicer,
    source: Option<&Source>,
    mut work: Work,
) -> Result<()> {
    let result = async {
        let count = queue::write_inputs(
            &work.path,
            &work.plate,
            &work.settings,
            work.originals,
            source,
        )
        .await?;
        slicer
            .slice(
                work.path.clone(),
                count,
                work.settings.selection.clone(),
                work.settings.filament_profiles(),
                &work.job,
            )
            .await?;
        let seconds = artifacts::estimated_seconds(&work.path.join("print.gcode.3mf"))?;
        queue::sync_files(&work.path)?;
        Ok::<_, Error>(seconds)
    }
    .await;
    let c = store.db.connection()?;
    let queued: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM print_jobs WHERE id=?1 AND state='queued')",
        [&work.job],
        |r| r.get(0),
    )?;
    let current = load(&c, &work.job)?;
    if !queued
        || current.as_ref().and_then(|r| r.id.as_ref()) != work.record.id.as_ref()
        || !plan(&c, &work.job, &slicer.profiles)
            .is_ok_and(|(p, s)| identity(&p, &s) == work.record.input)
    {
        let _ = std::fs::remove_dir_all(&work.path);
        return Ok(());
    }
    match result {
        Ok(seconds) => {
            work.record.state = "ready".into();
            work.record.seconds = Some(seconds);
        }
        Err(error) => {
            work.record.state = "failed".into();
            work.record.error = Some(queue::message(&error));
            work.record.id = None;
            let _ = std::fs::remove_dir_all(&work.path);
        }
    }
    save(&c, &work.job, &work.record)
}
pub(crate) fn start(store: Store, slicer: Option<Slicer>, source: Option<Source>) -> Result<()> {
    let Some(slicer) = slicer else {
        return Ok(());
    };
    store.db.connection()?.execute("UPDATE print_jobs SET estimate_json=json_set(estimate_json,'$.state','pending') WHERE state='queued' AND json_extract(estimate_json,'$.state')='calculating'",[])?;
    tokio::spawn(async move {
        // ponytail: scan at most 100 waiting jobs per printer; add explicit wakeups if this grows.
        loop {
            let result = match reserve(&store, &slicer) {
                Ok(Some(work)) => calculate(&store, &slicer, source.as_ref(), work).await,
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            };
            if result.is_err() {
                tracing::warn!("Queue estimate storage unavailable; will retry");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    Ok(())
}
pub(crate) fn reuse(
    store: &Store,
    job: &str,
    input: &Value,
    path: &Path,
    count: usize,
    settings: &Resolved,
) -> Result<bool> {
    let record = load(&*store.db.connection()?, job)?;
    let Some(record) = record.filter(|r| r.state == "ready" && &r.input == input) else {
        return Ok(false);
    };
    let Some(id) = record.id else {
        return Ok(false);
    };
    let previous = directory(store, job, &id)?;
    if !same_inputs(&previous, path, count) {
        return Ok(false);
    }
    for (name, sliced) in [("project.3mf", false), ("print.gcode.3mf", true)] {
        if artifacts::validate(
            &previous.join(name),
            count,
            &settings.selection,
            &settings.filament_profiles(),
            sliced,
        )
        .is_err()
        {
            return Ok(false);
        }
    }
    for name in ["project.3mf", "print.gcode.3mf"] {
        std::fs::copy(previous.join(name), path.join(name))?;
    }
    Ok(true)
}
pub(crate) fn reuse_for(
    store: &Store,
    job: &str,
    plate: &Plate,
    settings: &Resolved,
    path: &Path,
    count: usize,
) -> bool {
    reuse(
        store,
        job,
        &identity(plate, settings),
        path,
        count,
        settings,
    )
    .unwrap_or(false)
}
pub(crate) fn actual(
    store: &Store,
    job: &Job,
    plate: &Plate,
    settings: &Resolved,
    path: &Path,
) -> Result<()> {
    let c = store.db.connection()?;
    let matching:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM print_jobs WHERE id=?1 AND attempt_id=?2 AND state='preparing')",params![job.id,job.attempt_id],|r|r.get(0))?;
    if !matching {
        return Err(Error::Conflict("Execution is no longer active"));
    }
    let id = load(&c, &job.id)?.and_then(|r| r.id);
    let mut record = Record {
        input: identity(plate, settings),
        state: "ready".into(),
        seconds: None,
        error: None,
        id,
        attempt_id: job.attempt_id.clone(),
    };
    match artifacts::estimated_seconds(&path.join("print.gcode.3mf")) {
        Ok(seconds) => record.seconds = Some(seconds),
        Err(e) => {
            record.state = "failed".into();
            record.error = Some(queue::message(&e));
        }
    }
    save(&c, &job.id, &record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_input_cache_compares_original_geometry_and_material_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for name in [
            "0.3mf",
            "printer.json",
            "process.json",
            "filament.json",
            "secondary.json",
            "assemblies.json",
        ] {
            for dir in [a.path(), b.path()] {
                std::fs::write(dir.join(name), b"same").unwrap();
            }
        }
        assert!(same_inputs(a.path(), b.path(), 1));
        for name in ["0.3mf", "secondary.json", "assemblies.json"] {
            std::fs::write(b.path().join(name), b"changed").unwrap();
            assert!(!same_inputs(a.path(), b.path(), 1));
            std::fs::write(b.path().join(name), b"same").unwrap();
        }
        std::fs::write(b.path().join("1.3mf"), b"extra").unwrap();
        assert!(!same_inputs(a.path(), b.path(), 1));
    }
    #[test]
    fn input_changes_and_preparation_hide_the_previous_duration() {
        let input = json!({"quantity":2,"temperature":215,"version":"2.4.2"});
        let mut record = Record {
            input: input.clone(),
            state: "ready".into(),
            seconds: Some(1140),
            error: None,
            id: None,
            attempt_id: None,
        };
        assert_eq!(presentation(Some(&record), &input, false)["seconds"], 1140);
        assert_eq!(
            presentation(Some(&record), &input, true),
            json!({"state":"calculating","seconds":null,"error":null})
        );
        for changed in [
            json!({"quantity":3,"temperature":215,"version":"2.4.2"}),
            json!({"quantity":2,"temperature":220,"version":"2.4.2"}),
            json!({"quantity":2,"temperature":215,"version":"2.4.3"}),
        ] {
            assert_eq!(
                presentation(Some(&record), &changed, false),
                json!({"state":"pending","seconds":null,"error":null})
            );
        }
        record.state = "failed".into();
        record.seconds = None;
        record.error = Some("Slicing timed out".into());
        assert_eq!(
            presentation(Some(&record), &input, false),
            json!({"state":"failed","seconds":null,"error":"Slicing timed out"})
        );
    }
    #[test]
    fn reuse_requires_equal_models_profiles_and_count() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for name in [
            "0.stl",
            "1.stl",
            "printer.json",
            "process.json",
            "filament.json",
        ] {
            std::fs::write(a.path().join(name), name).unwrap();
            std::fs::write(b.path().join(name), name).unwrap();
        }
        assert!(same_inputs(a.path(), b.path(), 2));
        assert!(!same_inputs(a.path(), b.path(), 1));
        for name in ["0.stl", "filament.json"] {
            std::fs::write(b.path().join(name), "changed").unwrap();
            assert!(!same_inputs(a.path(), b.path(), 2));
            std::fs::write(b.path().join(name), name).unwrap();
        }
        std::fs::write(a.path().join("interface.json"), "second").unwrap();
        assert!(!same_inputs(a.path(), b.path(), 2));
        std::fs::write(b.path().join("interface.json"), "second").unwrap();
        assert!(same_inputs(a.path(), b.path(), 2));
        std::fs::write(b.path().join("interface.json"), "changed").unwrap();
        assert!(!same_inputs(a.path(), b.path(), 2));
        std::fs::remove_file(a.path().join("interface.json")).unwrap();
        assert!(!same_inputs(a.path(), b.path(), 2));
        std::fs::remove_file(b.path().join("process.json")).unwrap();
        assert!(!same_inputs(a.path(), b.path(), 2));
    }
}
