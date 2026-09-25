mod common;
use common::peers::Action;
use common::*;
use serde_json::{Value, json};
use std::{fs, sync::atomic::Ordering, thread, time::Duration};
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
fn stopped_print_retries_from_the_beginning_and_can_be_discarded() {
    let mut rig = Rig::new("stopped-retry");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    let original = rig.queue()["current"]["attempt_id"].clone();
    let inputs = rig.stored(&job, "execution_json");
    rig.report("FAILED");
    rig.phase("needs_attention");
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut edited = edit(&rig.get(&path));
    edited["conditions"]["sparse_infill_density"] = json!(25);
    rig.put(&path, &edited, 200);
    let stopped = rig.queue();
    assert_eq!(stopped["printer"]["ready_to_print"], false);
    assert_eq!(
        stopped["allowed"],
        json!({"next":false,"retry":true,"discard":true})
    );
    assert!(stopped["recovery"]["retry_reason"].is_null());
    let request = retry(&rig, &job);
    post_queue(&rig, &request, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    post_queue(&rig, &request, 200);
    assert_eq!(rig.broker.prints().len(), 2);
    let current = rig.queue();
    assert_ne!(current["current"]["attempt_id"], original);
    assert_eq!(current["current"]["id"], job["id"]);
    assert_eq!(current["current"]["state"], "preparing");
    let before: Value = serde_json::from_str(&inputs).unwrap();
    let after: Value = serde_json::from_str(&rig.stored(&job, "execution_json")).unwrap();
    assert_eq!(
        before["profiles"]["process.json"]["sparse_infill_density"],
        "15%"
    );
    assert_eq!(
        after["profiles"]["process.json"]["sparse_infill_density"],
        "25%"
    );
    assert_ne!(after["plate"]["version"], before["plate"]["version"]);
    assert_eq!(rig.ftp.contents()[1], rig.cached_artifact(&job, "gcode"));
    let sent = rig.broker.prints()[1].clone();
    assert_ne!(sent["file"], rig.broker.prints()[0]["file"]);
    assert_eq!(sent["ams_mapping"], json!([3]));
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":sent["sequence_id"],"result":"success"}}));
    rig.start_phase("accepted");
    assert_eq!(rig.queue()["current"]["state"], "preparing");
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FAILED");
    rig.phase("needs_attention");
    rig.send(
        json!({"type":"discard","expected_job":job["id"],"cleared":true}),
        200,
    );
    assert!(rig.queue()["current"].is_null());
    assert_eq!(rig.broker.prints().len(), 2);
}

#[test]
#[allow(clippy::too_many_lines)]
fn stopped_retry_rechecks_changes_after_transfer() {
    for continue_next in [false, true] {
        for change in [
            "RUNNING",
            "PREPARE",
            "PAUSE",
            "other-job",
            "error",
            "disconnect",
            "material",
            "nozzle",
        ] {
            let mut rig = Rig::new(&format!("stopped-transfer-{change}"));
            rig.launch();
            rig.seed();
            let job = rig.add(3);
            rig.next(&job, 200);
            until(|| rig.broker.prints().len() == 1, 12);
            rig.report("RUNNING");
            rig.phase("printing");
            rig.report("FAILED");
            rig.phase("needs_attention");
            let request = if continue_next {
                rig.discard();
                let next = rig.add(3);
                rig.command(json!({"type":"next","expected_job":next["id"],"removed_job":null,"cleared":true}), None)
            } else {
                retry(&rig, &job)
            };
            rig.ftp.action(Action::Wait);
            post_queue(&rig, &request, 200);
            until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
            match change {
                "disconnect" => {
                    rig.broker.action(Action::Disconnect);
                    until(|| rig.queue()["printer"]["synchronized"] == false, 10);
                }
                "material" => {
                    let slot = rig.slot(3);
                    rig.put(
                        &format!("/api/printers/p1/ams/{}", id(&slot)),
                        &json!({"revision":slot["revision"],"filament_id":null}),
                        204,
                    );
                }
                _ => {
                    let mut report = rig.full.clone();
                    report["print"]["gcode_state"] = json!("FAILED");
                    report["print"]["subtask_name"] =
                        rig.broker.prints()[0]["subtask_name"].clone();
                    report["print"]["gcode_file"] = rig.broker.prints()[0]["file"].clone();
                    let (field, value) = match change {
                        "other-job" => ("subtask_name", json!("other-job")),
                        "error" => ("print_error", json!(12)),
                        "nozzle" => ("nozzle_diameter", json!("0.6")),
                        state => ("gcode_state", json!(state)),
                    };
                    report["print"][field] = value.clone();
                    rig.broker.send(&report);
                    until(
                        || {
                            let status = rig.queue()["printer"].clone();
                            match change {
                                "other-job" => status["print"]["name"] == value,
                                "error" => status["print"]["error"] == value,
                                "nozzle" => status["nozzle_diameter"] == value,
                                _ => status["print"]["state"] == value,
                            }
                        },
                        10,
                    );
                }
            }
            rig.ftp.release();
            rig.start_phase("not_sent");
            assert_eq!(rig.broker.prints().len(), 1, "{change}");
            rig.phase("needs_attention");
        }
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
            "SELECT e.id,e.attempt_json,e.execution_json FROM print_jobs j JOIN print_executions e ON e.id=j.attempt_id WHERE j.id=?1",
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
    rig.report("FINISH");
    until(|| rig.queue()["allowed"]["discard"] == true, 12);
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

#[test]
fn stopped_discard_preserves_recovery_across_empty_queue_and_restart() {
    for initially_empty in [false, true] {
        let mut rig = Rig::new("discard-next-recovery");
        rig.launch();
        rig.seed();
        let first = rig.add(3);
        let next = (!initially_empty).then(|| rig.add(3));
        rig.next(&first, 200);
        until(|| rig.broker.prints().len() == 1, 12);
        rig.report("RUNNING");
        rig.phase("printing");
        rig.report("FAILED");
        rig.phase("needs_attention");
        if !initially_empty {
            rig.stop(false);
            rig.db()
                .execute_batch(
                    "ALTER TABLE printers DROP COLUMN recovery_attempt; PRAGMA user_version=17;",
                )
                .unwrap();
            rig.launch();
            rig.report("FAILED");
            until(|| rig.queue()["allowed"]["discard"] == true, 12);
        }
        rig.discard();
        assert!(rig.queue()["current"].is_null());
        assert!(!rig.store.join("jobs").join(id(&first)).exists());
        rig.stop(false);
        rig.launch();
        rig.report("FAILED");
        until(|| rig.queue()["printer"]["synchronized"] == true, 12);
        let next = next.unwrap_or_else(|| rig.add(3));
        let q = rig.queue();
        assert_eq!(q["printer"]["ready_to_print"], false);
        assert_eq!(q["allowed"]["next"], true);
        assert_eq!(rig.broker.prints().len(), 1);
        let request = rig.command(
            json!({"type":"next","expected_job":next["id"],"removed_job":null,"cleared":true}),
            Some(&q),
        );
        post_queue(&rig, &request, 200);
        until(|| rig.broker.prints().len() == 2, 12);
        post_queue(&rig, &request, 200);
        assert_eq!(rig.broker.prints().len(), 2);
        assert_eq!(rig.queue()["current"]["id"], next["id"]);
        assert_ne!(
            rig.broker.prints()[0]["subtask_name"],
            rig.broker.prints()[1]["subtask_name"]
        );
        rig.check();
    }
}

#[test]
fn unknown_sent_attempt_requires_matching_terminal_report_before_reprint() {
    let mut rig = Rig::new("unknown-reprint");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.stop(true);
    rig.launch();
    rig.start_phase("unknown");
    rig.idle();
    assert_eq!(rig.queue()["allowed"]["retry"], false);
    post_queue(&rig, &retry(&rig, &job), 409);
    assert_eq!(rig.broker.prints().len(), 1);
    rig.report("FAILED");
    until(|| rig.queue()["allowed"]["retry"] == true, 12);
    post_queue(&rig, &retry(&rig, &job), 200);
    until(|| rig.broker.prints().len() == 2, 12);
}

#[test]
fn explicit_reprint_refreshes_inputs_and_reuses_only_matching_cache() {
    for change in ["unchanged", "source", "material"] {
        let mut rig = Rig::new(&format!("latest-retry-{change}"));
        rig.launch();
        rig.seed();
        let job = rig.add(3);
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == 1, 12);
        rig.report("RUNNING");
        rig.phase("printing");
        rig.report("FAILED");
        rig.phase("needs_attention");
        let before = rig.traces();
        match change {
            "source" => {
                rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[0] = b'X';
            }
            "material" => {
                fs::write(rig.root.path().join("cli-hold"), "1").unwrap();
                rig.temperature(1, 225);
            }
            _ => {}
        }
        let request = retry(&rig, &job);
        post_queue(&rig, &request, 200);
        if change == "material" {
            until(|| rig.traces().len() > before.len(), 12);
            assert_eq!(rig.broker.prints().len(), 1);
            fs::remove_file(rig.root.path().join("cli-hold")).unwrap();
        }
        until(|| rig.broker.prints().len() == 2, 12);
        let after = rig.traces();
        if change == "unchanged" {
            assert_eq!(after.len(), before.len());
            assert_eq!(rig.ftp.contents()[0], rig.ftp.contents()[1]);
        } else {
            assert!(after.len() > before.len());
            if change == "source" {
                assert_ne!(
                    before.last().unwrap()["inputs"],
                    after.last().unwrap()["inputs"]
                );
            } else {
                let execution: Value =
                    serde_json::from_str(&rig.stored(&job, "execution_json")).unwrap();
                assert_eq!(
                    execution["profiles"]["filament.json"]["nozzle_temperature"],
                    json!(["225"])
                );
            }
        }
        let frozen = rig.stored(&job, "execution_json");
        rig.edit_conditions(&json!({"sparse_infill_density":35}));
        post_queue(&rig, &request, 200);
        assert_eq!(rig.stored(&job, "execution_json"), frozen);
        assert_eq!(rig.broker.prints().len(), 2);
    }
}

#[test]
fn reprint_blocks_deleted_invalid_and_unavailable_latest_inputs() {
    for change in ["deleted", "invalid", "source", "slicer"] {
        let mut rig = Rig::new(&format!("invalid-retry-{change}"));
        rig.launch();
        rig.seed();
        let job = rig.add(3);
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == 1, 12);
        rig.report("RUNNING");
        rig.phase("printing");
        rig.report("FAILED");
        rig.phase("needs_attention");
        match change {
            "deleted" => {
                rig.request(
                    "DELETE",
                    &format!("/api/plates/{}", id(&rig.plate)),
                    None,
                    204,
                );
            }
            "invalid" => {
                rig.edit_conditions(&json!({"filament_id":null}));
            }
            "source" => rig.files.lock().unwrap().clear(),
            "slicer" => {
                fs::write(rig.root.path().join("cli-fail"), "1").unwrap();
                rig.edit_conditions(&json!({"sparse_infill_density":25}));
            }
            _ => unreachable!(),
        }
        if matches!(change, "deleted" | "invalid") {
            let q = rig.queue();
            assert_eq!(q["allowed"]["retry"], false);
            assert!(q["recovery"]["retry_reason"].is_string());
            post_queue(&rig, &retry(&rig, &job), 409);
        } else {
            post_queue(&rig, &retry(&rig, &job), 200);
            rig.phase("needs_attention");
        }
        assert_eq!(rig.broker.prints().len(), 1);
        assert_eq!(rig.ftp.contents().len(), 1);
    }
}
