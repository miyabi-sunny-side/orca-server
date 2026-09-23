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
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
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
