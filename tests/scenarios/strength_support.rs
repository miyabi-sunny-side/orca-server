use crate::common::peers::Action;
use crate::common::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
const KEYS: [&str; 3] = [
    "sparse_infill_pattern",
    "sparse_infill_density",
    "wall_loops",
];
fn strength(value: &Value) -> Value {
    Value::Object(
        KEYS.iter()
            .map(|k| ((*k).to_owned(), value["conditions"][k].clone()))
            .collect(),
    )
}
#[allow(clippy::too_many_lines)]
pub fn strength_settings(browser: bool) {
    let mut rig = Rig::new("strength-settings");
    let default = json!({"sparse_infill_pattern":"adaptivecubic","sparse_infill_density":15.0,"wall_loops":2});
    let path = rig
        .root
        .path()
        .join("app/resources/profiles/BBL/process/standard.json");
    write_json(
        &path.with_file_name("quality.json"),
        &merge(
            &json_file(&path),
            &json!({"name":"0.16mm Fixture quality","top_shell_layers":"7","bottom_shell_layers":"4","top_shell_thickness":"0.9"}),
        ),
    );
    rig.launch();
    rig.seed();
    let defaults = rig.get("/api/default-settings");
    assert_eq!(strength(&defaults), default);
    assert_eq!(strength(&rig.plate), default);
    let patterns = array(&defaults["infill_patterns"]);
    assert_eq!(patterns.len(), 26);
    let unique: std::collections::BTreeSet<_> =
        patterns.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(unique.len(), 26);
    for pattern in patterns {
        let mut url = reqwest::Url::parse("http://fixture/api/slicer/process").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("machine", MACHINE),
            ("process", PROCESS),
            ("sparse_infill_pattern", pattern.as_str().unwrap()),
            ("sparse_infill_density", "100"),
            ("wall_loops", "3"),
        ]);
        let preview = rig.get(&format!("{}?{}", url.path(), url.query().unwrap()));
        assert_eq!(preview["sparse_infill_pattern"], *pattern);
        assert_eq!(preview["sparse_infill_density"], "100%");
        assert_eq!(preview["top_shell_layers"], "8");
        assert_eq!(preview["bottom_shell_layers"], "5");
    }
    for (change, status) in [
        (json!({"sparse_infill_pattern":"unknown"}), 400),
        (json!({"sparse_infill_density":100.1}), 400),
        (json!({"sparse_infill_density":-1}), 400),
        (json!({"wall_loops":2.5}), 422),
        (json!({"wall_loops":-1}), 422),
        (json!({"wall_loops":1001}), 400),
        (json!({"unknown":true}), 422),
    ] {
        rig.put(
            "/api/default-settings",
            &merge(&json!({"default_printer_id":"p1"}), &change),
            status,
        );
        let path = format!("/api/plates/{}", id(&rig.plate));
        let mut plate = edit(&rig.get(&path));
        plate["conditions"] = merge(&plate["conditions"], &change);
        rig.put(&path, &plate, status);
    }
    let original = rig.plate.clone();
    let changed =
        json!({"sparse_infill_pattern":"gyroid","sparse_infill_density":22.5,"wall_loops":4});
    rig.put(
        "/api/default-settings",
        &merge(&json!({"default_printer_id":"p1"}), &changed),
        204,
    );
    rig.put(
        "/api/default-settings",
        &json!({"default_printer_id":"p1"}),
        204,
    );
    assert_eq!(strength(&rig.get("/api/default-settings")), changed);
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&original))), original);
    let new=rig.post("/api/plates/import",&json!({"name":"New defaults","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":1}]}),201);
    assert_eq!(strength(&new), changed);
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(strength(&rig.get("/api/default-settings")), changed);
    assert_eq!(
        strength(&rig.get(&format!("/api/plates/{}", id(&new)))),
        changed
    );
    let job = rig.add(3);
    let first = rig.estimated(&job);
    assert_eq!(
        first["input"]["settings"]["profiles"]["process.json"]["sparse_infill_pattern"],
        "adaptivecubic"
    );
    rig.edit_conditions(&changed);
    let second = rig.estimated(&job);
    assert_ne!(first["id"], second["id"]);
    let process = &second["input"]["settings"]["profiles"]["process.json"];
    assert_eq!(
        (
            process["wall_loops"].as_str(),
            process["top_shell_layers"].as_str(),
            process["bottom_shell_layers"].as_str(),
            process["top_shell_thickness"].as_str()
        ),
        (Some("4"), Some("10"), Some("6"), Some("2"))
    );
    assert_eq!(process["sparse_infill_density"], "22.5%");
    assert_eq!(process["sparse_infill_pattern"], "gyroid");
    assert_eq!(process["brim_type"], "no_brim");
    assert_eq!(rig.plate["conditions"]["brim_enabled"], false);
    let mut previous = second;
    for enabled in [true, false] {
        rig.edit_conditions(&json!({"brim_enabled":enabled}));
        let current = rig.estimated(&job);
        assert_ne!(current["id"], previous["id"]);
        let process = &current["input"]["settings"]["profiles"]["process.json"];
        assert_eq!(
            process["brim_type"],
            if enabled { "outer_only" } else { "no_brim" }
        );
        assert_eq!(process["brim_width"], "5");
        assert_eq!(process["brim_object_gap"], "0.1");
        assert_eq!(process["wall_loops"], "4");
        assert_eq!(process["sparse_infill_density"], "22.5%");
        previous = current;
    }
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    let traces = rig.traces().len();
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(rig.traces().len(), traces);
    rig.report("RUNNING");
    rig.phase("printing");
    let frozen = rig.stored(&job, "execution_json");
    let attempt = rig.stored(&job, "attempt_json");
    let artifact = rig.stored(&job, "artifact_path");
    rig.edit_conditions(&default);
    rig.put(
        "/api/default-settings",
        &merge(&json!({"default_printer_id":"p1"}), &default),
        204,
    );
    assert_eq!(rig.stored(&job, "execution_json"), frozen);
    assert_eq!(rig.stored(&job, "attempt_json"), attempt);
    assert_eq!(
        serde_json::from_str::<Value>(&frozen).unwrap()["profiles"]["process.json"],
        previous["input"]["settings"]["profiles"]["process.json"]
    );
    let queued = rig.add(3);
    rig.estimated(&queued);
    rig.stop(false);
    let db = rig.db();
    legacy_schema::queue_v16(&db);
    for table in ["plates", "default_settings"] {
        for key in KEYS {
            db.execute(&format!("ALTER TABLE {table} DROP COLUMN {key}"), [])
                .unwrap();
        }
    }
    db.execute_batch("DROP TABLE print_history; ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; DROP TABLE plate_imports; PRAGMA user_version=9;").unwrap();
    drop(db);
    rig.launch();
    rig.report("RUNNING");
    rig.phase("printing");
    rig.estimated(&queued);
    assert_eq!(rig.stored(&job, "execution_json"), frozen);
    assert_eq!(rig.stored(&job, "attempt_json"), attempt);
    assert_eq!(rig.stored(&job, "artifact_path"), artifact);
    assert!(
        strength(&rig.get(&format!("/api/plates/{}", id(&rig.plate))))
            .as_object()
            .unwrap()
            .values()
            .all(Value::is_null)
    );
    assert_eq!(strength(&rig.get("/api/default-settings")), default);
    assert_eq!(
        rig.get(&format!("/api/plates/{}", id(&rig.plate)))["conditions"]["brim_enabled"],
        false
    );
    let legacy = rig.estimated(&queued)["input"]["settings"]["profiles"]["process.json"].clone();
    assert_eq!(legacy["sparse_infill_pattern"], "crosshatch");
    assert_eq!(legacy["wall_loops"], "2");
    assert_eq!(legacy["top_shell_layers"], "5");
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.ftp.uploads().len(), 1);
    rig.send(json!({"type":"remove","job_id":queued["id"]}), 200);
    if browser {
        rig.browser(
            "E2E_STRENGTH_CONTEXT",
            &json!({"legacy":rig.plate["id"],"material":rig.materials[0]["id"]}),
            None,
        );
    }
}

#[allow(dead_code)] // API-only target uses this scenario.
pub fn legacy_support() {
    let mut rig = Rig::new("legacy-support");
    rig.env.insert("P1_START_TIMEOUT_SECS".into(), "2".into());
    rig.launch();
    rig.seed();
    let job = rig.add(0);
    rig.next(&job, 200);
    rig.start_phase("unknown");
    assert_eq!(rig.broker.prints().len(), 1);
    rig.stop(false);
    let mut execution: Value = serde_json::from_str(&rig.stored(&job, "execution_json")).unwrap();
    for key in ["interface", "setting", "ams_slot_id"] {
        execution.as_object_mut().unwrap().remove(key);
    }
    execution["profiles"]["filament.json"]
        .as_object_mut()
        .unwrap()
        .remove("filament_colour");
    let db = rig.db();
    legacy_schema::queue_v16(&db);
    for key in ["support_enabled", "support_interface_filament_id"] {
        execution["plate"]["conditions"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        db.execute(&format!("ALTER TABLE plates DROP COLUMN {key}"), [])
            .unwrap();
    }
    db.execute(
        "UPDATE print_jobs SET execution_json=?1,estimate_json=NULL WHERE id=?2",
        [execution.to_string(), id(&job).to_owned()],
    )
    .unwrap();
    db.execute_batch("DROP TABLE print_history; ALTER TABLE plate_items DROP COLUMN roles_json; ALTER TABLE plates DROP COLUMN secondary_filament_id; DROP TABLE plate_imports; PRAGMA user_version=12;")
        .unwrap();
    drop(db);
    rig.launch();
    rig.idle();
    rig.phase("needs_attention");
    assert_eq!(rig.broker.prints().len(), 1);
    assert_eq!(rig.queue()["current"]["id"], job["id"]);
    let plate = rig.get(&format!("/api/plates/{}", id(&rig.plate)));
    assert_eq!(plate["conditions"]["support_enabled"], false);
    assert!(plate["conditions"]["support_interface_filament_id"].is_null());
    assert_eq!(
        rig.db()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        200,
    );
    until(|| rig.broker.prints().len() == 2, 12);
    assert_eq!(rig.broker.prints()[1]["ams_mapping"], json!([0]));
    rig.report("RUNNING");
    rig.phase("printing");
    rig.stop(false);
    rig.launch();
    rig.report("RUNNING");
    rig.phase("printing");
    assert_eq!(rig.broker.prints().len(), 2);
}

#[allow(clippy::too_many_lines)]
pub fn support_interface(browser: bool) {
    let mut rig = Rig::new("support-interface");
    rig.env.insert("P1_START_TIMEOUT_SECS".into(), "3".into());
    rig.launch();
    rig.seed();
    let white = rig.materials[0]["id"].clone();
    let blue = rig.materials[1]["id"].clone();
    let gf=rig.post("/api/filaments",&json!({"name":"接触面用の長い名前・PETG-GF 黒","vendor":"Fixture","material":"PETG-GF","color":"000000FF","bambu_filament_id":null}),201);
    rig.post(&format!("/api/filaments/{}/settings",id(&gf)),&json!({"machine_profile_key":MACHINE,"base_profile_key":"Generic PETG","overrides_json":{}}),201);
    assert_eq!(rig.plate["conditions"]["support_enabled"], false);
    assert!(rig.plate["conditions"]["support_interface_filament_id"].is_null());
    rig.edit_conditions(&json!({"support_enabled":true}));
    assert_eq!(
        rig.plate["conditions"]["support_interface_filament_id"],
        white
    );
    let job = rig.send(
        json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
        200,
    )["waiting"][0]
        .clone();
    let first = rig.estimated(&job);
    assert!(first["input"]["settings"].get("interface").is_none());
    assert_eq!(
        first["input"]["settings"]["profiles"]["process.json"]["support_interface_filament"],
        "1"
    );
    rig.edit_conditions(&json!({"support_interface_filament_id":blue}));
    let mut second = rig.estimated(&job);
    assert_ne!(second["id"], first["id"]);
    let settings = &second["input"]["settings"];
    assert_eq!(settings["ams_slot"], 0);
    assert_eq!(settings["interface"]["ams_slot_id"], "");
    assert_eq!(
        settings["profiles"]["process.json"]["support_interface_filament"],
        "2"
    );
    assert_eq!(
        settings["profiles"]["filament.json"]["filament_colour"],
        json!(["#FFFFFF"])
    );
    assert_eq!(
        settings["profiles"]["interface.json"]["filament_colour"],
        json!(["#00FFFF"])
    );
    for change in 0..3 {
        match change {
            0 => rig.temperature(1, 225),
            1 => rig.map_material(3, 1),
            _ => rig.temperature(0, 216),
        }
        let current = rig.estimated(&job);
        if change == 1 {
            assert_eq!(current["id"], second["id"]);
        } else {
            assert_ne!(current["id"], second["id"]);
        }
        second = current;
    }
    rig.edit_conditions(&json!({"support_enabled":false}));
    assert_eq!(
        rig.edit_conditions(&json!({"support_enabled":true}))["conditions"]["support_interface_filament_id"],
        blue
    );
    rig.estimated(&job);
    let slot = rig.slot(3);
    rig.put(
        &format!("/api/printers/p1/ams/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":null}),
        204,
    );
    let q = rig.get(&format!(
        "/api/queue?printer_id=p1&plate_id={}",
        id(&rig.plate)
    ));
    assert_eq!(q["admission"]["allowed"], false);
    assert!(
        q["admission"]["reason"]
            .as_str()
            .unwrap()
            .contains("support interface")
    );
    assert_eq!(q["waiting"][0]["estimate"]["state"], "ready");
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    rig.map_material(3, 1);
    rig.estimated(&job);
    if browser {
        rig.browser(
            "E2E_SUPPORT_CONTEXT",
            &json!({"white":white,"blue":blue,"gf":gf["id"]}),
            None,
        );
    }
    let before = rig.traces().len();
    rig.ftp.action(Action::Wait);
    rig.next(&job, 200);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
    assert_eq!(rig.traces().len(), before);
    let frozen = rig.stored(&job, "execution_json");
    rig.edit_conditions(&json!({"support_interface_filament_id":gf["id"]}));
    assert_eq!(rig.stored(&job, "execution_json"), frozen);
    rig.map_material(3, 1);
    rig.ftp.release();
    rig.start_phase("not_sent");
    assert!(rig.broker.prints().is_empty());
    let retry = |rig: &Rig| {
        rig.send(
            json!({"type":"retry","expected_job":job["id"],"cleared":true}),
            200,
        )
    };
    rig.ftp.reset_gate();
    rig.ftp.action(Action::Wait);
    retry(&rig);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
    let mut absent = rig.full.clone();
    absent["print"]["ams"]["tray_exist_bits"] = json!("1");
    rig.broker.send(&absent);
    until(|| rig.slot(3)["reported"]["present"] != true, 12);
    rig.ftp.release();
    rig.start_phase("not_sent");
    assert!(rig.broker.prints().is_empty());
    rig.idle();
    rig.map_material(3, 1);
    for (index, old) in [(1, 225), (0, 216)] {
        rig.ftp.reset_gate();
        rig.ftp.action(Action::Wait);
        retry(&rig);
        until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
        rig.temperature(index, old + 1);
        rig.ftp.release();
        rig.start_phase("not_sent");
        assert!(rig.broker.prints().is_empty());
        rig.temperature(index, old);
    }
    retry(&rig);
    until(|| rig.broker.prints().len() == 1, 12);
    assert_eq!(rig.broker.prints()[0]["ams_mapping"], json!([0, 3]));
    let active: Value = serde_json::from_str(&rig.stored(&job, "execution_json")).unwrap();
    assert_eq!(active["interface"]["filament"]["id"], blue);
    assert_eq!(
        active["plate"]["conditions"]["support_interface_filament_id"],
        blue
    );
    assert_eq!(
        active["profiles"],
        serde_json::from_str::<Value>(&frozen).unwrap()["profiles"]
    );
    rig.start_phase("unknown");
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(rig.queue()["current"]["state"], "needs_attention");
    assert_eq!(rig.broker.prints().len(), 1);
}
