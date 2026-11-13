//! Official Bambu Lab descriptions of `print_error` and HMS codes.
//!
//! `BambuStudio` downloads the same table per device type (the first three characters of the
//! serial number) from `e.bambulab.com/query.php`. The answer is kept as received under
//! `<PLATES_DIR>/codes/`, so descriptions survive restarts and an unreachable site. Only the
//! language and the device type are sent; a code the table lacks simply has no description.

use reqwest::Client;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, PoisonError, RwLock},
    time::{Duration, SystemTime},
};
use tokio::sync::Notify;

const OFFICIAL: &str = "https://e.bambulab.com/query.php";
const LANGUAGE: &str = "ja";
/// The published table changes with firmware releases; a week keeps it current.
const MAX_AGE: Duration = Duration::from_hours(7 * 24);
/// How long an unreachable site or a failed save waits before the next try.
const RETRY: Duration = Duration::from_hours(1);
// The latest stable Chrome on Windows: the site sits behind a bot filter.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";

#[derive(Debug, Default, PartialEq, Eq)]
struct Table {
    /// `print_error` and start `err_code`, by eight hex digits.
    errors: HashMap<String, String>,
    /// HMS, by sixteen hex digits.
    hms: HashMap<String, String>,
}

struct Codes {
    dir: PathBuf,
    /// `None` when fetching is switched off; saved tables are still used.
    url: Option<String>,
    tables: RwLock<HashMap<String, Arc<Table>>>,
    /// Device types of the configured printers.
    wanted: RwLock<HashSet<String>>,
    changed: Notify,
}

static CODES: OnceLock<Codes> = OnceLock::new();

/// The device type `BambuStudio` sends: the first three characters of the serial number.
#[must_use]
pub fn model(serial: &str) -> Option<String> {
    let model = serial.get(..3)?;
    model
        .bytes()
        .all(|b| b.is_ascii_alphanumeric())
        .then(|| model.to_ascii_uppercase())
}

/// Hex digits of a displayed code: `0300-4000` and `0300_0D00_0002_0001` lose their separators.
fn key(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Read the official answer. `None` unless it carries a table for this language.
fn parse(text: &str) -> Option<Table> {
    let value: Value = serde_json::from_str(text).ok()?;
    let list = |name: &str| -> HashMap<String, String> {
        value["data"][name][LANGUAGE]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let (code, intro) = (row["ecode"].as_str()?, row["intro"].as_str()?.trim());
                (!intro.is_empty()).then(|| (key(code), intro.to_owned()))
            })
            .collect()
    };
    let table = Table {
        errors: list("device_error"),
        hms: list("device_hms"),
    };
    (!table.errors.is_empty() || !table.hms.is_empty()).then_some(table)
}

impl Table {
    /// The description of a code as displayed, chosen by its length as `BambuStudio` does.
    fn describe(&self, code: &str) -> Option<&str> {
        let key = key(code);
        match key.len() {
            8 => self.errors.get(&key),
            16 => self.hms.get(&key),
            _ => None,
        }
        .map(String::as_str)
    }
}

/// Whether a table saved at `saved` is due for another download at `now`.
fn stale(saved: Option<SystemTime>, now: SystemTime) -> bool {
    saved.is_none_or(|saved| now.duration_since(saved).is_ok_and(|age| age >= MAX_AGE))
}

fn file(dir: &Path, model: &str) -> PathBuf {
    dir.join(format!("{LANGUAGE}_{model}.json"))
}

/// Use the tables saved under `root` and keep them current for the registered printers.
/// `BAMBU_ERROR_TABLE_URL` replaces the official address; `off` stops downloading.
pub fn open(root: &Path) {
    let url = match std::env::var("BAMBU_ERROR_TABLE_URL") {
        Ok(value) if value == "off" => None,
        Ok(value) if !value.is_empty() => Some(value),
        _ => Some(OFFICIAL.to_owned()),
    };
    let dir = root.join("codes");
    let mut tables = HashMap::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let Some(model) = name
            .to_str()
            .and_then(|name| name.strip_prefix(&format!("{LANGUAGE}_")))
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if let Some(table) = std::fs::read_to_string(entry.path())
            .ok()
            .as_deref()
            .and_then(parse)
        {
            tables.insert(model.to_owned(), Arc::new(table));
        }
    }
    let mut first = false;
    let codes = CODES.get_or_init(|| {
        first = true;
        Codes {
            dir,
            url,
            tables: RwLock::new(tables),
            wanted: RwLock::default(),
            changed: Notify::new(),
        }
    });
    if first {
        tokio::spawn(keep_current(codes));
    }
}

/// Keep the table of this printer's device type; called for every configured printer.
pub fn register(serial: &str) {
    if let (Some(codes), Some(model)) = (CODES.get(), model(serial))
        && codes
            .wanted
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(model)
    {
        codes.changed.notify_one();
    }
}

fn table(model: Option<&str>) -> Option<Arc<Table>> {
    CODES
        .get()?
        .tables
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(model?)
        .cloned()
}

/// The official description of one displayed code (`0300-4000`, `0300_0D00_0002_0001`).
#[must_use]
pub fn describe(model: Option<&str>, code: &str) -> Option<String> {
    table(model)?.describe(code).map(str::to_owned)
}

/// Descriptions of the codes a printer reports now, by the code as displayed.
#[must_use]
pub fn reported(model: Option<&str>, error: Option<u64>, hms: &Value) -> BTreeMap<String, String> {
    let codes = error
        .filter(|&code| code != 0)
        .map(crate::print_start::error_code)
        .into_iter()
        .chain(
            hms.as_array()
                .into_iter()
                .flatten()
                .filter_map(|code| code.as_str().map(str::to_owned)),
        );
    let Some(table) = table(model) else {
        return BTreeMap::new();
    };
    codes
        .filter_map(|code| {
            let text = table.describe(&code)?.to_owned();
            Some((code, text))
        })
        .collect()
}

async fn keep_current(codes: &'static Codes) {
    let Some(url) = &codes.url else {
        return;
    };
    let Ok(client) = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
    else {
        tracing::warn!("Error code table client could not start; codes stay without descriptions");
        return;
    };
    // Report a failure once per device type, and its recovery once.
    let mut failing: HashSet<String> = HashSet::new();
    let mut tried: HashMap<String, SystemTime> = HashMap::new();
    loop {
        let wanted: Vec<String> = codes
            .wanted
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        for model in wanted {
            let path = file(&codes.dir, &model);
            let saved = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let now = SystemTime::now();
            let recently = tried
                .get(&model)
                .is_some_and(|at| now.duration_since(*at).is_ok_and(|age| age < RETRY));
            let loaded = codes
                .tables
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(&model);
            if (loaded && !stale(saved, now)) || recently {
                continue;
            }
            tried.insert(model.clone(), now);
            match download(&client, url, &model).await {
                Ok((text, table)) => {
                    let (errors, hms) = (table.errors.len(), table.hms.len());
                    codes
                        .tables
                        .write()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(model.clone(), Arc::new(table));
                    let saved = save(&path, &text);
                    failing.remove(&model);
                    tracing::info!(model, errors, hms, saved, "Error code table updated");
                }
                Err(kind) => {
                    if failing.insert(model.clone()) {
                        tracing::warn!(
                            model,
                            kind,
                            "Error code table could not be downloaded; retrying hourly"
                        );
                    }
                }
            }
        }
        tokio::select! {
            () = codes.changed.notified() => {}
            () = tokio::time::sleep(Duration::from_mins(10)) => {}
        }
    }
}

async fn download(
    client: &Client,
    url: &str,
    model: &str,
) -> Result<(String, Table), &'static str> {
    let mut url = reqwest::Url::parse(url).map_err(|_| "address")?;
    url.query_pairs_mut()
        .append_pair("lang", LANGUAGE)
        .append_pair("d", model);
    let response = client.get(url).send().await.map_err(|_| "network")?;
    if !response.status().is_success() {
        return Err("status");
    }
    let text = response.text().await.map_err(|_| "body")?;
    let table = parse(&text).ok_or("content")?;
    Ok((text, table))
}

/// Replace the saved answer in one step, so a crash never leaves half a table.
fn save(path: &Path, text: &str) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let temporary = path.with_extension("json.tmp");
    std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&temporary, text).is_ok()
        && std::fs::rename(&temporary, path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{"result":0,"t":1,"ver":202611121903,"data":{
        "device_hms":{"ver":1,"ja":[
            {"ecode":"03000D0000020001","intro":"ベッドの原点復帰に異常があります。"},
            {"ecode":"0300400000020001","intro":"  シリアル通信にデータ異常があります。 "}]},
        "device_error":{"ver":1,"ja":[
            {"ecode":"03004000","intro":"Z軸の原点復帰に失敗しました。タスクは停止されました。"},
            {"ecode":"0500c011","intro":""},
            {"ecode":"0500C026"}]}}}"#;

    #[test]
    fn the_device_type_is_the_serial_prefix() {
        assert_eq!(model("01P00A000000000").as_deref(), Some("01P"));
        assert_eq!(model("03w").as_deref(), Some("03W"));
        assert_eq!(model("01"), None);
        assert_eq!(model("0/P00A"), None, "never sent as a query by accident");
        assert_eq!(model("日本語のシリアル"), None);
    }

    #[test]
    fn the_official_answer_describes_displayed_codes() {
        let table = parse(ANSWER).expect("the answer has a table");
        assert_eq!(
            table.describe("0300-4000"),
            Some("Z軸の原点復帰に失敗しました。タスクは停止されました。")
        );
        assert_eq!(
            table.describe("0300_0D00_0002_0001"),
            Some("ベッドの原点復帰に異常があります。")
        );
        assert_eq!(
            table.describe("0300_4000_0002_0001"),
            Some("シリアル通信にデータ異常があります。"),
            "a sixteen digit code is never read as the eight digit one"
        );
        assert_eq!(table.describe("0300-8010"), None, "an unlisted code");
        assert_eq!(table.describe("0500-C011"), None, "an empty description");
        assert_eq!(table.describe("0500-C026"), None, "a row without text");
        assert_eq!(table.describe(""), None);
    }

    #[test]
    fn answers_without_a_table_are_not_kept() {
        for text in [
            r#"{"result":201,"t":1,"ver":0,"msg":""}"#,
            r#"{"data":{"device_error":{"en":[{"ecode":"03004000","intro":"x"}]}}}"#,
            "<html>blocked</html>",
            "",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn a_saved_table_is_downloaded_again_after_a_week() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_hours(1000);
        assert!(stale(None, now), "nothing saved yet");
        assert!(!stale(Some(now - Duration::from_hours(167)), now));
        assert!(stale(Some(now - Duration::from_hours(168)), now));
        assert!(
            !stale(Some(now + Duration::from_hours(1)), now),
            "a clock set back does not start a download loop"
        );
    }
}
