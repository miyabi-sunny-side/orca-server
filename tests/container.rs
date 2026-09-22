#[path = "common/artifact.rs"]
mod artifact;
mod common;
#[path = "scenarios/support_cli.rs"]
mod support_scenario;
use artifact::*;
use common::*;
use serde_json::json;
use std::fs;
fn image() -> String {
    std::env::var("ORCA_TEST_IMAGE")
        .expect("ORCA_TEST_IMAGE must name the image built from this checkout")
}
#[test]
#[ignore = "requires Docker and the image built from this checkout"]
#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
fn persistent_headless_image() {
    let mut rig = Rig::in_container("container", &image());
    rig.launch();
    rig.seed();
    let reference = rig.plate.clone();
    let material =
        rig.get(&format!("/api/filaments/{}", id(&rig.materials[1])))["settings"][0].clone();
    let mut setting = json!({"machine_profile_key":material["machine_profile_key"],"base_profile_key":material["base_profile_key"],"overrides_json":material["overrides_json"]});
    setting["overrides_json"] = merge(
        &setting["overrides_json"],
        &json!({"bed_temperature_initial_layer":65,"bed_temperature":65}),
    );
    rig.put(
        &format!(
            "/api/filaments/{}/settings/{}",
            id(&rig.materials[1]),
            id(&material)
        ),
        &setting,
        200,
    );
    let name = rig.container.as_ref().unwrap().name.clone();
    assert_eq!(container::docker(&["exec", &name, "id", "-u"]), "10001");
    assert!(
        !container::docker(&["exec", &name, "env"])
            .lines()
            .any(|e| e.starts_with("DISPLAY=") || e.starts_with("WAYLAND_DISPLAY="))
    );
    assert_eq!(rig.get("/api/slicer/profiles")["version"], "2.4.2");
    assert!(
        String::from_utf8(rig.bytes("/"))
            .unwrap()
            .to_lowercase()
            .contains("<!doctype html")
    );
    let cube = fixture("cube.stl");
    rig.plate = rig.multipart_json(
        "/api/plates",
        &[("a.stl", cube.clone()), ("b.stl", cube.clone())],
        &[("name", "Two uploaded cubes")],
        201,
    );
    let job = rig.add(3);
    let plate = rig.plate.clone();
    until(
        || {
            let e = rig.waiting(&job)["estimate"].clone();
            assert_ne!(e["state"], "failed", "{e}");
            e["state"] == "ready"
        },
        120,
    );
    let seconds = rig.waiting(&job)["estimate"]["seconds"].as_u64().unwrap();
    assert!(seconds > 0 && rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
    let action = rig.add_action(Some(rig.specification(0)), Some(reference));
    let waiting = array(&rig.send(action, 200)["waiting"])
        .last()
        .unwrap()
        .clone();
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 90);
    let current = rig.queue()["current"].clone();
    let path = format!(
        "/data/plates/{}",
        current["artifact_path"].as_str().unwrap()
    );
    let container = rig.container.as_ref().unwrap();
    let project = container.copy(&format!("{path}/project.3mf"), "project.3mf");
    let printed = container.copy(&format!("{path}/print.gcode.3mf"), "print.gcode.3mf");
    assert_eq!(rig.ftp.contents()[0], printed);
    geometry(&project, &printed, 256., 250.);
    assert_eq!(prediction(&printed), seconds);
    assert_eq!(current["estimate"]["seconds"], seconds);
    let gcode = String::from_utf8(zip_read(&printed, "Metadata/plate_1.gcode")).unwrap();
    assert!(gcode.len() > 1000);
    let settings = zip_json(&printed, "Metadata/project_settings.config");
    assert_eq!(settings["textured_plate_temp_initial_layer"], json!(["65"]));
    assert_eq!(settings["textured_plate_temp"], json!(["65"]));
    let bed: Vec<_> = gcode
        .lines()
        .filter(|l| l.starts_with("M140 ") || l.starts_with("M190 "))
        .map(|l| parameter(l.split(';').next().unwrap(), 'S').unwrap())
        .collect();
    assert!(bed.contains(&65.) && bed.iter().all(|v| [0., 65.].contains(v)));
    fs::write(
        rig.output.join("first.log"),
        container::docker(&["logs", &name]),
    )
    .unwrap();
    rig.stop(false);
    rig.launch();
    let q = rig.queue();
    assert_eq!(q["current"]["id"], job["id"]);
    assert_eq!(q["current"]["state"], "needs_attention");
    assert_eq!(
        array(&q["waiting"]).iter().map(id).collect::<Vec<_>>(),
        [id(&waiting)]
    );
    assert_eq!(q["allowed"]["next"], false);
    assert_eq!(rig.get(&format!("/api/plates/{}", id(&plate))), plate);
    for model in array(&plate["models"]) {
        assert_eq!(
            rig.bytes(&format!("/api/plates/{}/files/{}", id(&plate), id(model))),
            cube
        );
    }
    rig.idle();
    assert_eq!(rig.broker.prints().len(), 1);
    until(
        || {
            container::docker(&["inspect", "--format", "{{.State.Health.Status}}", &name])
                == "healthy"
        },
        45,
    );
    write_json(
        &rig.output.join("result.json"),
        &json!({"image":image(),"uid":10001,"seconds":seconds,"persistent_queue":true,"uncertain_restart_no_resend":true,"healthy":true}),
    );
}
#[test]
#[ignore = "requires Docker and the image built from this checkout"]
fn support_matrix() {
    support_scenario::support(Rig::in_container("container-support", &image()));
}
