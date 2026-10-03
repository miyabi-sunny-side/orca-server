//! Persistent record of printer traffic and warnings, readable over HTTP and replayable.
//!
//! Entries are JSON lines in size-capped segment files under `<PLATES_DIR>/journal/`.
//! Known secrets and credential-named fields are replaced before anything reaches disk.

use serde_json::{Map, Value, json};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

const REDACTED: &str = "[redacted]";
const DEFAULT_MAX_BYTES: u64 = 128 * 1024 * 1024;
/// Segments per cap: deleting the oldest frees about an eighth of the journal.
const SEGMENTS: u64 = 8;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;

fn redact_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                let key = key.to_ascii_lowercase();
                if ["access_code", "password", "passwd"]
                    .iter()
                    .any(|name| key.contains(name))
                {
                    *field = Value::String(REDACTED.into());
                } else {
                    redact_fields(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_fields),
        _ => {}
    }
}

/// One JSON line. Credential-named fields and registered secrets never reach the result.
pub(crate) fn line(seq: u64, at: u64, mut fields: Value, secrets: &[String]) -> String {
    redact_fields(&mut fields);
    if let Some(map) = fields.as_object_mut() {
        map.insert("seq".into(), seq.into());
        map.insert("at".into(), at.into());
    }
    let mut text = fields.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        let quoted = Value::String(secret.clone()).to_string();
        text = text.replace(&quoted[1..quoted.len() - 1], REDACTED);
    }
    text
}

/// Which entries a reader asked for.
#[derive(Default, serde::Deserialize)]
pub struct Filter {
    pub printer: Option<String>,
    pub kind: Option<String>,
    pub direction: Option<String>,
    /// Inclusive bounds in Unix milliseconds.
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub limit: Option<usize>,
}

impl Filter {
    fn matches(&self, entry: &Value) -> bool {
        let same = |wanted: &Option<String>, key: &str| {
            wanted
                .as_deref()
                .is_none_or(|w| entry.get(key).and_then(Value::as_str) == Some(w))
        };
        let at = entry.get("at").and_then(Value::as_u64).unwrap_or(0);
        same(&self.printer, "printer")
            && same(&self.kind, "kind")
            && same(&self.direction, "dir")
            && self.since.is_none_or(|t| at >= t)
            && self.until.is_none_or(|t| at <= t)
    }
}

/// Oldest segments to delete so the total fits `max`; the newest (current) one is always kept.
fn prune(segments: &[(u64, u64)], max: u64) -> Vec<u64> {
    let mut total: u64 = segments.iter().map(|s| s.1).sum();
    let mut removed = Vec::new();
    for &(first, size) in &segments[..segments.len().saturating_sub(1)] {
        if total <= max {
            break;
        }
        total -= size;
        removed.push(first);
    }
    removed
}

/// Received report payloads, in order, from journal lines or a `GET /api/journal` response.
#[must_use]
pub fn reports(text: &str) -> Vec<Vec<u8>> {
    let entries = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(mut response)) if response.get("entries").is_some_and(Value::is_array) => {
            match response.remove("entries") {
                Some(Value::Array(entries)) => entries,
                _ => unreachable!("checked above"),
            }
        }
        _ => text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
    };
    entries
        .into_iter()
        .filter(|e| e["kind"] == "report" && e["dir"] == "in")
        .map(|mut e| match e["body"].take() {
            Value::String(raw) => raw.into_bytes(),
            body => body.to_string().into_bytes(),
        })
        .collect()
}

struct Writer {
    file: Option<fs::File>,
    next: u64,
    /// `(first seq, bytes)` per segment file, oldest first; the last one is being written.
    segments: Vec<(u64, u64)>,
    failing: bool,
}

struct Journal {
    dir: PathBuf,
    max: u64,
    writer: Mutex<Writer>,
    secrets: Mutex<Vec<String>>,
}

static JOURNAL: OnceLock<Journal> = OnceLock::new();

fn segment(dir: &Path, first: u64) -> PathBuf {
    dir.join(format!("{first:020}.jsonl"))
}

fn segments(dir: &Path) -> io::Result<Vec<(u64, u64)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(first) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".jsonl"))
            .and_then(|n| n.parse().ok())
        {
            found.push((first, entry.metadata()?.len()));
        }
    }
    found.sort_unstable();
    Ok(found)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Open the journal under `root` once per process; `JOURNAL_MAX_BYTES` caps its disk use.
/// # Errors
/// Rejects an invalid cap or an unusable directory.
pub fn open(root: &Path) -> io::Result<()> {
    let max = match std::env::var("JOURNAL_MAX_BYTES") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|v| *v >= SEGMENTS)
            .ok_or_else(|| {
                io::Error::other("JOURNAL_MAX_BYTES must be an integer of at least 8")
            })?,
        Err(std::env::VarError::NotPresent) => DEFAULT_MAX_BYTES,
        Err(_) => return Err(io::Error::other("JOURNAL_MAX_BYTES must be UTF-8")),
    };
    let dir = root.join("journal");
    fs::create_dir_all(&dir)?;
    let segments = segments(&dir)?;
    // Continue numbering after the last complete line; a torn tail line is skipped by readers.
    let next = match segments.last() {
        Some(&(first, _)) => fs::read_to_string(segment(&dir, first))?
            .lines()
            .rev()
            .find_map(|l| serde_json::from_str::<Value>(l).ok()?["seq"].as_u64())
            .map_or(first, |seq| seq + 1),
        None => 1,
    };
    let journal = Journal {
        dir,
        max,
        writer: Mutex::new(Writer {
            file: None,
            next,
            segments,
            failing: false,
        }),
        secrets: Mutex::default(),
    };
    JOURNAL
        .set(journal)
        .map_err(|_| io::Error::other("journal already open"))
}

impl Journal {
    fn append(&self, fields: Value) {
        let secrets = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let result = self.write(&mut writer, fields, &secrets);
        // Never trace from here: the warning layer writes back into this journal.
        match result {
            Err(error) if !writer.failing => {
                writer.failing = true;
                eprintln!("journal write failed: {error}");
            }
            Ok(()) if writer.failing => {
                writer.failing = false;
                eprintln!("journal writes recovered");
            }
            _ => {}
        }
    }

    fn write(&self, w: &mut Writer, fields: Value, secrets: &[String]) -> io::Result<()> {
        let full = w.segments.last().is_none_or(|s| s.1 >= self.max / SEGMENTS);
        if w.file.is_none() || full {
            // Each process starts a fresh segment, so a torn tail is never appended to.
            w.file = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(segment(&self.dir, w.next))?,
            );
            w.segments.push((w.next, 0));
        }
        let mut text = line(w.next, now_ms(), fields, secrets);
        text.push('\n');
        w.file
            .as_mut()
            .expect("opened above")
            .write_all(text.as_bytes())?;
        w.next += 1;
        if let Some(current) = w.segments.last_mut() {
            current.1 += text.len() as u64;
        }
        for first in prune(&w.segments, self.max) {
            match fs::remove_file(segment(&self.dir, first)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            w.segments.retain(|s| s.0 != first);
        }
        Ok(())
    }
}

/// Record one entry; a no-op until [`open`] succeeds.
pub(crate) fn record(fields: Value) {
    if let Some(journal) = JOURNAL.get() {
        journal.append(fields);
    }
}

/// Never write `value` to the journal from now on.
pub(crate) fn secret(value: &str) {
    if let Some(journal) = JOURNAL.get()
        && !value.is_empty()
    {
        let mut secrets = journal
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !secrets.iter().any(|s| s == value) {
            secrets.push(value.to_owned());
        }
    }
}

/// A raw payload as JSON when it parses, otherwise as lossy text.
fn payload(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
}

/// A report received from a printer.
pub(crate) fn report(printer: &str, epoch: u64, bytes: &[u8]) {
    record(
        json!({"printer":printer,"epoch":epoch,"kind":"report","dir":"in","body":payload(bytes)}),
    );
}

/// A request published to a printer, and whether the client queued it.
pub(crate) fn request(printer: &str, epoch: u64, bytes: &[u8], queued: bool) {
    record(
        json!({"printer":printer,"epoch":epoch,"kind":"request","dir":"out","queued":queued,"body":payload(bytes)}),
    );
}

/// A non-message event such as `connection`, `ftps` or `cli`.
pub(crate) fn event(printer: &str, epoch: Option<u64>, kind: &str, body: Value) {
    let mut fields = json!({"printer":printer,"epoch":epoch,"kind":kind});
    fields["body"] = body;
    record(fields);
}

/// Newest matching entries, oldest first, and whether older matches were left out.
fn read(filter: &Filter, limit: usize) -> io::Result<(Vec<Value>, bool)> {
    let Some(journal) = JOURNAL.get() else {
        return Ok((Vec::new(), false));
    };
    let mut found = Vec::new();
    for (first, _) in segments(&journal.dir)?.into_iter().rev() {
        let text = match fs::read_to_string(segment(&journal.dir, first)) {
            Ok(text) => text,
            // Pruned while reading.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str(l).ok())
        {
            if filter.matches(&entry) {
                if found.len() == limit {
                    found.reverse();
                    return Ok((found, true));
                }
                found.push(entry);
            }
        }
    }
    found.reverse();
    Ok((found, false))
}

/// `GET /api/journal`: filter by printer, kind, direction (`in`/`out`) and time; newest `limit`.
pub(crate) async fn api(
    axum::extract::Query(filter): axum::extract::Query<Filter>,
) -> crate::plates::Result<axum::Json<Value>> {
    use crate::plates::Error;
    if filter
        .direction
        .as_deref()
        .is_some_and(|d| d != "in" && d != "out")
    {
        return Err(Error::Invalid("direction must be in or out"));
    }
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid("limit must be 1-1000"));
    }
    let (entries, truncated) = crate::plate_api::blocking(move || {
        read(&filter, limit).map_err(|_| Error::Unavailable("Cannot read the journal"))
    })
    .await?;
    Ok(axum::Json(json!({"entries":entries,"truncated":truncated})))
}

/// Records warnings and errors from this application (not protocol libraries) to the journal.
pub struct Layer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if JOURNAL.get().is_none() {
            return;
        }
        let mut fields = Map::new();
        event.record(&mut Fields(&mut fields));
        let printer = fields.remove("printer");
        let meta = event.metadata();
        record(
            json!({"kind":"log","level":meta.level().as_str(),"target":meta.target(),"printer":printer,"body":fields}),
        );
    }
}

struct Fields<'a>(&'a mut Map<String, Value>);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lines_drop_registered_secrets_and_credential_fields() {
        let secrets = vec![
            "code-1234".to_owned(),
            "https://hook.example/a\"b".to_owned(),
        ];
        let text = line(
            7,
            1_000,
            json!({"kind":"report","printer":"p1","body":{"print":{
                "access_code":"other-value","nested":{"Password":"pw"},
                "note":"x code-1234 y","url":"https://hook.example/a\"b"}}}),
            &secrets,
        );
        assert!(!text.contains('\n'));
        for secret in ["code-1234", "other-value", "pw\"", "hook.example"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["seq"], 7);
        assert_eq!(value["at"], 1_000);
        assert_eq!(value["printer"], "p1");
        assert_eq!(value["body"]["print"]["access_code"], REDACTED);
        assert_eq!(value["body"]["print"]["nested"]["Password"], REDACTED);
        assert_eq!(value["body"]["print"]["note"], "x [redacted] y");
        assert_eq!(value["body"]["print"]["url"], REDACTED);
        // Empty secrets would redact everything; they are ignored.
        let plain = line(1, 2, json!({"body":"abc"}), &[String::new()]);
        assert_eq!(
            serde_json::from_str::<Value>(&plain).unwrap()["body"],
            "abc"
        );
    }

    #[test]
    fn filters_combine_printer_kind_direction_and_inclusive_time() {
        let entry = json!({"at":100,"printer":"p1","kind":"report","dir":"in"});
        let event = json!({"at":100,"printer":"p1","kind":"connection"});
        let f = |q: Value| serde_json::from_value::<Filter>(q).unwrap();
        assert!(f(json!({})).matches(&entry));
        assert!(
            f(json!({"printer":"p1","kind":"report","direction":"in","since":100,"until":100}))
                .matches(&entry)
        );
        for miss in [
            json!({"printer":"p2"}),
            json!({"kind":"request"}),
            json!({"direction":"out"}),
            json!({"since":101}),
            json!({"until":99}),
        ] {
            assert!(!f(miss.clone()).matches(&entry), "{miss}");
        }
        assert!(
            !f(json!({"direction":"in"})).matches(&event),
            "events have no direction"
        );
        assert!(f(json!({"kind":"connection"})).matches(&event));
    }

    #[test]
    fn pruning_removes_oldest_until_within_the_cap_but_keeps_the_current_segment() {
        let segments = [(1, 40), (5, 40), (9, 40)];
        assert_eq!(prune(&segments, 120), Vec::<u64>::new());
        assert_eq!(prune(&segments, 119), vec![1]);
        assert_eq!(prune(&segments, 80), vec![1]);
        assert_eq!(prune(&segments, 79), vec![1, 5]);
        assert_eq!(prune(&segments, 0), vec![1, 5]);
        assert_eq!(prune(&[(3, 500)], 10), Vec::<u64>::new());
        assert_eq!(prune(&[], 10), Vec::<u64>::new());
    }

    #[test]
    fn replay_reads_received_reports_in_order_from_lines_or_a_response() {
        let a = json!({"seq":1,"kind":"report","dir":"in","body":{"print":{"msg":0}}});
        let req = json!({"seq":2,"kind":"request","dir":"out","body":{"pushing":{}}});
        let raw = json!({"seq":3,"kind":"report","dir":"in","body":"not json"});
        let b = json!({"seq":4,"kind":"report","dir":"in","body":{"print":{"msg":1}}});
        let lines = format!("{a}\n{req}\n\n{raw}\n{{broken\n{b}\n");
        let expected = vec![
            br#"{"print":{"msg":0}}"#.to_vec(),
            b"not json".to_vec(),
            br#"{"print":{"msg":1}}"#.to_vec(),
        ];
        assert_eq!(reports(&lines), expected);
        let response = json!({"entries":[a, req, raw, b],"truncated":false}).to_string();
        assert_eq!(reports(&response), expected);
    }
}
