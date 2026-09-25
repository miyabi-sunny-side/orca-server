mod common;
use common::*;
use serde_json::json;

#[test]
#[allow(clippy::too_many_lines)] // Follow one printer through completion, cleanup and failures.
fn completed_history_survives_repeated_reports_cleanup_restart_and_deleted_references() {
    let mut rig = Rig::new("print-history");
    rig.launch();
    rig.seed();
    assert_eq!(rig.get("/api/history")["items"], json!([]));
    let first = rig.add(3);
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut renamed = edit(&rig.get(&path));
    renamed["name"] = json!("Renamed after start");
    rig.plate = rig.put(&path, &renamed, 200);
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    let history = rig.get("/api/history");
    assert_eq!(array(&history["items"]).len(), 1);
    assert_eq!(history["items"][0]["name"], first["name"]);
    assert_eq!(history["items"][0]["job_id"], first["id"]);
    assert_eq!(
        history["items"][0]["completed_at"],
        rig.queue()["printer"]["start"]["completed_at"]
    );
    rig.report("FINISH");
    assert_eq!(rig.get("/api/history"), history);
    rig.stop(false);
    rig.launch();
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    assert_eq!(rig.get("/api/history"), history);
    rig.send(
        json!({"type":"discard","expected_job":first["id"],"cleared":true}),
        200,
    );
    until(
        || {
            rig.rows("SELECT id FROM print_jobs WHERE id=?1", &[id(&first)])
                .is_empty()
        },
        12,
    );
    assert_eq!(rig.get("/api/history"), history);
    assert!(!rig.store.join("jobs").join(id(&first)).exists());

    rig.idle();
    let second = rig.add(3);
    rig.next(&second, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    let repeated = rig.get("/api/history");
    assert_eq!(array(&repeated["items"]).len(), 2);
    assert_eq!(
        repeated["items"][0]["plate_id"],
        repeated["items"][1]["plate_id"]
    );
    assert_ne!(
        repeated["items"][0]["attempt_id"],
        repeated["items"][1]["attempt_id"]
    );
    assert_eq!(repeated["items"][0]["name"], "Renamed after start");
    assert_eq!(repeated["items"][1], history["items"][0]);
    rig.send(
        json!({"type":"discard","expected_job":second["id"],"cleared":true}),
        200,
    );

    for outcome in ["FAILED", "IDLE", "PAUSE"] {
        rig.idle();
        let job = rig.add(3);
        let count = rig.broker.prints().len();
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == count + 1, 12);
        rig.report("RUNNING");
        rig.phase("printing");
        rig.report(outcome);
        rig.phase("needs_attention");
        assert_eq!(rig.get("/api/history"), repeated);
        rig.report("FAILED");
        until(|| rig.queue()["allowed"]["discard"] == true, 12);
        rig.send(
            json!({"type":"discard","expected_job":job["id"],"cleared":true}),
            200,
        );
        assert_eq!(rig.get("/api/history"), repeated);
    }
    let cancelled = rig.add(3);
    rig.send(json!({"type":"remove","job_id":cancelled["id"]}), 200);
    rig.request("DELETE", &path, None, 204);
    rig.stop(false);
    rig.launch();
    let deleted = rig.get("/api/history");
    assert_eq!(array(&deleted["items"]).len(), 2);
    for (entry, original) in array(&deleted["items"])
        .iter()
        .zip(array(&repeated["items"]))
    {
        assert_eq!(entry["available"], false);
        for field in ["id", "name", "completed_at", "attempt_id"] {
            assert_eq!(entry[field], original[field]);
        }
    }
    assert_eq!(rig.broker.prints().len(), 5);
    assert_eq!(rig.ftp.uploads().len(), 5);
}
