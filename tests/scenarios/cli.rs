use crate::{artifact::*, common::*};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub fn layout(appdir: &Path) {
    let mut rig = Rig::with_options("official-layout", "v3", Some(appdir));
    rig.launch();
    rig.seed();
    let mut spec = Value::Null;
    for (index, machine) in [MACHINE, "Bambu Lab A1 mini 0.2 nozzle"].iter().enumerate() {
        let query = reqwest::Url::parse_with_params(
            "http://localhost/api/slicer/profiles",
            [("machine", machine)],
        )
        .unwrap();
        let selection =
            rig.get(&format!("{}?{}", query.path(), query.query().unwrap()))["defaults"].clone();
        if index > 0 {
            let mut settings = printer_settings(&rig.get("/api/printers/p1"));
            settings["machine_profile_key"] = json!(machine);
            settings["default_process_profile_key"] = selection["process"].clone();
            let requests = rig.broker.requests().len();
            rig.put("/api/printers/p1", &settings, 200);
            until(|| rig.broker.requests().len() > requests, 12);
            rig.idle();
            let choices = rig.get(&format!(
                "/api/filaments/{}/profiles?{}",
                id(&rig.materials[1]),
                query.query().unwrap()
            ));
            let base = array(&choices)
                .iter()
                .find(|p| p["key"].as_str().unwrap().starts_with("Generic PLA"))
                .unwrap()["key"]
                .clone();
            rig.post(&format!("/api/filaments/{}/settings",id(&rig.materials[1])),&json!({"machine_profile_key":machine,"base_profile_key":base,"overrides_json":{"nozzle_temperature":215}}),201);
        }
        spec = merge(
            &rig.specification(3),
            &json!({"required_machine_profile_key":machine,"process_profile_key":selection["process"]}),
        );
        let action = rig.add_action(Some(spec.clone()), None);
        let job = array(&rig.send(action, 200)["waiting"])
            .last()
            .unwrap()
            .clone();
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == index + 1, 90);
        let project = fs::read(rig.artifact("project.3mf")).unwrap();
        let printed = fs::read(rig.artifact("print.gcode.3mf")).unwrap();
        assert_eq!(rig.ftp.contents().last(), Some(&printed));
        geometry(
            &project,
            &printed,
            if index == 0 { 256. } else { 180. },
            if index == 0 { 250. } else { 180. },
        );
        let settings = zip_json(&printed, "Metadata/project_settings.config");
        let gcode = String::from_utf8(zip_read(&printed, "Metadata/plate_1.gcode")).unwrap();
        assert!(gcode.len() > 1000 && gcode.contains("215"));
        assert_eq!(settings["printer_settings_id"], *machine);
        assert_eq!(
            settings["nozzle_diameter"],
            json!([if index == 0 { "0.4" } else { "0.2" }])
        );
        assert_eq!(settings["nozzle_temperature"][0], "215");
        assert!(
            !zip::ZipArchive::new(std::io::Cursor::new(&project))
                .unwrap()
                .file_names()
                .any(|n| std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gcode")))
        );
        fs::write(rig.output.join(format!("{index}-project.3mf")), project).unwrap();
        fs::write(rig.output.join(format!("{index}-print.gcode.3mf")), printed).unwrap();
        rig.finish();
        rig.discard();
    }
    let mut cube = rig.files.lock().unwrap()["parts/cube.stl"].clone();
    let count = u32::from_le_bytes(cube[80..84].try_into().unwrap()) as usize;
    for triangle in 0..count {
        for offset in (96 + triangle * 50..132 + triangle * 50).step_by(4) {
            let scaled = f32::from_le_bytes(cube[offset..offset + 4].try_into().unwrap()) * 20.;
            cube[offset..offset + 4].copy_from_slice(&scaled.to_le_bytes());
        }
    }
    rig.files
        .lock()
        .unwrap()
        .insert("parts/cube.stl".to_owned(), cube);
    let action = rig.add_action(Some(spec), None);
    let job = array(&rig.send(action, 200)["waiting"])
        .last()
        .unwrap()
        .clone();
    rig.next(&job, 200);
    rig.phase("needs_attention");
    assert_eq!(rig.broker.prints().len(), 2);
    assert_eq!(
        rig.get(&format!("/api/plates/{}", id(&rig.plate))),
        rig.plate
    );
}

pub fn estimates(appdir: &Path) {
    let mut rig = Rig::with_options("official-estimates", "v3", Some(appdir));
    rig.launch();
    rig.seed();
    let mut results = Vec::new();
    let mut last = None;
    for (quantity, process) in [(1, PROCESS), (2, PROCESS), (2, "0.16mm Optimal @BBL X1C")] {
        let mut plate = edit(&rig.configure(None, None));
        plate["conditions"]["process_profile_key"] = json!(process);
        plate["models"][0]["quantity"] = json!(quantity);
        rig.plate = rig.put(&format!("/api/plates/{}", id(&rig.plate)), &plate, 200);
        let job = rig.send(
            json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
            200,
        )["waiting"][0]
            .clone();
        let (snapshot, directory, bytes) = estimate(&rig, &job);
        let seconds = prediction(&bytes);
        let gcode = String::from_utf8(zip_read(&bytes, "Metadata/plate_1.gcode")).unwrap();
        let total = gcode
            .lines()
            .find_map(|line| line.split_once("total estimated time: ").map(|(_, v)| v))
            .unwrap();
        let parsed: u64 = total
            .split_whitespace()
            .map(|word| {
                let (number, unit) = word.split_at(word.len() - 1);
                number.parse::<u64>().unwrap()
                    * match unit {
                        "d" => 86400,
                        "h" => 3600,
                        "m" => 60,
                        "s" => 1,
                        _ => panic!("unknown duration unit"),
                    }
            })
            .sum();
        assert!(seconds.abs_diff(parsed) <= 1);
        assert_eq!(rig.waiting(&job)["estimate"]["seconds"], seconds);
        assert_eq!(
            zip_json(&bytes, "Metadata/project_settings.config")["print_settings_id"],
            process
        );
        assert_eq!(
            fs::read_dir(&directory)
                .unwrap()
                .filter(|p| p
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|e| e == "stl"))
                .count(),
            quantity
        );
        assert!(
            rig.broker.prints().is_empty()
                && rig.ftp.uploads().is_empty()
                && rig.queue()["current"].is_null()
        );
        fs::write(
            rig.output.join(format!("case-{}.gcode.3mf", results.len())),
            &bytes,
        )
        .unwrap();
        results.push(seconds);
        if results.len() < 3 {
            rig.send(json!({"type":"remove","job_id":job["id"]}), 200);
        }
        last = Some((job, snapshot, bytes));
    }
    assert!(results[1] > results[0] && results[2] != results[1]);
    let (job, snapshot, bytes) = last.unwrap();
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(
        serde_json::from_str::<Value>(&rig.stored(&job, "estimate_json")).unwrap(),
        snapshot
    );
    let count = || {
        fs::read_to_string(rig.output.join("server.log"))
            .unwrap()
            .matches("OrcaSlicer exited")
            .count()
    };
    let before = count();
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 120);
    assert_eq!(rig.ftp.contents()[0], bytes);
    assert_eq!(count(), before);
    assert_eq!(rig.queue()["current"]["estimate"]["seconds"], results[2]);
    write_json(
        &rig.output.join("result.json"),
        &json!({"seconds":results,"cached_bytes_reused":true}),
    );
}

#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
#[allow(clippy::float_cmp)] // Profile settings are exact integers or halves.
pub fn strength(appdir: &Path) {
    let mut rig = Rig::with_options("official-strength", "v3", Some(appdir));
    rig.launch();
    rig.seed();
    let mut results = Vec::new();
    let mut paths = Vec::new();
    for (pattern, density, walls) in [
        ("adaptivecubic", 15, 2),
        ("gyroid", 15, 2),
        ("adaptivecubic", 30, 2),
        ("adaptivecubic", 15, 4),
        ("adaptivecubic", 0, 2),
        ("adaptivecubic", 100, 2),
    ] {
        let mut plate = edit(&rig.configure(None, None));
        plate["models"][0]["quantity"] = json!(1);
        plate["conditions"] = merge(
            &plate["conditions"],
            &json!({"sparse_infill_pattern":pattern,"sparse_infill_density":density,"wall_loops":walls}),
        );
        rig.plate = rig.put(&format!("/api/plates/{}", id(&rig.plate)), &plate, 200);
        let job = rig.send(
            json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
            200,
        )["waiting"][0]
            .clone();
        let (_, directory, bytes) = estimate(&rig, &job);
        let settings = zip_json(&bytes, "Metadata/project_settings.config");
        let gcode = String::from_utf8(zip_read(&bytes, "Metadata/plate_1.gcode")).unwrap();
        assert_eq!(settings["sparse_infill_pattern"], pattern);
        assert_eq!(
            settings["sparse_infill_density"]
                .as_str()
                .unwrap()
                .trim_end_matches('%')
                .parse::<u32>()
                .unwrap(),
            density
        );
        for (key, value) in [
            ("wall_loops", f64::from(walls)),
            ("top_shell_layers", f64::from(5 * walls / 2)),
            ("bottom_shell_layers", f64::from(3 * walls / 2)),
            ("top_shell_thickness", f64::from(walls) / 2.),
            ("bottom_shell_thickness", 0.),
        ] {
            assert_eq!(
                settings[key].as_str().unwrap().parse::<f64>().unwrap(),
                value,
                "{key}"
            );
        }
        assert!(gcode.contains("M83"));
        let mut layers = Vec::new();
        let mut sparse_paths = Vec::new();
        for block in gcode.split("; CHANGE_LAYER").skip(1) {
            let mut amounts = BTreeMap::<String, f64>::new();
            let mut feature = "";
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("; FEATURE: ") {
                    feature = value;
                }
                if motion(line)
                    && !line.starts_with("G0 ")
                    && (parameter(line, 'X').is_some() || parameter(line, 'Y').is_some())
                    && let Some(e) = parameter(line, 'E').filter(|v| *v > 0.)
                {
                    *amounts.entry(feature.to_owned()).or_default() += e;
                    if feature == "Sparse infill" {
                        sparse_paths.push(line.to_owned());
                    }
                }
            }
            layers.push(amounts);
        }
        let sparse: Vec<_> = layers
            .iter()
            .enumerate()
            .filter_map(|(i, layer)| {
                (layer.get("Sparse infill").copied().unwrap_or(0.) > 0.).then_some(i)
            })
            .collect();
        let middle = &layers[10..layers.len() - 10];
        let median = |feature: &str| {
            let mut values: Vec<_> = middle
                .iter()
                .map(|v| v.get(feature).copied().unwrap_or(0.))
                .collect();
            values.sort_by(f64::total_cmp);
            f64::midpoint(values[(values.len() - 1) / 2], values[values.len() / 2])
        };
        assert!(
            middle
                .iter()
                .all(|v| v.get("Outer wall").copied().unwrap_or(0.) > 0.)
        );
        let result = json!({"pattern":pattern,"density":density,"walls":walls,"sparse_layers":sparse,"middle_inner":median("Inner wall"),"middle_sparse":median("Sparse infill"),"middle_solid":median("Internal solid infill"),"layer_extrusion":layers,"seconds":rig.waiting(&job)["estimate"]["seconds"]});
        if [0, 100].contains(&density) {
            assert!(sparse.is_empty());
        } else {
            assert!(sparse.len() > 60 && result["middle_sparse"].as_f64().unwrap() > 0.);
        }
        fs::write(
            rig.output.join(format!("case-{}.gcode.3mf", results.len())),
            bytes,
        )
        .unwrap();
        fs::copy(
            directory.join("process.json"),
            rig.output
                .join(format!("case-{}-process.json", results.len())),
        )
        .unwrap();
        paths.push(sparse_paths);
        results.push(result);
        rig.send(json!({"type":"remove","job_id":job["id"]}), 200);
    }
    assert_ne!(paths[0], paths[1]);
    assert!(
        results[2]["middle_sparse"].as_f64().unwrap()
            > results[0]["middle_sparse"].as_f64().unwrap() * 1.3
    );
    assert!(
        results[3]["middle_inner"].as_f64().unwrap()
            > results[0]["middle_inner"].as_f64().unwrap() * 2.5
    );
    let sparse = |i: usize| array(&results[i]["sparse_layers"]);
    assert!(sparse(3)[0].as_u64() > sparse(0)[0].as_u64());
    assert!(sparse(3).last().unwrap().as_u64() < sparse(0).last().unwrap().as_u64());
    assert_eq!(results[4]["middle_solid"], 0.);
    assert!(results[5]["middle_solid"].as_f64().unwrap() > 0.);
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    write_json(&rig.output.join("result.json"), &json!(results));
}
