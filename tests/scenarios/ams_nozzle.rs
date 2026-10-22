use crate::common::peers::Action;
use crate::common::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
#[allow(clippy::too_many_lines)]
pub fn ams_priority(browser: bool) {
    let mut rig = Rig::new("ams-priority");
    rig.broker.allow_options.store(true, Ordering::SeqCst);
    rig.launch();
    rig.seed();
    let fid = rig.materials[0]["id"].clone();
    let root = "/api/printers/p1/ams";
    let resolve_path = format!(
        "{root}/resolve?filament_id={}",
        fid.as_str().expect("seeded material id is a string")
    );
    let inv = || rig.get(root);
    let resolve = || rig.get(&resolve_path);
    let options = || {
        rig.broker
            .records
            .lock()
            .expect("broker records lock is not poisoned")
            .options
            .clone()
    };
    assert_eq!(
        resolve()["preferred_slot"]["slot_index"],
        0,
        "the only loaded slot is preferred"
    );
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        409,
    );
    assert!(
        options().is_empty(),
        "rejected auto-refill sends no print option"
    );
    let mut report = rig.full.clone();
    report["print"]["ams"]["tray_exist_bits"] = json!("b");
    report["print"]["ams"]["ams"][0]["tray"][1] = merge(
        &report["print"]["ams"]["ams"][0]["tray"][1],
        &json!({"tray_type":"PLA","tray_color":"FFFFFFFF"}),
    );
    report["print"] = merge(
        &report["print"],
        &json!({"support_filament_backup":true,"home_flag":0,"filam_bak":[3]}),
    );
    rig.broker.send(&report);
    until(|| inv()["slots"][1]["reported"]["present"] == true, 12);
    let slot = inv()["slots"][1].clone();
    rig.put(
        &format!("{root}/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":fid}),
        204,
    );
    let group = resolve()["candidates"].clone();
    assert_eq!(
        array(&group)
            .iter()
            .map(|s| s["slot_index"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0), json!(1)],
        "both slots holding the material are candidates"
    );
    let priority = json!({"filament_id":fid,"order":array(&group).iter().rev().map(|s|json!({"id":s["id"],"revision":s["revision"]})).collect::<Vec<_>>()});
    rig.put(&format!("{root}/priority"), &priority, 204);
    assert_eq!(
        resolve()["preferred_slot"]["slot_index"],
        1,
        "reversed priority prefers slot 1"
    );
    rig.put(&format!("{root}/priority"), &priority, 409);
    assert_eq!(
        inv()["auto_refill"]["enabled"],
        false,
        "auto-refill starts disabled"
    );
    assert_eq!(
        inv()["auto_refill"]["supported"],
        true,
        "printer reports auto-refill support"
    );
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        202,
    );
    until(|| options().len() == 1, 12);
    assert_eq!(
        inv()["auto_refill"]["enabled"],
        false,
        "auto-refill waits for the printer to confirm"
    );
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"home_flag":1024}}));
    until(|| inv()["auto_refill"]["enabled"] == true, 12);
    assert_eq!(
        inv()["slots"][0]["backup_peers"],
        json!([1]),
        "slot 0 backs up to slot 1"
    );
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"support_filament_backup":false}}));
    until(|| inv()["auto_refill"]["supported"] == false, 12);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        409,
    );
    assert_eq!(
        options().len(),
        1,
        "unsupported auto-refill sends no print option"
    );
    if browser {
        let actions = rig.broker.actions.clone();
        let control = HttpPeer::new(move |_, _, body| {
            let value: Value = serde_json::from_slice(body).expect("control request body is JSON");
            actions
                .send(Action::Report(
                    serde_json::to_vec(&value).expect("JSON value serializes"),
                    false,
                    format!("device/{}/report", crate::common::peers::SERIAL),
                ))
                .expect("broker action channel is open");
            (204, vec![])
        });
        rig.browser(
            "E2E_AMS_CONTEXT",
            &json!({"resolve":resolve_path,"control":control.base}),
            None,
        );
    }
    let preferred = resolve()["preferred_slot"].clone();
    rig.full = report;
    let job = rig.add(0);
    assert_eq!(
        job["ams_slot_id"], preferred["id"],
        "new job uses the preferred slot"
    );
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(
        rig.broker.prints()[0]["ams_mapping"],
        json!([preferred["slot_index"]]),
        "print maps the preferred slot"
    );
    rig.full["print"]["ams"]["tray_now"] = json!(
        preferred["slot_index"]
            .as_u64()
            .expect("preferred slot has an index")
            .to_string()
    );
    rig.report("RUNNING");
    rig.phase("printing");
    let before = rig.queue()["current"].clone();
    let switched = i32::from(preferred["slot_index"] == 0);
    rig.broker.send(&json!({"print":{"command":"push_status","msg":1,"ams":{"tray_now":switched.to_string()},"mc_percent":20}}));
    until(|| rig.queue()["current"]["actual_ams_slot"] == switched, 12);
    let after = rig.queue()["current"].clone();
    for key in ["ams_slot_id", "id", "attempt_id"] {
        assert_eq!(after[key], before[key], "AMS switch keeps the job {key}");
    }
    assert_eq!(
        after["state"], "printing",
        "AMS switch keeps the job printing"
    );
    assert_eq!(
        rig.broker.prints().len(),
        1,
        "AMS switch sends no extra print"
    );
    assert_eq!(
        rig.ftp.contents().len(),
        1,
        "AMS switch uploads nothing extra"
    );
    let sent = rig
        .broker
        .records
        .lock()
        .expect("broker records lock is not poisoned")
        .options
        .len();
    rig.stop(false);
    rig.launch();
    assert_eq!(
        rig.get(root)["current"],
        false,
        "AMS inventory is stale after restart"
    );
    rig.request("GET", &resolve_path, None, 409);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":false}),
        409,
    );
    assert_eq!(
        rig.broker
            .records
            .lock()
            .expect("broker records lock is not poisoned")
            .options
            .len(),
        sent,
        "stale inventory sends no print option"
    );
}
#[allow(clippy::too_many_lines)]
#[allow(dead_code)] // API-only target uses this scenario.
pub fn nozzle_material() {
    let mut rig = Rig::new("nozzle-material");
    let path = rig
        .root
        .path()
        .join("app/resources/profiles/BBL/filament/1.json");
    write_json(
        &path,
        &merge(
            &json_file(&path),
            &json!({"required_nozzle_HRC":["60"],"compatible_printers":[MACHINE]}),
        ),
    );
    rig.full["print"] = merge(
        &rig.full["print"],
        &json!({"nozzle_diameter":"0.4","nozzle_type":"stainless_steel"}),
    );
    rig.full["print"]["ams"]["ams"][0]["tray"][0]["tray_type"] = json!("PETG");
    rig.launch();
    rig.seed();
    let settings = printer_settings(&rig.get("/api/printers/p1"));
    let requests = rig.broker.requests().len();
    rig.put(
        "/api/printers/p1",
        &merge(&settings, &json!({"nozzle_material":"stainless_steel"})),
        200,
    );
    until(|| rig.broker.requests().len() > requests, 12);
    rig.idle();
    let gf=rig.post("/api/filaments",&json!({"name":"PETG-GF 白","vendor":"Fixture","material":"PETG-GF","color":"FFFFFFFF","bambu_filament_id":null}),201);
    let data = json!({"machine_profile_key":MACHINE,"base_profile_key":"Generic PETG","overrides_json":{"nozzle_temperature_initial_layer":250,"nozzle_temperature":240}});
    let path = format!("/api/filaments/{}/settings", id(&gf));
    let saved = rig.post(&path, &data, 201);
    for machine in ["Bambu Lab P1S 0.2 nozzle", "Bambu Lab A1 mini 0.2 nozzle"] {
        rig.post(
            &path,
            &merge(&data, &json!({"machine_profile_key":machine})),
            400,
        );
    }
    rig.put(
        &format!("{path}/{}", id(&saved)),
        &merge(&data, &json!({"overrides_json":{"nozzle_temperature":999}})),
        400,
    );
    assert_eq!(
        rig.get(&format!("/api/filaments/{}", id(&gf)))["settings"][0]["overrides_json"],
        data["overrides_json"],
        "rejected settings keep the saved overrides"
    );
    let slot = rig.slot(0);
    rig.put(
        &format!("/api/printers/p1/ams/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":gf["id"]}),
        204,
    );
    let spec = rig.specification(0);
    rig.configure(Some(spec.clone()), None);
    let admission_path = format!("/api/queue?printer_id=p1&plate_id={}", id(&rig.plate));
    assert_eq!(
        rig.get(&admission_path)["admission"]["allowed"],
        true,
        "matching nozzle admits the plate"
    );
    // The printer owns the machine and nozzle; a plate cannot name another one.
    for machine in ["Bambu Lab P1S 0.2 nozzle", "Bambu Lab A1 mini 0.2 nozzle"] {
        let mut data = edit(&rig.plate);
        data["conditions"]["required_machine_profile_key"] = json!(machine);
        let path = format!("/api/plates/{}", id(&rig.plate));
        rig.put(&path, &data, 422);
        assert_eq!(
            rig.get(&path),
            rig.plate,
            "rejected machine {machine} leaves the plate unchanged"
        );
    }
    let action = rig.add_action(Some(spec), None);
    let job = array(&rig.send(action, 200)["waiting"])
        .last()
        .expect("added job is waiting")
        .clone();
    for (field, value) in [
        ("nozzle_diameter", "0.2"),
        ("nozzle_type", "hardened_steel"),
    ] {
        let mut report = rig.full.clone();
        report["print"][field] = json!(value);
        rig.broker.send(&report);
        let key = if field == "nozzle_type" {
            "nozzle_material"
        } else {
            field
        };
        until(|| rig.queue()["printer"][key] == value, 12);
        assert_eq!(
            rig.get(&admission_path)["admission"]["allowed"],
            false,
            "mismatched {field} blocks admission"
        );
        rig.next(&job, 409);
        assert!(
            rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty(),
            "mismatched {field} neither prints nor uploads"
        );
        rig.idle();
        until(
            || rig.queue()["printer"][key] == rig.full["print"][field],
            12,
        );
    }
    assert_eq!(
        rig.get(&admission_path)["admission"]["allowed"],
        true,
        "restored nozzle admits the plate"
    );
    assert_eq!(
        rig.get("/api/printers/p1")["nozzle_material"],
        "stainless_steel",
        "printer keeps the configured nozzle material"
    );
    assert!(
        rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty(),
        "nothing printed before the next command"
    );
    let command = rig.command(
        json!({"type":"next","expected_job":job["id"],"removed_job":null,"cleared":true}),
        None,
    );
    rig.post("/api/queue?printer_id=p1", &command, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.post("/api/queue?printer_id=p1", &command, 200);
    assert_eq!(
        rig.broker.prints().len(),
        1,
        "repeated next command prints once"
    );
    assert_eq!(
        rig.ftp.uploads().len(),
        1,
        "repeated next command uploads once"
    );
    assert_eq!(
        rig.broker.prints()[0]["ams_mapping"],
        json!([0]),
        "print maps AMS slot 0"
    );
    let execution: Value = serde_json::from_str(&rig.stored(&job, "execution_json"))
        .expect("stored execution is JSON");
    let resolved = &execution["profiles"]["filament.json"];
    assert_eq!(
        resolved["nozzle_temperature"],
        json!(["240"]),
        "filament profile uses the override nozzle temperature"
    );
    assert_eq!(
        resolved["nozzle_temperature_initial_layer"],
        json!(["250"]),
        "filament profile uses the override initial layer temperature"
    );
    assert_eq!(
        resolved["required_nozzle_HRC"],
        json!(["60"]),
        "filament profile keeps the required nozzle hardness"
    );
    rig.finish();
    assert_eq!(rig.broker.prints().len(), 1, "finish sends no extra print");
    rig.ftp.check();
}

/// The AMS page's "状態を更新" through the real server to the isolated printer and back.
#[allow(dead_code)] // Only the Chromium target runs this scenario.
pub fn ams_refresh() {
    let mut rig = Rig::new("ams-refresh-browser");
    rig.launch();
    rig.seed();
    let mut white = rig.full.clone();
    white["print"]["ams"]["tray_exist_bits"] = json!("b");
    white["print"]["ams"]["ams"][0]["tray"][1] =
        json!({"id":"1","tray_type":"PLA","tray_color":"FFFFFFFF"});
    let mut reading = white.clone();
    reading["print"]["ams"]["tray_reading_bits"] = json!("8");
    reading["print"]["ams"]["tray_read_done_bits"] = json!("3");
    let reply = rig.broker.pushall_reply.clone();
    let records = rig.broker.records.clone();
    let actions = rig.broker.actions.clone();
    let control = HttpPeer::new(move |method, _, body| {
        if method == "POST" {
            let value: Value = serde_json::from_slice(body).expect("control request body is JSON");
            if value.get("reply").is_some() {
                *reply.lock().expect("pushall reply lock is not poisoned") = (!value["reply"]
                    .is_null())
                .then(|| serde_json::to_vec(&value["reply"]).expect("JSON value serializes"));
            }
            if let Some(report) = value.get("send") {
                actions
                    .send(Action::Report(
                        serde_json::to_vec(report).expect("JSON value serializes"),
                        false,
                        format!("device/{}/report", crate::common::peers::SERIAL),
                    ))
                    .expect("broker action channel is open");
            }
            if value["disconnect"] == true {
                actions
                    .send(Action::Disconnect)
                    .expect("broker action channel is open");
            }
        }
        let pushes = records
            .lock()
            .expect("broker records lock is not poisoned")
            .requests
            .len();
        (
            200,
            serde_json::to_vec(&json!({"pushes":pushes})).expect("JSON value serializes"),
        )
    });
    rig.browser(
        "E2E_AMS_REFRESH_CONTEXT",
        &json!({"control":control.base,"white":white,"reading":reading}),
        None,
    );
    rig.check();
}
