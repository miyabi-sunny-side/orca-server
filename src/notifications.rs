//! Durable, bounded delivery of print-completion messages.
use crate::{
    database::Database,
    plates::{Error, Result, Store},
};
use reqwest::{Client, Url};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    sync::atomic::Ordering,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const MAX_TRIES: u8 = 3;

fn webhook_url(value: &str) -> Result<Url> {
    let invalid = || Error::Invalid("Invalid DISCORD_WEBHOOK_URL");
    let url = Url::parse(value).map_err(|_| invalid())?;
    if value.trim() != value
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    let (id, token) = match url.scheme() {
        "discord" if url.path().is_empty() || url.path() == "/" => {
            (url.host_str().unwrap_or(""), url.username())
        }
        "https" if url.host_str() == Some("discord.com") && url.username().is_empty() => {
            let parts: Vec<_> = url.path().split('/').collect();
            if parts.len() != 5 || parts[1..3] != ["api", "webhooks"] {
                return Err(invalid());
            }
            (parts[3], parts[4])
        }
        _ => return Err(invalid()),
    };
    if id.is_empty()
        || id.len() > 32
        || !id.bytes().all(|b| b.is_ascii_digit())
        || token.is_empty()
        || token.len() > 256
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(invalid());
    }
    Url::parse(&format!(
        "https://discord.com/api/webhooks/{id}/{token}?wait=true"
    ))
    .map_err(|_| invalid())
}
fn public_url(value: &str) -> Result<Url> {
    let invalid = || Error::Invalid("Invalid ORCA_PUBLIC_URL");
    let mut url = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    if !url.path().ends_with('/') {
        url.path_segments_mut().map_err(|()| invalid())?.push("");
    }
    if url.as_str().len() > 768 {
        return Err(invalid());
    }
    Ok(url)
}
struct Event {
    attempt_id: String,
    job_id: String,
    printer_id: String,
    printer: String,
    plate: String,
    tries: u8,
}
fn payload(event: &Event, base: Option<&Url>) -> Value {
    fn name(value: &str) -> String {
        let mut result = String::new();
        for c in value.chars().take(200) {
            if c.is_control() {
                result.push(' ');
            } else if c == '@' {
                result.push('＠');
            } else {
                if "\\`*_~|<>[]()".contains(c) {
                    result.push('\\');
                }
                result.push(c);
            }
        }
        result
    }
    let mut content = format!(
        "印刷完了 · 取り外し待ち\nプリンター: {}\nプレート: {}\nジョブ: {}",
        name(&event.printer),
        name(&event.plate),
        event.job_id
    );
    if let Some(base) = base {
        let mut link = base.join("queue").expect("validated public URL");
        link.query_pairs_mut()
            .append_pair("printer_id", &event.printer_id);
        content.push('\n');
        content.push_str(link.as_str());
    }
    json!({"content":content,"allowed_mentions":{"parse":[]}})
}
#[derive(Debug, PartialEq)]
enum Outcome {
    Sent(String),
    Retry(f64),
    Unknown,
    Failed,
}
fn outcome(status: u16, body: &[u8], retry: Option<&str>) -> Outcome {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if status == 429 {
        let valid = |v: &f64| v.is_finite() && *v >= 0.0;
        let seconds = retry
            .and_then(|v| v.parse::<f64>().ok())
            .filter(valid)
            .or_else(|| value["retry_after"].as_f64().filter(valid))
            .unwrap_or(60.0);
        return Outcome::Retry(seconds);
    }
    if status >= 500 {
        return Outcome::Retry(0.0);
    }
    if (200..300).contains(&status) {
        return value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit()))
            .map_or(Outcome::Unknown, |id| Outcome::Sent(id.into()));
    }
    Outcome::Failed
}
fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}
pub(crate) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE print_notifications (
        attempt_id TEXT PRIMARY KEY, job_id TEXT NOT NULL, printer_id TEXT NOT NULL,
        printer_name TEXT NOT NULL, plate_name TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','unknown','sent','failed')),
        tries INTEGER NOT NULL DEFAULT 0 CHECK(tries BETWEEN 0 AND 3), next_at INTEGER NOT NULL DEFAULT 0,
        result TEXT, message_id TEXT
    ); PRAGMA user_version=8;")?;
    Ok(())
}
fn env(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(Error::Invalid(
            "Notification environment variables must be UTF-8",
        )),
    }
}
/// Start the completion sender when configured, before connecting printers.
/// # Errors
/// Rejects invalid configuration or unavailable notification storage.
pub fn start(store: &Store) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let Some(value) = env("DISCORD_WEBHOOK_URL")? else {
        return Ok(None);
    };
    let webhook = webhook_url(&value)?;
    let public = env("ORCA_PUBLIC_URL")?
        .as_deref()
        .map(public_url)
        .transpose()?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| Error::Invalid("Cannot initialize Discord notification client"))?;
    spawn(store.db.clone(), webhook, public, client).map(Some)
}
fn spawn(
    db: Database,
    webhook: Url,
    public: Option<Url>,
    client: Client,
) -> Result<tokio::task::JoinHandle<()>> {
    // A killed process may have sent the message before saving its acknowledgement.
    db.connection()?.execute("UPDATE print_notifications SET state='unknown',result='interrupted',next_at=MAX(next_at,?1) WHERE state='sending'",[now_ms().saturating_add(2000)])?;
    db.notifications_enabled.store(true, Ordering::Relaxed);
    Ok(tokio::spawn(async move {
        loop {
            let delay = if let Ok(delay) = deliver(&db, &webhook, public.as_ref(), &client).await {
                delay
            } else {
                tracing::warn!("Notification storage unavailable; delivery will retry");
                Duration::from_secs(5)
            };
            tokio::time::sleep(delay).await;
        }
    }))
}
fn reserve(db: &Database) -> Result<(Option<Event>, Duration)> {
    let mut c = db.connection()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = now_ms();
    // Keep a webhook-wide 429 pause across new jobs and process restarts.
    let pause: i64 = tx.query_row(
        "SELECT COALESCE(MAX(next_at),0) FROM print_notifications WHERE result='429'",
        [],
        |r| r.get(0),
    )?;
    let row=tx.query_row("SELECT attempt_id,job_id,printer_id,printer_name,plate_name,tries,next_at FROM print_notifications WHERE state IN ('pending','unknown') AND tries<3 ORDER BY rowid LIMIT 1",[],|r|Ok((Event{attempt_id:r.get(0)?,job_id:r.get(1)?,printer_id:r.get(2)?,printer:r.get(3)?,plate:r.get(4)?,tries:r.get(5)?},r.get::<_,i64>(6)?))).optional()?;
    let Some((mut event, due)) = row else {
        return Ok((None, Duration::from_secs(1)));
    };
    let wait = due.max(pause).saturating_sub(now);
    if wait > 0 {
        return Ok((None, Duration::from_millis(wait.unsigned_abs().min(60_000))));
    }
    event.tries += 1;
    tx.execute(
        "UPDATE print_notifications SET state='sending',tries=?1 WHERE attempt_id=?2",
        params![event.tries, event.attempt_id],
    )?;
    tx.commit()?;
    Ok((Some(event), Duration::ZERO))
}
async fn response(client: &Client, webhook: &Url, body: Value) -> (Outcome, String) {
    let Ok(mut response) = client
        .post(webhook.clone())
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
    else {
        return (Outcome::Unknown, "network".into());
    };
    let status = response.status().as_u16();
    let retry = response
        .headers()
        .get("retry-after")
        .and_then(|s| s.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= 64 * 1024 => {
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            _ => return (Outcome::Unknown, "response".into()),
        }
    }
    (
        outcome(status, &bytes, retry.as_deref()),
        status.to_string(),
    )
}
async fn deliver(
    db: &Database,
    webhook: &Url,
    public: Option<&Url>,
    client: &Client,
) -> Result<Duration> {
    let (event, wait) = reserve(db)?;
    let Some(event) = event else { return Ok(wait) };
    let (outcome, result) = response(client, webhook, payload(&event, public)).await;
    let backoff = Duration::from_secs(2u64.pow(u32::from(event.tries)));
    let (state, delay, message) = match outcome {
        Outcome::Sent(id) => ("sent", Duration::ZERO, Some(id)),
        Outcome::Failed => ("failed", Duration::ZERO, None),
        Outcome::Unknown => ("unknown", backoff, None),
        Outcome::Retry(seconds) => (
            if event.tries == MAX_TRIES {
                "failed"
            } else {
                "pending"
            },
            if seconds > 0.0 {
                Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX)
            } else {
                backoff
            },
            None,
        ),
    };
    let next = now_ms().saturating_add(i64::try_from(delay.as_millis()).unwrap_or(i64::MAX));
    loop {
        let saved = db.connection().and_then(|c| {
            c.execute("UPDATE print_notifications SET state=?1,result=?2,message_id=?3,next_at=?4 WHERE attempt_id=?5 AND state='sending'",params![state,result,message,next,event.attempt_id]).map_err(Error::from)
        });
        if saved.is_ok() {
            break;
        }
        // Retain the acknowledgement in memory; a storage failure must not resend it.
        tracing::warn!("Notification result storage unavailable; saving will retry");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    tracing::info!(attempt=%event.attempt_id,state,tries=event.tries,"Discord completion notification delivery recorded");
    Ok(Duration::from_secs(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    // Subprocess entrypoint for the Rust delivery test; this is a fixture service, not a skipped assertion.
    #[tokio::test]
    #[ignore = "subprocess service used by delivery_tests::isolated_notification_delivery"]
    async fn fixture_service() {
        tracing_subscriber::fmt().with_ansi(false).init();
        let root = std::path::PathBuf::from(std::env::var_os("PLATES_DIR").unwrap());
        let store = Store::open(&root).unwrap();
        if std::env::var("NOTIFICATION_TEST_ENABLED").unwrap() == "1" {
            let cert = std::fs::read(std::env::var("NOTIFICATION_TEST_CERT").unwrap()).unwrap();
            let client = Client::builder()
                .add_root_certificate(reqwest::Certificate::from_pem(&cert).unwrap())
                .timeout(Duration::from_millis(500))
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .unwrap();
            spawn(
                store.db.clone(),
                Url::parse(&std::env::var("NOTIFICATION_TEST_URL").unwrap()).unwrap(),
                Some(public_url("https://orca.example/").unwrap()),
                client,
            )
            .unwrap();
        } else {
            assert!(start(&store).unwrap().is_none());
        }
        let source = crate::scad::Source::new(&std::env::var("SCAD_LIVE_URL").unwrap()).unwrap();
        let slicer = crate::slicer::Slicer::new(
            std::env::var_os("ORCA_APPDIR").unwrap().into(),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let registry = crate::registry::router(&root, store, Some(slicer), Some(source)).unwrap();
        let listener =
            tokio::net::TcpListener::bind(format!("127.0.0.1:{}", std::env::var("PORT").unwrap()))
                .await
                .unwrap();
        axum::serve(listener, crate::with_mcp(registry))
            .await
            .unwrap();
    }
    #[test]
    fn configured_webhooks_normalize_without_forwarding_other_targets() {
        let expected = "https://discord.com/api/webhooks/123456/fixture_token-1?wait=true";
        assert_eq!(
            webhook_url("https://discord.com/api/webhooks/123456/fixture_token-1")
                .unwrap()
                .as_str(),
            expected
        );
        assert_eq!(
            webhook_url("discord://fixture_token-1@123456")
                .unwrap()
                .as_str(),
            expected
        );
        for bad in [
            "http://discord.com/api/webhooks/123456/fixture",
            "https://elsewhere.invalid/api/webhooks/123456/fixture",
            "https://discord.com.evil.test/api/webhooks/123456/fixture",
            "https://user@discord.com/api/webhooks/123456/fixture",
            "https://discord.com:444/api/webhooks/123456/fixture",
            "https://discord.com/api/webhooks/123456/fixture?thread_id=1",
            "discord://fixture@123456?foo=1",
            "discord://fixture@name",
            "discord://fixture:secret@123456",
            "discord://fixture@123456/extra",
            "discord://fixture%2Fsecret@123456",
            "https://discord.com/api/webhooks/123456/%0Asecret",
            " discord://fixture@123456",
        ] {
            let error = webhook_url(bad).expect_err("invalid endpoint");
            assert!(!format!("{error:?}").contains("fixture"));
        }
    }
    #[test]
    fn content_identifies_the_job_without_mentions_or_markdown_from_names() {
        let event = Event {
            attempt_id: "attempt".into(),
            job_id: "job-123".into(),
            printer_id: "printer / one".into(),
            printer: "@everyone **desk**".into(),
            plate: "<@123>\n[click](https://evil.test) `x`".into(),
            tries: 0,
        };
        let base = public_url("https://printer.example/tools/").unwrap();
        let value = payload(&event, Some(&base));
        let content = value["content"].as_str().unwrap();
        assert!(content.contains("取り外し待ち") && content.contains("job-123"));
        assert!(
            !content.contains("@everyone")
                && !content.contains("<@123>")
                && !content.contains("[click](")
        );
        assert!(content.contains("https://printer.example/tools/queue?printer_id=printer+%2F+one"));
        assert_eq!(value["allowed_mentions"]["parse"], json!([]));
        let large = Event {
            plate: "長".repeat(2000),
            ..event
        };
        assert!(
            payload(&large, None)["content"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                < 2000
        );
        assert!(public_url(&format!("https://example.com/{}", "a".repeat(1100))).is_err());
        for bad in [
            "file:///tmp/queue",
            "https://user:secret@host/",
            "http://host/?token=secret",
            "http://host/#fragment",
        ] {
            assert!(public_url(bad).is_err());
        }
    }
    #[test]
    fn acknowledgement_and_retry_classification_keep_uncertainty_visible() {
        assert_eq!(
            outcome(200, br#"{"id":"123456"}"#, None),
            Outcome::Sent("123456".into())
        );
        for (status, body) in [
            (200, b"not json".as_slice()),
            (200, b"{}".as_slice()),
            (204, b"".as_slice()),
        ] {
            assert_eq!(outcome(status, body, None), Outcome::Unknown);
        }
        assert_eq!(
            outcome(429, br#"{"retry_after":0.25}"#, None),
            Outcome::Retry(0.25)
        );
        assert_eq!(outcome(429, b"invalid", Some("2.5")), Outcome::Retry(2.5));
        assert_eq!(
            outcome(429, br#"{"retry_after":-1}"#, Some("NaN")),
            Outcome::Retry(60.0)
        );
        assert_eq!(outcome(429, b"{}", Some("86401")), Outcome::Retry(86401.0));
        assert_eq!(
            outcome(429, br#"{"retry_after":0.25}"#, Some("NaN")),
            Outcome::Retry(0.25)
        );
        assert_eq!(outcome(503, b"secret text", None), Outcome::Retry(0.0));
        for status in [301, 400, 401, 403, 404] {
            assert_eq!(outcome(status, b"secret text", None), Outcome::Failed);
        }
    }
}

#[cfg(test)]
#[path = "../tests/scenarios/notifications.rs"]
mod delivery_tests;
