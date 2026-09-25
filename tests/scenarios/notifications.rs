//! Private notification-client injection stays in cfg(test); all external peers are loopback Rust.
#[path = "../common/mod.rs"]
mod common;
use common::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const TOKEN: &str = "synthetic-notification-token";
type Reply = (u16, Value, Duration);
struct Webhook {
    url: String,
    requests: Arc<Mutex<Vec<(Instant, Value)>>>,
    actions: Arc<Mutex<VecDeque<Reply>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Webhook {
    fn new(root: &std::path::Path) -> Self {
        let output = Command::new("openssl")
            .args([
                "req",
                "-config",
                "/dev/null",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-keyout",
            ])
            .arg(root.join("webhook.key"))
            .arg("-out")
            .arg(root.join("webhook.pem"))
            .output()
            .unwrap();
        assert!(output.status.success());
        let config = common::peers::config(&root.join("webhook.pem"), &root.join("webhook.key"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "https://localhost:{}/api/webhooks/123/{TOKEN}?wait=true",
            listener.local_addr().unwrap().port()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::new(Mutex::new(VecDeque::<Reply>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let received = requests.clone();
        let replies = actions.clone();
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((raw, _)) => {
                        let config = config.clone();
                        let received = received.clone();
                        let replies = replies.clone();
                        workers.push(thread::spawn(move||{let stream=common::peers::tls(raw,&config).unwrap();let mut reader=BufReader::new(stream);let mut line=String::new();reader.read_line(&mut line).unwrap();assert_eq!(line.trim(),format!("POST /api/webhooks/123/{TOKEN}?wait=true HTTP/1.1"));let mut length=None;loop{line.clear();reader.read_line(&mut line).unwrap();if line=="\r\n"{break;}
if let Some((key,value))=line.split_once(':')&& key.eq_ignore_ascii_case("content-length"){length=Some(value.trim().parse::<usize>().unwrap());}}let length=length.unwrap();assert!(length<1024*1024);let mut body=vec![0;length];reader.read_exact(&mut body).unwrap();let payload=serde_json::from_slice(&body).unwrap();let count={let mut received=received.lock().unwrap();received.push((Instant::now(),payload));received.len()};let(status,body,delay)=replies.lock().unwrap().pop_front().unwrap_or((200,json!({"id":(1000+count).to_string()}),Duration::ZERO));thread::sleep(delay);let data=serde_json::to_vec(&body).unwrap();let header=format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",data.len());let _=reader.get_mut().write_all(&[header.as_bytes(),&data].concat());}));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("webhook accept: {e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url,
            requests,
            actions,
            stop,
            thread: Some(thread),
        }
    }
    fn len(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn respond(&self, status: u16, body: Value, delay: f64) {
        self.actions
            .lock()
            .unwrap()
            .push_back((status, body, Duration::from_secs_f64(delay)));
    }
}
impl Drop for Webhook {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
fn rows(rig: &Rig) -> Vec<Value> {
    let db = rig.db();
    db.prepare("SELECT job_id,state,message_id,tries,result FROM print_notifications ORDER BY rowid").unwrap().query_map([],|r|Ok(json!({"job_id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"message_id":r.get::<_,Option<String>>(2)?,"tries":r.get::<_,i64>(3)?,"result":r.get::<_,Option<String>>(4)?}))).unwrap().collect::<Result<_,_>>().unwrap()
}
fn last(rig: &Rig) -> Value {
    rows(rig).last().cloned().unwrap_or(Value::Null)
}
fn start_job(rig: &mut Rig) -> Value {
    rig.idle();
    let job = rig.add(3);
    let count = rig.broker.prints().len();
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == count + 1, 12);
    job
}
fn remove_current(rig: &Rig) {
    rig.discard();
    assert!(rig.queue()["current"].is_null());
}
fn sleep(seconds: f64) {
    thread::sleep(Duration::from_secs_f64(seconds));
}
#[test]
#[allow(clippy::too_many_lines)]
fn isolated_notification_delivery() {
    let mut rig = Rig::new("notifications");
    rig.binary = std::env::current_exe().unwrap();
    rig.arguments = [
        "--exact",
        "notifications::tests::fixture_service",
        "--ignored",
        "--nocapture",
    ]
    .map(str::to_owned)
    .to_vec();
    let webhook = Webhook::new(rig.root.path());
    rig.env
        .insert("NOTIFICATION_TEST_ENABLED".into(), "1".into());
    rig.env.insert(
        "NOTIFICATION_TEST_CERT".into(),
        rig.root.path().join("webhook.pem").display().to_string(),
    );
    rig.env
        .insert("NOTIFICATION_TEST_URL".into(), webhook.url.clone());
    rig.launch();
    rig.seed();
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut plate = edit(&rig.get(&path));
    plate["name"] = json!("@everyone **fixture** <@123>");
    rig.plate = rig.put(&path, &plate, 200);
    let first = start_job(&mut rig);
    let command = rig.broker.prints().last().unwrap().clone();
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":command["sequence_id"],"result":"success"}}));
    rig.start_phase("accepted");
    let mut value = rig.full.clone();
    value["print"] = merge(
        &value["print"],
        &json!({"gcode_state":"RUNNING","mc_percent":100,"subtask_name":command["subtask_name"],"gcode_file":command["file"]}),
    );
    rig.broker.send(&value);
    rig.phase("printing");
    sleep(1.1);
    assert_eq!(webhook.len(), 0);
    assert!(rows(&rig).is_empty());
    let stopped = rig.queue()["current"]["attempt_id"].clone();
    rig.report("FAILED");
    rig.phase("needs_attention");
    rig.send(
        json!({"type":"retry","expected_job":first["id"],"cleared":true}),
        200,
    );
    until(|| rig.broker.prints().len() == 2, 12);
    let restarted = rig.queue()["current"]["attempt_id"].clone();
    assert_ne!(restarted, stopped);
    assert!(rows(&rig).is_empty());
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    until(|| last(&rig)["state"] == "sent", 12);
    let sent = rows(&rig)[0].clone();
    assert_eq!(
        rig.db()
            .query_row("SELECT attempt_id FROM print_notifications", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
        restarted.as_str().unwrap()
    );
    assert_eq!(sent["job_id"], first["id"]);
    assert_eq!(sent["message_id"], "1001");
    let payload = webhook.requests.lock().unwrap()[0].1.clone();
    let text = payload["content"].as_str().unwrap();
    assert!(
        text.contains("取り外し待ち")
            && text.contains(id(&first))
            && text.contains("https://orca.example/queue?printer_id=p1")
    );
    assert!(!text.contains("@everyone") && !text.contains("<@123>"));
    assert_eq!(payload["allowed_mentions"], json!({"parse":[]}));
    for _ in 0..3 {
        rig.report("FINISH");
    }
    rig.stop(false);
    rig.launch();
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    sleep(1.2);
    assert_eq!(webhook.len(), 1);
    assert_eq!(rows(&rig)[0]["tries"], 1);
    remove_current(&rig);
    assert_eq!(rows(&rig).len(), 1);
    webhook.respond(200, json!({"id":"2000"}), 1.2);
    let second = start_job(&mut rig);
    rig.finish();
    until(|| webhook.len() == 2, 12);
    let started = Instant::now();
    remove_current(&rig);
    assert!(started.elapsed() < Duration::from_millis(500));
    until(|| last(&rig)["state"] == "unknown", 12);
    assert_eq!(last(&rig)["job_id"], second["id"]);
    until(|| last(&rig)["state"] == "sent", 12);
    assert_eq!(last(&rig)["tries"], 2);
    webhook.respond(429, json!({"retry_after":3.0}), 0.0);
    start_job(&mut rig);
    rig.finish();
    until(|| last(&rig)["result"] == "429", 12);
    let limited = webhook.requests.lock().unwrap().last().unwrap().0;
    let count = webhook.len();
    rig.stop(false);
    rig.launch();
    rig.report("FINISH");
    sleep(0.5);
    assert_eq!(webhook.len(), count);
    until(|| last(&rig)["state"] == "sent", 12);
    assert!(
        webhook
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .0
            .duration_since(limited)
            >= Duration::from_secs(3)
    );
    assert_eq!(last(&rig)["tries"], 2);
    remove_current(&rig);
    for (responses, state, tries) in [
        (
            vec![(503, json!({}), 0.0), (200, json!({"id":"3000"}), 0.0)],
            "sent",
            2,
        ),
        (vec![(503, json!({}), 0.0); 3], "failed", 3),
        (vec![(401, json!({"message":TOKEN}), 0.0)], "failed", 1),
        (vec![(200, json!({"id":"4000"}), 1.2); 3], "unknown", 3),
    ] {
        for (status, body, delay) in responses {
            webhook.respond(status, body, delay);
        }
        start_job(&mut rig);
        rig.finish();
        until(
            || last(&rig)["state"] == state && last(&rig)["tries"] == tries,
            20,
        );
        let count = webhook.len();
        sleep(1.2);
        assert_eq!(webhook.len(), count);
        assert_eq!(rig.queue()["current"]["state"], "awaiting_removal");
        remove_current(&rig);
    }
    webhook.respond(200, json!({"id":"5000"}), 1.2);
    start_job(&mut rig);
    let count = webhook.len();
    rig.finish();
    until(|| webhook.len() > count, 12);
    assert_eq!(last(&rig)["state"], "sending");
    rig.stop(true);
    rig.launch();
    rig.report("FINISH");
    assert_eq!(last(&rig)["state"], "unknown");
    until(|| last(&rig)["state"] == "sent", 12);
    assert_eq!(last(&rig)["tries"], 2);
    remove_current(&rig);
    rig.db().execute_batch("CREATE TRIGGER fixture_fail_ack BEFORE UPDATE ON print_notifications WHEN NEW.state='sent' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    start_job(&mut rig);
    let count = webhook.len();
    rig.finish();
    until(|| webhook.len() > count, 12);
    sleep(0.2);
    assert_eq!(last(&rig)["state"], "sending");
    rig.db()
        .execute_batch("DROP TRIGGER fixture_fail_ack;")
        .unwrap();
    let count = webhook.len();
    until(|| last(&rig)["state"] == "sent", 12);
    assert_eq!(webhook.len(), count);
    remove_current(&rig);
    let count = webhook.len();
    let job = start_job(&mut rig);
    let command = rig.broker.prints().last().unwrap().clone();
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":command["sequence_id"],"result":"fail"}}));
    rig.phase("needs_attention");
    sleep(1.2);
    assert_eq!(webhook.len(), count);
    rig.idle();
    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        200,
    );
    until(|| rig.broker.prints().last() != Some(&command), 12);
    rig.finish();
    until(|| last(&rig)["state"] == "sent", 12);
    assert_eq!(webhook.len(), count + 1);
    remove_current(&rig);
    let cancelled = rig.add(3);
    rig.send(json!({"type":"remove","job_id":cancelled["id"]}), 200);
    sleep(1.2);
    assert_eq!(webhook.len(), count + 1);
    rig.stop(false);
    rig.env
        .insert("NOTIFICATION_TEST_ENABLED".into(), "0".into());
    rig.launch();
    start_job(&mut rig);
    rig.finish();
    let count = webhook.len();
    let before = rows(&rig).len();
    sleep(1.2);
    assert_eq!(webhook.len(), count);
    assert_eq!(rows(&rig).len(), before);
    rig.stop(false);
    legacy_schema::queue_v16(&rig.db());
    rig.db().execute_batch("DROP TABLE print_history; ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; ALTER TABLE plates DROP COLUMN sparse_infill_pattern; ALTER TABLE plates DROP COLUMN sparse_infill_density; ALTER TABLE plates DROP COLUMN wall_loops; ALTER TABLE default_settings DROP COLUMN sparse_infill_pattern; ALTER TABLE default_settings DROP COLUMN sparse_infill_density; ALTER TABLE default_settings DROP COLUMN wall_loops; DROP TABLE print_notifications; ALTER TABLE print_jobs DROP COLUMN estimate_json; PRAGMA user_version=7;").unwrap();
    rig.env
        .insert("NOTIFICATION_TEST_ENABLED".into(), "1".into());
    rig.launch();
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    sleep(1.2);
    assert!(rows(&rig).is_empty());
    assert_eq!(webhook.len(), count);
    remove_current(&rig);
    for path in [
        "/api/queue?printer_id=p1",
        "/api/printers",
        "/api/plates",
        "/api/about",
    ] {
        assert!(!rig.get(path).to_string().contains(TOKEN));
    }
    assert!(
        !fs::read_to_string(rig.output.join("server.log"))
            .unwrap()
            .contains(TOKEN)
    );
    rig.stop(false);
}
