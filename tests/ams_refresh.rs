mod common;
use common::peers::Action;
use common::*;
use serde_json::{Value, json};
use std::{thread, time::Duration};

const REFRESH: &str = "/api/printers/p1/ams/refresh";

fn reply(rig: &Rig, report: Option<&Value>) {
    *rig.broker.pushall_reply.lock().unwrap() = report.map(|r| serde_json::to_vec(r).unwrap());
}
/// A refresh that waits for the server's full 15 s printer timeout (the harness client stops at 10 s).
fn refresh_slowly(rig: &Rig, expected: u16) -> Value {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
        .post(format!("{}{REFRESH}", rig.base))
        .json(&json!({}))
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), expected);
    response.json().unwrap()
}
fn pushes(rig: &Rig) -> usize {
    rig.broker.requests().len()
}
/// The fixture's full report with white loaded in slot 2 (index 1).
fn white_in_two(rig: &Rig) -> Value {
    let mut report = rig.full.clone();
    report["print"]["ams"]["tray_exist_bits"] = json!("b");
    report["print"]["ams"]["ams"][0]["tray"][1] =
        json!({"id":"1","tray_type":"PLA","tray_color":"FFFFFFFF"});
    report
}

#[test]
fn a_manual_refresh_asks_the_printer_and_returns_its_new_full_report() {
    let mut rig = Rig::new("ams-refresh");
    rig.launch();
    rig.seed();
    let before = pushes(&rig);
    for _ in 0..3 {
        rig.get("/api/printers/p1/ams");
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(pushes(&rig), before, "ordinary reads never ask the printer");

    // Synchronized already, yet the button still asks the printer and shows its answer.
    assert_eq!(rig.queue()["printer"]["synchronized"], true);
    reply(&rig, Some(&white_in_two(&rig)));
    let view = rig.post(REFRESH, &json!({}), 200);
    assert_eq!(pushes(&rig), before + 1);
    assert_eq!(view["slots"][1]["reported"]["material"], "PLA");
    assert_eq!(view["slots"][1]["reported"]["present"], true);
    assert_eq!(view["refresh"]["reading"], false);
    assert!(view["refresh"]["report_at"].is_u64());

    // The same content again is still a completed refresh.
    let again = rig.post(REFRESH, &json!({}), 200);
    assert_eq!(pushes(&rig), before + 2);
    assert_eq!(again["slots"][1]["reported"]["material"], "PLA");

    // Material reading in progress is reported as such, not as finished.
    let mut reading = white_in_two(&rig);
    reading["print"]["ams"]["tray_reading_bits"] = json!("8");
    reading["print"]["ams"]["tray_read_done_bits"] = json!("3");
    reply(&rig, Some(&reading));
    assert_eq!(
        rig.post(REFRESH, &json!({}), 200)["refresh"]["reading"],
        true
    );

    // A second press while one is waiting is refused; the first still completes.
    reply(&rig, None);
    thread::scope(|scope| {
        let first = scope.spawn(|| rig.post(REFRESH, &json!({}), 200));
        until(|| pushes(&rig) == before + 4, 5);
        rig.post(REFRESH, &json!({}), 409);
        assert_eq!(pushes(&rig), before + 4, "no second request while waiting");
        rig.broker.send(&white_in_two(&rig));
        assert_eq!(
            first.join().unwrap()["slots"][1]["reported"]["present"],
            true
        );
    });
    rig.check();
}

#[test]
fn a_manual_refresh_fails_visibly_without_a_new_full_ams_report() {
    let mut rig = Rig::new("ams-refresh-failures");
    rig.launch();
    rig.seed();

    // Neither a diff nor a full report without the AMS completes the request.
    let mut without_ams = rig.full.clone();
    without_ams["print"].as_object_mut().unwrap().remove("ams");
    reply(&rig, Some(&without_ams));
    thread::scope(|scope| {
        let waiting = scope.spawn(|| refresh_slowly(&rig, 504));
        thread::sleep(Duration::from_secs(1));
        rig.broker.send(&json!({"print":{"command":"push_status","msg":1,
            "ams":{"ams":[{"id":"0","tray":[{"id":"1","tray_type":"PLA","tray_color":"FFFFFFFF"}]}]}}}));
        assert_eq!(
            waiting.join().unwrap()["error"],
            "The printer did not send a full report in time"
        );
    });

    // A disconnect while waiting is a connection failure, and retrying works afterwards.
    reply(&rig, None);
    thread::scope(|scope| {
        let requests = rig.broker.requests().len();
        let waiting = scope.spawn(|| rig.post(REFRESH, &json!({}), 502));
        until(|| rig.broker.requests().len() > requests, 5);
        rig.broker.action(Action::Disconnect);
        assert!(
            waiting.join().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("connection")
        );
    });
    until(|| rig.queue()["printer"]["synchronized"] == false, 12);
    rig.broker.send(&rig.full);
    until(|| rig.queue()["printer"]["synchronized"] == true, 12);

    // A report that cannot be saved is not shown as refreshed.
    reply(&rig, Some(&rig.full));
    rig.db()
        .execute("UPDATE printers SET host='203.0.113.9' WHERE id='p1'", [])
        .unwrap();
    rig.post(REFRESH, &json!({}), 503);
    rig.db()
        .execute("UPDATE printers SET host='127.0.0.1' WHERE id='p1'", [])
        .unwrap();
    rig.post(REFRESH, &json!({}), 200);
    rig.request(
        "POST",
        "/api/printers/missing/ams/refresh",
        Some(&json!({})),
        404,
    );
    rig.check();
}
