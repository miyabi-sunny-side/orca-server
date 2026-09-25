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
    until(|| rig.get(&slice)["state"] == "calculating", 12);
    assert!(array(&rig.queue()["waiting"]).is_empty());
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.hold(false);
    until(|| rig.get(&slice)["state"] == "ready", 12);
    let snapshot = rig.get(&slice);
    assert_eq!(snapshot["seconds"], 1140);
    let bytes: Vec<u8> = rig
        .db()
        .query_row(
            "SELECT gcode FROM plate_slices WHERE plate_id=?1",
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
    assert_eq!(rig.get(&slice)["seconds"], 1140);
    let mut changed = edit(&rig.get(&path));
    changed["conditions"]["sparse_infill_density"] = json!(25);
    rig.plate = rig.put(&path, &changed, 200);
    until(
        || rig.get(&slice)["state"] == "ready" && rig.traces().len() == calls + 2,
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
    until(|| rig.get(&slice)["state"] == "ready", 12);
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
            rig.get(&slice)["state"] == "ready"
                && rig.traces().last().unwrap()["profiles"]["filament"]["nozzle_temperature"]
                    == json!(["225"])
        },
        12,
    );
    let before = rig.traces().len();
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Changed");
    until(
        || rig.traces().len() >= before + 2 && rig.get(&slice)["state"] == "ready",
        40,
    );
    let valid = rig.files.lock().unwrap().remove("parts/cube.stl").unwrap();
    until(|| rig.get(&slice)["state"] == "failed", 40);
    assert!(rig.get(&slice)["seconds"].is_null());
    rig.files
        .lock()
        .unwrap()
        .insert("parts/cube.stl".into(), valid);
    rig.post(&slice, &json!({}), 202);
    until(|| rig.get(&slice)["state"] == "ready", 12);
    let before = rig.traces().len();
    rig.db()
        .execute(
            "UPDATE plate_slices SET gcode=x'62726f6b656e',checked_at=0 WHERE plate_id=?1",
            [id(&rig.plate)],
        )
        .unwrap();
    until(
        || rig.traces().len() >= before + 2 && rig.get(&slice)["state"] == "ready",
        12,
    );
    rig.db()
        .execute(
            "UPDATE plate_slices SET record_json='{}',checked_at=0 WHERE plate_id=?1",
            [id(&rig.plate)],
        )
        .unwrap();
    until(|| rig.get(&slice)["state"] == "ready", 12);
    assert_eq!(rig.get(&slice)["seconds"], 1140);
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
    until(|| rig.get(&slice)["state"] == "ready", 12);
    rig.hold(true);
    rig.edit_conditions(&json!({"sparse_infill_density":21}));
    until(|| rig.get(&slice)["state"] == "calculating", 12);
    let before = rig.traces().len();
    rig.edit_conditions(&json!({"sparse_infill_density":31}));
    assert!(rig.get(&slice)["seconds"].is_null());
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Updated");
    rig.hold(false);
    until(|| rig.get(&slice)["state"] == "ready", 12);
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
    until(|| rig.get(&slice)["state"] == "calculating", 12);
    rig.files.lock().unwrap().get_mut("parts/cube.stl").unwrap()[..7].copy_from_slice(b"Source2");
    rig.hold(false);
    until(|| rig.get(&slice)["state"] == "ready", 12);
    assert_eq!(rig.traces().len(), before + 4);
    rig.hold(true);
    rig.edit_conditions(&json!({"sparse_infill_density":41}));
    until(|| rig.get(&slice)["state"] == "calculating", 12);
    rig.stop(true);
    rig.launch();
    rig.idle();
    rig.hold(false);
    until(|| rig.get(&slice)["state"] == "ready", 12);
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
        || rig.get(&format!("{path}/slice"))["state"] == "calculating",
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
