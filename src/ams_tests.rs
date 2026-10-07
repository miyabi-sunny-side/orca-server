use super::*;
use crate::{database::Settings, printer_state::State};
use serde_json::{Value, json};

fn device() -> Device {
    Device {
        id: "p".into(),
        settings: Settings {
            name: "P1S".into(),
            host: "127.0.0.1".into(),
            serial: "TEST".into(),
            access_code: "fixture".into(),
            tls_certificate: "fixture".into(),
            machine_profile_key: "machine".into(),
            default_process_profile_key: "process".into(),
            bed_type: crate::profiles::BEDS[0].into(),
            nozzle_material: "unknown".into(),
            mqtt_port: 8883,
            ftps_port: 990,
            start_timeout_secs: 600,
            camera_port: 6000,
        },
    }
}
fn observe(db: &Database, state: &mut State, value: &Value) {
    assert!(state.apply(&serde_json::to_vec(value).unwrap(), 10));
    db.observe_ams(&device(), &state.status(10)).unwrap();
}
fn full(mask: &str) -> Value {
    json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"ams":{"tray_exist_bits":mask,"ams":[{"id":"0","tray":[
        {"id":"0","tray_type":"PLA","tray_color":"000000FF","tray_info_idx":"GFA01","tag_uid":"one"},
        {"id":"1","tray_type":"PLA","tray_color":"000000FF","tray_info_idx":"GFA01","tag_uid":"two"},
        {"id":"2","tray_type":"PLA","tray_color":"FFFFFFFF","tray_info_idx":"GFA01","tag_uid":"white"}
    ]}]}}})
}
fn order(db: &Database) -> Vec<AmsSlot> {
    db.resolve_slots("p", "black", "machine").unwrap()
}
fn seed(db: &Database) {
    db.connection().unwrap().execute_batch("INSERT INTO filament_products VALUES ('matte','PLA Matte','Bambu','PLA','GFA01');
        INSERT INTO filaments VALUES ('black','matte','黒','000000FF'),('white','matte','白','FFFFFFFF');
        INSERT INTO filament_settings VALUES ('s','matte','machine','base','{}');").unwrap();
}
#[test]
fn load_order_manual_priority_and_replacement_survive_polling_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
    seed(&db);
    let mut state = State::new(true);
    state.connected();
    observe(&db, &mut state, &full("5"));
    let first = order(&db)[0].clone();
    assert_eq!(first.slot_index, 0);
    observe(&db, &mut state, &full("7"));
    let slots = order(&db);
    assert_eq!(
        slots.iter().map(|s| s.slot_index).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(slots[0].load_order < slots[1].load_order);
    let reversed: Vec<_> = slots
        .iter()
        .rev()
        .map(|s| SlotRevision {
            id: s.id.clone(),
            revision: s.revision,
        })
        .collect();
    db.prioritize_slots("p", "black", "machine", &reversed)
        .unwrap();
    assert_eq!(order(&db)[0].slot_index, 1);
    assert!(
        db.prioritize_slots("p", "black", "machine", &reversed)
            .is_err()
    ); // stale edit
    observe(&db, &mut state, &full("7"));
    assert_eq!(order(&db)[0].slot_index, 1);
    assert!(
        db.resolve_slots("p", "black", "other-machine")
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.resolve_slots("p", "white", "machine").unwrap().len(), 1);
    let saved = order(&db);
    drop(db);
    let db = Database::open(dir.path(), || Ok(None)).unwrap();
    assert_eq!(order(&db)[0].id, saved[0].id);
    assert_eq!(order(&db)[0].load_order, saved[0].load_order);
    // An unknown inventory report is not proof of unloading. Its rows cannot be selected.
    observe(
        &db,
        &mut state,
        &json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}),
    );
    assert!(order(&db).is_empty());
    observe(&db, &mut state, &full("7"));
    assert_eq!(order(&db)[0].slot_index, 1);
    assert_eq!(order(&db)[0].load_order, saved[0].load_order);
    observe(&db, &mut state, &full("5"));
    observe(&db, &mut state, &full("7"));
    assert_eq!(order(&db)[0].slot_index, 0);
    assert!(order(&db)[1].load_order > saved[0].load_order);
    let mut changed = full("7");
    changed["print"]["ams"]["ams"][0]["tray"][0]["tag_uid"] = json!("replacement");
    observe(&db, &mut state, &changed);
    assert_eq!(order(&db)[0].slot_index, 1);
    let bad = vec![SlotRevision {
        id: first.id,
        revision: 0,
    }];
    assert!(db.prioritize_slots("p", "black", "machine", &bad).is_err());
}
#[test]
fn tagless_unobserved_exchange_needs_manual_priority_and_exact_product_color() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
    seed(&db);
    let mut report = full("7");
    for t in report["print"]["ams"]["ams"][0]["tray"]
        .as_array_mut()
        .unwrap()
    {
        t["tag_uid"] = json!("0000");
    }
    let mut state = State::new(true);
    state.connected();
    observe(&db, &mut state, &report.clone());
    assert!(order(&db).is_empty());
    let slots = db.ams_slots("p").unwrap();
    for s in &slots[..2] {
        db.map_slot("p", &s.id, s.revision, Some("black")).unwrap();
    }
    let before = order(&db);
    observe(&db, &mut state, &report);
    assert_eq!(order(&db)[0].load_order, before[0].load_order);
    db.connection().unwrap().execute_batch("INSERT INTO filament_products VALUES ('other','Different','Maker','PLA',NULL);INSERT INTO filaments VALUES ('other-black','other','黒','000000FF');INSERT INTO filament_settings VALUES ('os','other','machine','base','{}');").unwrap();
    let slot = &order(&db)[1];
    db.map_slot("p", &slot.id, slot.revision, Some("other-black"))
        .unwrap();
    assert_eq!(order(&db).len(), 1);
    assert_eq!(
        db.resolve_slots("p", "other-black", "machine").unwrap()[0].slot_index,
        1
    );
    let slot = &order(&db)[0];
    db.map_slot("p", &slot.id, slot.revision, None).unwrap();
    assert!(order(&db).is_empty());
}
#[test]
fn schema_four_unknown_load_order_initializes_by_slot_and_keeps_references() {
    let dir = tempfile::tempdir().unwrap();
    let c = Connection::open(dir.path().join("orca.sqlite3")).unwrap();
    c.execute_batch(include_str!("../tests/fixtures/schema-v3.sql"))
        .unwrap();
    c.execute_batch("INSERT INTO printers VALUES ('p','P1S','127.0.0.1','TEST','fixture','fixture','machine','process','textured_plate','unknown',8883,990,600,0,NULL);
        INSERT INTO filaments VALUES ('black','PLA','Bambu','PLA','000000FF','GFA01');
        INSERT INTO filament_settings VALUES ('s','black','machine','base','{}');
        INSERT INTO ams_slots(id,printer_id,ams_id,slot_index,filament_id,mapping_source,present) VALUES ('last','p',0,3,'black','manual',1),('first','p',0,0,'black','manual',1),('unknown','p',0,1,'black','manual',NULL);").unwrap();
    c.pragma_update(None, "foreign_keys", false).unwrap();
    crate::products::migrate(&c).unwrap();
    drop(c);
    let db = Database::open(dir.path(), || Ok(None)).unwrap();
    let ordered = order(&db);
    assert_eq!(
        ordered.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["first", "last"]
    );
    assert!(ordered[0].load_order < ordered[1].load_order);
    assert_eq!(
        db.connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        23
    );
}

#[test]
fn saved_observations_name_each_assignment_change_and_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
    seed(&db);
    let mut state = State::new(true);
    state.connected();
    let mut apply = |value: &Value| {
        assert!(state.apply(&serde_json::to_vec(value).unwrap(), 10));
        db.observe_ams(&device(), &state.status(10)).unwrap()
    };
    let first = apply(&full("7"));
    assert_eq!(first.saved, 3);
    let reasons: Vec<_> = first
        .changes
        .iter()
        .map(|c| (c.slot_index, c.reason, c.filament_after.as_deref()))
        .collect();
    assert_eq!(
        reasons,
        [
            (0, "new_slot", Some("black")),
            (1, "new_slot", Some("black")),
            (2, "new_slot", Some("white"))
        ]
    );
    let repeat = apply(&full("7"));
    assert!(
        repeat.changes.is_empty() && repeat.saved == 0,
        "an unchanged report saves nothing"
    );

    let unloaded = apply(&full("5"));
    assert_eq!(unloaded.changes.len(), 1);
    let change = &unloaded.changes[0];
    assert_eq!((change.slot_index, change.reason), (1, "identity_changed"));
    assert_eq!(
        (
            change.filament_before.as_deref(),
            change.filament_after.as_deref()
        ),
        (Some("black"), None)
    );
    assert_eq!(
        (change.source_before.as_str(), change.source_after.as_str()),
        ("automatic", "unassigned")
    );

    let absent = apply(
        &json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}),
    );
    assert_eq!(
        absent.changes.len(),
        2,
        "slots with an assignment are cleared"
    );
    assert!(absent.changes.iter().all(|c| c.reason == "unreported"));
    assert_eq!(absent.saved, 3, "the empty slot also loses its presence");
}

/// P1S power-up: slots are read one by one. Unread slots are reported with only their id,
/// while `tray_exist_bits` (feed sensors) is already final and so is absent from later diffs.
#[test]
fn power_up_reading_keeps_every_loaded_slot_once_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
    seed(&db);
    let mut state = State::new(true);
    state.connected();
    let black = |id: &str| json!({"id":id,"tray_type":"PLA","tray_color":"000000FF","tray_info_idx":"GFA01","tag_uid":format!("black-{id}")});
    let white = json!({"id":"3","tray_type":"PLA","tray_color":"FFFFFFFF","tray_info_idx":"GFA01","tag_uid":"white-3"});
    let unread = |id: &str| json!({"id":id});
    let diff = |trays: Value, bits: Value| {
        let mut ams = json!({"ams":[{"id":"0","tray":trays}]});
        ams.as_object_mut()
            .unwrap()
            .extend(bits.as_object().unwrap().clone());
        json!({"print":{"command":"push_status","msg":1,"ams":ams}})
    };
    observe(
        &db,
        &mut state,
        &json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,
        "ams":{"tray_exist_bits":"b","tray_reading_bits":"1","tray_read_done_bits":"0",
            "ams":[{"id":"0","tray":[unread("0"),unread("1"),unread("2"),unread("3")]}]}}}),
    );
    observe(
        &db,
        &mut state,
        &diff(
            json!([black("0"), unread("1"), unread("2"), unread("3")]),
            json!({"tray_reading_bits":"2","tray_read_done_bits":"1"}),
        ),
    );
    observe(
        &db,
        &mut state,
        &diff(
            json!([black("0"), black("1"), unread("2"), unread("3")]),
            json!({"tray_reading_bits":"8","tray_read_done_bits":"3"}),
        ),
    );
    observe(
        &db,
        &mut state,
        &diff(
            json!([black("0"), black("1"), unread("2"), white]),
            json!({"tray_reading_bits":"0","tray_read_done_bits":"b"}),
        ),
    );
    let slots = db.ams_slots("p").unwrap();
    let seen: Vec<_> = slots
        .iter()
        .map(|s| (s.slot_index, s.reported.present, s.filament_id.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            (0, Some(true), Some("black")),
            (1, Some(true), Some("black")),
            (2, Some(false), None),
            (3, Some(true), Some("white"))
        ]
    );
    assert_eq!(order(&db).len(), 2, "both black slots can print");
}

/// Reports 14308→14331 observed on 2026-09-30 after a restart with black in slots 1·2 and white in 4.
#[test]
#[allow(clippy::too_many_lines)] // One logged report sequence, kept in order.
fn logged_restart_sequence_settles_on_black_one_two_and_white_four() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), || Ok(Some(device()))).unwrap();
    seed(&db);
    let mut state = State::new(true);
    state.connected();
    let tray = |id: &str, color: &str| json!({"id":id,"tray_type":"PLA","tray_color":color,"tray_info_idx":"GFA01","tag_uid":format!("tag-{id}")});
    let unread = |id: &str| json!({"id":id});
    let black = "000000FF";
    let white = "FFFFFFFF";
    let loaded = json!([
        tray("0", black),
        tray("1", black),
        unread("2"),
        tray("3", white)
    ]);
    let report = |msg: u64, trays: Value, bits: Value| {
        let mut ams = json!({"ams":[{"id":"0","tray":trays}]});
        ams.as_object_mut()
            .unwrap()
            .extend(bits.as_object().unwrap().clone());
        json!({"print":{"command":"push_status","msg":msg,"gcode_state":"IDLE","print_error":0,"ams":ams}})
    };
    observe(
        &db,
        &mut state,
        &report(
            0,
            loaded.clone(),
            json!({"tray_exist_bits":"b","tray_read_done_bits":"b"}),
        ),
    );
    let assigned = |db: &Database| -> Vec<Option<String>> {
        db.ams_slots("p")
            .unwrap()
            .into_iter()
            .map(|s| s.filament_id)
            .collect()
    };
    let expected = vec![
        Some("black".to_owned()),
        Some("black".to_owned()),
        None,
        Some("white".to_owned()),
    ];
    assert_eq!(assigned(&db), expected, "before the restart");

    // 14308: full report, every tray id-only while reading starts.
    observe(
        &db,
        &mut state,
        &report(
            0,
            json!([unread("0"), unread("1"), unread("2"), unread("3")]),
            json!({"tray_exist_bits":"b","tray_reading_bits":"1","tray_read_done_bits":"0"}),
        ),
    );
    let present: Vec<_> = db
        .ams_slots("p")
        .unwrap()
        .iter()
        .map(|s| s.reported.present)
        .collect();
    assert_eq!(present, [Some(true), Some(true), Some(false), Some(true)]);
    // 14313, 14320, 14328: diffs without tray_exist_bits as each slot is read.
    observe(
        &db,
        &mut state,
        &report(
            1,
            json!([tray("0", black), unread("1"), unread("2"), unread("3")]),
            json!({"tray_reading_bits":"2","tray_read_done_bits":"1"}),
        ),
    );
    observe(
        &db,
        &mut state,
        &report(
            1,
            json!([tray("0", black), tray("1", black), unread("2"), unread("3")]),
            json!({"tray_reading_bits":"8","tray_read_done_bits":"3"}),
        ),
    );
    observe(
        &db,
        &mut state,
        &report(1, loaded.clone(), json!({"tray_read_done_bits":"b"})),
    );
    // 14331: reading finished.
    observe(
        &db,
        &mut state,
        &report(1, loaded, json!({"tray_reading_bits":"0"})),
    );
    assert_eq!(assigned(&db), expected);
    let present: Vec<_> = db
        .ams_slots("p")
        .unwrap()
        .iter()
        .map(|s| s.reported.present)
        .collect();
    assert_eq!(present, [Some(true), Some(true), Some(false), Some(true)]);
    assert_eq!(db.resolve_slots("p", "white", "machine").unwrap().len(), 1);

    // An explicit removal still clears the slot.
    observe(
        &db,
        &mut state,
        &report(
            1,
            json!([tray("0", black), tray("1", black), unread("2"), unread("3")]),
            json!({"tray_exist_bits":"3"}),
        ),
    );
    assert_eq!(assigned(&db)[3], None);
    assert_eq!(db.ams_slots("p").unwrap()[3].reported.present, Some(false));
}
