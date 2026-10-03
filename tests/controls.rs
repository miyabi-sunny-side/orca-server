mod common;
use common::peers::Action;
use common::*;
use serde_json::{Value, json};

fn control(rig: &Rig, body: &Value, expected: u16) -> Value {
    rig.post("/api/printers/p1/control", body, expected)
}
/// Operator controls received, without the version request sent on every connection.
fn operator(rig: &Rig) -> Vec<Value> {
    rig.broker
        .records
        .lock()
        .unwrap()
        .controls
        .iter()
        .filter(|c| c.get("info").is_none())
        .cloned()
        .collect()
}
fn status(rig: &Rig) -> Value {
    rig.get("/api/printer/status?printer_id=p1")
}
/// A report naming the current attempt the way the production P1S did on 2026-10-01:
/// the file name as `subtask_name` and no `gcode_file`.
fn report_as_file(rig: &Rig, state: &str, error: u64) {
    let command = rig.broker.prints().last().unwrap().clone();
    let mut full = rig.full.clone();
    full["print"]["gcode_state"] = json!(state);
    full["print"]["print_error"] = json!(error);
    full["print"]["subtask_name"] = command["file"].clone();
    full["print"]["gcode_file"] = json!("");
    rig.broker.send(&full);
}

#[test]
fn controls_are_sent_once_and_answered_by_the_printer() {
    let mut rig = Rig::new("controls-replies");
    rig.launch();
    rig.idle();

    let reply = control(&rig, &json!({"action":"light","on":false}), 200);
    assert_eq!(reply, json!({"reply":"success","reason":null}));
    let reply = control(&rig, &json!({"action":"fan","fan":"aux","percent":50}), 200);
    assert_eq!(reply["reply"], "success");
    let controls = operator(&rig);
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0]["system"]["command"], "ledctrl");
    assert_eq!(controls[0]["system"]["led_mode"], "off");
    assert_eq!(controls[1]["print"]["param"], "M106 P2 S128\n");
    let journal = rig.get("/api/journal?kind=request&direction=out&limit=10");
    assert!(
        array(&journal["entries"])
            .iter()
            .any(|e| e["body"]["system"]["command"] == "ledctrl"),
        "controls are recorded"
    );

    *rig.broker.control_reply.lock().unwrap() = Some("fail".into());
    assert_eq!(
        control(&rig, &json!({"action":"pause"}), 200)["reply"],
        "rejected"
    );
    *rig.broker.control_reply.lock().unwrap() = None;
    let silent = control(&rig, &json!({"action":"resume"}), 200);
    assert_eq!(silent["reply"], "none");
    assert!(silent["reason"].as_str().unwrap().contains("No reply"));

    for (bad, code) in [
        (json!({"action":"speed","level":9}), 400),
        (json!({"action":"nozzle_temperature","celsius":400}), 400),
        (json!({"action":"gcode","line":"M999"}), 422),
    ] {
        rig.request("POST", "/api/printers/p1/control", Some(&bad), code);
    }
    rig.request(
        "POST",
        "/api/printers/missing/control",
        Some(&json!({"action":"pause"})),
        404,
    );
    assert_eq!(operator(&rig).len(), 4, "invalid controls are never sent");

    // Without a synchronized connection nothing is sent.
    rig.broker.action(Action::Disconnect);
    until(|| status(&rig)["synchronized"] == false, 12);
    control(&rig, &json!({"action":"pause"}), 409);
    assert_eq!(operator(&rig).len(), 4);
    rig.check();
}

#[test]
fn live_status_reports_temperatures_fans_light_and_hms() {
    let mut rig = Rig::new("controls-live");
    rig.launch();
    let mut full = rig.full.clone();
    full["print"]["nozzle_temper"] = json!(212.5);
    full["print"]["bed_temper"] = json!(60);
    full["print"]["big_fan1_speed"] = json!("15");
    full["print"]["lights_report"] = json!([{"node":"chamber_light","mode":"on"}]);
    full["print"]["hms"] = json!([{"attr":0x0300_0D00_u32,"code":0x0001_0004}]);
    rig.broker.send(&full);
    until(
        || status(&rig)["live"]["temperatures"]["nozzle"] == 212.5,
        12,
    );
    rig.broker.send(
        &json!({"print":{"command":"push_status","msg":1,"nozzle_temper":180,
        "lights_report":[{"node":"chamber_light","mode":"off"}]}}),
    );
    until(|| status(&rig)["live"]["light"] == false, 12);
    let live = status(&rig)["live"].clone();
    assert_eq!(live["temperatures"]["nozzle"], 180.0);
    assert_eq!(
        live["temperatures"]["bed"], 60.0,
        "differences keep other values"
    );
    assert_eq!(live["fans"]["aux"], 100);
    assert_eq!(live["hms"], json!(["0300_0D00_0001_0004"]));
    let firmware = status(&rig)["firmware"].clone();
    assert_eq!(
        firmware,
        json!([{"name":"ota","sw_ver":"01.08.02.00","hw_ver":""}])
    );
    rig.check();
}

#[test]
fn pause_resume_and_stop_follow_the_printer_and_keep_recovery() {
    let mut rig = Rig::new("controls-pause");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    rig.report("RUNNING");
    rig.phase("printing");

    let objects = rig.queue()["current"]["objects"].clone();
    assert_eq!(
        objects,
        json!([{"id":45,"name":"0.stl"},{"id":56,"name":"1.stl"}])
    );
    assert_eq!(
        control(&rig, &json!({"action":"skip_objects","objects":[56]}), 200)["reply"],
        "success"
    );
    assert_eq!(
        control(&rig, &json!({"action":"pause"}), 200)["reply"],
        "success"
    );
    rig.report("PAUSE");
    rig.start_phase("paused");
    let q = rig.queue();
    assert_eq!(
        q["current"]["state"], "printing",
        "a pause is not a failure"
    );
    assert_eq!(
        q["allowed"],
        json!({"next":false,"retry":false,"discard":false})
    );
    assert_eq!(
        control(&rig, &json!({"action":"resume"}), 200)["reply"],
        "success"
    );
    rig.report("RUNNING");
    rig.start_phase("printing");

    assert_eq!(
        control(&rig, &json!({"action":"stop"}), 200)["reply"],
        "success"
    );
    rig.report("FAILED");
    rig.phase("needs_attention");
    let q = rig.queue();
    assert_eq!(q["current"]["failure"]["kind"], "stopped");
    assert_eq!(q["allowed"]["retry"], true);
    assert_eq!(q["allowed"]["discard"], true);
    let kinds: Vec<_> = operator(&rig)
        .iter()
        .map(|c| c["print"]["command"].clone())
        .collect();
    assert_eq!(
        kinds,
        [
            json!("skip_objects"),
            json!("pause"),
            json!("resume"),
            json!("stop")
        ]
    );
    assert_eq!(operator(&rig)[0]["print"]["obj_list"], json!([56]));
    assert_eq!(rig.broker.prints().len(), 1, "controls never start a print");
    rig.check();
}

/// The production case from 2026-10-01: after a device error the printer reported FAILED with the
/// file name as job name and no file, which was judged as another job and blocked recovery.
#[test]
fn a_failed_attempt_reported_by_file_name_can_be_retried_or_discarded() {
    let mut rig = Rig::new("controls-identity");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    report_as_file(&rig, "RUNNING", 0);
    rig.phase("printing");
    report_as_file(&rig, "RUNNING", 0x0300_400C);
    rig.phase("needs_attention");
    report_as_file(&rig, "FAILED", 0);
    until(|| rig.queue()["allowed"]["retry"] == true, 12);
    let d = rig.get("/api/printers/p1/diagnostics");
    assert_eq!(d["decision"]["evidence"]["identity"], "target");
    assert_eq!(rig.queue()["allowed"]["discard"], true);
    // An unrelated job's FAILED report still does not release it.
    let mut other = rig.full.clone();
    other["print"]["gcode_state"] = json!("FAILED");
    other["print"]["subtask_name"] = json!("someone-else.gcode.3mf");
    rig.broker.send(&other);
    until(|| rig.queue()["allowed"]["retry"] == false, 12);
    rig.check();
}

#[test]
fn start_options_are_saved_on_the_plate_and_sent_with_the_start() {
    let mut rig = Rig::new("controls-start-options");
    rig.launch();
    rig.seed();
    let plate = rig.configure(None, None);
    assert_eq!(
        plate["conditions"]["start_options"],
        json!({"bed_leveling":true,"flow_calibration":true,"timelapse":true,"vibration_calibration":false}),
        "BambuStudio defaults"
    );
    let mut data = edit(&plate);
    data["conditions"]["start_options"] = json!({"timelapse":false,"bed_leveling":false});
    let saved = rig.put(&format!("/api/plates/{}", id(&plate)), &data, 200);
    assert_eq!(saved["conditions"]["start_options"]["timelapse"], false);
    assert_eq!(
        saved["conditions"]["start_options"]["flow_calibration"],
        true
    );
    rig.plate = saved;
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 20);
    let start = &rig.broker.prints()[0];
    assert_eq!(
        (
            &start["timelapse"],
            &start["bed_leveling"],
            &start["flow_cali"],
            &start["vibration_cali"]
        ),
        (&json!(false), &json!(false), &json!(true), &json!(false))
    );
    data["conditions"]["start_options"] = json!({"layer_inspect":true});
    rig.put(&format!("/api/plates/{}", id(&plate)), &data, 422);
    rig.check();
}
