mod common;
use common::*;
use serde_json::{Value, json};

/// A full report like the production P1S sent on 2026-10-03 after its AMS was unplugged and
/// filament was inserted into the external spool path (`ams:[]`, `tray_now` 254).
fn without_ams(rig: &Rig, spool_material: &str) -> Value {
    let mut report = rig.full.clone();
    report["print"]["ams"] =
        json!({"ams":[],"ams_exist_bits":"0","tray_exist_bits":"0","tray_now":"254"});
    report["print"]["vt_tray"] =
        json!({"id":"254","tray_type":spool_material,"tray_color":"00000000","remain":0});
    report
}
/// `base` reporting `state` for the latest start.
fn reporting(rig: &Rig, base: &Value, state: &str, error: u64) -> Value {
    let prints = rig.broker.prints();
    let command = prints.last().unwrap();
    let mut report = base.clone();
    report["print"] = merge(
        &report["print"],
        &json!({"gcode_state":state,"print_error":error,"subtask_name":command["subtask_name"],"gcode_file":command["file"]}),
    );
    report
}
fn retry(rig: &Rig, job: &Value) -> Value {
    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        200,
    )
}

#[test]
fn unplugging_the_ams_prints_the_existing_queue_from_the_external_spool() {
    let mut rig = Rig::new("external-spool");
    rig.launch();
    rig.seed();
    // Queued while the AMS was attached, as in production.
    let first = rig.add(3);
    let second = rig.add(3);
    assert_eq!(first["feed"], "ams");
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    assert_eq!(rig.broker.prints()[0]["use_ams"], true);
    assert_eq!(rig.broker.prints()[0]["ams_mapping"], json!([3]));
    // The AMS jams (0700-8007) and the print stops (0300-400C).
    rig.report("RUNNING");
    rig.phase("printing");
    rig.broker
        .send(&reporting(&rig, &rig.full, "PAUSE", 0x0700_8007));
    rig.broker
        .send(&reporting(&rig, &rig.full, "FAILED", 0x0300_400C));
    rig.phase("needs_attention");

    // The user unplugs the AMS and inserts the filament directly; nothing else is operated.
    rig.broker
        .send(&reporting(&rig, &without_ams(&rig, ""), "FAILED", 0));
    until(|| rig.queue()["printer"]["ams"]["units"] == json!([]), 12);
    assert_eq!(rig.get("/api/printers/p1/ams")["slots"], json!([]));
    let q = rig.queue();
    assert_eq!(q["recovery"]["retry_reason"], Value::Null, "{q}");
    assert_eq!(q["allowed"]["retry"], true);
    assert_eq!(q["current"]["feed"], "external");
    let waiting = rig.waiting(&second);
    assert_eq!(waiting["feed"], "external");
    assert_eq!(waiting["hold_reason"], Value::Null, "{waiting}");
    assert_eq!(waiting["ams_slot_id"], Value::Null);
    let admission = rig.get(&format!(
        "/api/queue?printer_id=p1&plate_id={}",
        id(&rig.plate)
    ))["admission"]
        .clone();
    assert_eq!(admission["allowed"], true, "{admission}");
    assert_eq!(admission["feed"], "external");

    retry(&rig, &first);
    until(|| rig.broker.prints().len() == 2, 20);
    let start = &rig.broker.prints()[1];
    assert_eq!(start["use_ams"], false);
    assert_eq!(start["ams_mapping"], json!([0]));
    assert_eq!(rig.queue()["current"]["feed"], "external");

    // A restart keeps the frozen input and never resends the start.
    rig.stop(false);
    rig.launch();
    rig.broker
        .send(&reporting(&rig, &without_ams(&rig, "PLA"), "RUNNING", 0));
    rig.phase("printing");
    rig.broker
        .send(&reporting(&rig, &without_ams(&rig, "PLA"), "FINISH", 0));
    rig.phase("awaiting_removal");
    assert_eq!(rig.broker.prints().len(), 2, "no start was resent");
    let stored = rig.rows(
        "SELECT e.ams_slot_id,json_extract(e.attempt_json,'$.external') FROM print_executions e WHERE e.job_id=?1 ORDER BY e.rowid DESC LIMIT 1",
        &[id(&first)],
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

    // A reported spool of another material holds the next job; the right one releases it.
    rig.broker.send(&without_ams(&rig, "PETG"));
    until(|| !rig.waiting(&second)["hold_reason"].is_null(), 12);
    assert!(
        rig.waiting(&second)["hold_reason"]
            .as_str()
            .unwrap()
            .contains("External spool reports a different material")
    );
    rig.broker.send(&without_ams(&rig, "PLA"));
    until(|| rig.waiting(&second)["hold_reason"].is_null(), 12);
    rig.next(&second, 200);
    until(|| rig.broker.prints().len() == 3, 20);
    assert_eq!(rig.broker.prints()[2]["use_ams"], false);
    rig.check();
}

#[test]
fn plugging_the_ams_back_returns_jobs_to_their_ams_slot() {
    let mut rig = Rig::new("external-spool-ams");
    rig.launch();
    rig.seed();
    let plate = rig.configure(None, None);
    rig.broker.send(&without_ams(&rig, ""));
    until(|| rig.queue()["printer"]["ams"]["units"] == json!([]), 12);
    // Added without the AMS: the queue takes it for the external spool.
    let added = rig.send(
        json!({"type":"add","plate_id":plate["id"],"plate_version":plate["version"]}),
        200,
    );
    let job = array(&added["waiting"])[0].clone();
    assert_eq!(job["feed"], "external");
    rig.broker.send(&rig.full);
    until(|| rig.waiting(&job)["feed"] == "ams", 12);
    assert_eq!(array(&rig.get("/api/printers/p1/ams")["slots"]).len(), 4);
    // The returned AMS needs its spools confirmed again before the queue uses a slot.
    assert!(
        rig.waiting(&job)["hold_reason"]
            .as_str()
            .unwrap()
            .contains("AMS")
    );
    let slot = rig.slot(3);
    rig.put(
        &format!("/api/printers/p1/ams/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":plate["conditions"]["filament_id"]}),
        204,
    );
    until(|| rig.waiting(&job)["hold_reason"].is_null(), 12);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    let start = &rig.broker.prints()[0];
    assert_eq!(start["use_ams"], true);
    assert_eq!(start["ams_mapping"], json!([3]));
    // The removed per-job choice is rejected, not ignored.
    rig.post(
        "/api/queue?printer_id=p1",
        &rig.command(
            json!({"type":"feed","job_id":job["id"],"feed":"external"}),
            None,
        ),
        422,
    );
    rig.check();
}
