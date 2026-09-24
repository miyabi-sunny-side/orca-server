use crate::common::peers::{Action, Peer, SECRET};
use crate::common::*;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, thread, time::Duration};
fn profiles(rig: &Rig, machine: &str) -> Value {
    let mut url = reqwest::Url::parse("http://fixture/api/slicer/profiles").unwrap();
    url.query_pairs_mut().append_pair("machine", machine);
    rig.get(&format!("{}?{}", url.path(), url.query().unwrap()))
}
fn queue(rig: &Rig, pid: &str) -> Value {
    rig.get(&format!("/api/queue?printer_id={pid}"))
}
fn command(rig: &Rig, pid: &str, action: Value, status: u16) -> Value {
    let q = queue(rig, pid);
    rig.post(
        &format!("/api/queue?printer_id={pid}"),
        &rig.command(action, Some(&q)),
        status,
    )
}
fn status(rig: &Rig, pid: &str) -> Value {
    rig.get(&format!("/api/printer/status?printer_id={pid}"))
}
fn inventory(rig: &Rig, pid: &str) -> Value {
    rig.get(&format!("/api/printers/{pid}/ams"))
}
fn mapping(rig: &Rig, pid: &str, slot: &Value, material: Value, expected: u16) {
    let mut body = json!({"revision":slot["revision"]});
    body["filament_id"] = material;
    rig.put(
        &format!("/api/printers/{pid}/ams/{}", id(slot)),
        &body,
        expected,
    );
}
fn slot(rig: &Rig, pid: &str, index: u64) -> Value {
    array(&inventory(rig, pid)["slots"])
        .iter()
        .find(|s| s["ams_id"] == 0 && s["slot_index"] == index)
        .unwrap()
        .clone()
}
#[allow(clippy::too_many_lines)]
pub fn registry(appdir: Option<&Path>, browser: bool) {
    let mut rig = Rig::with_options("registry", "v1", appdir);
    rig.env
        .retain(|key, _| !key.starts_with("P1_") && key != "SCAD_LIVE_URL");
    let pem = fs::read_to_string(rig.root.path().join("trusted.pem")).unwrap();
    let peers: Vec<_> = (0..2)
        .map(|i| {
            (
                Peer::broker(rig.root.path(), "trusted", &format!("PRINTER{i}")),
                Peer::ftps(rig.root.path(), "trusted"),
            )
        })
        .collect();
    let full: Value = serde_json::from_slice(&fixture("p1_status.json")).unwrap();
    let plate_id = uuid::Uuid::new_v4().to_string();
    let revision = uuid::Uuid::new_v4().to_string();
    let artifacts = rig.store.join(&plate_id).join("revisions").join(&revision);
    fs::create_dir_all(&artifacts).unwrap();
    fs::write(artifacts.join("0.stl"), fixture("cube.stl")).unwrap();
    fs::write(
        artifacts.join("print.gcode.3mf"),
        fixture("p1_print.gcode.3mf"),
    )
    .unwrap();
    write_json(
        &rig.store.join(&plate_id).join("plate.json"),
        &json!({"format_version":1,"id":plate_id,"revision":revision,"name":"Routing cube","settings":{},"models":[{"name":"cube.stl","path":format!("revisions/{revision}/0.stl"),"source":null}],"project":null,"print":format!("revisions/{revision}/print.gcode.3mf")}),
    );
    let a1 = "Bambu Lab A1 mini 0.2 nozzle";
    rig.launch();
    assert_eq!(rig.get("/api/printers"), json!([]));
    assert!(
        array(&rig.get("/api/printers/profiles"))
            .iter()
            .any(|m| m["key"] == a1 && m["nozzle_diameter"] == "0.2")
    );
    let a1_profiles = profiles(&rig, a1);
    let p1_profiles = rig.get("/api/slicer/profiles");
    assert_eq!(a1_profiles["defaults"]["machine"], a1);
    assert!(!array(&a1_profiles["processes"]).contains(&p1_profiles["defaults"]["process"]));
    rig.request("GET", "/api/slicer/profiles?machine=unknown", None, 400);
    if browser {
        rig.browser_env("E2E_REGISTRY_SETTINGS",&json!({"name":"UI second","host":"127.0.0.1","serial":"UISECOND","access_code":SECRET,"tls_certificate":pem,"machine_profile_key":MACHINE,"default_process_profile_key":p1_profiles["defaults"]["process"],"bed_type":p1_profiles["defaults"]["bed"],"nozzle_material":"unknown"}),None,&[("E2E_REGISTRY_CERT",rig.root.path().join("trusted.pem").display().to_string())]);
    }
    let mut settings = Vec::new();
    let mut ids = Vec::new();
    for (i, profiles) in [&p1_profiles, &a1_profiles].iter().enumerate() {
        let data = json!({"name":format!("Printer {i}"),"host":"127.0.0.1","serial":format!("PRINTER{i}"),"access_code":SECRET,"tls_certificate":pem,"machine_profile_key":profiles["printer"],"default_process_profile_key":profiles["defaults"]["process"],"bed_type":profiles["defaults"]["bed"],"nozzle_material":"unknown","mqtt_port":peers[i].0.port,"ftps_port":peers[i].1.port,"start_timeout_secs":60});
        let saved = rig.post("/api/printers", &data, 201);
        ids.push(id(&saved).to_owned());
        settings.push(data);
    }
    rig.post("/api/printers", &settings[0], 409);
    for (i, (peer, _)) in peers.iter().enumerate() {
        until(|| peer.requests().len() == 1, 12);
        let mut report = full.clone();
        report["print"]["mc_percent"] = json!(i * 17);
        peer.send(&report);
        until(|| status(&rig, &ids[i])["ready_to_print"] == true, 12);
    }
    assert_eq!(status(&rig, &ids[0])["print"]["percent"], 0);
    assert_eq!(status(&rig, &ids[1])["print"]["percent"], 17);
    rig.request("GET", "/api/printer/status", None, 409);
    rig.request("GET", "/api/printer/status?printer_id=missing", None, 404);
    let material=rig.post("/api/filaments",&json!({"name":"PLA for routing","vendor":"Fixture","material":"PLA","color":"FFFFFFFF","bambu_filament_id":null}),201);
    rig.post(&format!("/api/filaments/{}/settings",id(&material)),&json!({"machine_profile_key":MACHINE,"base_profile_key":p1_profiles["defaults"]["filament"],"overrides_json":{}}),201);
    for pid in &ids {
        mapping(&rig, pid, &slot(&rig, pid, 0), material["id"].clone(), 204);
    }
    let path = format!("/api/plates/{plate_id}");
    let mut saved = rig.get(&path);
    assert!(
        saved["conditions"]
            .as_object()
            .unwrap()
            .iter()
            .all(
                |(k, v)| if matches!(k.as_str(), "brim_enabled" | "support_enabled") {
                    v == false
                } else {
                    v.is_null()
                }
            )
    );
    saved = edit(&saved);
    saved["conditions"] = json!({"filament_id":material["id"],"required_machine_profile_key":MACHINE,"process_profile_key":p1_profiles["defaults"]["process"],"bed_type":p1_profiles["defaults"]["bed"]});
    saved = rig.put(&path, &saved, 200);
    let add = json!({"type":"add","plate_id":plate_id,"plate_version":saved["version"]});
    command(&rig, &ids[1], add.clone(), 409);
    let other = command(&rig, &ids[0], add.clone(), 200)["waiting"][0].clone();
    let mut changed = merge(
        &settings[1],
        &json!({"machine_profile_key":MACHINE,"default_process_profile_key":p1_profiles["defaults"]["process"],"access_code":"","tls_certificate":""}),
    );
    rig.put(&format!("/api/printers/{}", ids[1]), &changed, 200);
    until(|| peers[1].0.requests().len() == 2, 12);
    peers[1].0.send(&full);
    until(|| status(&rig, &ids[1])["ready_to_print"] == true, 12);
    mapping(
        &rig,
        &ids[1],
        &slot(&rig, &ids[1], 0),
        material["id"].clone(),
        204,
    );
    let view = command(&rig, &ids[1], add, 200);
    let job = view["waiting"][0].clone();
    assert_eq!(array(&view["waiting"]).len(), 1);
    assert_eq!(array(&queue(&rig, &ids[0])["waiting"]).len(), 1);
    assert_eq!(view["waiting"][0]["required_machine_profile_key"], MACHINE);
    rig.request("DELETE", &format!("/api/printers/{}", ids[1]), None, 409);
    changed["name"] = json!("Second edited");
    rig.put(&format!("/api/printers/{}", ids[1]), &changed, 200);
    command(
        &rig,
        &ids[1],
        json!({"type":"next","expected_job":job["id"],"removed_job":null,"cleared":true}),
        200,
    );
    rig.put(&format!("/api/printers/{}", ids[1]), &settings[1], 409);
    command(
        &rig,
        &ids[0],
        json!({"type":"next","expected_job":other["id"],"removed_job":null,"cleared":true}),
        200,
    );
    until(|| peers.iter().all(|(p, _)| p.prints().len() == 1), 90);
    for (pid, (_, ftp)) in ids.iter().zip(&peers) {
        assert_eq!(ftp.uploads().len(), 1);
        let current = queue(&rig, pid)["current"].clone();
        assert_eq!(
            ftp.contents()[0],
            fs::read(
                rig.store
                    .join(current["artifact_path"].as_str().unwrap())
                    .join("print.gcode.3mf")
            )
            .unwrap()
        );
        rig.request("DELETE", &format!("/api/printers/{pid}"), None, 409);
    }
    rig.stop(false);
    rig.env.insert(
        "P1_IP".into(),
        "not-a-valid-IP-ignored-after-initialization".into(),
    );
    rig.launch();
    let listed = rig.get("/api/printers");
    let got: std::collections::BTreeSet<_> = array(&listed).iter().map(id).collect();
    assert_eq!(got, ids.iter().map(String::as_str).collect());
    assert_eq!(
        array(&listed).iter().find(|p| p["id"] == ids[1]).unwrap()["name"],
        "Second edited"
    );
    thread::sleep(Duration::from_millis(500));
    assert!(peers.iter().all(|(p, _)| p.prints().len() == 1));
    for (pid, (peer, _)) in ids.iter().zip(&peers) {
        rig.request("DELETE", &format!("/api/printers/{pid}"), None, 409);
        until(|| peer.requests().len() >= 2, 12);
        peer.send(&full);
        until(|| status(&rig, pid)["ready_to_print"] == true, 12);
        let job = queue(&rig, pid)["current"].clone();
        assert_eq!(job["state"], "needs_attention");
        command(
            &rig,
            pid,
            json!({"type":"discard","expected_job":job["id"],"cleared":true}),
            200,
        );
        rig.request("DELETE", &format!("/api/printers/{pid}"), None, 204);
    }
    rig.stop(false);
    rig.launch();
    assert_eq!(rig.get("/api/printers"), json!([]));
    assert_eq!(
        fs::metadata(rig.store.join("orca.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
    rig.stop(false);
}
fn delta(tray: Value) -> Value {
    let mut report = json!({"print":{"command":"push_status","msg":1,"ams":{"ams":[{"id":"0"}]}}});
    report["print"]["ams"]["ams"][0]["tray"] = Value::Array(vec![tray]);
    report
}
#[allow(clippy::too_many_lines)]
pub fn filament_ams(appdir: Option<&Path>, browser: bool) {
    let mut rig = Rig::with_options("filament-ams", "v1", appdir);
    rig.env
        .retain(|k, _| !k.starts_with("P1_") && k != "SCAD_LIVE_URL");
    let pem = fs::read_to_string(rig.root.path().join("trusted.pem")).unwrap();
    let peers: Vec<_> = (0..2)
        .map(|i| Peer::broker(rig.root.path(), "trusted", &format!("MATERIAL{i}")))
        .collect();
    if appdir.is_none() {
        let root = rig.root.path().join("app/resources/profiles/BBL/filament");
        let mut profile = json_file(&root.join("0.json"));
        profile["name"] = json!("Bambu PLA Matte @BBL X1C");
        profile["compatible_printers"] = json!([MACHINE]);
        let petg_path = root.join("1.json");
        let mut petg = json_file(&petg_path);
        petg["compatible_printers"] = json!([MACHINE]);
        write_json(&petg_path, &petg);
        write_json(&root.join("matte.json"), &profile);
    }
    let full = json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":"f","ams_exist_bits":"1","insert_flag":true,"power_on_flag":true,"ams":[{"id":"0","tray":[{"id":"0","tray_type":"PETG","tray_color":"FFFFFFFF","nozzle_temp_min":240,"nozzle_temp_max":260,"remain":-1},{"id":"1","tray_type":"PLA","tray_info_idx":"GFA01","tray_sub_brands":"PLA Matte","tray_color":"000000FF","tag_uid":"SYNTHETICBLACK","remain":42},{"id":"2","tray_type":"PLA","tray_info_idx":"GFA01","tray_sub_brands":"PLA Matte","tray_color":"FFFFFFFF","tag_uid":"SYNTHETICWHITE","remain":-1},{"id":"3","tray_type":"PETG","tray_color":"00AAFFFF","nozzle_temp_min":220,"nozzle_temp_max":260,"remain":50}]}]}}});
    rig.launch();
    assert_eq!(rig.get("/api/filaments"), json!([]));
    let profiles = rig.get("/api/slicer/profiles");
    let machine = profiles["printer"].as_str().unwrap();
    let mut ids = Vec::new();
    for (i, peer) in peers.iter().enumerate() {
        let saved=rig.post("/api/printers",&json!({"name":format!("材料確認 {i}"),"host":"127.0.0.1","serial":format!("MATERIAL{i}"),"access_code":SECRET,"tls_certificate":pem,"machine_profile_key":machine,"default_process_profile_key":profiles["defaults"]["process"],"bed_type":profiles["defaults"]["bed"],"nozzle_material":"stainless_steel","mqtt_port":peer.port,"ftps_port":1}),201);
        ids.push(id(&saved).to_owned());
    }
    let mut materials = Vec::new();
    for (name, vendor, kind, color, bid) in [
        (
            "ガラス繊維入りPETG",
            "Third party",
            "PETG-GF",
            "FFFFFFFF",
            None,
        ),
        (
            "PLA Matte 黒",
            "Bambu Lab",
            "PLA",
            "000000FF",
            Some("GFA01"),
        ),
        (
            "PLA Matte 白",
            "Bambu Lab",
            "PLA",
            "FFFFFFFF",
            Some("GFA01"),
        ),
        ("透明ブルーPETG", "Third party", "PETG", "00AAFFFF", None),
    ] {
        materials.push(rig.post("/api/filaments",&json!({"name":name,"vendor":vendor,"material":kind,"color":color,"bambu_filament_id":bid}),201));
    }
    let [gf, black, white, petg] = <&[Value; 4]>::try_from(materials.as_slice()).unwrap();
    let mut settings = Vec::new();
    for (i, material) in materials.iter().enumerate() {
        let mut url = reqwest::Url::parse(&format!(
            "http://fixture/api/filaments/{}/profiles",
            id(material)
        ))
        .unwrap();
        url.query_pairs_mut().append_pair("machine", machine);
        let choices = rig.get(&format!("{}?{}", url.path(), url.query().unwrap()));
        let key = if matches!(i, 1 | 2) {
            "Bambu PLA Matte @BBL X1C"
        } else {
            "Generic PETG"
        };
        let choice = array(&choices).iter().find(|p| p["key"] == key).unwrap();
        let overrides = if matches!(i, 1 | 2) {
            json!({})
        } else {
            json!({"nozzle_temperature_initial_layer":if i==0{250}else{240},"nozzle_temperature":if i==0{240}else{220}})
        };
        settings.push(rig.post(&format!("/api/filaments/{}/settings",id(material)),&json!({"machine_profile_key":machine,"base_profile_key":choice["key"],"overrides_json":overrides}),201));
    }
    let data = |v: &Value| {
        let mut data = edit(v);
        data.as_object_mut().unwrap().remove("filament_id");
        data
    };
    let path = format!("/api/filaments/{}/settings", id(gf));
    rig.post(&path, &data(&settings[0]), 409);
    rig.post(
        &path,
        &merge(
            &data(&settings[0]),
            &json!({"machine_profile_key":"Bambu Lab A1 mini 0.2 nozzle"}),
        ),
        400,
    );
    rig.post(
        &path,
        &merge(
            &data(&settings[0]),
            &json!({"overrides_json":{"filament_start_gcode":"G1 X1"}}),
        ),
        422,
    );
    for peer in &peers {
        until(|| peer.requests().len() == 1, 12);
        peer.send(&full);
    }
    until(
        || {
            inventory(&rig, &ids[0])["current"] == true
                && array(&inventory(&rig, &ids[0])["slots"]).len() == 4
        },
        12,
    );
    until(|| inventory(&rig, &ids[1])["current"] == true, 12);
    let slots = inventory(&rig, &ids[0])["slots"].clone();
    assert_eq!(
        array(&slots)
            .iter()
            .map(|s| s["filament_id"].clone())
            .collect::<Vec<_>>(),
        vec![
            Value::Null,
            black["id"].clone(),
            white["id"].clone(),
            Value::Null
        ]
    );
    assert!(slots[0]["reported"]["remaining_percent"].is_null());
    assert_eq!(slots[1]["reported"]["brand"], "PLA Matte");
    assert_eq!(slots[1]["detect_on_insert"], true);
    assert_eq!(slots[1]["detect_on_power_up"], true);
    assert_eq!(slots[2]["setting"]["overrides_json"], json!({}));
    assert!(
        !slots[2]["setting"]["resolved"]["nozzle_temperature"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    mapping(&rig, &ids[0], &slots[0], gf["id"].clone(), 204);
    mapping(&rig, &ids[0], &slots[3], petg["id"].clone(), 204);
    let slots = inventory(&rig, &ids[0])["slots"].clone();
    assert_eq!(slots[0]["filament"]["material"], "PETG-GF");
    assert_eq!(slots[0]["reported"]["material"], "PETG");
    assert_eq!(
        slots[0]["setting"]["resolved"]["nozzle_temperature_initial_layer"],
        "250"
    );
    assert_eq!(slots[0]["setting"]["resolved"]["nozzle_temperature"], "240");
    assert_eq!(slots[3]["reported"]["temperature_max"], 260);
    assert_eq!(slots[3]["setting"]["resolved"]["nozzle_temperature"], "220");
    assert!(inventory(&rig, &ids[1])["slots"][0]["filament_id"].is_null());
    rig.request("DELETE", &format!("/api/filaments/{}", id(gf)), None, 409);
    if browser {
        let actions = peers[0].actions.clone();
        let full = full.clone();
        let control = HttpPeer::new(move |_, _, body| {
            let value: Value = serde_json::from_slice(body).unwrap();
            if value["disconnect"] == true {
                actions.send(Action::Disconnect).unwrap();
            } else {
                let value = if value["full"] == true {
                    full.clone()
                } else {
                    delta(value)
                };
                actions
                    .send(Action::Report(
                        serde_json::to_vec(&value).unwrap(),
                        false,
                        "device/MATERIAL0/report".into(),
                    ))
                    .unwrap();
            }
            (204, vec![])
        });
        rig.browser("E2E_FILAMENT_CONTEXT",&json!({"printer":ids[0],"gf":gf["id"],"black":black["id"],"white":white["id"],"petg":petg["id"],"machine":machine,"control":control.base}),None);
    }
    peers[0].send(&delta(json!({"id":"0","tray_color":"000000FF"})));
    until(
        || {
            rig.db()
                .query_row(
                    "SELECT filament_id FROM ams_slots WHERE printer_id=?1 AND slot_index=0",
                    [&ids[0]],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap()
                .is_none()
        },
        12,
    );
    peers[0].send(&delta(json!({"id":"0","tray_color":"FFFFFFFF"})));
    until(
        || inventory(&rig, &ids[0])["slots"][0]["reported"]["color"] == "FFFFFFFF",
        12,
    );
    assert!(inventory(&rig, &ids[0])["slots"][0]["filament_id"].is_null());
    mapping(&rig, &ids[0], &slots[0], gf["id"].clone(), 409);
    mapping(
        &rig,
        &ids[0],
        &inventory(&rig, &ids[0])["slots"][0],
        gf["id"].clone(),
        204,
    );
    mapping(
        &rig,
        &ids[0],
        &inventory(&rig, &ids[0])["slots"][1],
        gf["id"].clone(),
        204,
    );
    peers[0].send(&delta(
        json!({"id":"1","tray_color":"FFFFFFFF","tag_uid":"REPLACEMENTWHITE"}),
    ));
    until(
        || inventory(&rig, &ids[0])["slots"][1]["filament_id"] == white["id"],
        12,
    );
    peers[0].send(&delta(json!({"id":"1","tray_info_idx":""})));
    until(
        || inventory(&rig, &ids[0])["slots"][1]["filament_id"].is_null(),
        12,
    );
    assert_eq!(
        inventory(&rig, &ids[1])["slots"][1]["filament_id"],
        black["id"]
    );
    rig.stop(false);
    rig.launch();
    let current = inventory(&rig, &ids[0]);
    assert_eq!(current["current"], false);
    assert_eq!(array(&current["slots"]).len(), 4);
    assert!(
        array(&current["slots"])
            .iter()
            .all(|s| s["current"] == false)
    );
    mapping(&rig, &ids[0], &current["slots"][0], gf["id"].clone(), 409);
    assert_eq!(
        rig.get(&format!("/api/filaments/{}", id(gf)))["settings"][0]["resolved"]["nozzle_temperature"],
        "240"
    );
    for peer in &peers {
        until(|| peer.requests().len() >= 2, 12);
        peer.send(&full);
    }
    until(|| inventory(&rig, &ids[0])["current"] == true, 12);
    assert_eq!(
        inventory(&rig, &ids[0])["slots"][1]["filament_id"],
        black["id"]
    );
    assert_eq!(
        rig.db()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        16
    );
    assert!(
        peers
            .iter()
            .all(|p| p.prints().is_empty() && p.records.lock().unwrap().options.is_empty())
    );
    rig.stop(false);
}
