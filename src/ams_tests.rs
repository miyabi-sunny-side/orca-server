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
        11
    );
}
