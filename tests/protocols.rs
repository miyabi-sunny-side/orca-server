mod common;
use common::peers::{Action, Peer, SECRET, certificate};
use common::*;
use serde_json::json;
use std::{sync::atomic::Ordering, thread, time::Duration};

#[test]
fn isolated_queue_uploads_exact_artifact() {
    let mut rig = Rig::new("fixture-smoke");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    let command = &rig.broker.prints()[0];
    assert_eq!(command["url"], format!("ftp:///{}", rig.ftp.uploads()[0]));
    assert_eq!(command["ams_mapping"], json!([3]));
    assert_eq!(command["param"], "Metadata/plate_1.gcode");
    assert_eq!(command["use_ams"], true);
    assert_eq!(
        rig.ftp.contents()[0],
        std::fs::read(rig.artifact("print.gcode.3mf")).unwrap()
    );
    rig.check();
}

#[allow(clippy::too_many_lines)] // Follow synchronization, reconnect and certificate rejection together.
fn mqtt(version: &str) {
    let mut rig = Rig::with_options("mqtt", version, None);
    rig.env.insert("LOG_LEVEL".into(), "trace".into());
    rig.full["print"]["access_code"] = json!(SECRET);
    rig.launch();
    let status = || rig.get("/api/printer/status");
    assert_eq!(status()["connection"], "synchronizing");
    assert_eq!(status()["ready_to_print"], false);
    rig.broker
        .report(serde_json::to_vec(&rig.full).unwrap(), true, None);
    rig.broker.send(
        &json!({"print":{"command":"push_status","msg":1,"gcode_state":"IDLE","print_error":0}}),
    );
    thread::sleep(Duration::from_millis(300));
    assert_eq!(status()["synchronized"], false);
    rig.broker.send(&rig.full);
    until(|| status()["ready_to_print"] == true, 12);
    rig.broker.send(&json!({"print":{"command":"push_status","msg":1,"gcode_state":"RUNNING","mc_percent":25,"ams":{"ams":[{"id":"0","tray":[{"id":"0","remain":60}]}]}}}));
    until(|| status()["print"]["percent"] == 25, 12);
    let observed = status();
    assert_eq!(observed["ready_to_print"], false);
    assert_eq!(observed["ams"]["units"][0]["trays"][0]["material"], "PLA");
    assert_eq!(
        observed["ams"]["units"][0]["trays"][0]["remaining_percent"],
        60
    );
    rig.broker.report(
        serde_json::to_vec(&rig.full).unwrap(),
        false,
        Some("device/OTHER/report"),
    );
    thread::sleep(Duration::from_millis(200));
    assert_eq!(status()["print"]["state"], "RUNNING");
    rig.broker.action(Action::Disconnect);
    until(|| status()["connection"] == "disconnected", 12);
    until(|| rig.broker.requests().len() == 2, 12);
    assert_eq!(status()["synchronized"], false);
    assert!(status()["print"]["state"].is_null());
    rig.broker
        .send(&json!({"print":{"command":"push_status","msg":1,"mc_percent":50}}));
    thread::sleep(Duration::from_millis(200));
    assert_eq!(status()["ready_to_print"], false);
    let mut resumed = rig.full.clone();
    resumed["print"]["gcode_state"] = json!("RUNNING");
    resumed["print"]["mc_percent"] = json!(50);
    rig.broker.send(&resumed);
    until(|| status()["synchronized"] == true, 12);
    assert_eq!(status()["print"]["state"], "RUNNING");
    assert_eq!(status()["ready_to_print"], false);
    rig.broker.report(b"broken".to_vec(), false, None);
    until(|| status()["synchronized"] == false, 12);
    rig.broker.send(&rig.full);
    until(|| status()["ready_to_print"] == true, 12);
    rig.broker.action(Action::InvalidLogin);
    until(|| status()["connection"] == "disconnected", 12);
    rig.stop(false);
    certificate(rig.root.path(), "other", version);
    let wrong = Peer::broker(rig.root.path(), "other", common::peers::SERIAL);
    rig.env.insert(
        "PLATES_DIR".into(),
        rig.root.path().join("wrong-store").display().to_string(),
    );
    rig.env
        .insert("P1_MQTT_PORT".into(), wrong.port.to_string());
    rig.launch();
    until(|| wrong.tls_failures.load(Ordering::SeqCst) > 0, 12);
    until(
        || rig.get("/api/printer/status")["connection"] == "disconnected",
        12,
    );
    assert_eq!(rig.get("/api/printer/status")["ready_to_print"], false);
    assert!(wrong.requests().is_empty());
    rig.stop(false);
    rig.env.retain(|k, _| !k.starts_with("P1_"));
    rig.env.insert(
        "PLATES_DIR".into(),
        rig.root
            .path()
            .join("unconfigured-store")
            .display()
            .to_string(),
    );
    rig.launch();
    let status = rig.get("/api/printer/status");
    assert_eq!(status["connection"], "unconfigured");
    assert_eq!(status["ready_to_print"], false);
    rig.stop(false);
    rig.check();
    for path in [
        &rig.store,
        &rig.root.path().join("wrong-store"),
        &rig.root.path().join("unconfigured-store"),
    ] {
        no_secret_files(path);
    }
}
#[test]
fn mqtt_v1() {
    mqtt("v1");
}
#[test]
fn mqtt_v3() {
    mqtt("v3");
}

fn printer_start(version: &str) {
    let mut rig = Rig::with_options("printer-start", version, None);
    rig.env.insert("P1_START_TIMEOUT_SECS".into(), "3".into());
    certificate(rig.root.path(), "other", version);
    let wrong = Peer::ftps(rig.root.path(), "other");
    rig.launch();
    rig.seed();
    let mut job = rig.add(3);
    rig.next(&job, 200);
    until(|| rig.broker.prints().len() == 1, 12);
    let command = rig.broker.prints()[0].clone();
    assert_eq!(command["url"], format!("ftp:///{}", rig.ftp.uploads()[0]));
    assert_eq!(command["param"], "Metadata/plate_1.gcode");
    assert_eq!(command["ams_mapping"], json!([3]));
    assert_eq!(command["use_ams"], true);
    assert_eq!(
        rig.ftp.contents().last().unwrap(),
        &std::fs::read(rig.artifact("print.gcode.3mf")).unwrap()
    );
    rig.start_phase("awaiting_confirmation");
    rig.broker.send(
        &json!({"print":{"command":"project_file","sequence_id":"wrong","result":"success"}}),
    );
    thread::sleep(Duration::from_millis(100));
    rig.start_phase("awaiting_confirmation");
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":command["sequence_id"],"result":"success"}}));
    rig.start_phase("accepted");
    assert_eq!(rig.queue()["current"]["state"], "preparing");
    let mut unrelated = rig.full.clone();
    unrelated["print"] = merge(
        &unrelated["print"],
        &json!({"gcode_state":"RUNNING","subtask_name":"not-our-job","gcode_file":"other.gcode.3mf"}),
    );
    rig.broker.send(&unrelated);
    thread::sleep(Duration::from_millis(100));
    rig.start_phase("accepted");
    rig.report("RUNNING");
    rig.start_phase("printing");
    rig.broker.action(Action::Disconnect);
    rig.start_phase("unknown");
    until(|| rig.broker.requests().len() == 2, 12);
    assert_eq!(rig.broker.prints().len(), 1);
    rig.report("RUNNING");
    rig.start_phase("printing");
    rig.report("FINISH");
    rig.start_phase("finished");
    rig.discard();
    rig.idle();
    job = rig.add(3);
    rig.ftp.action(Action::Fail);
    rig.next(&job, 200);
    rig.start_phase("upload_failed");
    assert_eq!(rig.broker.prints().len(), 1);
    let retry = |rig: &Rig, job: &serde_json::Value| {
        rig.send(
            json!({"type":"retry","expected_job":job["id"],"cleared":true}),
            200,
        )
    };
    rig.ftp.action(Action::Wait);
    retry(&rig, &job);
    until(|| rig.ftp.received.load(Ordering::SeqCst), 10);
    let mut busy = rig.full.clone();
    busy["print"]["gcode_state"] = json!("RUNNING");
    rig.broker.send(&busy);
    until(|| rig.queue()["printer"]["ready_to_print"] == false, 12);
    rig.ftp.release();
    rig.start_phase("not_sent");
    assert_eq!(rig.broker.prints().len(), 1);
    rig.idle();
    retry(&rig, &job);
    until(|| rig.broker.prints().len() == 2, 12);
    let rejected = rig.broker.prints().last().unwrap().clone();
    rig.broker.send(&json!({"print":{"command":"project_file","sequence_id":rejected["sequence_id"],"result":"fail"}}));
    rig.start_phase("rejected");
    retry(&rig, &job);
    until(|| rig.broker.prints().len() == 3, 12);
    rig.start_phase("unknown");
    until(|| rig.broker.requests().len() == 3, 12);
    assert_eq!(rig.broker.prints().len(), 3);
    rig.idle();
    rig.send(
        json!({"type":"retry","expected_job":job["id"],"cleared":false}),
        409,
    );
    rig.discard();
    let mut settings = printer_settings(&rig.get("/api/printers/p1"));
    settings["ftps_port"] = json!(wrong.port);
    let requests = rig.broker.requests().len();
    rig.put("/api/printers/p1", &settings, 200);
    until(|| rig.broker.requests().len() > requests, 12);
    rig.idle();
    job = rig.add(3);
    rig.next(&job, 200);
    rig.start_phase("upload_failed");
    assert!(wrong.tls_failures.load(Ordering::SeqCst) > 0);
    assert!(wrong.uploads().is_empty());
    assert_eq!(rig.broker.prints().len(), 3);
    rig.check();
}
#[test]
fn print_start_v1() {
    printer_start("v1");
}
#[test]
fn print_start_v3() {
    printer_start("v3");
}

mod codec {
    use crate::common::wire::*;
    use std::io;

    #[test]
    fn mqtt_lengths_and_truncated_input() {
        assert_eq!(packet(0xc0, &[]), [0xc0, 0]);
        assert_eq!(packet(0x30, &[42; 128])[..3], [0x30, 0x80, 1]);
        assert_eq!(
            read_packet(&mut &[0x30, 3, 7, 8, 9][..]).unwrap(),
            (0x30, vec![7, 8, 9])
        );
        assert_eq!(
            read_packet(&mut &[0x30, 3, 7][..]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(read_packet(&mut &[0x30, 0xff, 0xff, 0xff, 0xff, 0][..]).is_err());
        assert!(read_packet(&mut &[0x30, 0xff, 0xff, 0x7f][..]).is_err());
    }
}

fn no_secret_files(path: &std::path::Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            no_secret_files(&path);
        } else if path.file_name().unwrap() != "orca.sqlite3"
            && !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("orca.sqlite3-")
        {
            let bytes = std::fs::read(path).unwrap();
            assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));
        }
    }
}
