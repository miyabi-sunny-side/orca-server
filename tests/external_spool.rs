mod common;
use common::*;
use serde_json::{Value, json};

/// A full report like the P1S sends with the AMS detached and an unset external spool.
fn without_ams(rig: &Rig, spool_material: &str) -> Value {
    let mut report = rig.full.clone();
    report["print"]["ams"] =
        json!({"ams":[],"ams_exist_bits":"0","tray_exist_bits":"0","tray_now":"254"});
    report["print"]["vt_tray"] =
        json!({"id":"254","tray_type":spool_material,"tray_color":"00000000","remain":0});
    report
}
fn admission(rig: &Rig, feed: &str) -> Value {
    rig.get(&format!(
        "/api/queue?printer_id=p1&plate_id={}&feed={feed}",
        id(&rig.plate)
    ))["admission"]
        .clone()
}
fn add(rig: &Rig, feed: &str) -> Value {
    let action = json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"],"feed":feed});
    array(&rig.send(action, 200)["waiting"])
        .last()
        .unwrap()
        .clone()
}
fn waiting(rig: &Rig, job: &Value) -> Value {
    rig.waiting(job)
}

#[test]
fn external_spool_jobs_print_without_the_ams_and_ams_jobs_never_switch() {
    let mut rig = Rig::new("external-spool");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    // Queued while the AMS was attached: this job stays an AMS job.
    let ams_job = add(&rig, "ams");
    assert_eq!(ams_job["feed"], "ams");

    rig.broker.send(&without_ams(&rig, ""));
    until(|| rig.queue()["printer"]["external_spool"]["id"] == 254, 12);
    let held = waiting(&rig, &ams_job);
    assert_eq!(held["feed"], "ams", "an AMS job is never switched silently");
    assert!(
        held["hold_reason"].as_str().unwrap().contains("AMS"),
        "{held}"
    );
    assert_eq!(admission(&rig, "ams")["allowed"], false);
    let external = admission(&rig, "external");
    assert_eq!(external["allowed"], true, "{external}");
    assert_eq!(external["feed"], "external");

    // The user switches the held job to the external spool, and adds a new external job.
    rig.send(
        json!({"type":"feed","job_id":ams_job["id"],"feed":"external"}),
        200,
    );
    let switched = waiting(&rig, &ams_job);
    assert_eq!(switched["feed"], "external");
    assert!(switched["hold_reason"].is_null(), "{switched}");
    assert!(switched["ams_slot_id"].is_null());
    let second = add(&rig, "external");

    // A reported spool of another material holds both; the unset spool releases them.
    rig.broker.send(&without_ams(&rig, "PETG"));
    until(|| !waiting(&rig, &second)["hold_reason"].is_null(), 12);
    assert!(
        waiting(&rig, &second)["hold_reason"]
            .as_str()
            .unwrap()
            .contains("External spool reports a different material")
    );
    assert_eq!(admission(&rig, "external")["allowed"], false);
    rig.broker.send(&without_ams(&rig, "PLA"));
    until(|| waiting(&rig, &second)["hold_reason"].is_null(), 12);

    rig.next(&ams_job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    let start = &rig.broker.prints()[0];
    assert_eq!(start["use_ams"], false);
    assert_eq!(start["ams_mapping"], json!([0]));
    assert_eq!(rig.ftp.uploads().len(), 1);
    let current = rig.queue()["current"].clone();
    assert_eq!(current["feed"], "external");

    // A restart keeps the frozen input and never resends the start.
    rig.stop(false);
    rig.launch();
    let mut report = without_ams(&rig, "PLA");
    let command = &rig.broker.prints()[0];
    report["print"]["gcode_state"] = json!("RUNNING");
    report["print"]["subtask_name"] = command["subtask_name"].clone();
    report["print"]["gcode_file"] = command["file"].clone();
    rig.broker.send(&report);
    rig.phase("printing");
    report["print"]["gcode_state"] = json!("FINISH");
    rig.broker.send(&report);
    rig.phase("awaiting_removal");
    assert_eq!(rig.broker.prints().len(), 1, "no start was resent");
    assert_eq!(rig.queue()["current"]["feed"], "external");
    let stored = rig.rows(
        "SELECT e.ams_slot_id,json_extract(e.attempt_json,'$.external') FROM print_executions e WHERE e.job_id=?1",
        &[id(&ams_job)],
    );
    assert_eq!(
        stored,
        vec![vec![
            rusqlite::types::Value::Text(String::new()),
            rusqlite::types::Value::Integer(1)
        ]]
    );
    // No AMS slot row was invented for the external spool.
    let slots = rig.rows("SELECT count(*) FROM ams_slots WHERE ams_id>=4", &[]);
    assert_eq!(slots, vec![vec![rusqlite::types::Value::Integer(0)]]);

    // The next external job starts the same way after removal.
    rig.next(&second, 200);
    until(|| rig.broker.prints().len() == 2, 20);
    assert_eq!(rig.broker.prints()[1]["use_ams"], false);
    rig.check();
}

#[test]
fn an_ams_job_keeps_its_slot_mapping_and_feed_changes_are_limited() {
    let mut rig = Rig::new("external-spool-ams");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    assert_eq!(job["feed"], "ams");
    rig.send(
        json!({"type":"feed","job_id":job["id"],"feed":"external"}),
        200,
    );
    rig.send(json!({"type":"feed","job_id":job["id"],"feed":"ams"}), 200);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    let start = &rig.broker.prints()[0];
    assert_eq!(start["use_ams"], true);
    assert_eq!(start["ams_mapping"], json!([3]));
    rig.report("RUNNING");
    rig.phase("printing");
    let body = rig.command(
        json!({"type":"feed","job_id":job["id"],"feed":"external"}),
        None,
    );
    rig.post("/api/queue?printer_id=p1", &body, 409);
    rig.check();
}
