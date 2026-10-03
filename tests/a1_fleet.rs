//! Three A1 mini printers, two with an AMS lite and one feeding from the external spool:
//! reports, controls and starts stay with their own printer.
mod common;
use common::peers::{Peer, SECRET};
use common::*;
use serde_json::{Value, json};

const A1: &str = "Bambu Lab A1 mini 0.2 nozzle";

fn status(rig: &Rig, pid: &str) -> Value {
    rig.get(&format!("/api/printer/status?printer_id={pid}"))
}
fn command(rig: &Rig, pid: &str, action: Value) -> Value {
    let q = rig.get(&format!("/api/queue?printer_id={pid}"));
    rig.post(
        &format!("/api/queue?printer_id={pid}"),
        &rig.command(action, Some(&q)),
        200,
    )
}

#[test]
#[allow(clippy::too_many_lines)] // One fleet from registration to three starts.
fn three_a1_minis_with_and_without_ams_lite_never_cross() {
    let mut rig = Rig::new("a1-fleet");
    rig.env.retain(|key, _| !key.starts_with("P1_"));
    let pem = std::fs::read_to_string(rig.root.path().join("trusted.pem")).unwrap();
    let peers: Vec<_> = (0..3)
        .map(|i| {
            (
                Peer::broker(rig.root.path(), "trusted", &format!("A1MINI{i}")),
                Peer::ftps(rig.root.path(), "trusted"),
            )
        })
        .collect();
    rig.launch();
    let mut url = reqwest::Url::parse("http://fixture/api/slicer/profiles").unwrap();
    url.query_pairs_mut().append_pair("machine", A1);
    let profiles = rig.get(&format!("{}?{}", url.path(), url.query().unwrap()));
    let mut ids = Vec::new();
    for (i, (broker, ftp)) in peers.iter().enumerate() {
        let saved = rig.post("/api/printers", &json!({"name":format!("A1 mini {i}"),"host":"127.0.0.1",
            "serial":format!("A1MINI{i}"),"access_code":SECRET,"tls_certificate":pem,"machine_profile_key":A1,
            "default_process_profile_key":profiles["defaults"]["process"],"bed_type":profiles["defaults"]["bed"],
            "nozzle_material":"unknown","mqtt_port":broker.port,"ftps_port":ftp.port,"start_timeout_secs":60}), 201);
        ids.push(id(&saved).to_owned());
    }
    let full = |i: usize| {
        let mut report = rig.full.clone();
        report["print"]["nozzle_diameter"] = json!("0.2");
        report["print"]["nozzle_temper"] = json!(200 + i);
        if i == 1 {
            report["print"]["ams"] =
                json!({"ams":[],"ams_exist_bits":"0","tray_exist_bits":"0","tray_now":"254"});
            report["print"]["vt_tray"] = json!({"id":"254","tray_type":"PLA"});
        }
        report
    };
    for (i, (broker, _)) in peers.iter().enumerate() {
        until(|| !broker.requests().is_empty(), 12);
        broker.send(&full(i));
        until(|| status(&rig, &ids[i])["ready_to_print"] == true, 12);
    }
    for (i, pid) in ids.iter().enumerate() {
        let s = status(&rig, pid);
        assert_eq!(
            s["live"]["temperatures"]["nozzle"],
            f64::from(200 + u8::try_from(i).unwrap())
        );
        assert_eq!(
            s["ams"]["units"].as_array().map_or(0, Vec::len),
            usize::from(i != 1)
        );
    }

    rig.post(
        &format!("/api/printers/{}/control", ids[1]),
        &json!({"action":"light","on":false}),
        200,
    );
    rig.post(
        &format!("/api/printers/{}/control", ids[2]),
        &json!({"action":"speed","level":1}),
        200,
    );
    rig.post(
        &format!("/api/printers/{}/control", ids[2]),
        &json!({"action":"move","axis":"Y","mm":10}),
        200,
    );
    let operator = |i: usize| -> Vec<Value> {
        peers[i]
            .0
            .records
            .lock()
            .unwrap()
            .controls
            .iter()
            .filter(|c| c.get("info").is_none())
            .cloned()
            .collect()
    };
    assert!(operator(0).is_empty());
    assert_eq!(operator(1)[0]["system"]["command"], "ledctrl");
    assert_eq!(operator(1).len(), 1);
    assert_eq!(operator(2)[0]["print"]["command"], "print_speed");
    assert!(
        operator(2)[1]["print"]["param"]
            .as_str()
            .unwrap()
            .contains("G1 Y-10.0 F3000"),
        "the A1 bed moves the other way"
    );
    assert_eq!(operator(2).len(), 2);

    let material = rig.post(
        "/api/filaments",
        &json!({"name":"A1 PLA","vendor":"Fixture","material":"PLA",
        "color":"FFFFFFFF","bambu_filament_id":null}),
        201,
    );
    rig.post(
        &format!("/api/filaments/{}/settings", id(&material)),
        &json!({"machine_profile_key":A1,
        "base_profile_key":profiles["defaults"]["filament"],"overrides_json":{}}),
        201,
    );
    for i in [0, 2] {
        let inventory = rig.get(&format!("/api/printers/{}/ams", ids[i]));
        let slot = array(&inventory["slots"])
            .iter()
            .find(|s| s["slot_index"] == 0)
            .unwrap()
            .clone();
        rig.put(
            &format!("/api/printers/{}/ams/{}", ids[i], id(&slot)),
            &json!({"revision":slot["revision"],"filament_id":material["id"]}),
            204,
        );
    }
    let plate = rig.post(
        "/api/plates/import",
        &json!({"name":"A1 cube","models":[{"name":"parts/cube.stl",
        "source":"parts/cube.stl","quantity":2}],"conditions":{"required_machine_profile_key":A1,
        "filament_id":material["id"],"process_profile_key":profiles["defaults"]["process"],
        "bed_type":profiles["defaults"]["bed"]}}),
        201,
    );
    for (i, pid) in ids.iter().enumerate() {
        // The printer without an AMS lite takes the job for its external spool.
        let feed = if i == 1 { "external" } else { "ams" };
        let added = command(
            &rig,
            pid,
            json!({"type":"add","plate_id":plate["id"],"plate_version":plate["version"]}),
        );
        let job = added["waiting"][0].clone();
        assert_eq!(job["feed"], feed);
        command(
            &rig,
            pid,
            json!({"type":"next","expected_job":job["id"],"removed_job":null,"cleared":true}),
        );
    }
    until(
        || peers.iter().all(|(broker, _)| broker.prints().len() == 1),
        60,
    );
    for (i, (broker, ftp)) in peers.iter().enumerate() {
        let start = &broker.prints()[0];
        assert_eq!(start["use_ams"], i != 1, "printer {i}");
        assert_eq!(ftp.uploads().len(), 1);
        let current = rig.get(&format!("/api/queue?printer_id={}", ids[i]))["current"].clone();
        assert_eq!(
            start["subtask_name"],
            format!("orca-{}", current["attempt_id"].as_str().unwrap())
        );
    }
    for (broker, _) in &peers {
        broker.check();
    }
    rig.check();
}
