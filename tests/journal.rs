mod common;
use common::peers::{Action, SECRET};
use common::*;
use serde_json::{Value, json};
use std::fs;

const WEBHOOK: &str = "https://discord.com/api/webhooks/123456789012345678/journal-probe-token";

fn journal(rig: &Rig, query: &str) -> Value {
    let value = rig.get(&format!("/api/journal?{query}"));
    let text = value.to_string();
    assert!(!text.contains(SECRET), "access code in the journal API");
    assert!(
        !text.contains("journal-probe-token"),
        "webhook in the journal API"
    );
    value
}
fn entries(rig: &Rig, query: &str) -> Vec<Value> {
    array(&journal(rig, query)["entries"]).to_vec()
}
fn seqs(entries: &[Value]) -> Vec<u64> {
    entries.iter().map(|e| e["seq"].as_u64().unwrap()).collect()
}
fn on_disk(rig: &Rig) -> (String, u64) {
    let mut text = String::new();
    let mut bytes = 0;
    for file in fs::read_dir(rig.store.join("journal")).unwrap() {
        let path = file.unwrap().path();
        bytes += fs::metadata(&path).unwrap().len();
        text += &fs::read_to_string(path).unwrap();
    }
    (text, bytes)
}

#[test]
fn traffic_is_recorded_in_order_filtered_redacted_and_kept_across_restarts() {
    let mut rig = Rig::new("journal-traffic");
    rig.env.insert("DISCORD_WEBHOOK_URL".into(), WEBHOOK.into());
    rig.launch();
    rig.seed();

    // The connection, the full-report request and its answer appear in that order.
    let all = entries(&rig, "printer=p1&limit=1000");
    let position = |pred: &dyn Fn(&Value) -> bool| all.iter().position(pred).unwrap();
    let connected = position(&|e| e["kind"] == "connection" && e["body"]["event"] == "connected");
    let subscribed = position(&|e| e["body"]["event"] == "subscribed" && e["body"]["ok"] == true);
    let pushall = position(&|e| {
        e["kind"] == "request" && e["dir"] == "out" && e["body"]["pushing"]["command"] == "pushall"
    });
    let report = position(&|e| e["kind"] == "report" && e["dir"] == "in");
    assert!(connected < subscribed && subscribed < pushall && pushall < report);
    assert_eq!(all[report]["body"], rig.full);
    assert!(all[report]["epoch"].is_u64() && all[report]["at"].is_u64());
    let s = seqs(&all);
    assert!(s.windows(2).all(|w| w[0] < w[1]), "oldest first: {s:?}");

    // Secrets inside a received report are replaced before storage.
    let mut probe = rig.full.clone();
    probe["print"]["access_code"] = json!("echoed-value");
    probe["print"]["note"] = json!(format!("{SECRET} {WEBHOOK}"));
    rig.broker.send(&probe);
    until(
        || {
            entries(&rig, "kind=report&limit=1")[0]["body"]["print"]["note"]
                == "[redacted] [redacted]"
        },
        12,
    );
    assert_eq!(
        entries(&rig, "kind=report&limit=1")[0]["body"]["print"]["access_code"],
        "[redacted]"
    );

    // A failed transfer leaves its stage and the application's warning.
    let job = rig.add(3);
    rig.ftp.action(Action::Fail);
    rig.next(&job, 200);
    until(|| rig.queue()["current"]["state"] == "needs_attention", 20);
    let ftps = entries(&rig, "kind=ftps");
    assert_eq!(ftps[0]["body"]["event"], "upload");
    assert_eq!(ftps[1]["body"]["event"], "result");
    assert_eq!(ftps[1]["body"]["failed_stage"], "transfer");
    until(|| !entries(&rig, "kind=log").is_empty(), 12);
    let warning = entries(&rig, "kind=log&printer=p1").pop().unwrap();
    assert_eq!(warning["level"], "WARN");
    assert_eq!(warning["body"]["stage"], "transfer");

    let requests = rig.broker.requests().len();
    rig.broker.action(Action::Disconnect);
    until(|| rig.broker.requests().len() > requests, 20);
    let disconnected = entries(&rig, "kind=connection")
        .into_iter()
        .find(|e| e["body"]["event"] == "disconnected")
        .unwrap();
    assert_eq!(disconnected["body"]["reason"], "closed");

    // Filters: direction, kind, printer, inclusive time and limit.
    assert!(
        entries(&rig, "direction=in&limit=1000")
            .iter()
            .all(|e| e["kind"] == "report")
    );
    assert!(
        entries(&rig, "direction=out&limit=1000")
            .iter()
            .all(|e| e["kind"] == "request")
    );
    assert!(entries(&rig, "printer=missing").is_empty());
    let newest = journal(&rig, "limit=1");
    assert_eq!(newest["truncated"], true);
    let at = newest["entries"][0]["at"].as_u64().unwrap();
    let since = entries(&rig, &format!("since={at}&limit=1000"));
    assert!(!since.is_empty() && since.iter().all(|e| e["at"].as_u64() >= Some(at)));
    let until_ = entries(&rig, &format!("until={}&limit=1000", at - 1));
    assert!(until_.iter().all(|e| e["at"].as_u64() < Some(at)));
    for bad in ["direction=up", "limit=0", "limit=1001", "since=soon"] {
        rig.request("GET", &format!("/api/journal?{bad}"), None, 400);
    }

    let (text, _) = on_disk(&rig);
    assert!(
        !text.contains(SECRET)
            && !text.contains("journal-probe-token")
            && !text.contains("echoed-value")
    );
    assert!(!text.contains("CERTIFICATE"));

    // Records survive a restart and numbering continues.
    let last = *seqs(&entries(&rig, "limit=1")).last().unwrap();
    rig.stop(false);
    rig.launch();
    let after = entries(&rig, "limit=1000");
    assert!(
        after.iter().any(|e| e["seq"] == 1),
        "pre-restart records kept"
    );
    assert!(after.iter().any(|e| e["seq"].as_u64() > Some(last)));
    rig.check();
}

#[test]
fn the_oldest_segments_are_removed_beyond_the_cap() {
    let mut rig = Rig::new("journal-cap");
    let cap: u64 = 16 * 1024;
    rig.env.insert("JOURNAL_MAX_BYTES".into(), cap.to_string());
    rig.launch();
    for _ in 0..80 {
        rig.broker.send(&rig.full);
    }
    until(
        || entries(&rig, "limit=1")[0]["seq"].as_u64() > Some(80),
        12,
    );
    until(
        || {
            entries(&rig, "limit=1000")
                .first()
                .is_some_and(|e| e["seq"] != 1)
        },
        12,
    );
    let (_, bytes) = on_disk(&rig);
    assert!(bytes <= cap, "{bytes} bytes exceed the {cap} byte cap");
    let kept = seqs(&entries(&rig, "limit=1000"));
    assert!(
        kept.windows(2).all(|w| w[1] == w[0] + 1),
        "contiguous newest records: {kept:?}"
    );
    rig.check();
}

/// Reports recorded on one server, sent again to a fresh one, reach the same state.
#[test]
fn recorded_reports_replay_to_the_same_state() {
    let mut recorded = Rig::new("journal-record");
    recorded.launch();
    recorded.seed();
    let requests = recorded.broker.requests().len();
    recorded.broker.action(Action::Disconnect);
    until(|| recorded.broker.requests().len() > requests, 20);
    // Power-up: the full report lists bare trays, later diffs fill slots 0 and 3.
    let loaded = recorded.full["print"]["ams"]["ams"][0]["tray"].clone();
    let trays = |read: &[usize]| -> Value {
        (0..4)
            .map(|i| {
                if read.contains(&i) {
                    loaded[i].clone()
                } else {
                    json!({"id":i.to_string()})
                }
            })
            .collect()
    };
    let mut report = recorded.full.clone();
    report["print"]["ams"]["ams"][0]["tray"] = trays(&[]);
    report["print"]["ams"]["tray_reading_bits"] = json!("1");
    recorded.broker.send(&report);
    until(|| recorded.slot(0)["reported"]["material"].is_null(), 12);
    for read in [&[0][..], &[0, 3][..]] {
        recorded
            .broker
            .send(&json!({"print":{"command":"push_status","msg":1,
            "ams":{"ams":[{"id":"0","tray":trays(read)}],"tray_read_done_bits":"9"}}}));
    }
    until(|| recorded.slot(3)["reported"]["material"] == "PLA", 12);
    let observed = |rig: &Rig| {
        let status = rig.get("/api/printer/status?printer_id=p1");
        let ams = rig.get("/api/printers/p1/ams");
        let slots: Vec<Value> = (0..4)
            .map(|i| {
                array(&ams["slots"])
                    .iter()
                    .find(|v| v["slot_index"] == i && v["ams_id"] == 0)
                    .map_or(Value::Null, |v| v["reported"].clone())
            })
            .collect();
        (status["print"]["state"].clone(), slots)
    };
    let expected = observed(&recorded);
    let present: Vec<_> = expected.1.iter().map(|s| s["present"].clone()).collect();
    assert_eq!(
        present,
        [json!(true), json!(false), json!(false), json!(true)]
    );

    let reports = orca_server::journal::reports(
        &recorded
            .get("/api/journal?printer=p1&kind=report&direction=in&limit=1000")
            .to_string(),
    );
    assert!(reports.len() >= 4);
    let mut replayed = Rig::new("journal-replay");
    replayed.launch();
    for bytes in reports {
        replayed.broker.report(bytes, false, None);
    }
    until(|| observed(&replayed) == expected, 12);
    recorded.check();
    replayed.check();
}
