//! Chromium assertions stay in TypeScript; Rust owns their isolated API and protocol peers.
use crate::common::*;
use crate::scenarios::import;
use crate::scenarios::plates;
use crate::scenarios::registry;
use serde_json::json;
fn appdir() -> std::path::PathBuf {
    std::env::var_os("ORCA_APPDIR")
        .expect("ORCA_APPDIR required for browser official CLI scenarios")
        .into()
}
#[test]
#[ignore = "requires Chromium"]
fn creation_defaults() {
    plates::creation_defaults(true);
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
fn plate_duplication() {
    plates::plate_duplication(true);
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
/// Real CLI: a plate names no machine, shows each printer's result, and starts with the
/// printer's process and bed over isolated MQTT/FTPS.
#[test]
#[ignore = "requires Chromium and official Orca 2.4.2"]
fn printer_slices() {
    let mut rig = Rig::with_options("browser-printer-slices", "v3", Some(&appdir()));
    rig.launch();
    rig.seed();
    rig.large_model();
    // The printer, not the plate, decides the process and bed of the start.
    let mut printer = printer_settings(&rig.get("/api/printers/p1"));
    printer["default_process_profile_key"] = json!("0.16mm Optimal @BBL X1C");
    printer["bed_type"] = json!("Cool Plate");
    rig.put("/api/printers/p1", &printer, 200);
    let (peer, mini) = rig.register_mini();
    rig.configure_material(id(&rig.materials[1]), "Bambu Lab A1 mini 0.4 nozzle");
    rig.load_material(&peer, &mini, &rig.materials[1]["id"]);
    rig.browser(
        "E2E_PRINTER_SLICES_CONTEXT",
        &json!({"mini":mini,"material":rig.materials[1]["id"],"model":"parts/large.stl"}),
        None,
    );
    until(|| rig.broker.prints().len() == 1, 30);
    assert!(peer.prints().is_empty());
    let uploaded = rig.ftp.contents().last().unwrap().clone();
    let settings = zip_json(&uploaded, "Metadata/project_settings.config");
    assert_eq!(settings["print_settings_id"], "0.16mm Optimal @BBL X1C");
    assert_eq!(settings["curr_bed_type"], "Cool Plate");
    assert_eq!(settings["printer_settings_id"], MACHINE);
    std::fs::write(rig.output.join("started.gcode.3mf"), uploaded).unwrap();
    rig.check();
}
