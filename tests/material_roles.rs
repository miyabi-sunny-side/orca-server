#[path = "common/artifact.rs"]
mod artifact;
mod common;
use artifact::*;
use common::peers::Action;
use common::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn variant(single: bool, scaled: bool) -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    let original = fixture("material-roles.3mf");
    let mut input = zip::ZipArchive::new(Cursor::new(original)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut entry = input.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name() == "3D/3dmodel.model" {
            let mut text = String::from_utf8(bytes).unwrap();
            if single {
                text = text.replace(r#"<component objectid="4"/>"#, "");
            }
            if scaled {
                text = text.replace(
                    r#"<item objectid="10"/>"#,
                    r#"<item objectid="10" transform="2 0 0 0 1 0 0 0 1 0 0 0"/>"#,
                );
            }
            bytes = text.into_bytes();
        }
        output
            .start_file(entry.name(), zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
#[ignore = "requires official Orca 2.4.2"]
#[allow(clippy::too_many_lines)] // One queue exercises changes before admission, upload and explicit retries.
fn role_updates_and_ams_changes_cannot_replace_frozen_inputs_or_start_wrong_materials() {
    use std::sync::atomic::Ordering;
    let app: PathBuf = std::env::var_os("ORCA_APPDIR")
        .expect("official AppDir")
        .into();
    let mut rig = Rig::with_options("role-fences", "v3", Some(&app));
    rig.launch();
    rig.seed();
    let raw = fixture("material-roles.3mf");
    let uploaded = rig.multipart_json(
        "/api/plates/files",
        &[("role.3mf", raw.clone())],
        &[("name", "Role upload")],
        201,
    );
    assert_eq!(
        uploaded["models"][0]["roles"],
        json!(["primary", "secondary"])
    );
    assert_eq!(
        rig.http
            .get(format!(
                "{}/api/plates/{}/original",
                rig.base,
                id(&uploaded)
            ))
            .send()
            .unwrap()
            .bytes()
            .unwrap()
            .as_ref(),
        raw
    );
    rig.files
        .lock()
        .unwrap()
        .insert("role.3mf".into(), raw.clone());
    rig.plate=rig.post("/api/plates/import",&json!({"name":"Role changes","models":[{"name":"role.3mf","source":"role.3mf","quantity":2}]}),201);
    let admission = |rig: &Rig| {
        rig.get(&format!(
            "/api/queue?printer_id=p1&plate_id={}",
            id(&rig.plate)
        ))
    };
    assert_eq!(admission(&rig)["admission"]["allowed"], false);
    rig.edit_conditions(&json!({"secondary_filament_id":rig.materials[1]["id"]}));
    let job = rig.send(
        json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
        200,
    )["waiting"][0]
        .clone();
    let (_, _, estimated) = estimate(&rig, &job);
    let slot = rig.slot(3);
    rig.put(
        &format!("/api/printers/p1/ams/{}", id(&slot)),
        &json!({"revision":slot["revision"],"filament_id":null}),
        204,
    );
    assert_eq!(admission(&rig)["admission"]["allowed"], false);
    rig.next(&job, 409);
    assert!(rig.broker.prints().is_empty());
    rig.map_material(3, 1);
    estimate(&rig, &job);
    rig.ftp.action(Action::Wait);
    rig.next(&job, 200);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 12);
    let frozen = rig.stored(&job, "execution_json");
    rig.files
        .lock()
        .unwrap()
        .insert("role.3mf".into(), variant(true, true));
    rig.edit_conditions(&json!({"secondary_filament_id":null}));
    assert_eq!(rig.stored(&job, "execution_json"), frozen);
    rig.map_material(3, 1);
    rig.ftp.release();
    rig.start_phase("not_sent");
    assert!(rig.broker.prints().is_empty());
    for absent in [true, false] {
        rig.idle();
        rig.map_material(3, 1);
        rig.ftp.reset_gate();
        rig.ftp.action(Action::Wait);
        rig.send(
            json!({"type":"retry","expected_job":job["id"],"cleared":true}),
            200,
        );
        until(|| rig.ftp.received.load(Ordering::SeqCst), 12);
        if absent {
            let mut report = rig.full.clone();
            report["print"]["ams"]["tray_exist_bits"] = json!("1");
            rig.broker.send(&report);
            until(|| rig.slot(3)["reported"]["present"] != true, 12);
        } else {
            rig.broker.action(Action::Disconnect);
            until(|| rig.queue()["printer"]["synchronized"] != true, 12);
        }
        rig.ftp.release();
        rig.start_phase("not_sent");
        assert!(rig.broker.prints().is_empty());
    }
    rig.idle();
    rig.map_material(3, 1);
    rig.ftp.reset_gate();
    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":true}),
        200,
    );
    until(|| rig.broker.prints().len() == 1, 120);
    assert_eq!(rig.broker.prints()[0]["ams_mapping"], json!([0, 3]));
    let printed = rig.ftp.contents().last().unwrap().clone();
    assert_eq!(boxes(&printed), boxes(&estimated));
    assert_eq!(
        array(&zip_json(&printed, "Metadata/project_settings.config")["filament_type"]).len(),
        2
    );
    rig.finish();
    rig.discard();
    // A new execution sees the new role set. A subsequent shape change invalidates its estimate.
    rig.plate = rig.get(&format!("/api/plates/{}", id(&rig.plate)));
    let next = rig.send(
        json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
        200,
    )["waiting"][0]
        .clone();
    let (_, _, before) = estimate(&rig, &next);
    rig.files
        .lock()
        .unwrap()
        .insert("role.3mf".into(), variant(true, false));
    rig.next(&next, 200);
    until(|| rig.broker.prints().len() == 2, 120);
    let after = rig.ftp.contents().last().unwrap().clone();
    assert_ne!(boxes(&before), boxes(&after));
    assert_eq!(rig.broker.prints()[1]["ams_mapping"], json!([0]));
    rig.finish();
    rig.discard();
}

#[test]
#[ignore = "requires official Orca 2.4.2"]
#[allow(clippy::too_many_lines)] // Keep each prepare, artifact inspection and isolated printer observation together.
fn official_role_assemblies_preserve_material_order_geometry_and_ams() {
    let app: PathBuf = std::env::var_os("ORCA_APPDIR")
        .expect("official AppDir")
        .into();
    let mut rig = Rig::with_options("material-roles", "v3", Some(&app));
    rig.full["print"]["ams"]["tray_exist_bits"] = json!("b");
    rig.full["print"]["ams"]["ams"][0]["tray"][1] = merge(
        &rig.full["print"]["ams"]["ams"][0]["tray"][1],
        &json!({"tray_type":"PETG","tray_color":"FFFFFFFF"}),
    );
    rig.launch();
    rig.seed();
    rig.files
        .lock()
        .unwrap()
        .insert("parts/roles.3mf".into(), fixture("material-roles.3mf"));
    let other = rig.post(
        "/api/filaments",
        &json!({"name":"Support PETG","vendor":"Fixture","material":"PETG","color":"FFFFFFFF"}),
        201,
    );
    rig.post(&format!("/api/filaments/{}/settings",id(&other)), &json!({"machine_profile_key":MACHINE,"base_profile_key":"Generic PETG","overrides_json":{}}),201);
    rig.materials.push(other);
    rig.map_material(1, 2);
    let mut results = Vec::new();
    for (name, same, support) in [
        ("two-role", false, false),
        ("same-material", true, false),
        ("with-interface", false, true),
        ("same-with-interface", true, true),
    ] {
        if support {
            // Lift the secondary part to require the independent third interface material.
            rig.files.lock().unwrap().insert(
                "parts/roles.3mf".into(),
                fixture("material-roles-support.3mf"),
            );
        }
        let conditions = json!({"required_machine_profile_key":MACHINE,"filament_id":rig.materials[0]["id"],
            "secondary_filament_id":rig.materials[usize::from(!same)]["id"],"process_profile_key":PROCESS,"bed_type":BED,
            "support_enabled":support,"support_interface_filament_id":if support {rig.materials[2]["id"].clone()} else {Value::Null}});
        // Resolve an existing STL reference after the producer publishes only the matching 3MF.
        rig.plate = rig.post("/api/plates/import",&json!({"name":name,"models":[{"name":"roles.stl","source":"parts/roles.stl","quantity":2}],"conditions":conditions}),201);
        assert_eq!(
            rig.plate["models"][0]["roles"],
            json!(["primary", "secondary"])
        );
        let saved = rig.plate.clone();
        let copy = rig.post(
            &format!("/api/plates/{}/duplicate", id(&saved)),
            &json!({"name":"Role copy"}),
            201,
        );
        assert_eq!(copy["conditions"], saved["conditions"]);
        assert_eq!(copy["models"][0]["roles"], saved["models"][0]["roles"]);
        let job = rig.send(
            json!({"type":"add","plate_id":saved["id"],"plate_version":saved["version"]}),
            200,
        )["waiting"][0]
            .clone();
        let (_, directory, bytes) = estimate(&rig, &job);
        let project = fs::read(directory.join("project.3mf")).unwrap();
        // Selected-plate arrangement may move assemblies to reserve the support tower.
        // Both stages must retain two complete, full-size, non-overlapping assemblies.
        for artifact in [&project, &bytes] {
            let bounds = boxes(artifact);
            assert_eq!(bounds.len(), 2);
            assert!(
                (0..2)
                    .any(|a| bounds[0][a][1] <= bounds[1][a][0]
                        || bounds[1][a][1] <= bounds[0][a][0])
            );
            for bound in bounds {
                for (axis, limit) in bound.iter().zip([256., 256., 250.]) {
                    assert!(0. <= axis[0] && axis[1] <= limit);
                }
                let mut xy = [bound[0][1] - bound[0][0], bound[1][1] - bound[1][0]];
                xy.sort_by(f64::total_cmp);
                assert!(
                    (xy[0] - 10.).abs() < 0.001
                        && (xy[1] - 20.).abs() < 0.001
                        && (bound[2][1] - bound[2][0] - if support { 10. } else { 2. }).abs()
                            < 0.001
                );
            }
        }
        let model_text =
            String::from_utf8(zip_read(&bytes, "Metadata/model_settings.config")).unwrap();
        let model = roxmltree::Document::parse(&model_text).unwrap();
        let objects: Vec<_> = model
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("object"))
            .collect();
        assert_eq!(objects.len(), 2);
        for object in objects {
            let parts: Vec<_> = object
                .children()
                .filter(|n| n.has_tag_name("part"))
                .collect();
            assert_eq!(parts.len(), 2);
            let indices: Vec<_> = parts
                .iter()
                .map(|p| {
                    p.children()
                        .find(|n| {
                            n.has_tag_name("metadata") && n.attribute("key") == Some("extruder")
                        })
                        .unwrap()
                        .attribute("value")
                        .unwrap()
                })
                .collect();
            assert_eq!(indices, if same { vec!["1", "1"] } else { vec!["1", "2"] });
        }
        let settings = zip_json(&bytes, "Metadata/project_settings.config");
        assert_eq!(
            array(&settings["filament_type"]).len(),
            if support && !same {
                3
            } else if support {
                2
            } else if same {
                1
            } else {
                2
            }
        );
        if support {
            assert_eq!(
                settings["support_interface_filament"],
                if same { "2" } else { "3" }
            );
        }
        let info = String::from_utf8(zip_read(&bytes, "Metadata/slice_info.config")).unwrap();
        let info = roxmltree::Document::parse(&info).unwrap();
        for index in 1..=if same { 1 } else { 2 } {
            assert!(info.descendants().any(|n| n.has_tag_name("filament")
                && n.attribute("id") == Some(index.to_string().as_str())
                && n.attribute("used_for_object") == Some("true")));
        }
        let gcode = String::from_utf8(zip_read(&bytes, "Metadata/plate_1.gcode")).unwrap();
        if support {
            assert!(gcode.lines().any(|l| l == if same { "T1" } else { "T2" }));
            assert!(info.descendants().any(|n| n.has_tag_name("filament")
                && n.attribute("id") == Some(if same { "2" } else { "3" })
                && n.attribute("used_for_support") == Some("true")));
        }
        if !same {
            assert!(
                (support || gcode.lines().any(|l| l == "T0"))
                    && gcode.lines().any(|l| l == "T1")
                    && gcode.contains("; FLUSH_START")
            );
        }
        let prior = rig.broker.prints().len();
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == prior + 1, 120);
        let printed = rig.ftp.contents().last().unwrap().clone();
        assert_eq!(printed, bytes);
        let mapping = rig.broker.prints().last().unwrap()["ams_mapping"].clone();
        assert_eq!(
            mapping,
            if support && same {
                json!([0, 1])
            } else if support {
                json!([0, 3, 1])
            } else if same {
                json!([0])
            } else {
                json!([0, 3])
            }
        );
        fs::write(rig.output.join(format!("{name}.gcode.3mf")), &bytes).unwrap();
        results.push(json!({"case":name,"seconds":prediction(&bytes),"ams_mapping":mapping,"bounds":boxes(&project),"materials":settings["filament_type"]}));
        rig.finish();
        rig.discard();
    }
    fs::write(
        rig.output.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    if std::env::var_os("ORCA_ROLE_UI").is_some() {
        rig.files
            .lock()
            .unwrap()
            .insert("parts/roles.3mf".into(), fixture("material-roles.3mf"));
        rig.browser(
            "E2E_MATERIAL_ROLES_CONTEXT",
            &json!({"primary":rig.materials[0]["id"],"secondary":rig.materials[1]["id"]}),
            None,
        );
    }
}
