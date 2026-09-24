use crate::common::*;
use serde_json::json;
fn ready(rig: &Rig) {
    until(
        || {
            let q = rig.queue();
            let jobs = array(&q["waiting"]);
            assert!(
                jobs.iter().all(|j| j["estimate"]["state"] != "failed"),
                "{jobs:?}"
            );
            jobs.iter().all(|j| j["estimate"]["state"] == "ready")
        },
        30,
    );
}
#[allow(clippy::too_many_lines)]
pub fn compact_queue(browser: bool) {
    let mut rig = Rig::new("compact-queue");
    rig.full["print"]["ams"]["ams"][0]["tray"][0]["tray_type"] = json!("PETG");
    rig.launch();
    rig.seed();
    let active = rig.add(3);
    let active_plate = rig.plate.clone();
    let material=rig.post("/api/filaments",&json!({"name":"PETG-GF 黒","vendor":"Fixture","material":"PETG-GF","color":"FFFFFFFF","bambu_filament_id":null}),201);
    let mut data = json!({"machine_profile_key":MACHINE,"base_profile_key":"Generic PETG","overrides_json":{"nozzle_temperature_initial_layer":250,"nozzle_temperature":240,"bed_temperature_initial_layer":65,"bed_temperature":65}});
    let settings_path = format!("/api/filaments/{}/settings", id(&material));
    let setting = rig.post(&settings_path, &data, 201);
    let slot = rig.slot(0);
    rig.put(
        &format!("/api/printers/p1/ams/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":material["id"]}),
        204,
    );
    let mut conditions = rig.specification(0);
    conditions.as_object_mut().unwrap().remove("ams_slot_id");
    conditions["bed_type"] = json!("Cool Plate");
    let plate=rig.post("/api/plates/import",&json!({"name":"PETG-GF 前側ケース","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":2}],"conditions":conditions}),201);
    let action = json!({"type":"add","plate_id":plate["id"],"plate_version":plate["version"]});
    let one = array(&rig.send(action.clone(), 200)["waiting"])
        .last()
        .unwrap()
        .clone();
    let two = array(&rig.send(action.clone(), 200)["waiting"])
        .last()
        .unwrap()
        .clone();
    ready(&rig);
    let originals = rig.rows("SELECT * FROM plate_items ORDER BY id", &[]);
    let ids: Vec<_> = array(&rig.queue()["waiting"])
        .iter()
        .map(|j| j["id"].clone())
        .collect();
    rig.stop(false);
    rig.db().execute_batch("ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; PRAGMA user_version=10;").unwrap();
    rig.launch();
    rig.idle();
    ready(&rig);
    assert_eq!(
        rig.rows("PRAGMA user_version", &[]),
        vec![vec![rusqlite::types::Value::Integer(15)]]
    );
    assert_eq!(
        rig.rows("SELECT * FROM plate_items ORDER BY id", &[]),
        originals
    );
    assert_eq!(
        array(&rig.queue()["waiting"])
            .iter()
            .map(|j| j["id"].clone())
            .collect::<Vec<_>>(),
        ids
    );
    rig.next(&active, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    let sql = "SELECT execution_json,attempt_json,artifact_path FROM print_jobs WHERE id=?1";
    let frozen = rig.rows(sql, &[id(&active)]);
    let before = (rig.broker.prints().len(), rig.ftp.uploads().len());
    data["overrides_json"]["bed_temperature_initial_layer"] = json!(0);
    data["overrides_json"]["bed_temperature"] = json!(0);
    rig.put(&format!("{settings_path}/{}", id(&setting)), &data, 200);
    let reason = "Selected build plate temperature is missing or zero for this material";
    until(
        || {
            array(&rig.queue()["waiting"])
                .iter()
                .all(|j| j["estimate"]["state"] == "failed" && j["estimate"]["error"] == reason)
        },
        12,
    );
    let admission =
        rig.get(&format!("/api/queue?printer_id=p1&plate_id={}", id(&plate)))["admission"].clone();
    assert_eq!(admission["allowed"], false);
    assert_eq!(admission["reason"], reason);
    if browser {
        let control = rig.control();
        rig.browser(
            "E2E_COMPACT_CONTEXT",
            &json!({"plate":plate["id"],"material":material["id"],"job":one["id"]}),
            Some(&control),
        );
    } else {
        data["overrides_json"]["bed_temperature_initial_layer"] = json!(65);
        data["overrides_json"]["bed_temperature"] = json!(65);
        rig.put(&format!("{settings_path}/{}", id(&setting)), &data, 200);
        rig.request("DELETE", &format!("/api/plates/{}", id(&plate)), None, 204);
    }
    ready(&rig);
    assert_eq!(
        rig.get(&format!("/api/filaments/{}", id(&material)))["settings"][0]["overrides_json"],
        json!({"nozzle_temperature_initial_layer":250,"nozzle_temperature":240,"bed_temperature_initial_layer":65,"bed_temperature":65})
    );
    assert_eq!((rig.broker.prints().len(), rig.ftp.uploads().len()), before);
    rig.request(
        "DELETE",
        &format!("/api/plates/{}", id(&active_plate)),
        None,
        204,
    );
    for p in [&plate, &active_plate] {
        let path = format!("/api/plates/{}", id(p));
        rig.request("GET", &path, None, 404);
        rig.put(&path, &edit(p), 409);
        assert!(
            !array(&rig.get(&format!("/api/plates?q={}", id(p))))
                .iter()
                .any(|x| x["id"] == p["id"])
        );
    }
    rig.send(action, 409);
    assert_eq!(
        rig.rows("SELECT * FROM plate_items ORDER BY id", &[]),
        originals
    );
    assert_eq!(rig.rows(sql, &[id(&active)]), frozen);
    let q = rig.queue();
    assert_eq!(q["current"]["plate_deleted"], true);
    assert!(
        array(&q["waiting"])
            .iter()
            .all(|j| j["plate_deleted"] == true)
    );
    let stale = rig.command(
        json!({"type":"move","job_id":one["id"],"index":1}),
        Some(&q),
    );
    rig.send(json!({"type":"move","job_id":two["id"],"index":0}), 200);
    rig.post("/api/queue?printer_id=p1", &stale, 409);
    assert_eq!(rig.queue()["waiting"][0]["id"], two["id"]);
    assert_eq!((rig.broker.prints().len(), rig.ftp.uploads().len()), before);
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    rig.next(&two, 200);
    until(|| rig.broker.prints().len() == 2, 12);
    assert_eq!(rig.broker.prints()[1]["ams_mapping"], json!([0]));
    rig.report("RUNNING");
    rig.phase("printing");
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(rig.get("/api/plates"), json!([]));
    assert_eq!(array(&rig.queue()["waiting"]).len(), 1);
    assert_eq!(rig.queue()["current"]["plate_deleted"], true);
    assert_eq!(rig.broker.prints().len(), 2);
    assert_eq!(rig.ftp.uploads().len(), 2);
}
