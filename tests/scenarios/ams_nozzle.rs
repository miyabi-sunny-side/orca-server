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
    let resolve_path = format!("{root}/resolve?filament_id={}", fid.as_str().unwrap());
    let inv = || rig.get(root);
    let resolve = || rig.get(&resolve_path);
    let options = || rig.broker.records.lock().unwrap().options.clone();
    assert_eq!(resolve()["preferred_slot"]["slot_index"], 0);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        409,
    );
    assert!(options().is_empty());
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
        vec![json!(0), json!(1)]
    );
    let priority = json!({"filament_id":fid,"order":array(&group).iter().rev().map(|s|json!({"id":s["id"],"revision":s["revision"]})).collect::<Vec<_>>()});
    rig.put(&format!("{root}/priority"), &priority, 204);
    assert_eq!(resolve()["preferred_slot"]["slot_index"], 1);
    rig.put(&format!("{root}/priority"), &priority, 409);
    assert_eq!(inv()["auto_refill"]["enabled"], false);
    assert_eq!(inv()["auto_refill"]["supported"], true);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        202,
    );
    until(|| options().len() == 1, 12);
    assert_eq!(inv()["auto_refill"]["enabled"], false);
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"home_flag":1024}}));
    until(|| inv()["auto_refill"]["enabled"] == true, 12);
    assert_eq!(inv()["slots"][0]["backup_peers"], json!([1]));
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"support_filament_backup":false}}));
    until(|| inv()["auto_refill"]["supported"] == false, 12);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":true}),
        409,
    );
    assert_eq!(options().len(), 1);
    if browser {
        let actions = rig.broker.actions.clone();
        let control = HttpPeer::new(move |_, _, body| {
            let value: Value = serde_json::from_slice(body).unwrap();
            actions
                .send(Action::Report(
                    serde_json::to_vec(&value).unwrap(),
                    false,
                    format!("device/{}/report", crate::common::peers::SERIAL),
                ))
                .unwrap();
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
    assert_eq!(job["ams_slot_id"], preferred["id"]);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(
        rig.broker.prints()[0]["ams_mapping"],
        json!([preferred["slot_index"]])
    );
    rig.full["print"]["ams"]["tray_now"] =
        json!(preferred["slot_index"].as_u64().unwrap().to_string());
    rig.report("RUNNING");
    rig.phase("printing");
    let before = rig.queue()["current"].clone();
    let switched = i32::from(preferred["slot_index"] == 0);
    rig.broker.send(&json!({"print":{"command":"push_status","msg":1,"ams":{"tray_now":switched.to_string()},"mc_percent":20}}));
    until(|| rig.queue()["current"]["actual_ams_slot"] == switched, 12);
    let after = rig.queue()["current"].clone();
    for key in ["ams_slot_id", "id", "attempt_id"] {
        assert_eq!(after[key], before[key]);
    }
    assert_eq!(after["state"], "printing");
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.contents().len(), 1);
    let sent = rig.broker.records.lock().unwrap().options.len();
    rig.stop(false);
    rig.launch();
    assert_eq!(rig.get(root)["current"], false);
    rig.request("GET", &resolve_path, None, 409);
    rig.put(
        &format!("{root}/auto-refill"),
        &json!({"enabled":false}),
        409,
    );
    assert_eq!(rig.broker.records.lock().unwrap().options.len(), sent);
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
        data["overrides_json"]
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
    assert_eq!(rig.get(&admission_path)["admission"]["allowed"], true);
    for machine in ["Bambu Lab P1S 0.2 nozzle", "Bambu Lab A1 mini 0.2 nozzle"] {
        let mut data = edit(&rig.plate);
        data["conditions"]["required_machine_profile_key"] = json!(machine);
        let path = format!("/api/plates/{}", id(&rig.plate));
        rig.put(&path, &data, 409);
        assert_eq!(rig.get(&path), rig.plate);
    }
    let action = rig.add_action(Some(spec), None);
    let job = array(&rig.send(action, 200)["waiting"])
        .last()
        .unwrap()
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
        assert_eq!(rig.get(&admission_path)["admission"]["allowed"], false);
        rig.next(&job, 409);
        assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
        rig.idle();
        until(
            || rig.queue()["printer"][key] == rig.full["print"][field],
            12,
        );
    }
    assert_eq!(rig.get(&admission_path)["admission"]["allowed"], true);
    assert_eq!(
        rig.get("/api/printers/p1")["nozzle_material"],
        "stainless_steel"
    );
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    let command = rig.command(
        json!({"type":"next","expected_job":job["id"],"removed_job":null,"cleared":true}),
        None,
    );
    rig.post("/api/queue?printer_id=p1", &command, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.post("/api/queue?printer_id=p1", &command, 200);
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.uploads().len(), 1);
    assert_eq!(rig.broker.prints()[0]["ams_mapping"], json!([0]));
    let resolved = rig.traces()[0]["profiles"]["filament"].clone();
    assert_eq!(resolved["nozzle_temperature"], json!(["240"]));
    assert_eq!(resolved["nozzle_temperature_initial_layer"], json!(["250"]));
    assert_eq!(resolved["required_nozzle_HRC"], json!(["60"]));
    rig.finish();
    assert_eq!(rig.broker.prints().len(), 1);
    rig.ftp.check();
}
