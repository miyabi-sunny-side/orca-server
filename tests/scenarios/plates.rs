use crate::common::peers::{Peer, SECRET};
use crate::common::*;
use serde_json::{Value, json};
use std::fs;
fn save(rig: &Rig, plate: &Value, conditions: Value) -> Value {
    let current = rig.get(&format!("/api/plates/{}", id(plate)));
    let mut body = edit(&current);
    body["conditions"] = conditions;
    rig.put(&format!("/api/plates/{}", id(plate)), &body, 200)
}
fn admission(rig: &Rig, plate: &Value, pid: &str) -> Value {
    rig.get(&format!(
        "/api/queue?printer_id={pid}&plate_id={}",
        id(plate)
    ))
}
fn add(rig: &Rig, plate: &Value, pid: &str, expected: u16) -> Value {
    let q = admission(rig, plate, pid);
    rig.post(
        &format!("/api/queue?printer_id={pid}"),
        &rig.command(
            json!({"type":"add","plate_id":plate["id"],"plate_version":plate["version"]}),
            Some(&q),
        ),
        expected,
    )
}
fn create(rig: &Rig, conditions: Option<Value>) -> Value {
    let mut body = json!({"name":"Initial defaults","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":2}]});
    if let Some(conditions) = conditions {
        body["conditions"] = conditions;
    }
    rig.post("/api/plates/import", &body, 201)
}
fn nullable(conditions: &Value) -> Value {
    Value::Object(
        conditions
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !matches!(k.as_str(), "brim_enabled" | "support_enabled"))
            .map(|k| (k.clone(), Value::Null))
            .collect(),
    )
}
fn new_peer(rig: &Rig, serial: &str, machine: &str) -> (Peer, Value) {
    let peer = Peer::broker(rig.root.path(), "trusted", serial);
    let settings = printer_settings(&rig.get("/api/printers/p1"));
    let device=rig.post("/api/printers",&merge(&settings,&json!({"name":serial,"serial":serial,"mqtt_port":peer.port,"machine_profile_key":machine,"access_code":SECRET,"tls_certificate":fs::read_to_string(rig.root.path().join("trusted.pem")).unwrap()})),201);
    until(|| !peer.requests().is_empty(), 12);
    peer.send(&rig.full);
    until(
        || {
            rig.get(&format!("/api/printer/status?printer_id={}", id(&device)))["synchronized"]
                == true
        },
        12,
    );
    (peer, device)
}
fn assign(rig: &Rig, pid: &str, index: u64, material: Value) {
    let inventory = rig.get(&format!("/api/printers/{pid}/ams"));
    let slot = array(&inventory["slots"])
        .iter()
        .find(|s| s["slot_index"] == index)
        .unwrap();
    let mut body = json!({"revision":slot["revision"]});
    body["filament_id"] = material;
    rig.put(&format!("/api/printers/{pid}/ams/{}", id(slot)), &body, 204);
}

pub fn plate_duplication(browser: bool) {
    let mut rig = Rig::new("plate-duplication");
    for name in [
        "gridfinity/base-front.stl",
        "gridfinity/base-back.stl",
        "gridfinity/bin.stl",
    ] {
        rig.files
            .lock()
            .unwrap()
            .insert(name.into(), fixture("cube.stl"));
    }
    rig.launch();
    rig.seed();
    let base = rig.configure(None, None);
    let conditions = merge(
        &base["conditions"],
        &json!({"wall_loops":4,"sparse_infill_density":30,"sparse_infill_pattern":"gyroid","brim_enabled":true,"support_enabled":true,"support_interface_filament_id":rig.materials[1]["id"]}),
    );
    let source = rig.post("/api/plates/import", &json!({"name":"天馬ルームケース: base前","models":[{"name":"gridfinity/base-front.stl","source":"gridfinity/base-front.stl","quantity":10}],"conditions":conditions}),201);
    let path = format!("/api/plates/{}/duplicate", id(&source));
    let copy = rig.post(&path, &json!({"name":"API copy"}), 201);
    assert_eq!(copy["conditions"], source["conditions"]);
    assert_ne!(copy["models"][0]["id"], source["models"][0]["id"]);
    for body in [json!({"name":""}), json!({"name":"x","models":[]})] {
        rig.post(
            &path,
            &body,
            if body.get("models").is_some() {
                422
            } else {
                400
            },
        );
    }
    rig.request("DELETE", &format!("/api/plates/{}", id(&copy)), None, 204);
    rig.post(
        &format!("/api/plates/{}/duplicate", id(&copy)),
        &json!({"name":"deleted"}),
        404,
    );
    if browser {
        rig.browser(
            "E2E_DUPLICATE_CONTEXT",
            &json!({"source":source,"second_material":rig.materials[0]["id"]}),
            None,
        );
    }
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&source))), source);
    assert!(array(&rig.queue()["waiting"]).is_empty());
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.stop(false);
    rig.launch();
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&source))), source);
}

#[allow(clippy::too_many_lines)]
pub fn plate_admission(browser: bool) {
    let mut rig = Rig::new("plate-admission");
    rig.launch();
    rig.seed();
    let mut base = rig.configure(None, None);
    let fid = rig.materials[1]["id"].clone();
    let conditions = base["conditions"].clone();
    let mini = "Bambu Lab A1 mini 0.2 nozzle";
    for key in [
        None,
        Some("required_machine_profile_key"),
        Some("filament_id"),
        Some("process_profile_key"),
        Some("bed_type"),
    ] {
        let mut missing = conditions.clone();
        if let Some(key) = key {
            missing[key] = Value::Null;
        } else {
            missing = nullable(&conditions);
        }
        if missing["required_machine_profile_key"].is_null() {
            missing["process_profile_key"] = Value::Null;
        }
        let plate = save(&rig, &base, missing);
        assert_eq!(admission(&rig, &plate, "p1")["admission"]["allowed"], false);
        add(&rig, &plate, "p1", 409);
        assert_eq!(rig.queue()["waiting"], json!([]));
    }
    base = save(&rig, &base, conditions.clone());
    for (key, value, status) in [
        ("required_machine_profile_key", "unowned machine", 409),
        ("filament_id", "missing material", 404),
        ("process_profile_key", "wrong process", 400),
        ("bed_type", "wrong bed", 400),
    ] {
        let mut data = edit(&base);
        data["conditions"][key] = json!(value);
        rig.put(&format!("/api/plates/{}", id(&base)), &data, status);
        assert_eq!(rig.get(&format!("/api/plates/{}", id(&base))), base);
    }
    rig.post(
        &format!("/api/filaments/{}/settings", fid.as_str().unwrap()),
        &json!({"machine_profile_key":mini,"base_profile_key":FILAMENT,"overrides_json":{}}),
        201,
    );
    let peers: Vec<_> = (0..3)
        .map(|n| new_peer(&rig, &format!("MINI{n}"), mini))
        .collect();
    assert_eq!(array(&rig.get("/api/printers")).len(), 4);
    add(&rig, &base, id(&peers[0].1), 409);
    let mini_plate=rig.post("/api/plates/import",&json!({"name":"mini plate","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":10}],"conditions":merge(&conditions,&json!({"required_machine_profile_key":mini}))}),201);
    for (_, device) in &peers {
        assert_eq!(
            admission(&rig, &mini_plate, id(device))["admission"]["allowed"],
            false
        );
        add(&rig, &mini_plate, id(device), 409);
    }
    assign(&rig, id(&peers[2].1), 3, fid.clone());
    assert_eq!(
        admission(&rig, &mini_plate, id(&peers[2].1))["admission"]["allowed"],
        true
    );
    add(&rig, &mini_plate, id(&peers[2].1), 200);
    for (_, device) in &peers[..2] {
        add(&rig, &mini_plate, id(device), 409);
    }
    for bits in ["1", "0"] {
        let mut report = rig.full.clone();
        report["print"]["ams"]["tray_exist_bits"] = json!(bits);
        rig.broker.send(&report);
        until(|| rig.slot(3)["reported"]["present"] != true, 12);
        assert_eq!(admission(&rig, &base, "p1")["admission"]["allowed"], false);
        add(&rig, &base, "p1", 409);
    }
    rig.idle();
    until(|| rig.slot(3)["reported"]["present"] == true, 12);
    assign(&rig, "p1", 3, fid.clone());
    let mut busy = rig.full.clone();
    busy["print"]["gcode_state"] = json!("RUNNING");
    rig.broker.send(&busy);
    until(|| rig.queue()["printer"]["ready_to_print"] == false, 12);
    assert_eq!(admission(&rig, &base, "p1")["admission"]["allowed"], true);
    add(&rig, &base, "p1", 200);
    assert!(rig.broker.prints().is_empty() && peers.iter().all(|(p, _)| p.prints().is_empty()));
    let q = admission(&rig, &base, "p1");
    let request = rig.command(
        json!({"type":"add","plate_id":base["id"],"plate_version":base["version"]}),
        Some(&q),
    );
    let old_version = base["version"].clone();
    base = save(
        &rig,
        &base,
        merge(&conditions, &json!({"bed_type":"High Temp Plate"})),
    );
    rig.post("/api/queue?printer_id=p1", &request, 409);
    add(
        &rig,
        &merge(&base, &json!({"version":old_version})),
        "p1",
        409,
    );
    assert_eq!(rig.queue()["waiting"][0]["bed_type"], "High Temp Plate");
    rig.idle();
    for (_, device) in &peers[..2] {
        assign(&rig, id(device), 3, fid.clone());
    }
    if browser {
        rig.browser("E2E_PLATE_CONTEXT",&json!({"devices":peers.iter().map(|(_,d)|id(d)).collect::<Vec<_>>(),"filament":fid,"machine":MACHINE,"mini":mini,"process":PROCESS,"bed":BED}),None);
    }
    rig.stop(false);
    rig.launch();
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&base))), base);
    assert_eq!(admission(&rig, &base, "p1")["admission"]["allowed"], false);
    add(&rig, &base, "p1", 409);
    assert!(rig.broker.prints().is_empty() && peers.iter().all(|(p, _)| p.prints().is_empty()));
    rig.stop(false);
}

#[allow(clippy::too_many_lines)]
pub fn creation_defaults(browser: bool) {
    let mut rig = Rig::new("creation-defaults");
    let quality = "0.16mm Fixture quality";
    let path = rig
        .root
        .path()
        .join("app/resources/profiles/BBL/process/standard.json");
    let mut extra = json_file(&path);
    extra["name"] = json!(quality);
    write_json(&path.with_file_name("quality.json"), &extra);
    rig.launch();
    rig.seed();
    let defaults = || rig.get("/api/default-settings");
    let first = defaults();
    assert_eq!(first["default_printer_id"], "p1");
    assert_eq!(
        first["conditions"],
        json!({"required_machine_profile_key":MACHINE,"filament_id":rig.materials[0]["id"],"process_profile_key":PROCESS,"bed_type":BED,"sparse_infill_pattern":"adaptivecubic","sparse_infill_density":15.0,"wall_loops":2,"brim_enabled":false,"support_enabled":false,"support_interface_filament_id":null})
    );
    assert!(first["reason"].is_null());
    let mut settings = printer_settings(&rig.get("/api/printers/p1"));
    settings["bed_type"] = json!("Cool Plate");
    let requests = rig.broker.requests().len();
    rig.put("/api/printers/p1", &settings, 200);
    let initial = create(&rig, Some(nullable(&first["conditions"])));
    assert_eq!(
        initial["conditions"],
        merge(&first["conditions"], &json!({"bed_type":"Cool Plate"}))
    );
    let uploaded = rig.upload("Uploaded defaults", &[("cube.stl", fixture("cube.stl"))]);
    assert_eq!(uploaded["conditions"], initial["conditions"]);
    assert_eq!(
        rig.response(
            "PUT",
            "/api/default-settings",
            Some(&json!({"default_printer_id":null})),
            Some("https://outside.invalid")
        )
        .0,
        403
    );
    let manual = create(
        &rig,
        Some(json!({"filament_id":rig.materials[1]["id"],"bed_type":"High Temp Plate"})),
    );
    assert_eq!(
        manual["conditions"],
        merge(
            &initial["conditions"],
            &json!({"filament_id":rig.materials[1]["id"],"bed_type":"High Temp Plate"})
        )
    );
    let mut old = save(&rig, &initial, nullable(&first["conditions"]));
    assert!(old["conditions"].as_object().unwrap().iter().all(|(k, v)| {
        if matches!(k.as_str(), "brim_enabled" | "support_enabled") {
            v == false
        } else {
            v.is_null()
        }
    }));
    assert_eq!(rig.broker.requests().len(), requests);
    assert!(rig.broker.prints().is_empty());
    let unsupported=rig.post("/api/filaments",&json!({"name":"No machine setting","vendor":"Fixture","material":"PLA","color":"FFFFFFFF","bambu_filament_id":null}),201);
    assign(&rig, "p1", 0, unsupported["id"].clone());
    assert_eq!(
        defaults()["conditions"]["filament_id"],
        rig.materials[1]["id"]
    );
    assign(&rig, "p1", 0, Value::Null);
    assert_eq!(
        defaults()["conditions"]["filament_id"],
        rig.materials[1]["id"]
    );
    let mut report = rig.full.clone();
    report["print"]["ams"]["tray_exist_bits"] = json!("1");
    rig.broker.send(&report);
    until(|| defaults()["conditions"]["filament_id"].is_null(), 12);
    assert_eq!(defaults()["reason"], "material");
    rig.idle();
    until(|| rig.slot(3)["reported"]["present"] == true, 12);
    assign(&rig, "p1", 0, rig.materials[0]["id"].clone());
    assign(&rig, "p1", 3, rig.materials[1]["id"].clone());
    if browser {
        rig.browser("E2E_DEFAULTS_CONTEXT",&json!({"machine":MACHINE,"process":PROCESS,"bed":"Cool Plate","first":rig.materials[0]["id"],"second":rig.materials[1]["id"],"old":old["id"]}),None);
    }
    old = save(&rig, &old, nullable(&first["conditions"]));
    // MCP's independently discovered creation_defaults_match_rest covers the same creation/clear contract.
    rig.plate = manual.clone();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    let before = rig.queue()["current"].clone();
    let traces = rig.traces();
    let requests = rig.broker.requests().len();
    let uploads = rig.ftp.uploads().len();
    settings = printer_settings(&rig.get("/api/printers/p1"));
    settings["bed_type"] = json!("High Temp Plate");
    settings["default_process_profile_key"] = json!(quality);
    rig.put("/api/printers/p1", &settings, 200);
    assert_eq!(rig.queue()["current"], before);
    assert_eq!(rig.traces(), traces);
    assert_eq!(rig.broker.requests().len(), requests);
    assert_eq!(rig.ftp.uploads().len(), uploads);
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&manual))), rig.plate);
    settings["machine_profile_key"] = json!("Bambu Lab P1S 0.2 nozzle");
    rig.put("/api/printers/p1", &settings, 409);
    let peers: Vec<_> = (0..2)
        .map(|n| new_peer(&rig, &format!("DEFAULT{n}"), MACHINE))
        .collect();
    let chosen = id(&peers[0].1);
    let other = id(&peers[1].1);
    assert_eq!(rig.get("/api/default-settings")["default_printer_id"], "p1");
    if browser {
        rig.browser(
            "E2E_DEFAULTS_CONTEXT",
            &json!({"devices":[chosen,other]}),
            None,
        );
    }
    rig.put(
        "/api/default-settings",
        &json!({"default_printer_id":chosen}),
        204,
    );
    assert!(rig.get("/api/default-settings")["conditions"]["filament_id"].is_null());
    rig.put(
        "/api/default-settings",
        &json!({"default_printer_id":"missing"}),
        404,
    );
    assert_eq!(
        rig.get("/api/default-settings")["default_printer_id"],
        chosen
    );
    rig.stop(false);
    rig.launch();
    let defaults = rig.get("/api/default-settings");
    assert_eq!(defaults["default_printer_id"], chosen);
    assert!(defaults["conditions"]["filament_id"].is_null());
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&old))), old);
    rig.request("DELETE", &format!("/api/printers/{chosen}"), None, 204);
    let defaults = rig.get("/api/default-settings");
    assert!(defaults["default_printer_id"].is_null());
    assert_eq!(defaults["reason"], "printer_selection");
    rig.request("DELETE", &format!("/api/printers/{other}"), None, 204);
    assert_eq!(rig.get("/api/default-settings")["default_printer_id"], "p1");
    assert_eq!(
        rig.db()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        16
    );
    let db = rig.db();
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(default_settings)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "id",
            "default_printer_id",
            "sparse_infill_pattern",
            "sparse_infill_density",
            "wall_loops"
        ]
    );
    let rows: Vec<String> = db
        .prepare("SELECT default_printer_id FROM default_settings")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, ["p1"]);
    assert_eq!(rig.broker.prints().len(), 1);
    assert!(peers.iter().all(|(p, _)| p.prints().is_empty()));
    rig.stop(false);
}
#[allow(dead_code)] // API-only target uses this recovery scenario.
pub fn recover_default_process() {
    let mut rig = Rig::new("default-recovery");
    rig.launch();
    rig.stop(false);
    rig.db()
        .execute(
            "UPDATE printers SET default_process_profile_key='removed profile' WHERE id='p1'",
            [],
        )
        .unwrap();
    // This invalid registration cannot start MQTT until its default process is corrected.
    let port = rig.env.remove("P1_MQTT_PORT").unwrap();
    rig.launch();
    rig.env.insert("P1_MQTT_PORT".into(), port);
    let broken = rig.get("/api/printers/p1");
    assert!(!broken["configuration_error"].is_null());
    let mut settings = printer_settings(&broken);
    settings["default_process_profile_key"] = json!(PROCESS);
    let requests = rig.broker.requests().len();
    let fixed = rig.put("/api/printers/p1", &settings, 200);
    assert!(fixed["configuration_error"].is_null());
    until(|| rig.broker.requests().len() > requests, 12);
    rig.idle();
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}

#[allow(clippy::too_many_lines)]
pub fn filament_picker(browser: bool) {
    let mut rig = Rig::new("filament-picker");
    rig.launch();
    rig.seed();
    let create_material = |name: &str, color: &str| {
        rig.post("/api/filaments", &json!({"name":name,"vendor":"Fixture","material":"PLA","color":color,"bambu_filament_id":null}),201)
    };
    let other = create_material("別のP1Sに装填した長い製品名・PLA 黄", "FFFF00FF");
    let mini_material = create_material("mini専用 PLA 赤", "FF0000FF");
    let future = create_material("将来用・未装填の長い製品名 PETG-GF 黒", "000000FF");
    rig.post(
        &format!("/api/filaments/{}/settings", id(&other)),
        &json!({"machine_profile_key":MACHINE,"base_profile_key":FILAMENT,"overrides_json":{}}),
        201,
    );
    rig.post(
        &format!("/api/filaments/{}/settings", id(&future)),
        &json!({"machine_profile_key":MACHINE,"base_profile_key":FILAMENT,"overrides_json":{}}),
        201,
    );
    let mini = "Bambu Lab A1 mini 0.2 nozzle";
    let peers = [
        new_peer(&rig, "SECOND", MACHINE),
        new_peer(&rig, "MINI", mini),
    ];
    assign(&rig, id(&peers[0].1), 0, other["id"].clone());
    assign(&rig, id(&peers[0].1), 3, rig.materials[0]["id"].clone());
    assign(&rig, id(&peers[1].1), 0, mini_material["id"].clone());
    let mut url = reqwest::Url::parse("http://fixture/api/plate-filaments").unwrap();
    url.query_pairs_mut().append_pair("machine", MACHINE);
    let path = format!("{}?{}", url.path(), url.query().unwrap());
    let candidates = rig.get(&path);
    let ids: Vec<_> = array(&candidates["filaments"])
        .iter()
        .map(|f| id(f).to_owned())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&id(&other).to_owned()));
    assert!(!ids.contains(&id(&mini_material).to_owned()));
    assert_eq!(
        array(&rig.get("/api/plate-filaments")["filaments"]).len(),
        4
    );
    assert_eq!(
        array(&rig.get("/api/plate-filaments?include_unloaded=true")["filaments"]).len(),
        5
    );
    assert_eq!(array(&rig.get("/api/filaments")).len(), 5);
    let spec = merge(&rig.specification(0), &json!({"filament_id":other["id"]}));
    let plate = rig.configure(Some(spec), None);
    assert_eq!(admission(&rig, &plate, "p1")["admission"]["allowed"], false);
    add(&rig, &plate, "p1", 409);
    assert_eq!(
        admission(&rig, &plate, id(&peers[0].1))["admission"]["allowed"],
        true
    );
    rig.configure(Some(rig.specification(0)), None);
    if browser {
        let actions = rig.broker.actions.clone();
        let second = peers[0].0.actions.clone();
        let third = peers[1].0.actions.clone();
        let full = rig.full.clone();
        let db_path = rig.store.join("orca.sqlite3");
        // The fixture alone can change observations or seed a broken historical reference.
        let control = HttpPeer::new(move |_, _, body| {
            let v: Value = serde_json::from_slice(body).unwrap();
            if v["missing"] == true {
                let db = rusqlite::Connection::open(&db_path).unwrap();
                db.pragma_update(None, "foreign_keys", false).unwrap();
                db.execute(
                    "UPDATE plates SET filament_id='missing-material' WHERE id=?1",
                    [v["plate_id"].as_str().unwrap()],
                )
                .unwrap();
            } else {
                for (sender, serial) in [
                    (&actions, crate::common::peers::SERIAL),
                    (&second, "SECOND"),
                    (&third, "MINI"),
                ] {
                    if v["offline"] == true {
                        sender
                            .send(crate::common::peers::Action::Disconnect)
                            .unwrap();
                    } else {
                        let mut report = full.clone();
                        if v["empty"] == true {
                            report["print"]["ams"]["tray_exist_bits"] = json!("0");
                        }
                        sender
                            .send(crate::common::peers::Action::Report(
                                serde_json::to_vec(&report).unwrap(),
                                false,
                                format!("device/{serial}/report"),
                            ))
                            .unwrap();
                    }
                }
            }
            (200, b"{}".to_vec())
        });
        rig.browser("E2E_FILAMENT_PICKER_CONTEXT",&json!({"plate":rig.plate["id"],"white":rig.materials[0]["id"],"blue":rig.materials[1]["id"],"other":other["id"],"mini_material":mini_material["id"],"future":future["id"],"machine":MACHINE,"mini":mini,"second":peers[0].1["id"],"control":control.base}),None);
    }
    peers[0].0.action(crate::common::peers::Action::Disconnect);
    until(
        || {
            rig.get(&path)["printers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == peers[0].1["id"] && p["state"] == "unconfirmed")
        },
        12,
    );
    assert!(
        !array(&rig.get(&path)["filaments"])
            .iter()
            .any(|f| f["id"] == other["id"])
    );
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}
