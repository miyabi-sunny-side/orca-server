use crate::{
    plates::{Error, Result},
    print_start::Phase,
    printer_state::Status,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize)]
struct Job {
    id: String,
    plate_id: String,
    revision: String,
    name: String,
    ams_slot: u8,
    #[serde(skip)]
    directory: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JobPhase {
    Starting,
    Printing,
    AwaitingRemoval,
    NeedsAttention,
}

#[derive(Serialize)]
struct Current {
    job: Job,
    phase: JobPhase,
    attempt_id: Option<String>,
    message: Option<String>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    generation: u64,
    request_id: String,
    action: Action,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Add {
        plate_id: String,
        revision: String,
        ams_slot: u8,
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

#[derive(Serialize)]
struct Availability {
    next: bool,
    retry: bool,
    discard: bool,
}

#[derive(Default, Serialize)]
struct Queue {
    generation: u64,
    current: Option<Current>,
    waiting: Vec<Job>,
    #[serde(skip)]
    last_request: Option<Command>,
}

impl Queue {
    fn check(&self, request: &Command) -> Result<bool> {
        if uuid::Uuid::parse_str(&request.request_id).is_err() {
            return Err(Error::Invalid("request_id must be a UUID"));
        }
        if let Some(last) = &self.last_request
            && last.request_id == request.request_id
        {
            return if last == request {
                Ok(true)
            } else {
                Err(Error::Conflict(
                    "request_id was already used with different input",
                ))
            };
        }
        if request.generation != self.generation {
            return Err(Error::Conflict("Queue changed; refresh before acting"));
        }
        Ok(false)
    }
    fn record(&mut self, request: Command) {
        self.generation = self.generation.wrapping_add(1);
        self.last_request = Some(request);
    }
    fn add(&mut self, job: Job) -> Result<()> {
        if self.waiting.len() >= 100 {
            return Err(Error::Conflict("Queue holds at most 100 waiting jobs"));
        }
        self.waiting.push(job);
        Ok(())
    }
    fn move_job(&mut self, id: &str, index: usize) -> Result<()> {
        if index >= self.waiting.len() {
            return Err(Error::Invalid("Queue index is out of range"));
        }
        let job = self.remove(id)?;
        self.waiting.insert(index, job);
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<Job> {
        let index = self
            .waiting
            .iter()
            .position(|job| job.id == id)
            .ok_or(Error::Conflict("Waiting job is no longer available"))?;
        Ok(self.waiting.remove(index))
    }
    fn check_next(&self, expected: &str, cleared: bool, ready: bool) -> Result<()> {
        if !cleared || !ready {
            return Err(Error::Conflict(
                "Confirm the cleared build plate and wait for a ready printer",
            ));
        }
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.phase != JobPhase::AwaitingRemoval)
        {
            return Err(Error::Conflict(
                "Current print is active or needs attention",
            ));
        }
        if self.waiting.first().is_none_or(|job| job.id != expected) {
            return Err(Error::Conflict(
                "Expected next job is no longer at the head",
            ));
        }
        Ok(())
    }
    fn availability(&self, ready: bool) -> Availability {
        Availability {
            next: self
                .waiting
                .first()
                .is_some_and(|job| self.check_next(&job.id, true, ready).is_ok()),
            retry: self
                .current
                .as_ref()
                .is_some_and(|c| self.check_recovery(&c.job.id, true, ready, true).is_ok()),
            discard: self
                .current
                .as_ref()
                .is_some_and(|c| self.check_recovery(&c.job.id, true, ready, false).is_ok()),
        }
    }
    fn begin(&mut self, expected: &str, cleared: bool, ready: bool, retry: bool) -> Result<Job> {
        let job = if retry {
            self.check_recovery(expected, cleared, ready, true)?;
            self.current
                .as_ref()
                .expect("current was checked")
                .job
                .clone()
        } else {
            self.check_next(expected, cleared, ready)?;
            self.waiting.remove(0)
        };
        self.current = Some(Current {
            job: job.clone(),
            phase: JobPhase::Starting,
            attempt_id: None,
            message: None,
        });
        Ok(job)
    }
    fn check_recovery(
        &self,
        expected: &str,
        cleared: bool,
        ready: bool,
        retry: bool,
    ) -> Result<()> {
        let allowed = self.current.as_ref().is_some_and(|c| {
            c.job.id == expected
                && (c.phase == JobPhase::NeedsAttention
                    || (!retry && c.phase == JobPhase::AwaitingRemoval))
        });
        if !allowed || !cleared || !ready {
            return Err(Error::Conflict(
                "Inspect the expected inactive job and wait for the printer to be ready",
            ));
        }
        Ok(())
    }
    fn discard(&mut self, expected: &str, cleared: bool, ready: bool) -> Result<Job> {
        self.check_recovery(expected, cleared, ready, false)?;
        Ok(self.current.take().expect("current was checked").job)
    }
    fn failed(&mut self, message: &str) {
        if let Some(current) = &mut self.current {
            current.phase = JobPhase::NeedsAttention;
            current.message = Some(message.to_owned());
        }
    }
    fn observe(&mut self, printer: &Status) {
        let Some(current) = &mut self.current else {
            return;
        };
        let Some(id) = &current.attempt_id else {
            return;
        };
        let (phase, message) = match printer.start.as_ref().filter(|a| &a.id == id) {
            Some(a) => (
                match a.phase {
                    Phase::Uploading | Phase::AwaitingConfirmation | Phase::Accepted => {
                        JobPhase::Starting
                    }
                    Phase::Printing => JobPhase::Printing,
                    Phase::Finished => JobPhase::AwaitingRemoval,
                    _ => JobPhase::NeedsAttention,
                },
                a.message.map(str::to_owned),
            ),
            None => (
                JobPhase::NeedsAttention,
                Some("Printer is tracking a different start; inspect the printer".into()),
            ),
        };
        if current.phase != phase || current.message != message {
            current.phase = phase;
            current.message = message;
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

fn freeze(
    source: &crate::plates::Store,
    root: &std::path::Path,
    id: &str,
    revision: &str,
    ams_slot: u8,
    machine: &str,
) -> Result<Job> {
    if ams_slot >= 16 {
        return Err(Error::Invalid("ams_slot must be 0..15"));
    }
    let plate = source.get(id)?;
    if plate.revision != revision {
        return Err(Error::Conflict("Plate revision changed"));
    }
    let print = plate.print.as_ref().ok_or(Error::Conflict(
        "Slice the plate before adding it to the queue",
    ))?;
    crate::print_start::material_for(&source.read_file(id, print)?, machine)?;
    let directory = tempfile::Builder::new().prefix("job-").tempdir_in(root)?;
    let target = directory.path().join(id);
    std::fs::create_dir_all(target.join("revisions").join(revision))?;
    for path in plate
        .models
        .iter()
        .map(|model| &model.path)
        .chain(plate.project.iter())
        .chain(plate.print.iter())
    {
        std::fs::write(target.join(path), source.read_file(id, path)?)?;
    }
    if source.get(id)?.revision != revision {
        return Err(Error::Conflict("Plate changed during snapshot"));
    }
    std::fs::write(
        target.join("plate.json"),
        serde_json::to_vec(&plate).map_err(|_| Error::Invalid("Cannot serialize saved job"))?,
    )?;
    Ok(Job {
        id: uuid::Uuid::new_v4().to_string(),
        plate_id: plate.id,
        revision: plate.revision,
        name: plate.name,
        ams_slot,
        directory: directory.keep(),
    })
}

use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct Service {
    machine: String,
    // ponytail: one queue lock includes snapshot copies; add reservations only if reads become too slow.
    queue: Mutex<Queue>,
    source: crate::plates::Store,
    printer: crate::printer::Printer,
    files: Arc<tempfile::TempDir>,
}

fn view(queue: &Queue, printer: &Status) -> serde_json::Value {
    serde_json::json!({"request_id":uuid::Uuid::new_v4().to_string(),"allowed":queue.availability(printer.ready_to_print),"generation":queue.generation,"current":queue.current,"waiting":queue.waiting,"printer":printer})
}
fn cleanup(job: &Job) {
    if std::fs::remove_dir_all(&job.directory).is_err() {
        tracing::warn!(
            "Cannot remove a completed job snapshot; the queue temporary root will clean it at shutdown"
        );
    }
}
impl Service {
    pub(crate) fn new(
        source: crate::plates::Store,
        printer: crate::printer::Printer,
        machine: String,
    ) -> std::io::Result<Self> {
        Ok(Self {
            machine,
            queue: Mutex::new(Queue::default()),
            source,
            printer,
            files: Arc::new(tempfile::Builder::new().prefix("orca-queue-").tempdir()?),
        })
    }
    pub(crate) async fn in_use(&self) -> bool {
        let queue = self.queue.lock().await;
        queue.current.is_some() || !queue.waiting.is_empty()
    }
    pub(crate) async fn read(&self) -> serde_json::Value {
        let mut queue = self.queue.lock().await;
        let status = self.printer.status().await;
        queue.observe(&status);
        view(&queue, &status)
    }
    async fn recover(
        &self,
        queue: &Queue,
        expected: &str,
        cleared: bool,
        printer: &Status,
        retry: bool,
    ) -> Result<()> {
        queue.check_recovery(expected, cleared, printer.ready_to_print, retry)?;
        if let Some(attempt) = &printer.start
            && attempt.phase == Phase::Unknown
        {
            self.printer.resolve(&attempt.id, true).await?;
        }
        Ok(())
    }
    async fn start(&self, queue: &mut Queue, job: &Job) {
        let Ok(store) = crate::plates::Store::open(&job.directory) else {
            queue.failed("Saved job files are unavailable");
            return;
        };
        match self
            .printer
            .start(
                store,
                job.plate_id.clone(),
                crate::printer::StartRequest {
                    revision: job.revision.clone(),
                    ams_slot: job.ams_slot,
                },
            )
            .await
        {
            Ok(attempt) => {
                queue
                    .current
                    .as_mut()
                    .expect("begin installed current")
                    .attempt_id = Some(attempt.id);
            }
            Err(error) => queue.failed(match error {
                Error::Invalid(message)
                | Error::Conflict(message)
                | Error::Unavailable(message)
                | Error::Upstream(message) => message,
                _ => "Could not start the saved job; check the printer and job files",
            }),
        }
    }
    pub(crate) async fn apply(&self, request: Command) -> Result<serde_json::Value> {
        let mut queue = self.queue.lock().await;
        let status = self.printer.status().await;
        queue.observe(&status);
        if queue.check(&request)? {
            return Ok(view(&queue, &status));
        }
        match &request.action {
            Action::Add {
                plate_id,
                revision,
                ams_slot,
            } => {
                let source = self.source.clone();
                let root = self.files.clone();
                let (id, revision, slot) = (plate_id.clone(), revision.clone(), *ams_slot);
                let machine = self.machine.clone();
                let job = crate::plate_api::blocking(move || {
                    freeze(&source, root.path(), &id, &revision, slot, &machine)
                })
                .await?;
                if let Err(error) = queue.add(job.clone()) {
                    cleanup(&job);
                    return Err(error);
                }
            }
            Action::Move { job_id, index } => queue.move_job(job_id, *index)?,
            Action::Remove { job_id } => cleanup(&queue.remove(job_id)?),
            Action::Next {
                expected_job,
                cleared,
            } => {
                let previous = queue.current.as_ref().map(|c| c.job.clone());
                let job = queue.begin(expected_job, *cleared, status.ready_to_print, false)?;
                self.start(&mut queue, &job).await;
                if let Some(previous) = previous {
                    cleanup(&previous);
                }
            }
            Action::Retry {
                expected_job,
                cleared,
            } => {
                self.recover(&queue, expected_job, *cleared, &status, true)
                    .await?;
                let job = queue.begin(expected_job, *cleared, status.ready_to_print, true)?;
                self.start(&mut queue, &job).await;
            }
            Action::Discard {
                expected_job,
                cleared,
            } => {
                self.recover(&queue, expected_job, *cleared, &status, false)
                    .await?;
                cleanup(&queue.discard(expected_job, *cleared, status.ready_to_print)?);
            }
        }
        queue.record(request);
        let status = self.printer.status().await;
        queue.observe(&status);
        Ok(view(&queue, &status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer_state::State;
    use serde_json::json;

    fn job(id: &str) -> Job {
        Job {
            id: id.into(),
            plate_id: "plate".into(),
            revision: "revision".into(),
            name: id.into(),
            ams_slot: 3,
            directory: std::path::PathBuf::from(id),
        }
    }
    fn request(generation: u64, id: u128) -> Command {
        Command {
            generation,
            request_id: uuid::Uuid::from_u128(id).to_string(),
            action: Action::Next {
                expected_job: "B".into(),
                cleared: true,
            },
        }
    }
    fn ready() -> crate::printer_state::Status {
        let mut state = State::new(true);
        state.connected();
        state.apply(include_bytes!("../tests/fixtures/p1_status.json"), 10);
        state.status(10)
    }

    #[test]
    fn availability_tracks_current_job_and_printer_readiness() {
        let mut q = Queue::default();
        assert!(!q.availability(true).next);
        q.add(job("A")).unwrap();
        q.add(job("B")).unwrap();
        assert!(q.availability(true).next);
        assert!(!q.availability(false).next);
        q.begin("A", true, true, false).unwrap();
        assert!(!q.availability(true).next);
        assert!(!q.availability(true).retry);
        assert!(!q.availability(true).discard);
        q.failed("Inspect the printer");
        assert!(!q.availability(true).next);
        assert!(q.availability(true).retry);
        assert!(q.availability(true).discard);
        assert!(!q.availability(false).retry);
        assert!(!q.availability(false).discard);
        q.current.as_mut().unwrap().phase = JobPhase::AwaitingRemoval;
        assert!(q.availability(true).next);
        assert!(!q.availability(true).retry);
        assert!(q.availability(true).discard);
        q.remove("B").unwrap();
        assert!(!q.availability(true).next);
        assert!(q.availability(true).discard);
    }

    #[test]
    fn completed_a_waits_for_removal_and_exactly_one_fresh_request_starts_b() {
        let mut q = Queue::default();
        q.add(job("A")).unwrap();
        q.add(job("B")).unwrap();
        assert!(q.begin("A", false, true, false).is_err());
        assert!(q.begin("A", true, false, false).is_err());
        q.begin("A", true, true, false).unwrap();
        q.current.as_mut().unwrap().attempt_id = Some("attempt-a".into());
        let mut printer = ready();
        let mut a =
            crate::print_start::Attempt::new("plate".into(), "revision".into(), 3, "PLA".into());
        a.id = "attempt-a".into();
        a.phase = crate::print_start::Phase::Printing;
        printer.start = Some(a.clone());
        q.observe(&printer);
        assert_eq!(q.current.as_ref().unwrap().phase, JobPhase::Printing);
        assert!(q.begin("B", true, true, false).is_err());
        a.phase = crate::print_start::Phase::Finished;
        printer.start = Some(a);
        q.observe(&printer);
        assert_eq!(q.current.as_ref().unwrap().phase, JobPhase::AwaitingRemoval);
        assert_eq!(q.waiting[0].id, "B");
        let next = request(q.generation, 1);
        assert!(!q.check(&next).unwrap());
        q.begin("B", true, true, false).unwrap();
        q.record(next.clone());
        assert!(q.check(&next).unwrap());
        let stale = request(next.generation, 2);
        assert!(q.check(&stale).is_err());
        let reused = request(q.generation, 1);
        assert!(q.check(&reused).is_err());
        assert_eq!(q.current.as_ref().unwrap().job.id, "B");
        assert!(q.waiting.is_empty());
    }

    #[test]
    fn failure_and_mismatched_observation_require_explicit_recovery() {
        let mut q = Queue::default();
        q.add(job("A")).unwrap();
        q.add(job("B")).unwrap();
        q.begin("A", true, true, false).unwrap();
        q.current.as_mut().unwrap().attempt_id = Some("missing".into());
        q.observe(&ready());
        assert_eq!(q.current.as_ref().unwrap().phase, JobPhase::NeedsAttention);
        assert!(q.begin("B", true, true, false).is_err());
        assert!(q.begin("A", false, true, true).is_err());
        assert_eq!(q.begin("A", true, true, true).unwrap().id, "A");
        assert_eq!(q.waiting[0].id, "B");
        assert!(q.discard("A", true, true).is_err());
        q.failed("Check printer");
        assert!(q.discard("A", false, true).is_err());
        assert!(q.discard("A", true, false).is_err());
        assert_eq!(q.discard("A", true, true).unwrap().id, "A");
        assert!(q.current.is_none());
        assert_eq!(q.waiting[0].id, "B");
    }

    #[test]
    fn reorder_and_delete_only_touch_waiting_jobs_and_requests_are_strict() {
        let mut q = Queue::default();
        q.add(job("A")).unwrap();
        q.add(job("B")).unwrap();
        q.add(job("C")).unwrap();
        q.move_job("C", 0).unwrap();
        assert_eq!(
            q.waiting.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            vec!["C", "A", "B"]
        );
        assert!(q.move_job("A", 3).is_err());
        q.begin("C", true, true, false).unwrap();
        assert!(q.remove("C").is_err());
        assert_eq!(q.remove("A").unwrap().id, "A");
        assert_eq!(q.waiting[0].id, "B");
        let mut bad = request(q.generation, 1);
        bad.request_id = "bad".into();
        assert!(q.check(&bad).is_err());
        let invalid = json!({"generation":0,"request_id":"id","action":{"type":"next","expected_job":"B","cleared":true,"unexpected":true}});
        assert!(serde_json::from_value::<Command>(invalid).is_err());
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::plates::{Input, ModelInput, Store};
    use serde_json::json;
    fn input(name: &str) -> Input {
        Input {
            name: name.into(),
            settings: json!({"quality":name}),
            models: (0..2)
                .map(|i| ModelInput {
                    name: format!("cube-{i}.stl"),
                    source: None,
                    data: include_bytes!("../tests/fixtures/cube.stl").to_vec(),
                })
                .collect(),
        }
    }
    #[test]
    fn queued_files_and_settings_survive_source_replacement() {
        let source_root = tempfile::tempdir().unwrap();
        let jobs = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let source = Store::open(source_root.path()).unwrap();
        let plate = source.save(None, input("before")).unwrap();
        let bytes = include_bytes!("../tests/fixtures/p1_print.gcode.3mf");
        std::fs::write(output.path().join("project.3mf"), bytes).unwrap();
        std::fs::write(output.path().join("print.gcode.3mf"), bytes).unwrap();
        let plate = source.save_artifacts(&plate, output.path()).unwrap();
        let frozen = freeze(
            &source,
            jobs.path(),
            &plate.id,
            &plate.revision,
            3,
            crate::profiles::PRINTER,
        )
        .unwrap();
        source.save(Some(&plate.id), input("after")).unwrap();
        let saved = Store::open(&frozen.directory).unwrap();
        let copy = saved.get(&plate.id).unwrap();
        assert_eq!(copy.revision, plate.revision);
        assert_eq!(copy.name, "before");
        assert_eq!(copy.settings, json!({"quality":"before"}));
        assert_eq!(frozen.ams_slot, 3);
        for path in copy
            .models
            .iter()
            .map(|m| &m.path)
            .chain(copy.project.iter())
            .chain(copy.print.iter())
        {
            assert!(!saved.read_file(&plate.id, path).unwrap().is_empty());
        }
        assert_eq!(
            saved
                .read_file(&plate.id, copy.print.as_ref().unwrap())
                .unwrap(),
            bytes
        );
        assert!(
            freeze(
                &source,
                jobs.path(),
                &plate.id,
                &plate.revision,
                3,
                crate::profiles::PRINTER
            )
            .is_err()
        );
    }
}
