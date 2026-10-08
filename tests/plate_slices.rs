mod common;
use common::*;
use serde_json::json;
use std::{
    thread,
    time::{Duration, Instant},
};

#[test]
fn save_slices_without_queue_and_reuses_blob_across_jobs_and_restart() {
    let mut rig = Rig::new("plate-slices");
    rig.launch();
    rig.hold(true);
    let started = Instant::now();
    rig.seed();
    rig.configure(None, None);
    assert!(started.elapsed() < Duration::from_secs(5));
    let path = format!("/api/plates/{}", id(&rig.plate));
    let slice = format!("{path}/slice");
    until(|| rig.slice(&slice)["state"] == "calculating", 12);
    assert!(array(&rig.queue()["waiting"]).is_empty());
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.hold(false);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    let snapshot = rig.slice(&slice);
    assert_eq!(snapshot["seconds"], 1140);
    let bytes: Vec<u8> = rig
        .db()
        .query_row(
            "SELECT gcode FROM plate_slices WHERE plate_id=?1 AND printer_id='p1'",
            [id(&rig.plate)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(bytes.starts_with(b"PK"));
    let calls = rig.traces().len();
    let mut renamed = edit(&rig.get(&path));
    renamed["name"] = json!("Renamed without changing print inputs");
    rig.plate = rig.put(&path, &renamed, 200);
    let first = rig.add(3);
    let second = rig.add(3);
    rig.ready(&first);
    rig.ready(&second);
    thread::sleep(Duration::from_millis(600));
    assert_eq!(rig.traces().len(), calls);
    rig.stop(false);
    rig.launch();
    rig.idle();
    rig.ready(&first);
    assert_eq!(rig.traces().len(), calls);
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(rig.traces().len(), calls);
    rig.finish();
    rig.discard();
    rig.send(json!({"type":"remove","job_id":second["id"]}), 200);
    assert_eq!(rig.slice(&slice)["seconds"], 1140);
    let mut changed = edit(&rig.get(&path));
    changed["conditions"]["sparse_infill_density"] = json!(25);
    rig.plate = rig.put(&path, &changed, 200);
    until(
        || rig.slice(&slice)["state"] == "ready" && rig.traces().len() == calls + 2,
        12,
    );
    assert_eq!(
        rig.traces().last().unwrap()["profiles"]["process"]["sparse_infill_density"],
        "25%"
    );
    assert_eq!(rig.broker.prints().len(), 1);
    let third = rig.add(3);
    rig.next(&third, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    assert_eq!(rig.traces().len(), calls + 2);
    rig.check();
}

#[test]
fn cache_revalidates_sources_and_recovers_corruption_without_ams() {
    let mut rig = Rig::new("plate-slice-refresh");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let path = format!("/api/plates/{}", id(&rig.plate));
    let slice = format!("{path}/slice");
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    for index in [0, 3] {
        let slot = rig.slot(index);
        rig.put(
            &format!("/api/printers/p1/ams/{}", id(&slot)),
            &json!({"revision":slot["revision"],"filament_id":null}),
            204,
        );
    }
    rig.temperature(1, 225);
    until(
        || {
            rig.slice(&slice)["state"] == "ready"
                && rig.traces().last().unwrap()["profiles"]["filament"]["nozzle_temperature"]
                    == json!(["225"])
        },
        12,
    );
    let before = rig.traces().len();
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Changed");
    until(
        || rig.traces().len() >= before + 2 && rig.slice(&slice)["state"] == "ready",
        40,
    );
    let valid = rig.files.lock().unwrap().remove("parts/cube.stl").unwrap();
    until(|| rig.slice(&slice)["state"] == "failed", 40);
    assert!(rig.slice(&slice)["seconds"].is_null());
    rig.files
        .lock()
        .unwrap()
        .insert("parts/cube.stl".into(), valid);
    rig.post(&slice, &json!({}), 202);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    let before = rig.traces().len();
    rig.db()
        .execute(
            "UPDATE plate_slices SET gcode=x'62726f6b656e',checked_at=0 WHERE plate_id=?1",
            [id(&rig.plate)],
        )
        .unwrap();
    until(
        || rig.traces().len() >= before + 2 && rig.slice(&slice)["state"] == "ready",
        12,
    );
    rig.db()
        .execute(
            "UPDATE plate_slices SET record_json='{}',checked_at=0 WHERE plate_id=?1",
            [id(&rig.plate)],
        )
        .unwrap();
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    assert_eq!(rig.slice(&slice)["seconds"], 1140);
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.check();
}

#[test]
fn concurrent_edit_restart_and_delete_never_publish_old_results() {
    let mut rig = Rig::new("plate-slice-races");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let path = format!("/api/plates/{}", id(&rig.plate));
    let slice = format!("{path}/slice");
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    rig.hold(true);
    rig.edit_conditions(&json!({"sparse_infill_density":21}));
    until(|| rig.slice(&slice)["state"] == "calculating", 12);
    let before = rig.traces().len();
    rig.edit_conditions(&json!({"sparse_infill_density":31}));
    assert!(rig.slice(&slice)["seconds"].is_null());
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Updated");
    rig.hold(false);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    assert!(rig.traces().len() > before);
    assert_eq!(
        rig.traces().last().unwrap()["profiles"]["process"]["sparse_infill_density"],
        "31%"
    );
    // A source-only update during computation must also discard the earlier input.
    rig.hold(true);
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Source1");
    let before = rig.traces().len();
    rig.post(&slice, &json!({}), 202);
    until(|| rig.slice(&slice)["state"] == "calculating", 12);
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Source2");
    rig.hold(false);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    assert_eq!(rig.traces().len(), before + 4);
    rig.hold(true);
    rig.edit_conditions(&json!({"sparse_infill_density":41}));
    until(|| rig.slice(&slice)["state"] == "calculating", 12);
    rig.stop(true);
    rig.launch();
    rig.idle();
    rig.hold(false);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    assert_eq!(
        rig.traces().last().unwrap()["profiles"]["process"]["sparse_infill_density"],
        "41%"
    );
    assert!(rig.broker.prints().is_empty());
    let job = rig.add(3);
    rig.ftp.action(common::peers::Action::Wait);
    rig.next(&job, 200);
    until(
        || rig.ftp.received.load(std::sync::atomic::Ordering::SeqCst),
        12,
    );
    rig.request("DELETE", &path, None, 204);
    assert_eq!(
        rig.db()
            .query_row(
                "SELECT count(*) FROM plate_slices WHERE plate_id=?1",
                [id(&rig.plate)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    rig.ftp.release();
    until(|| rig.broker.prints().len() == 1, 12);
    rig.finish();
    rig.discard();
    assert_eq!(array(&rig.get("/api/history")["items"]).len(), 1);
    rig.check();
}

#[test]
fn concurrent_queue_start_joins_the_background_slice_without_blocking_edits() {
    let mut rig = Rig::new("plate-slice-single-flight");
    rig.launch();
    rig.hold(true);
    rig.seed();
    rig.configure(None, None);
    let path = format!("/api/plates/{}", id(&rig.plate));
    until(
        || rig.slice(&format!("{path}/slice"))["state"] == "calculating",
        12,
    );
    let first = rig.add(3);
    let second = rig.add(3);
    rig.next(&first, 200);
    let started = Instant::now();
    let mut renamed = edit(&rig.get(&path));
    renamed["name"] = json!("Edited while slicing");
    rig.put(&path, &renamed, 200);
    assert_eq!(array(&rig.queue()["waiting"]).len(), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(rig.broker.prints().is_empty());
    rig.hold(false);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.ready(&second);
    assert_eq!(rig.traces().len(), 2); // One arrange and one slice, shared with preparation.
    assert_eq!(rig.ftp.contents()[0], rig.cached_artifact(&second, "gcode"));
    rig.check();
}

#[test]
fn each_printer_has_its_own_result_and_only_its_conditions_invalidate_it() {
    let mut rig = Rig::new("plate-slice-printers");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let slice = format!("/api/plates/{}/slice", id(&rig.plate));
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    let p1 = rig.slice(&slice);
    let p1_output = || -> (Vec<u8>, i64) {
        rig.db()
            .query_row(
                "SELECT gcode,generated_at FROM plate_slices WHERE printer_id='p1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let p1_saved = p1_output();
    // A second printer computes every plate without touching the first printer's result.
    let mini = "Bambu Lab A1 mini 0.2 nozzle";
    let peer = common::peers::Peer::broker(rig.root.path(), "trusted", "MINI");
    let mut settings = printer_settings(&rig.get("/api/printers/p1"));
    settings = merge(
        &settings,
        &json!({"name":"A1 mini","serial":"MINI","mqtt_port":peer.port,"machine_profile_key":mini,
            "access_code":common::peers::SECRET,"tls_certificate":std::fs::read_to_string(rig.root.path().join("trusted.pem")).unwrap()}),
    );
    let device = rig.post("/api/printers", &settings, 201);
    let pid = id(&device).to_owned();
    // The material has no setting for the A1 mini yet.
    until(
        || rig.slice_on(&slice, &pid)["reason"] == "material_setting",
        12,
    );
    assert_eq!(rig.slice_on(&slice, &pid)["state"], "failed");
    assert_eq!(rig.slice_on(&slice, &pid)["printer_name"], "A1 mini");
    let calls = rig.traces().len();
    let material = id(&rig.materials[1]).to_owned();
    rig.post(
        &format!("/api/filaments/{material}/settings"),
        &json!({"machine_profile_key":mini,"base_profile_key":FILAMENT,"overrides_json":{}}),
        201,
    );
    until(|| rig.slice_on(&slice, &pid)["state"] == "ready", 12);
    assert_eq!(rig.traces().len(), calls + 2);
    let mini_trace = rig.traces().last().unwrap().clone();
    assert_eq!(mini_trace["profiles"]["printer"]["name"], mini);
    assert_eq!(rig.slice(&slice), p1);
    assert_eq!(p1_output(), p1_saved);
    // The printer's bed belongs to its own result only.
    settings = printer_settings(&rig.get(&format!("/api/printers/{pid}")));
    settings["bed_type"] = json!("High Temp Plate");
    rig.put(&format!("/api/printers/{pid}"), &settings, 200);
    until(
        || rig.traces().len() == calls + 4 && rig.slice_on(&slice, &pid)["state"] == "ready",
        12,
    );
    assert!(
        rig.traces()[calls + 2]["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "High Temp Plate")
    );
    assert_eq!(rig.slice(&slice), p1);
    assert_eq!(p1_output(), p1_saved);
    // Removing the printer removes its results; the remaining printer keeps its own.
    rig.request("DELETE", &format!("/api/printers/{pid}"), None, 204);
    let printers: Vec<String> = rig
        .db()
        .prepare("SELECT printer_id FROM plate_slices")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(printers, ["p1"]);
    assert_eq!(array(&rig.get(&slice)["printers"]).len(), 1);
    assert_eq!(p1_output(), p1_saved);
    assert!(rig.broker.prints().is_empty() && peer.prints().is_empty());
    rig.check();
}

const QUEUE: &str = "SELECT j.id,j.state,j.position,j.attempt_id,p.queue_generation,p.queue_request FROM print_jobs j JOIN printers p ON p.id=j.printer_id ORDER BY j.position";
/// What preparation froze; the attempt's own progress may change across a restart.
const FROZEN_INPUT: &str = "SELECT id,job_id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,artifact_path,execution_json,estimate_json FROM print_executions ORDER BY id";
/// Schema 23 as production held it on 2026-10-07: 22 plates on the one printer's machine,
/// process and bed, 4 of them recording timelapse, with a print running and one waiting.
#[test]
#[allow(clippy::too_many_lines)] // One migration observed end to end through the server.
fn production_like_schema_23_keeps_plates_results_and_frozen_work() {
    let mut rig = Rig::new("plate-slice-schema-24");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let first = rig.plate.clone();
    let mut plates = vec![first.clone()];
    for n in 1..22 {
        plates.push(rig.post(
            &format!("/api/plates/{}/duplicate", id(&first)),
            &json!({"name":format!("Plate {n:02}")}),
            201,
        ));
    }
    for (n, plate) in plates.iter_mut().enumerate() {
        let path = format!("/api/plates/{}", id(plate));
        let mut data = edit(&rig.get(&path));
        data["conditions"]["start_options"]["timelapse"] = json!(n % 6 == 0);
        data["conditions"]["wall_loops"] = json!(2 + n % 3);
        data["conditions"]["brim_enabled"] = json!(n % 2 == 1);
        *plate = rig.put(&path, &data, 200);
    }
    assert_eq!(
        plates
            .iter()
            .filter(|p| p["conditions"]["start_options"]["timelapse"] == true)
            .count(),
        4
    );
    rig.plate = plates[0].clone();
    for plate in &plates {
        until(
            || rig.slice(&format!("/api/plates/{}/slice", id(plate)))["state"] == "ready",
            60,
        );
    }
    let printing = rig.add(3);
    rig.next(&printing, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.plate = plates[6].clone();
    let waiting = rig.add(3);
    let removed = rig.post(
        &format!("/api/plates/{}/duplicate", id(&first)),
        &json!({"name":"Removed"}),
        201,
    );
    rig.request(
        "DELETE",
        &format!("/api/plates/{}", id(&removed)),
        None,
        204,
    );
    let listed = rig.get("/api/plates");
    let slices: Vec<_> = plates
        .iter()
        .map(|p| rig.slice(&format!("/api/plates/{}/slice", id(p))))
        .collect();
    rig.stop(false);
    let db = rig.db();
    legacy_schema::plate_conditions_v23(&db);
    // Frozen executions of schema 23 carried the conditions inside their plate as well.
    db.execute_batch(&format!(
        "UPDATE print_executions SET execution_json=json_set(execution_json,
            '$.plate.conditions.required_machine_profile_key','{MACHINE}',
            '$.plate.conditions.process_profile_key','{PROCESS}',
            '$.plate.conditions.bed_type','{BED}')"
    ))
    .unwrap();
    let legacy_slices = rig.rows("SELECT * FROM plate_slices ORDER BY plate_id", &[]);
    assert_eq!(legacy_slices.len(), 22);
    let frozen = rig.rows("SELECT * FROM print_executions ORDER BY id", &[]);
    let frozen_input = rig.rows(FROZEN_INPUT, &[]);
    let queue = rig.rows(QUEUE, &[]);
    let deleted = rig.rows(
        "SELECT id,name,deleted,required_machine_profile_key FROM plates WHERE deleted=1",
        &[],
    );
    assert_eq!(deleted.len(), 1);
    drop(db);
    let calls = rig.traces().len();
    // Migrate without the server first: nothing but the migration touches the database.
    drop(orca_server::plates::Store::open(&rig.store).unwrap());
    assert_eq!(
        rig.rows("PRAGMA user_version", &[]),
        vec![vec![rusqlite::types::Value::Integer(24)]]
    );
    assert_eq!(
        rig.rows(
            "SELECT plate_id,printer_id FROM plate_slices WHERE printer_id!='p1'",
            &[]
        ),
        Vec::<Vec<rusqlite::types::Value>>::new()
    );
    assert_eq!(
        rig.rows("SELECT count(*) FROM plate_slices", &[]),
        vec![vec![rusqlite::types::Value::Integer(22)]]
    );
    assert_eq!(
        rig.rows("SELECT * FROM print_executions ORDER BY id", &[]),
        frozen
    );
    assert_eq!(rig.rows(QUEUE, &[]), queue);
    assert_eq!(
        rig.rows("SELECT id,name,deleted FROM plates WHERE deleted=1", &[]),
        deleted
            .iter()
            .map(|row| row[..3].to_vec())
            .collect::<Vec<_>>()
    );
    rig.launch();
    rig.report("RUNNING");
    rig.phase("printing");
    assert_eq!(rig.get("/api/plates"), listed);
    for (plate, before) in plates.iter().zip(&slices) {
        assert_eq!(rig.get(&format!("/api/plates/{}", id(plate))), *plate);
        assert_eq!(
            rig.slice(&format!("/api/plates/{}/slice", id(plate))),
            *before
        );
    }
    assert_eq!(rig.rows(FROZEN_INPUT, &[]), frozen_input);
    let view = rig.queue();
    assert_eq!(view["current"]["id"], printing["id"]);
    assert_eq!(view["current"]["bed_type"], BED);
    assert_eq!(view["waiting"][0]["id"], waiting["id"]);
    assert_eq!(view["waiting"][0]["process_profile_key"], PROCESS);
    assert_eq!(view["waiting"][0]["estimate"]["seconds"], 1140);
    // The migration reused every result and resent nothing.
    thread::sleep(Duration::from_millis(600));
    assert_eq!(rig.traces().len(), calls);
    assert_eq!(rig.broker.prints().len(), 1);
    // The running print finishes from its frozen input; the next one uses the printer's conditions.
    rig.finish();
    rig.next(&waiting, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    let start = &rig.broker.prints()[1];
    assert_eq!(start["timelapse"], true);
    assert_eq!(rig.traces().len(), calls);
    rig.check();
}

#[test]
fn retry_and_cached_records_keep_the_gcode_prediction() {
    let mut rig = Rig::new("plate-slice-retry-seconds");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let slice = format!("/api/plates/{}/slice", id(&rig.plate));
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    let calls = rig.traces().len();
    rig.post(&slice, &json!({}), 202);
    until(|| rig.slice(&slice)["state"] == "ready", 12);
    assert_eq!(rig.slice(&slice)["seconds"], 1140);
    // Records saved by the earlier retry bug: ready with a cached G-code but no seconds.
    rig.db()
        .execute(
            "UPDATE plate_slices SET record_json=json_set(record_json,'$.seconds',NULL),checked_at=0 WHERE plate_id=?1",
            [id(&rig.plate)],
        )
        .unwrap();
    until(|| rig.slice(&slice)["seconds"] == 1140, 12);
    assert_eq!(rig.traces().len(), calls);
    rig.check();
}
