use crate::{artifact::*, common::*};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
fn inspect(
    bytes: &[u8],
    enabled: bool,
    distinct: bool,
    unused: bool,
    temperatures: &[(u32, u32, usize)],
    bed: (u32, u32),
) -> Value {
    let settings = zip_json(bytes, "Metadata/project_settings.config");
    let text = String::from_utf8(zip_read(bytes, "Metadata/slice_info.config")).unwrap();
    let xml = roxmltree::Document::parse(&text).unwrap();
    let gcode = String::from_utf8(zip_read(bytes, "Metadata/plate_1.gcode")).unwrap();
    assert_eq!(settings["enable_support"], if enabled { "1" } else { "0" });
    assert_eq!(settings["enforce_support_layers"], "0");
    let expected = if distinct { 2 } else { 1 };
    assert_eq!(array(&settings["filament_settings_id"]).len(), expected);
    assert_eq!(array(&settings["filament_type"]).len(), expected);
    assert_eq!(
        settings["textured_plate_temp"],
        json!(vec![bed.1.to_string(); expected])
    );
    assert_eq!(
        settings["textured_plate_temp_initial_layer"],
        json!(vec![bed.0.to_string(); expected])
    );
    for (key, initial) in [
        ("nozzle_temperature", false),
        ("nozzle_temperature_initial_layer", true),
    ] {
        let values: Vec<_> = temperatures
            .iter()
            .flat_map(|t| {
                vec![
                    if initial {
                        t.0.to_string()
                    } else {
                        t.1.to_string()
                    };
                    t.2
                ]
            })
            .collect();
        assert_eq!(settings[key], json!(values));
    }
    if enabled {
        assert!(
            ["normal(auto)", "tree(auto)"].contains(&settings["support_type"].as_str().unwrap())
        );
        assert_eq!(settings["support_filament"], "1");
        assert_eq!(settings["support_interface_filament"], expected.to_string());
        assert_eq!(settings["support_interface_not_for_body"], "1");
        for key in [
            "support_interface_top_layers",
            "support_top_z_distance",
            "support_interface_spacing",
        ] {
            assert!(settings[key].as_str().unwrap().parse::<f64>().unwrap() > 0.);
        }
    }
    if distinct {
        assert_eq!(settings["enable_prime_tower"], "1");
        for key in [
            "flush_into_infill",
            "flush_into_objects",
            "flush_into_support",
        ] {
            assert_eq!(settings[key], "0");
        }
        for i in [1, 2] {
            assert!(
                settings["flush_volumes_matrix"][i]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
                    > 0.
            );
        }
        assert!(
            array(&settings["flush_multiplier"]).iter().all(|v| v
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                > 0.)
        );
    }
    let mut tool = 0;
    let mut feature = "";
    let mut extrusion = BTreeMap::<(String, u32), f64>::new();
    let mut flush = false;
    let mut purge = 0.;
    let mut switches = Vec::new();
    let mut bed_commands = Vec::new();
    let mut nozzle_commands = Vec::new();
    for line in gcode.lines() {
        if let Some(value) = line.strip_prefix("; FEATURE: ") {
            feature = value;
        }
        match line.trim() {
            "; FLUSH_START" => flush = true,
            "; FLUSH_END" => flush = false,
            _ => {}
        }
        let command = line.split(';').next().unwrap().trim();
        if ["T0", "T1"].contains(&command) {
            tool = command[1..].parse().unwrap();
            switches.push(tool);
        }
        if motion(command)
            && let Some(e) = parameter(command, 'E').filter(|v| *v > 0.)
        {
            if flush {
                purge += e;
            }
            if parameter(command, 'X').is_some() || parameter(command, 'Y').is_some() {
                *extrusion.entry((feature.to_owned(), tool)).or_default() += e;
            }
        }
        match command.split_whitespace().next() {
            Some("M140" | "M190") => {
                bed_commands.push(parameter(command, 'S').expect("bed command temperature"));
            }
            Some("M104" | "M109") => {
                if let Some(v) = parameter(command, 'S') {
                    nozzle_commands.push(v);
                }
            }
            _ => {}
        }
    }
    assert!(
        !bed_commands.is_empty()
            && bed_commands
                .iter()
                .all(|v| [0., f64::from(bed.0), f64::from(bed.1)].contains(v))
    );
    let indices = |kind: &str| {
        extrusion
            .iter()
            .filter_map(|((k, i), amount)| (k == kind && *amount > 0.).then_some(*i))
            .collect::<BTreeSet<_>>()
    };
    let active = indices("Support interface");
    let body = indices("Support");
    assert!(extrusion.keys().all(|(kind, i)| {
        ![
            "Inner wall",
            "Outer wall",
            "Sparse infill",
            "Internal solid infill",
            "Bottom surface",
            "Top surface",
        ]
        .contains(&kind.as_str())
            || *i == 0
    }));
    if enabled && !unused {
        assert_eq!(active, BTreeSet::from([u32::from(distinct)]));
        assert_eq!(body, BTreeSet::from([0]));
    } else {
        assert!(active.is_empty() && body.is_empty());
    }
    if distinct && !unused {
        assert!(switches.contains(&0) && switches.contains(&1) && purge > 0.);
        assert!(
            extrusion
                .iter()
                .any(|((kind, _), amount)| kind == "Prime tower" && *amount > 0.)
        );
        assert!(
            temperatures
                .iter()
                .all(|t| nozzle_commands.contains(&f64::from(t.1)))
        );
    }
    let used: Vec<u32> = xml
        .descendants()
        .filter(|n| n.has_tag_name("filament"))
        .map(|n| n.attribute("id").unwrap().parse().unwrap())
        .collect();
    assert_eq!(
        used,
        if distinct && !unused {
            vec![1, 2]
        } else {
            vec![1]
        }
    );
    json!({"seconds":prediction(bytes),"types":settings["filament_type"],"colors":settings["filament_colour"],"support_body_indices":body,"interface_indices":active,"switches":switches,"purge_filament_mm":purge,"bed_commands":bed_commands,"nozzle_commands":nozzle_commands})
}
#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
pub fn support(mut rig: Rig) {
    rig.files.lock().unwrap().insert(
        "parts/support-cantilever.stl".into(),
        fixture("support-cantilever.stl"),
    );
    rig.full["print"]["ams"]["tray_exist_bits"] = json!("b");
    rig.full["print"]["ams"]["ams"][0]["tray"][1] = merge(
        &rig.full["print"]["ams"]["ams"][0]["tray"][1],
        &json!({"tray_type":"PETG","tray_color":"FFFFFFFF"}),
    );
    rig.launch();
    rig.seed();
    assert_eq!(rig.get("/api/slicer/profiles")["version"], "2.4.2");
    let petg=rig.post("/api/filaments",&json!({"name":"PETG 白","vendor":"Fixture","material":"PETG","color":"FFFFFFFF","bambu_filament_id":null}),201);
    rig.materials.push(petg.clone());
    let temps = [(220, 215, 2), (230, 225, 2), (250, 255, 1)];
    let beds = [(60, 55), (45, 40), (75, 70)];
    for (i, base) in [FILAMENT, FILAMENT, "Generic PETG"].iter().enumerate() {
        let data = json!({"machine_profile_key":MACHINE,"base_profile_key":base,"overrides_json":{"nozzle_temperature_initial_layer":temps[i].0,"nozzle_temperature":temps[i].1,"bed_temperature_initial_layer":beds[i].0,"bed_temperature":beds[i].1}});
        let old = rig.get(&format!("/api/filaments/{}", id(&rig.materials[i])))["settings"].clone();
        let path = format!("/api/filaments/{}/settings", id(&rig.materials[i]));
        if array(&old).is_empty() {
            rig.post(&path, &data, 201);
        } else {
            rig.put(&format!("{path}/{}", id(&old[0])), &data, 200);
        }
    }
    rig.map_material(1, 2);
    let before = rig.get(&format!("/api/filaments/{}", id(&petg)))["settings"].clone();
    let mut results = Vec::new();
    for (name, on, main, interface, unused) in [
        ("off", false, 0, 0, false),
        ("same", true, 0, 0, false),
        ("pla-colors", true, 0, 1, false),
        ("pla-petg", true, 0, 2, false),
        ("petg-pla", true, 2, 0, false),
        ("unused", true, 0, 2, true),
    ] {
        let distinct = on && main != interface;
        let source = if unused {
            "parts/cube.stl"
        } else {
            "parts/support-cantilever.stl"
        };
        let conditions = json!({"required_machine_profile_key":MACHINE,"filament_id":rig.materials[main]["id"],"process_profile_key":PROCESS,"bed_type":BED,"support_enabled":on,"support_interface_filament_id":rig.materials[interface]["id"]});
        rig.plate=rig.post("/api/plates/import",&json!({"name":name,"models":[{"name":source,"source":source,"quantity":1}],"conditions":conditions}),201);
        let job = rig.send(
            json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
            200,
        )["waiting"][0]
            .clone();
        until(
            || {
                let e = rig.waiting(&job)["estimate"].clone();
                assert_ne!(e["state"], "failed", "{e}");
                e["state"] == "ready"
            },
            120,
        );
        let bytes = if let Some(container) = &rig.container {
            let paths = container::docker(&[
                "exec",
                &container.name,
                "find",
                &format!("/data/plates/jobs/{}", id(&job)),
                "-name",
                "print.gcode.3mf",
            ]);
            let paths: Vec<_> = paths.lines().collect();
            assert_eq!(paths.len(), 1);
            container.copy(paths[0], &format!("{name}.gcode.3mf"))
        } else {
            let (_, _, bytes) = estimate(&rig, &job);
            fs::write(rig.output.join(format!("{name}.gcode.3mf")), &bytes).unwrap();
            bytes
        };
        let selected = if distinct {
            vec![temps[main], temps[interface]]
        } else {
            vec![temps[main]]
        };
        let result = inspect(&bytes, on, distinct, unused, &selected, beds[main]);
        assert_eq!(result["seconds"], rig.waiting(&job)["estimate"]["seconds"]);
        let count = rig.broker.prints().len();
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == count + 1, 120);
        assert_eq!(rig.ftp.contents().last(), Some(&bytes));
        let mut mapping = vec![[0, 3, 1][main]];
        if distinct {
            mapping.push([0, 3, 1][interface]);
        }
        assert_eq!(
            rig.broker.prints().last().unwrap()["ams_mapping"],
            json!(mapping)
        );
        assert_eq!(
            rig.queue()["current"]["estimate"]["seconds"],
            result["seconds"]
        );
        results.push(merge(&result, &json!({"case":name,"mapping":mapping})));
        rig.finish();
        rig.discard();
        rig.idle();
    }
    assert_eq!(
        rig.get(&format!("/api/filaments/{}", id(&petg)))["settings"],
        before
    );
    assert_eq!(rig.broker.prints().len(), 6);
    write_json(&rig.output.join("result.json"), &json!(results));
}
