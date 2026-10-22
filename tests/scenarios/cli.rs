use crate::{artifact::*, common::*};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
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
        .expect("profile query URL is valid");
        let selection = rig.get(&format!(
            "{}?{}",
            query.path(),
            query.query().expect("profile query has parameters")
        ))["defaults"]
            .clone();
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
                query.query().expect("profile query has parameters")
            ));
            let base = array(&choices)
                .iter()
                .find(|p| {
                    p["key"]
                        .as_str()
                        .expect("profile key is a string")
                        .starts_with("Generic PLA")
                })
                .expect("a Generic PLA profile is offered")["key"]
                .clone();
            rig.post(&format!("/api/filaments/{}/settings",id(&rig.materials[1])),&json!({"machine_profile_key":machine,"base_profile_key":base,"overrides_json":{"nozzle_temperature":215}}),201);
        }
        // The printer's machine and process (changed above) decide the slice.
        spec = rig.specification(3);
        let action = rig.add_action(Some(spec.clone()), None);
        let job = array(&rig.send(action, 200)["waiting"])
            .last()
            .expect("added job is waiting")
            .clone();
        rig.next(&job, 200);
        until(|| rig.broker.prints().len() == index + 1, 90);
        let project = fs::read(rig.artifact("project.3mf")).expect("project artifact is readable");
        let printed =
            fs::read(rig.artifact("print.gcode.3mf")).expect("print artifact is readable");
        assert_eq!(
            rig.ftp.contents().last(),
            Some(&printed),
            "uploaded bytes match the print artifact"
        );
        geometry(
            &project,
            &printed,
            if index == 0 { 256. } else { 180. },
            if index == 0 { 250. } else { 180. },
        );
        let settings = zip_json(&printed, "Metadata/project_settings.config");
        let gcode = String::from_utf8(zip_read(&printed, "Metadata/plate_1.gcode"))
            .expect("G-code is UTF-8");
        assert!(
            gcode.len() > 1000 && gcode.contains("215"),
            "G-code is substantial and uses the 215 nozzle override"
        );
        assert_eq!(
            settings["printer_settings_id"], *machine,
            "slice uses the printer's machine profile"
        );
        assert_eq!(
            settings["nozzle_diameter"],
            json!([if index == 0 { "0.4" } else { "0.2" }]),
            "nozzle diameter follows the machine profile"
        );
        assert_eq!(
            settings["nozzle_temperature"][0], "215",
            "material override sets the nozzle temperature"
        );
        assert!(
            !zip::ZipArchive::new(std::io::Cursor::new(&project))
                .expect("project artifact is a zip archive")
                .file_names()
                .any(|n| std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gcode"))),
            "project artifact contains no G-code"
        );
        fs::write(rig.output.join(format!("{index}-project.3mf")), project)
            .expect("output directory is writable");
        fs::write(rig.output.join(format!("{index}-print.gcode.3mf")), printed)
            .expect("output directory is writable");
        rig.finish();
        rig.discard();
    }
    let mut cube = rig.files.lock().expect("files lock is not poisoned")["parts/cube.stl"].clone();
    let count = u32::from_le_bytes(
        cube[80..84]
            .try_into()
            .expect("STL triangle count is 4 bytes"),
    ) as usize;
    for triangle in 0..count {
        for offset in (96 + triangle * 50..132 + triangle * 50).step_by(4) {
            let scaled = f32::from_le_bytes(
                cube[offset..offset + 4]
                    .try_into()
                    .expect("STL float is 4 bytes"),
            ) * 20.;
            cube[offset..offset + 4].copy_from_slice(&scaled.to_le_bytes());
        }
    }
    rig.files
        .lock()
        .expect("files lock is not poisoned")
        .insert("parts/cube.stl".to_owned(), cube);
    let action = rig.add_action(Some(spec), None);
    let job = array(&rig.send(action, 200)["waiting"])
        .last()
        .expect("added job is waiting")
        .clone();
    rig.next(&job, 200);
    rig.phase("needs_attention");
    assert_eq!(
        rig.broker.prints().len(),
        2,
        "oversized model is not printed"
    );
    assert_eq!(
        rig.get(&format!("/api/plates/{}", id(&rig.plate))),
        rig.plate,
        "failed slice leaves the plate unchanged"
    );
}

#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
pub fn estimates(appdir: &Path) {
    let mut rig = Rig::with_options("official-estimates", "v3", Some(appdir));
    rig.launch();
    rig.seed();
    let mut results = Vec::new();
    let mut last = None;
    for (quantity, process) in [(1, PROCESS), (2, PROCESS), (2, "0.16mm Optimal @BBL X1C")] {
        let mut printer = printer_settings(&rig.get("/api/printers/p1"));
        printer["default_process_profile_key"] = json!(process);
        rig.put("/api/printers/p1", &printer, 200);
        let mut plate = edit(&rig.configure(None, None));
        plate["models"][0]["quantity"] = json!(quantity);
        rig.plate = rig.put(&format!("/api/plates/{}", id(&rig.plate)), &plate, 200);
        let job = rig.send(
            json!({"type":"add","plate_id":rig.plate["id"],"plate_version":rig.plate["version"]}),
            200,
        )["waiting"][0]
            .clone();
        let (snapshot, directory, bytes) = estimate(&rig, &job);
        let seconds = prediction(&bytes);
        let gcode =
            String::from_utf8(zip_read(&bytes, "Metadata/plate_1.gcode")).expect("G-code is UTF-8");
        let total = gcode
            .lines()
            .find_map(|line| line.split_once("total estimated time: ").map(|(_, v)| v))
            .expect("G-code reports the total estimated time");
        let parsed: u64 = total
            .split_whitespace()
            .map(|word| {
                let (number, unit) = word.split_at(word.len() - 1);
                number
                    .parse::<u64>()
                    .expect("duration amount is an integer")
                    * match unit {
                        "d" => 86400,
                        "h" => 3600,
                        "m" => 60,
                        "s" => 1,
                        _ => panic!("unknown duration unit"),
                    }
            })
            .sum();
        assert!(
            seconds.abs_diff(parsed) <= 1,
            "prediction {seconds}s matches G-code total {parsed}s"
        );
        assert_eq!(
            rig.waiting(&job)["estimate"]["seconds"],
            seconds,
            "job estimate matches the prediction"
        );
        assert_eq!(
            zip_json(&bytes, "Metadata/project_settings.config")["print_settings_id"],
            process,
            "slice uses the printer's process profile"
        );
        assert_eq!(
            boxes(&fs::read(directory.join("project.3mf")).expect("project file is readable"))
                .len(),
            quantity,
            "project holds one object per quantity"
        );
        assert!(
            rig.broker.prints().is_empty()
                && rig.ftp.uploads().is_empty()
                && rig.queue()["current"].is_null(),
            "estimating does not print"
        );
        fs::write(
            rig.output.join(format!("case-{}.gcode.3mf", results.len())),
            &bytes,
        )
        .expect("output directory is writable");
        results.push(seconds);
        if results.len() < 3 {
            rig.send(json!({"type":"remove","job_id":job["id"]}), 200);
        }
        last = Some((job, snapshot, bytes));
    }
    assert!(
        results[1] > results[0] && results[2] != results[1],
        "estimates follow quantity and process: {results:?}"
    );
    let (job, snapshot, bytes) = last.expect("loop ran at least once");
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(
        serde_json::from_str::<Value>(&rig.stored(&job, "estimate_json"))
            .expect("stored estimate is JSON"),
        snapshot,
        "estimate survives a restart"
    );
    let count = || {
        fs::read_to_string(rig.output.join("server.log"))
            .expect("server log is readable")
            .matches("OrcaSlicer exited")
            .count()
    };
    let before = count();
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 120);
    assert_eq!(
        rig.ftp.contents()[0],
        bytes,
        "upload reuses the cached slice"
    );
    assert_eq!(count(), before, "printing does not run the slicer again");
    assert_eq!(
        rig.queue()["current"]["estimate"]["seconds"],
        results[2],
        "current job keeps its estimate"
    );
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
        let gcode =
            String::from_utf8(zip_read(&bytes, "Metadata/plate_1.gcode")).expect("G-code is UTF-8");
        assert_eq!(
            settings["sparse_infill_pattern"], pattern,
            "slice uses the plate's infill pattern"
        );
        assert_eq!(
            settings["sparse_infill_density"]
                .as_str()
                .expect("infill density is a string")
                .trim_end_matches('%')
                .parse::<u32>()
                .expect("infill density is a percentage"),
            density,
            "slice uses the plate's infill density"
        );
        for (key, value) in [
            ("wall_loops", f64::from(walls)),
            ("top_shell_layers", f64::from(5 * walls / 2)),
            ("bottom_shell_layers", f64::from(3 * walls / 2)),
            ("top_shell_thickness", f64::from(walls) / 2.),
            ("bottom_shell_thickness", 0.),
        ] {
            assert_eq!(
                settings[key]
                    .as_str()
                    .expect("shell setting is a string")
                    .parse::<f64>()
                    .expect("shell setting is numeric"),
                value,
                "{key}"
            );
        }
        assert!(gcode.contains("M83"), "G-code uses relative extrusion");
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
                .all(|v| v.get("Outer wall").copied().unwrap_or(0.) > 0.),
            "every middle layer has an outer wall"
        );
        let result = json!({"pattern":pattern,"density":density,"walls":walls,"sparse_layers":sparse,"middle_inner":median("Inner wall"),"middle_sparse":median("Sparse infill"),"middle_solid":median("Internal solid infill"),"layer_extrusion":layers,"seconds":rig.waiting(&job)["estimate"]["seconds"]});
        if [0, 100].contains(&density) {
            assert!(sparse.is_empty(), "density {density} has no sparse infill");
        } else {
            assert!(
                sparse.len() > 60
                    && result["middle_sparse"]
                        .as_f64()
                        .expect("median is a number")
                        > 0.,
                "density {density} has sparse infill through the middle"
            );
        }
        fs::write(
            rig.output.join(format!("case-{}.gcode.3mf", results.len())),
            bytes,
        )
        .expect("output directory is writable");
        fs::copy(
            directory.join("process.json"),
            rig.output
                .join(format!("case-{}-process.json", results.len())),
        )
        .expect("process file is copyable");
        paths.push(sparse_paths);
        results.push(result);
        rig.send(json!({"type":"remove","job_id":job["id"]}), 200);
    }
    assert_ne!(
        paths[0], paths[1],
        "infill pattern changes the sparse paths"
    );
    assert!(
        results[2]["middle_sparse"]
            .as_f64()
            .expect("median is a number")
            > results[0]["middle_sparse"]
                .as_f64()
                .expect("median is a number")
                * 1.3,
        "higher density extrudes more sparse infill"
    );
    assert!(
        results[3]["middle_inner"]
            .as_f64()
            .expect("median is a number")
            > results[0]["middle_inner"]
                .as_f64()
                .expect("median is a number")
                * 2.5,
        "more walls extrude more inner wall"
    );
    let sparse = |i: usize| array(&results[i]["sparse_layers"]);
    assert!(
        sparse(3)[0].as_u64() > sparse(0)[0].as_u64(),
        "more walls start sparse infill later"
    );
    assert!(
        sparse(3).last().expect("sparse layers exist").as_u64()
            < sparse(0).last().expect("sparse layers exist").as_u64(),
        "more walls end sparse infill earlier"
    );
    assert_eq!(
        results[4]["middle_solid"], 0.,
        "0% density has no internal solid infill"
    );
    assert!(
        results[5]["middle_solid"]
            .as_f64()
            .expect("median is a number")
            > 0.,
        "100% density is internal solid infill"
    );
    assert!(
        rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty(),
        "estimating does not print"
    );
    write_json(&rig.output.join("result.json"), &json!(results));
}

/// A model that fits the P1S (256 mm) but not the A1 mini (180 mm) bed, and a material
/// without a setting for the A1 mini, are each named for that printer only.
pub fn printer_fit(appdir: &Path) {
    let mut rig = Rig::with_options("official-printer-fit", "v3", Some(appdir));
    rig.launch();
    rig.seed();
    rig.large_model();
    let plate = rig.post(
        "/api/plates/import",
        &json!({"name":"Large tray","models":[{"name":"parts/large.stl","source":"parts/large.stl","quantity":1}],
            "conditions":{"filament_id":rig.materials[1]["id"]}}),
        201,
    );
    let slice = format!("/api/plates/{}/slice", id(&plate));
    let (peer, pid) = rig.register_mini();
    until(
        || rig.slice_on(&slice, &pid)["reason"] == "material_setting",
        60,
    );
    rig.configure_material(id(&rig.materials[1]), "Bambu Lab A1 mini 0.4 nozzle");
    until(
        || rig.slice(&slice)["state"] == "ready" && rig.slice_on(&slice, &pid)["state"] == "failed",
        180,
    );
    let p1 = rig.slice(&slice);
    assert!(
        p1["seconds"].as_u64().expect("slice seconds is an integer") > 0,
        "{p1}"
    );
    let fit = rig.slice_on(&slice, &pid);
    assert_eq!(fit["reason"], "unfit", "{fit}");
    assert_eq!(
        fit["printer_name"], "A1 mini",
        "unfit slice names the printer"
    );
    fs::write(
        rig.output.join("printer-fit.json"),
        serde_json::to_vec_pretty(&rig.get(&slice)).expect("slice state serializes"),
    )
    .expect("output directory is writable");
    // Admission follows the pair: the P1S takes the plate, the A1 mini does not.
    let admission = |printer: &str| {
        rig.get(&format!(
            "/api/queue?printer_id={printer}&plate_id={}",
            id(&plate)
        ))["admission"]
            .clone()
    };
    assert_eq!(admission("p1")["allowed"], true, "the P1S admits the plate");
    rig.load_material(&peer, &pid, &rig.materials[1]["id"]);
    let refused = admission(&pid);
    assert_eq!(refused["allowed"], false, "the A1 mini refuses the plate");
    assert!(
        refused["reason"]
            .as_str()
            .expect("refusal has a reason")
            .contains("do not fit"),
        "{refused}"
    );
    assert!(
        rig.broker.prints().is_empty() && peer.prints().is_empty(),
        "nothing is printed"
    );
    rig.check();
}
