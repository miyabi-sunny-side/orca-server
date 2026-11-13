use crate::common::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

const Z_HOMING: &str = "Z軸の原点復帰に失敗しました。タスクは停止されました。";
const BED_HOMING: &str = "ベッドの原点復帰に異常があります。";

fn answer() -> Vec<u8> {
    json!({"result":0,"t":1,"ver":1,"data":{
        "device_error":{"ver":1,"ja":[{"ecode":"03004000","intro":Z_HOMING}]},
        "device_hms":{"ver":1,"ja":[{"ecode":"03000D0000020001","intro":BED_HOMING}]}}})
    .to_string()
    .into_bytes()
}

/// The running print reports an error and two HMS entries, one of them unlisted.
fn failing(rig: &Rig, error: u32) {
    let sent = rig.broker.prints()[0].clone();
    let mut report = rig.full.clone();
    report["print"] = merge(
        &report["print"],
        &json!({"gcode_state":"RUNNING","subtask_name":sent["subtask_name"],
            "gcode_file":sent["file"],"print_error":error,
            "hms":[{"attr":0x0300_0D00_u32,"code":0x0002_0001},{"attr":0x0300_0A00_u32,"code":0x0001_0005}]}),
    );
    rig.broker.send(&report);
}

#[test]
fn official_descriptions_reach_the_queue_and_survive_an_unreachable_site() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    let site = HttpPeer::new(move |method, uri, _| {
        seen.lock()
            .expect("mutex is not poisoned")
            .push(format!("{method} {uri}"));
        (200, answer())
    });
    let mut rig = Rig::new("codes");
    rig.env.insert(
        "BAMBU_ERROR_TABLE_URL".into(),
        format!("{}/query.php", site.base),
    );
    rig.launch();
    rig.seed();
    let saved = rig.root.path().join("plates/codes/ja_TES.json");
    until(|| saved.is_file(), 12);
    assert_eq!(
        *asked.lock().expect("mutex is not poisoned"),
        ["GET /query.php?lang=ja&d=TES"],
        "only the language and the device type are sent, once"
    );

    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    failing(&rig, 0x0300_4000);
    rig.phase("needs_attention");
    let explained = json!({"0300-4000":Z_HOMING,"0300_0D00_0002_0001":BED_HOMING});
    let view = rig.queue();
    assert_eq!(
        view["printer"]["descriptions"], explained,
        "an unlisted HMS code has no description"
    );
    assert_eq!(
        view["printer"]["live"]["hms"],
        json!(["0300_0D00_0002_0001", "0300_0A00_0001_0005"])
    );
    assert_eq!(view["current"]["failure"]["code"], "0300-4000");
    assert_eq!(view["current"]["failure"]["description"], Z_HOMING);
    assert_eq!(
        rig.get("/api/printer/status?printer_id=p1")["descriptions"],
        explained
    );

    // A code the table lacks keeps its raw value and gains nothing.
    failing(&rig, 0x0300_8010);
    until(
        || rig.queue()["current"]["failure"]["code"] == "0300-8010",
        12,
    );
    assert!(rig.queue()["current"]["failure"]["description"].is_null());

    // The saved answer is used when the site is gone and downloading is off.
    rig.stop(false);
    drop(site);
    rig.env.insert("BAMBU_ERROR_TABLE_URL".into(), "off".into());
    rig.launch();
    failing(&rig, 0x0300_4000);
    until(|| rig.queue()["printer"]["descriptions"] == explained, 12);
    rig.check();
}

#[test]
fn printing_does_not_depend_on_the_table() {
    // Nothing listens here: the download fails and the table stays empty.
    let mut rig = Rig::new("codes-unreachable");
    rig.env.insert(
        "BAMBU_ERROR_TABLE_URL".into(),
        "http://127.0.0.1:9/query.php".into(),
    );
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    let sent = rig.broker.prints()[0].clone();
    let mut report = rig.full.clone();
    report["print"] = merge(
        &report["print"],
        &json!({"gcode_state":"RUNNING","subtask_name":sent["subtask_name"],
            "gcode_file":sent["file"],"print_error":0x0300_4000_u32}),
    );
    rig.broker.send(&report);
    rig.phase("needs_attention");
    let view = rig.queue();
    assert_eq!(view["current"]["failure"]["code"], "0300-4000");
    assert!(view["current"]["failure"]["description"].is_null());
    assert_eq!(view["printer"]["descriptions"], json!({}));
    assert!(
        !rig.root.path().join("plates/codes").exists(),
        "a failed download saves nothing"
    );
}
