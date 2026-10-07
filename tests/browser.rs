//! Chromium assertions stay in TypeScript; Rust owns their isolated API and protocol peers.
#[path = "scenarios/ams_nozzle.rs"]
mod ams;
mod common;
#[path = "scenarios/compact.rs"]
mod compact;
#[path = "scenarios/import.rs"]
mod import;
#[path = "scenarios/strength_support.rs"]
mod material;
#[path = "scenarios/plates.rs"]
mod plates;
#[path = "scenarios/registry.rs"]
mod registry;
use common::*;
use serde_json::json;
fn appdir() -> std::path::PathBuf {
    std::env::var_os("ORCA_APPDIR")
        .expect("ORCA_APPDIR required for browser official CLI scenarios")
        .into()
}
#[test]
#[ignore = "requires Chromium"]
fn plate_admission() {
    plates::plate_admission(true);
}
#[test]
#[ignore = "requires Chromium"]
fn creation_defaults() {
    plates::creation_defaults(true);
}
#[test]
#[ignore = "requires Chromium"]
fn plate_archive() {
    plates::plate_archive(true);
}
#[test]
#[ignore = "requires Chromium"]
fn strength_settings() {
    material::strength_settings(true);
}
#[test]
#[ignore = "requires Chromium"]
fn support_interface() {
    material::support_interface(true);
}
#[test]
#[ignore = "requires Chromium"]
fn ams_refresh() {
    ams::ams_refresh();
}
#[test]
#[ignore = "requires Chromium"]
fn ams_priority() {
    ams::ams_priority(true);
}
#[test]
#[ignore = "requires Chromium"]
fn compact_queue() {
    compact::compact_queue(true);
}
#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn printer_registry() {
    registry::registry(Some(&appdir()), true);
}
#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn filament_ams() {
    registry::filament_ams(Some(&appdir()), true);
}
#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn model_import() {
    import::model_import(Some(&appdir()), true);
}
#[test]
#[ignore = "requires Chromium"]
fn estimates() {
    let mut rig = Rig::new("browser-estimates");
    rig.launch();
    rig.seed();
    rig.configure(None, None);
    let control = rig.control();
    rig.browser(
        "E2E_ESTIMATE_CONTEXT",
        &json!({"plate_id":rig.plate["id"]}),
        Some(&control),
    );
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.uploads().len(), 1);
}
fn queue(appdir: Option<&std::path::Path>) {
    let mut rig = Rig::with_options("browser-queue", "v3", appdir);
    let control = rig.control();
    rig.launch();
    rig.seed();
    for name in ["B · 小物ケース", "C · 取り付けパーツ", "D · 予備のパーツ"] {
        rig.post("/api/plates/import",&json!({"name":name,"models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":2}]}),201);
    }
    rig.browser("E2E_QUEUE_CONTEXT", &json!({}), Some(&control));
    assert_eq!(rig.ftp.contents().len(), rig.broker.prints().len() + 1);
    write_json(
        &rig.output.join("protocol-result.json"),
        &json!({"prints":rig.broker.prints().len(),"uploads":rig.ftp.uploads().len(),"official_cli":appdir.is_some()}),
    );
}
#[test]
#[ignore = "requires Chromium"]
fn queue_fixture() {
    queue(None);
}
#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn queue_official() {
    queue(Some(&appdir()));
}

#[test]
#[ignore = "requires Chromium"]
fn filament_picker() {
    plates::filament_picker(true);
}

#[test]
#[ignore = "requires Chromium"]
fn plate_duplication() {
    plates::plate_duplication(true);
}

#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn queue_context_menu() {
    let mut rig = Rig::with_options("queue-context-menu", "v3", Some(&appdir()));
    rig.launch();
    rig.seed();
    let plate = rig.configure(None, None);
    let mut body = edit(&plate);
    body["name"] = json!("bin · 1モデル10個");
    body["models"][0]["quantity"] = json!(10);
    rig.plate = rig.put(&format!("/api/plates/{}", id(&plate)), &body, 200);
    rig.send(
        json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
        200,
    );
    let items = rig.rows("SELECT * FROM plate_items ORDER BY id", &[]);
    let plates = rig.get("/api/plates");
    let control = rig.control();
    rig.browser(
        "E2E_QUEUE_MENU_CONTEXT",
        &json!({"plate":rig.plate}),
        Some(&control),
    );
    assert_eq!(rig.get("/api/plates"), plates);
    assert_eq!(
        rig.rows("SELECT * FROM plate_items ORDER BY id", &[]),
        items
    );
    assert_eq!(
        rig.broker.prints().len(),
        1,
        "only the explicit original start is sent"
    );
    assert_eq!(rig.ftp.uploads().len(), 1);
    let queue = rig.queue();
    assert!(queue["current"].is_null());
    assert_eq!(array(&queue["waiting"]).len(), 2);
    assert!(
        array(&queue["waiting"])
            .iter()
            .all(|job| job["state"] == "queued"
                && job["attempt_id"].is_null()
                && job["artifact_path"].is_null())
    );
}

#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn print_history() {
    let mut rig = Rig::with_options("history-live", "v3", Some(&appdir()));
    rig.full["print"]["ams"]["tray_exist_bits"] = json!("b");
    rig.full["print"]["ams"]["ams"][0]["tray"][1] = merge(
        &rig.full["print"]["ams"]["ams"][0]["tray"][1],
        &json!({"tray_type":"PETG","tray_color":"FFFFFFFF"}),
    );
    rig.launch();
    rig.seed();
    let original = rig.add(3);
    rig.next(&original, 200);
    until(|| rig.broker.prints().len() == 1, 60);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FINISH");
    rig.phase("awaiting_removal");
    let history = rig.get("/api/history");
    rig.send(
        json!({"type":"discard","expected_job":original["id"],"cleared":true}),
        200,
    );
    rig.idle();
    let other = rig.post("/api/filaments", &json!({"name":"History support PETG","vendor":"Fixture","material":"PETG","color":"FFFFFFFF"}),201);
    rig.post(&format!("/api/filaments/{}/settings",id(&other)), &json!({"machine_profile_key":MACHINE,"base_profile_key":"Generic PETG","overrides_json":{}}),201);
    rig.materials.push(other);
    rig.map_material(1, 2);
    rig.files.lock().unwrap().insert(
        "parts/roles.3mf".into(),
        fixture("material-roles-support.3mf"),
    );
    let path = format!("/api/plates/{}", id(&rig.plate));
    let mut changed = edit(&rig.get(&path));
    changed["name"] = json!("Current roles and quantity");
    changed["models"] = json!([{"name":"roles.3mf","source":"parts/roles.3mf","quantity":3}]);
    changed["conditions"] = merge(
        &changed["conditions"],
        &json!({"filament_id":rig.materials[0]["id"],"secondary_filament_id":rig.materials[1]["id"],"support_enabled":true,"support_interface_filament_id":rig.materials[2]["id"]}),
    );
    rig.plate = rig.put(&path, &changed, 200);
    // A pre-existing waiting item must stay before both independent history additions.
    rig.send(
        json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
        200,
    );
    let control = rig.control();
    rig.browser(
        "E2E_HISTORY_CONTEXT",
        &json!({"plate":rig.plate,"history":history}),
        Some(&control),
    );
    let queue = rig.queue();
    assert!(queue["current"].is_null());
    assert_eq!(array(&queue["waiting"]).len(), 3);
    for job in array(&queue["waiting"]) {
        assert_eq!(job["state"], "queued");
        assert!(job["attempt_id"].is_null() && job["artifact_path"].is_null());
        let estimate = rig.estimated(job);
        assert_eq!(estimate["input"]["plate"]["models"][0]["quantity"], 3);
        assert_eq!(
            estimate["input"]["plate"]["models"][0]["source"],
            "parts/roles.3mf"
        );
        for key in [
            "filament_id",
            "secondary_filament_id",
            "support_interface_filament_id",
        ] {
            assert_eq!(
                estimate["input"]["plate"]["conditions"][key],
                rig.plate["conditions"][key]
            );
        }
    }
    assert_eq!(rig.get("/api/history"), history);
    assert_eq!(rig.get(&path)["models"], rig.plate["models"]);
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.uploads().len(), 1);
}

#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn stopped_recovery() {
    let mut rig = Rig::with_options("browser-recovery", "v3", Some(&appdir()));
    rig.launch();
    rig.seed();
    let first = rig.add(3);
    let next = rig.add(3);
    let control = rig.control();
    rig.browser(
        "E2E_RECOVERY_CONTEXT",
        &json!({"next_job":next["id"]}),
        Some(&control),
    );
    assert_eq!(rig.broker.prints().len(), 3);
    assert_eq!(rig.ftp.contents().len(), 3);
    let artifact = zip_json(&rig.ftp.contents()[1], "Metadata/project_settings.config");
    assert_eq!(artifact["sparse_infill_density"], "25%");
    assert_eq!(rig.queue()["current"]["id"], next["id"]);
    assert!(!rig.store.join("jobs").join(id(&first)).exists());
}

#[test]
#[ignore = "requires Chromium"]
fn external_spool() {
    let mut rig = Rig::new("browser-external-spool");
    rig.launch();
    rig.seed();
    // As in production on 2026-10-03: jobs queued with the AMS, the first fails on an AMS jam.
    let first = rig.add(3);
    rig.add(3);
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FAILED");
    rig.phase("needs_attention");
    // The user unplugs the AMS and inserts the filament into the external spool path.
    rig.full["print"]["ams"] =
        json!({"ams":[],"ams_exist_bits":"0","tray_exist_bits":"0","tray_now":"254"});
    rig.full["print"]["vt_tray"] = json!({"id":"254","tray_type":"","tray_color":"00000000"});
    rig.report("FAILED");
    until(|| rig.queue()["allowed"]["retry"] == true, 12);
    let control = rig.control();
    rig.browser("E2E_EXTERNAL_CONTEXT", &json!({}), Some(&control));
    let prints = rig.broker.prints();
    assert_eq!(prints.len(), 3);
    assert_eq!(prints[0]["use_ams"], true);
    assert_eq!(prints[1]["use_ams"], false);
    assert_eq!(prints[2]["use_ams"], false);
}

#[test]
#[ignore = "requires Chromium"]
fn printer_controls() {
    let mut rig = Rig::new("browser-printer-controls");
    let jpeg = fixture("camera.jpg");
    let (port, _) = common::peers::camera(rig.root.path(), common::peers::frame(&jpeg));
    rig.env.insert("P1_CAMERA_PORT".into(), port.to_string());
    rig.ftp
        .records
        .lock()
        .unwrap()
        .files
        .insert("/timelapse/video_1.mp4".into(), vec![0; 2048]);
    rig.full["print"]["nozzle_temper"] = json!(212.4);
    rig.full["print"]["nozzle_target_temper"] = json!(220);
    rig.full["print"]["bed_temper"] = json!(60);
    rig.full["print"]["bed_target_temper"] = json!(60);
    rig.full["print"]["lights_report"] = json!([{"node":"chamber_light","mode":"on"}]);
    rig.full["print"]["hms"] = json!([{"attr":0x0300_0D00_u32,"code":0x0001_0004}]);
    rig.launch();
    rig.seed();
    let plate = rig.configure(None, None);
    rig.add(3);
    let control = rig.control();
    rig.browser(
        "E2E_DEVICE_CONTEXT",
        &json!({"plate":plate["id"]}),
        Some(&control),
    );
    let prints = rig.broker.prints();
    assert_eq!(prints.len(), 1);
    assert_eq!(prints[0]["timelapse"], false, "the plate option was sent");
    let commands: Vec<_> = rig
        .broker
        .records
        .lock()
        .unwrap()
        .controls
        .iter()
        .filter_map(|c| {
            c.get("print")
                .or_else(|| c.get("system"))
                .map(|b| b["command"].clone())
        })
        .collect();
    for expected in ["pause", "resume", "stop", "ledctrl", "ams_filament_setting"] {
        assert!(
            commands.contains(&json!(expected)),
            "{expected}: {commands:?}"
        );
    }
    assert!(
        !rig.ftp
            .records
            .lock()
            .unwrap()
            .files
            .contains_key("/timelapse/video_1.mp4")
    );
}

/// Every main screen with the same data, measured and photographed. `E2E_UI_PHASE` names the run.
#[test]
#[ignore = "requires Chromium"]
fn ui_audit() {
    let mut rig = Rig::new("browser-ui-audit");
    // The production P1S report from 2026-10-03, with the AMS of the fixture so slots can be assigned.
    let production: serde_json::Value =
        serde_json::from_slice(&fixture("p1s_production_2026-10-03.json")).unwrap();
    let ams = rig.full["print"]["ams"].clone();
    rig.full = production;
    rig.full["print"]["ams"] = ams;
    rig.full["print"]["gcode_state"] = json!("IDLE");
    rig.launch();
    rig.seed();
    let plate = rig.configure(None, None);
    let first = rig.add(3);
    for _ in 0..2 {
        rig.add(3);
    }
    rig.next(&first, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    rig.report("RUNNING");
    rig.phase("printing");
    // The production failure: a device error, then FAILED named by its file.
    rig.report_index("RUNNING", None);
    let command = rig.broker.prints()[0].clone();
    let mut failed = rig.full.clone();
    failed["print"]["gcode_state"] = json!("RUNNING");
    failed["print"]["print_error"] = json!(0x0300_400C);
    failed["print"]["subtask_name"] = command["file"].clone();
    rig.broker.send(&failed);
    rig.phase("needs_attention");
    let control = rig.control();
    let phase = std::env::var("E2E_UI_PHASE").unwrap_or_else(|_| "after".into());
    rig.browser_env(
        "E2E_UI_AUDIT_CONTEXT",
        &json!({"plate":plate["id"]}),
        Some(&control),
        &[("E2E_UI_PHASE", phase)],
    );
}
