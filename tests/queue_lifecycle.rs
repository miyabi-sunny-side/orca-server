mod common;
use common::peers::Action;
use common::*;
use serde_json::{Value, json};
use std::{
    fs,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};
fn retry(rig: &Rig, job: &Value) -> Value {
    rig.command(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        None,
    )
}
fn post_queue(rig: &Rig, command: &Value, expected: u16) -> Value {
    rig.post("/api/queue?printer_id=p1", command, expected)
}
fn restore_assignments(rig: &Rig) {
    for (i, m) in [(0, 0), (3, 1)] {
        let slot = rig.slot(i);
        rig.put(
            &format!("/api/printers/p1/ams/{}", id(&slot)),
            &json!({"revision":slot["revision"],"filament_id":rig.materials[m]["id"]}),
            204,
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn queue_survives_crashes_without_replaying_prints() {
    let mut rig = Rig::new("queue-recovery");
    rig.launch();
    rig.seed();
    let first = rig.add(3);
    let second = rig.add(0);
    let action = rig.add_action(None, None);
    let request = rig.command(action, None);
    post_queue(&rig, &request, 200);
    post_queue(&rig, &request, 200);
    assert_eq!(array(&rig.queue()["waiting"]).len(), 3);
    let last = array(&rig.queue()["waiting"]).last().unwrap().clone();
    rig.send(json!({"type":"move","job_id":last["id"],"index":0}), 200);
    assert_eq!(rig.queue()["waiting"][0]["id"], last["id"]);
    rig.send(json!({"type":"remove","job_id":last["id"]}), 200);
    let stale = rig.command(
        json!({"type":"next","expected_job":first["id"],"removed_job":null,"cleared":true}),
        None,
    );
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut altered = edit(&rig.get(&path));
    altered["conditions"]["filament_id"] = Value::Null;
    rig.plate = rig.put(&path, &altered, 200);
    post_queue(&rig, &stale, 409);
    rig.next(&first, 409);
    assert_eq!(rig.queue()["waiting"][0]["state"], "queued");
    assert!(!rig.queue()["waiting"][0]["hold_reason"].is_null());
    rig.configure(None, None);
    let settings = printer_settings(&rig.get("/api/printers/p1"));
    let requests = rig.broker.requests().len();
    rig.put(
        "/api/printers/p1",
        &merge(
            &settings,
            &json!({"machine_profile_key":"Bambu Lab P1S 0.2 nozzle"}),
        ),
        200,
    );
    until(|| rig.broker.requests().len() > requests, 12);
    rig.idle();
    rig.next(&first, 409);
    assert!(
        rig.queue()["waiting"][0]["hold_reason"]
            .as_str()
            .unwrap()
            .contains("nozzle")
    );
    let requests = rig.broker.requests().len();
    rig.put("/api/printers/p1", &settings, 200);
    until(|| rig.broker.requests().len() > requests, 12);
    rig.idle();
    restore_assignments(&rig);
    let original = rig.files.lock().unwrap()["parts/cube.stl"].clone();
    rig.files.lock().unwrap().clear();
    rig.next(&first, 200);
    rig.phase("needs_attention");
    assert!(rig.broker.prints().is_empty());
    let changed_bytes = [b"Updated".as_slice(), &original[7..]].concat();
    rig.files
        .lock()
        .unwrap()
        .insert("parts/cube.stl".into(), changed_bytes.clone());
    let hashfile = rig.root.path().join("latest.stl");
    fs::write(&hashfile, changed_bytes).unwrap();
    let changed = sha256(&hashfile);
    rig.ftp.action(Action::Wait);
    let request = retry(&rig, &first);
    post_queue(&rig, &request, 200);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
    let (attempt, phase, execution): (String, String, String) = rig
        .db()
        .query_row(
            "SELECT attempt_id,attempt_json,execution_json FROM print_jobs WHERE id=?1",
            [id(&first)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(!attempt.is_empty());
    assert_eq!(
        serde_json::from_str::<Value>(&phase).unwrap()["phase"],
        "uploading"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&execution).unwrap()["profiles"]["filament.json"]["nozzle_temperature"],
        json!(["215"])
    );
    assert_eq!(
        rig.traces().last().unwrap()["inputs"],
        json!([changed, changed])
    );
    assert_eq!(
        rig.ftp.contents().last().unwrap(),
        &fs::read(rig.artifact("print.gcode.3mf")).unwrap()
    );
    rig.stop(true);
    rig.ftp.release();
    rig.launch();
    assert_eq!(rig.queue()["printer"]["synchronized"], false);
    rig.idle();
    rig.phase("needs_attention");
    thread::sleep(Duration::from_millis(200));
    assert!(rig.broker.prints().is_empty());
    post_queue(&rig, &request, 409);
    let request = retry(&rig, &first);
    post_queue(&rig, &request, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    post_queue(&rig, &request, 200);
    assert_eq!(rig.broker.prints().len(), 1);
    let snapshot = rig.output.join("before-ack.sqlite3");
    rig.db()
        .execute("VACUUM INTO ?1", [snapshot.to_str().unwrap()])
        .unwrap();
    rig.stop(true);
    rig.launch();
    rig.idle();
    rig.phase("needs_attention");
    thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.queue()["current"]["state"], "needs_attention");
    rig.finish();
    rig.stop(false);
    fs::copy(snapshot, rig.store.join("orca.sqlite3")).unwrap();
    rig.launch();
    rig.idle();
    rig.phase("needs_attention");
    assert_eq!(rig.broker.prints().len(), 1);
    post_queue(&rig, &request, 409);
    rig.discard();
    assert!(rig.queue()["current"].is_null());
    assert_eq!(array(&rig.queue()["waiting"]).len(), 1);
    assert!(!rig.store.join("jobs").join(id(&first)).exists());
    let q = rig.queue();
    let one = rig.command(
        json!({"type":"next","expected_job":second["id"],"removed_job":null,"cleared":true}),
        Some(&q),
    );
    let mut two = one.clone();
    two["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(
        rig.response(
            "POST",
            "/api/queue?printer_id=p1",
            Some(&one),
            Some("https://elsewhere.invalid")
        )
        .0,
        403
    );
    let replies = thread::scope(|scope| {
        let first =
            scope.spawn(|| rig.response("POST", "/api/queue?printer_id=p1", Some(&one), None));
        let second =
            scope.spawn(|| rig.response("POST", "/api/queue?printer_id=p1", Some(&two), None));
        [first.join().unwrap(), second.join().unwrap()]
    });
    let mut codes = [replies[0].0, replies[1].0];
    codes.sort_unstable();
    assert_eq!(codes, [200, 409]);
    let accepted = if replies[0].0 == 200 { &one } else { &two };
    until(|| rig.broker.prints().len() == 2, 12);
    post_queue(&rig, accepted, 200);
    assert_eq!(rig.broker.prints().len(), 2);
    let command = rig.broker.prints()[1].clone();
    assert_eq!(command["ams_mapping"], json!([0]));
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":command["sequence_id"],"result":"success"}}));
    rig.start_phase("accepted");
    rig.stop(true);
    rig.launch();
    rig.idle();
    rig.phase("needs_attention");
    assert_eq!(rig.broker.prints().len(), 2);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.stop(false);
    rig.launch();
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    rig.stop(false);
    rig.launch();
    rig.idle();
    rig.phase("awaiting_removal");
    assert_eq!(rig.broker.prints().len(), 2);
    let removal = rig.command(
        json!({"type":"discard","expected_job":second["id"],"cleared":true}),
        None,
    );
    post_queue(&rig, &removal, 200);
    post_queue(&rig, &removal, 200);
    assert!(rig.queue()["current"].is_null());
    assert_eq!(
        rig.db()
            .query_row("SELECT count(*) FROM print_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let job = rig.add(3);
    rig.ftp.reset_gate();
    rig.ftp.action(Action::Wait);
    rig.next(&job, 200);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
    let mut swapped = rig.full.clone();
    swapped["print"]["ams"]["ams"][0]["tray"][3]["tray_color"] = json!("000000FF");
    rig.broker.send(&swapped);
    until(|| rig.slot(3)["filament_id"].is_null(), 12);
    rig.ftp.release();
    rig.phase("needs_attention");
    assert_eq!(rig.broker.prints().len(), 2);
    rig.stop(false);
    rig.db()
        .execute(
            "UPDATE print_jobs SET state='cancelled' WHERE id=?1",
            [id(&job)],
        )
        .unwrap();
    assert!(rig.store.join("jobs").join(id(&job)).exists());
    rig.launch();
    assert!(!rig.store.join("jobs").join(id(&job)).exists());
    assert_eq!(
        rig.db()
            .query_row("SELECT count(*) FROM print_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    rig.check();
}
fn saved_estimate(rig: &Rig, job: &Value) -> Value {
    let text: String = rig
        .db()
        .query_row(
            "SELECT estimate_json FROM print_jobs WHERE id=?1",
            [id(job)],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn cache(rig: &Rig, job: &Value) -> std::path::PathBuf {
    rig.store.join("jobs").join(id(job)).join(format!(
        "estimate-{}",
        saved_estimate(rig, job)["id"].as_str().unwrap()
    ))
}
#[test]
#[allow(clippy::too_many_lines)]
fn estimates_invalidate_recover_and_reuse_without_printing() {
    let mut rig = Rig::new("estimates");
    assert!(!rig.env.contains_key("DISCORD_WEBHOOK_URL"));
    assert!(!rig.env.contains_key("ORCA_PUBLIC_URL"));
    rig.launch();
    rig.seed();
    rig.hold(true);
    let action = rig.add_action(None, None);
    let command = rig.command(action, None);
    let started = Instant::now();
    let q = post_queue(&rig, &command, 200);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!q["waiting"][0]["estimate"].is_null());
    let first = q["waiting"][0].clone();
    post_queue(&rig, &command, 200);
    assert_eq!(array(&rig.queue()["waiting"]).len(), 1);
    until(
        || rig.waiting(&first)["estimate"]["state"] == "calculating",
        12,
    );
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.hold(false);
    rig.ready(&first);
    assert_eq!(rig.traces().len(), 2);
    let persisted = saved_estimate(&rig, &first);
    rig.stop(false);
    rig.launch();
    rig.idle();
    rig.ready(&first);
    thread::sleep(Duration::from_millis(400));
    assert_eq!(saved_estimate(&rig, &first), persisted);
    assert_eq!(rig.traces().len(), 2);
    let before = rig.traces().len();
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(rig.traces().len(), before);
    assert_eq!(rig.queue()["current"]["estimate"]["seconds"], 1140);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.hold(true);
    let second = rig.add(3);
    until(
        || rig.waiting(&second)["estimate"]["state"] == "calculating",
        12,
    );
    assert_eq!(rig.queue()["current"]["id"], first["id"]);
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.uploads().len(), 1);
    let old_cache = cache(&rig, &second);
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut plate = edit(&rig.get(&path));
    plate["name"] = json!(format!("{} updated", plate["name"].as_str().unwrap()));
    rig.plate = rig.put(&path, &plate, 200);
    assert_eq!(
        rig.waiting(&second)["estimate"],
        json!({"state":"pending","seconds":null,"error":null})
    );
    rig.hold(false);
    rig.ready(&second);
    assert!(!old_cache.exists());
    assert_eq!(
        saved_estimate(&rig, &second)["input"]["plate"]["name"],
        rig.plate["name"]
    );
    rig.finish();
    rig.discard();
    let setting =
        rig.get(&format!("/api/filaments/{}", id(&rig.materials[1])))["settings"][0].clone();
    let mut data = json!({"machine_profile_key":setting["machine_profile_key"],"base_profile_key":setting["base_profile_key"],"overrides_json":setting["overrides_json"]});
    data["overrides_json"]["nozzle_temperature"] = json!(220);
    rig.hold(true);
    rig.put(
        &format!(
            "/api/filaments/{}/settings/{}",
            id(&rig.materials[1]),
            id(&setting)
        ),
        &data,
        200,
    );
    assert!(rig.waiting(&second)["estimate"]["seconds"].is_null());
    rig.hold(false);
    rig.ready(&second);
    assert_eq!(
        saved_estimate(&rig, &second)["input"]["settings"]["profiles"]["filament.json"]["nozzle_temperature"],
        json!(["220"])
    );
    {
        let mut files = rig.files.lock().unwrap();
        let data = files.get_mut("parts/cube.stl").unwrap();
        data[..7].copy_from_slice(b"Updated");
    }
    let before = rig.traces().len();
    rig.next(&second, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    assert_eq!(rig.traces().len(), before + 2);
    let hashfile = rig.root.path().join("new.stl");
    fs::write(&hashfile, &rig.files.lock().unwrap()["parts/cube.stl"]).unwrap();
    let expected = sha256(&hashfile);
    assert_eq!(
        rig.traces().last().unwrap()["inputs"],
        json!([expected, expected])
    );
    rig.finish();
    rig.discard();
    rig.hold(true);
    let third = rig.add(3);
    until(
        || rig.waiting(&third)["estimate"]["state"] == "calculating",
        12,
    );
    let old = cache(&rig, &third);
    rig.stop(true);
    rig.launch();
    rig.idle();
    until(
        || {
            saved_estimate(&rig, &third)["id"].as_str().unwrap()
                != old
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .strip_prefix("estimate-")
                    .unwrap()
        },
        12,
    );
    rig.hold(false);
    rig.ready(&third);
    assert!(!old.exists());
    assert_eq!(rig.broker.prints().len(), 2);
    fs::write(cache(&rig, &third).join("print.gcode.3mf"), b"broken").unwrap();
    let before = rig.traces().len();
    rig.next(&third, 200);
    until(|| rig.broker.prints().len() == 3, 12);
    assert_eq!(rig.traces().len(), before + 2);
    rig.finish();
    rig.discard();
    rig.hold(true);
    let cancelled = rig.add(3);
    until(
        || rig.waiting(&cancelled)["estimate"]["state"] == "calculating",
        12,
    );
    rig.send(json!({"type":"remove","job_id":cancelled["id"]}), 200);
    assert!(!rig.store.join("jobs").join(id(&cancelled)).exists());
    rig.hold(false);
    let failure = rig.root.path().join("cli-fail");
    flag(&failure, true);
    let failed = rig.add(3);
    until(|| rig.waiting(&failed)["estimate"]["state"] == "failed", 12);
    assert!(rig.waiting(&failed)["estimate"]["seconds"].is_null());
    flag(&failure, false);
    rig.send(json!({"type":"reestimate","job_id":failed["id"]}), 200);
    rig.ready(&failed);
    assert_eq!(rig.broker.prints().len(), 3);
    assert_eq!(rig.ftp.uploads().len(), 3);
    assert!(!rig.store.join("jobs").join(id(&cancelled)).exists());
    let blocked = rig.store.join("jobs").join(id(&failed));
    fs::remove_dir_all(&blocked).unwrap();
    fs::write(&blocked, b"blocked-directory").unwrap();
    rig.send(json!({"type":"reestimate","job_id":failed["id"]}), 200);
    until(|| rig.waiting(&failed)["estimate"]["state"] == "failed", 3);
    assert!(rig.waiting(&failed)["estimate"]["seconds"].is_null());
    let following = rig.add(3);
    rig.ready(&following);
    rig.send(json!({"type":"remove","job_id":following["id"]}), 200);
    fs::remove_file(blocked).unwrap();
    rig.send(json!({"type":"reestimate","job_id":failed["id"]}), 200);
    rig.ready(&failed);
    rig.stop(false);
    rig.db().execute_batch("DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; ALTER TABLE plates DROP COLUMN sparse_infill_pattern; ALTER TABLE plates DROP COLUMN sparse_infill_density; ALTER TABLE plates DROP COLUMN wall_loops; ALTER TABLE default_settings DROP COLUMN sparse_infill_pattern; ALTER TABLE default_settings DROP COLUMN sparse_infill_density; ALTER TABLE default_settings DROP COLUMN wall_loops; ALTER TABLE print_jobs DROP COLUMN estimate_json; PRAGMA user_version=8;").unwrap();
    rig.launch();
    rig.idle();
    rig.ready(&failed);
    assert_eq!(rig.broker.prints().len(), 3);
    rig.send(json!({"type":"remove","job_id":failed["id"]}), 200);
    rig.check();
}
