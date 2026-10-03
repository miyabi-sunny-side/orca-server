mod common;
use common::peers::{Action, SECRET};
use common::*;
use serde_json::{Value, json};
use std::{fs, thread, time::Duration};

fn diagnostics(rig: &Rig) -> Value {
    let value = rig.get("/api/printers/p1/diagnostics");
    let text = value.to_string();
    assert!(!text.contains(SECRET), "credential in diagnostics");
    assert!(!text.contains("CERTIFICATE"), "certificate in diagnostics");
    value
}
/// Logs without terminal colors, so fields read as `key=value`.
fn log(rig: &Rig) -> String {
    let raw = fs::read_to_string(rig.output.join("server.log")).unwrap_or_default();
    let mut plain = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            chars.by_ref().find(|c| *c == 'm');
        } else {
            plain.push(c);
        }
    }
    plain
}
fn data_version(c: &rusqlite::Connection) -> i64 {
    c.query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap()
}
fn empty_idle(rig: &Rig) {
    let mut report = rig.full.clone();
    report["print"]["subtask_name"] = json!("");
    report["print"]["gcode_file"] = json!("");
    rig.broker.send(&report);
}
fn stopped(rig: &mut Rig) -> Value {
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FAILED");
    rig.phase("needs_attention");
    job
}

#[test]
fn diagnostics_relate_a_reconnect_to_the_held_attempt_without_writing() {
    let mut rig = Rig::new("diagnostics-power-cycle");
    let job = stopped(&mut rig);
    let d = diagnostics(&rig);
    assert_eq!(d["printer_id"], "p1");
    assert_eq!(d["job"]["id"], job["id"]);
    assert_eq!(d["decision"]["evidence"]["identity"], "target");
    assert_eq!(d["decision"]["evidence"]["terminal"], true);
    assert_eq!(d["attempt"]["stored"]["failure"]["kind"], "stopped");

    let requests = rig.broker.requests().len();
    rig.broker.action(Action::Disconnect);
    until(|| rig.broker.requests().len() > requests, 20);
    rig.broker.send(&rig.full);
    until(
        || diagnostics(&rig)["connection"]["synchronized"] == true,
        12,
    );
    let held = diagnostics(&rig);
    let connection = &held["connection"];
    assert_eq!(connection["state"], "connected");
    assert_eq!(connection["disconnect_reason"], "closed");
    assert_eq!(connection["failures"], 0, "recovered by the full report");
    assert!(connection["snapshot_at"].is_u64() && connection["disconnected_at"].is_u64());
    assert_eq!(held["printer"]["state"], "IDLE");
    assert!(held["printer"]["name"].is_null());
    for side in ["memory", "stored"] {
        assert_eq!(held["attempt"][side]["phase"], "unknown", "{side}");
        assert_eq!(held["attempt"][side]["connection_lost"], true, "{side}");
    }
    let evidence = &held["decision"]["evidence"];
    assert_eq!(evidence["identity"], "none");
    assert_eq!(evidence["current_connection"], false);
    assert_eq!(evidence["full_snapshot"], true);
    assert_eq!(held["decision"]["allowed"]["retry"], false);
    assert_eq!(
        held["decision"]["reasons"]["retry_reason"],
        "Wait for a matching terminal report for the previous start"
    );

    let db = rig.db();
    let version = data_version(&db);
    let generation = held["queue_generation"].clone();
    for _ in 0..5 {
        diagnostics(&rig);
    }
    assert_eq!(
        data_version(&db),
        version,
        "diagnostics wrote to the database"
    );
    assert_eq!(diagnostics(&rig)["queue_generation"], generation);
    assert_eq!(rig.broker.prints().len(), 1);

    empty_idle(&rig);
    until(
        || diagnostics(&rig)["attempt"]["stored"]["phase"] == "resolved",
        12,
    );
    let released = diagnostics(&rig);
    assert_eq!(released["decision"]["allowed"]["retry"], true);
    let q = rig.queue();
    assert_eq!(released["decision"]["allowed"], q["allowed"]);
    assert_eq!(released["decision"]["reasons"], q["recovery"]);

    let text = log(&rig);
    for expected in [
        "P1 MQTT disconnected",
        "reason=\"closed\"",
        "P1 status synchronized",
        "failures=1",
        "Print attempt phase changed",
        "to=Resolved",
        "Queue recovery decision changed",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in logs");
    }
    assert_eq!(
        text.matches("P1 MQTT disconnected").count(),
        1,
        "one disconnect, one warning"
    );
    rig.check();
}

#[test]
fn upload_and_save_failures_keep_their_stage_and_cause() {
    let mut rig = Rig::new("diagnostics-failures");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.ftp.action(Action::Fail);
    rig.next(&job, 200);
    until(|| rig.queue()["current"]["state"] == "needs_attention", 20);
    let d = diagnostics(&rig);
    assert_eq!(d["upload_failure"]["stage"], "transfer");
    assert_eq!(d["attempt"]["stored"]["phase"], "upload_failed");
    assert_eq!(rig.broker.prints().len(), 0);
    assert!(log(&rig).contains("stage=\"transfer\""));

    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        200,
    );
    until(|| rig.broker.prints().len() == 1, 20);
    rig.report("RUNNING");
    rig.phase("printing");
    // The row changes behind the server: memory and the database now disagree.
    rig.db()
        .execute(
            "UPDATE print_jobs SET state='cancelled' WHERE id=?1",
            [id(&job)],
        )
        .unwrap();
    rig.report("FAILED");
    until(|| diagnostics(&rig)["saves"]["failing"] == true, 12);
    let d = diagnostics(&rig);
    assert_eq!(d["saves"]["last_failure"]["operation"], "attempt");
    assert_eq!(d["saves"]["last_failure"]["kind"], "conflict");
    assert_eq!(d["attempt"]["memory"]["phase"], "unknown");
    assert!(d["attempt"]["stored"].is_null());
    thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        log(&rig)
            .matches("Print observation could not be saved")
            .count(),
        1,
        "a repeating save failure is logged once"
    );

    rig.request("GET", "/api/printers/missing/diagnostics", None, 404);
    rig.db()
        .execute_batch("ALTER TABLE print_jobs RENAME TO print_jobs_hidden")
        .unwrap();
    rig.request("GET", "/api/printers/p1/diagnostics", None, 503);
    rig.db()
        .execute_batch("ALTER TABLE print_jobs_hidden RENAME TO print_jobs")
        .unwrap();
    diagnostics(&rig);
    rig.check();
}

/// The value logged as `key=...` on the first line containing `message` after `from`.
fn field(text: &str, message: &str, key: &str) -> Option<String> {
    let line = text.lines().find(|l| l.contains(message))?;
    let start = line.find(&format!(" {key}="))? + key.len() + 2;
    Some(line[start..].split(' ').next()?.to_owned())
}

#[test]
fn ams_reports_are_traced_from_receipt_to_committed_assignments() {
    let mut rig = Rig::new("diagnostics-ams");
    rig.launch();
    rig.seed();
    let d = diagnostics(&rig);
    let observation = &d["observation"];
    assert!(observation["ams_reports"].as_u64() >= Some(1));
    assert_eq!(observation["ams_bits"]["exist"], "9");
    assert_eq!(observation["snapshot_request"]["trigger"], "subscribed");
    assert_eq!(observation["snapshot_request"]["sent"], true);

    let mut tagged = rig.full.clone();
    tagged["print"]["ams"]["ams"][0]["tray"][2]["tag_uid"] = json!("SYNTHTAG0001");
    tagged["print"]["ams"]["ams"][0]["tray"][2]["tray_uuid"] = json!(format!("uuid-{SECRET}"));
    rig.broker.send(&tagged);
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"ams":{
        "tray_exist_bits":"9","tray_reading_bits":"1","tray_read_done_bits":"8"}}}));
    until(
        || diagnostics(&rig)["observation"]["ams_bits"]["reading"] == "1",
        12,
    );

    let assigned = rig.slot(0)["filament_id"].clone();
    assert!(!assigned.is_null());
    let mut without_ams = rig.full.clone();
    without_ams["print"].as_object_mut().unwrap().remove("ams");
    rig.broker.send(&without_ams);
    until(
        || array(&rig.get("/api/printers/p1/ams")["slots"]).is_empty(),
        12,
    );
    let mapped = rig.rows(
        "SELECT filament_id FROM ams_slots WHERE ams_id=0 AND slot_index=0",
        &[],
    );
    assert_eq!(mapped, vec![vec![rusqlite::types::Value::Null]]);

    let text = log(&rig);
    assert!(
        !text.contains("SYNTHTAG0001") && !text.contains("uuid-"),
        "tray identifiers in logs"
    );
    let absent = text
        .lines()
        .find(|l| l.contains("AMS report applied") && l.contains(r#""ams":"absent""#))
        .expect("the full report without AMS is logged");
    let report = absent
        .split(" report=")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert!(
        absent
            .contains(r#""after":{"color":null,"material":null,"profile":null,"state":"absent"}"#)
    );
    let saved = text
        .lines()
        .find(|l| l.contains("AMS assignment changed") && l.contains(&format!(" report={report} ")))
        .expect("the committed change carries the same report number");
    assert!(saved.contains(r#"source="mqtt""#) && saved.contains(r#""reason":"unreported""#));
    assert_eq!(
        field(&text, "Full printer report requested", "trigger").as_deref(),
        Some(r#""subscribed""#)
    );

    let requests = rig.broker.requests().len();
    rig.broker.action(Action::Disconnect);
    until(|| rig.broker.requests().len() > requests, 20);
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"ams":{"tray_reading_bits":"1"}}}));
    until(
        || diagnostics(&rig)["observation"]["last_ignored"]["reason"] == "unsynchronized_diff",
        12,
    );
    assert!(log(&rig).contains(r#"reason="unsynchronized_diff""#));
    rig.broker.send(&rig.full);
    until(|| rig.slot(0)["reported"]["present"] == true, 12);
    rig.check();
}

#[test]
fn slots_read_one_by_one_after_power_up_stay_loaded() {
    let mut rig = Rig::new("ams-power-up-reading");
    rig.launch();
    rig.seed();
    let requests = rig.broker.requests().len();
    rig.broker.action(Action::Disconnect);
    until(|| rig.broker.requests().len() > requests, 20);
    let loaded = rig.full["print"]["ams"]["ams"][0]["tray"].clone();
    let trays = |read: &[usize]| -> Value {
        (0..4)
            .map(|i| {
                if read.contains(&i) {
                    loaded[i].clone()
                } else {
                    json!({"id":i.to_string()})
                }
            })
            .collect()
    };
    let mut report = rig.full.clone();
    report["print"]["ams"]["ams"][0]["tray"] = trays(&[]);
    report["print"]["ams"]["tray_reading_bits"] = json!("1");
    rig.broker.send(&report);
    until(|| rig.slot(0)["reported"]["material"].is_null(), 12);
    for read in [&[0][..], &[0, 3][..]] {
        rig.broker
            .send(&json!({"print":{"command":"push_status","msg":1,
            "ams":{"ams":[{"id":"0","tray":trays(read)}],"tray_read_done_bits":"9"}}}));
    }
    until(|| rig.slot(3)["reported"]["material"] == "PLA", 12);
    for index in [0, 3] {
        let slot = rig.slot(index);
        assert_eq!(slot["reported"]["present"], true, "slot {index}");
        assert_eq!(slot["reported"]["material"], "PLA", "slot {index}");
    }
    assert_eq!(rig.slot(1)["reported"]["present"], false);
    rig.check();
}
