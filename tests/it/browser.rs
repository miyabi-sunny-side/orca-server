//! Chromium assertions stay in TypeScript; Rust owns their isolated API and protocol peers.
use crate::common::*;
use serde_json::json;

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
