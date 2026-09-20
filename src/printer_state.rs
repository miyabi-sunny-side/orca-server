use serde::Serialize;
use serde_json::{Map, Value};

pub const MAX_AGE_SECS: u64 = 60;

#[derive(Clone, Default, Serialize)]
pub struct PrintStatus {
    pub state: Option<String>,
    pub percent: Option<u8>,
    pub remaining_minutes: Option<u32>,
    pub error: Option<u64>,
    pub job_id: Option<String>,
    pub file: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct Tray {
    pub id: u8,
    pub present: Option<bool>,
    pub material: Option<String>,
    pub color: Option<String>,
    pub remaining_percent: Option<u8>,
}

#[derive(Clone, Default, Serialize)]
pub struct Unit {
    pub id: u8,
    pub humidity: Option<u8>,
    pub trays: Vec<Tray>,
}

#[derive(Clone, Default, Serialize)]
pub struct AmsStatus {
    pub current_tray: Option<u16>,
    pub units: Vec<Unit>,
}

#[derive(Serialize)]
pub struct Status {
    pub start: Option<crate::print_start::Attempt>,
    pub connection: &'static str,
    pub synchronized: bool,
    pub updated_at: Option<u64>,
    pub ready_to_print: bool,
    pub print: PrintStatus,
    pub ams: Option<AmsStatus>,
}

pub struct State {
    pub start: Option<crate::print_start::Attempt>,
    pub epoch: u64,
    connection: &'static str,
    synchronized: bool,
    updated_at: Option<u64>,
    print: PrintStatus,
    ams: Option<AmsStatus>,
    tray_bits: Option<u16>,
}

impl State {
    pub fn new(configured: bool) -> Self {
        Self {
            start: None,
            epoch: 0,
            connection: if configured {
                "connecting"
            } else {
                "unconfigured"
            },
            synchronized: false,
            updated_at: None,
            print: PrintStatus::default(),
            ams: None,
            tray_bits: None,
        }
    }
    pub fn connected(&mut self) {
        let start = self.start.take();
        let epoch = self.epoch.wrapping_add(1);
        *self = Self::new(true);
        self.start = start;
        self.epoch = epoch;
        self.connection = "connected";
    }
    pub fn disconnected(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(start) = &mut self.start {
            start.disconnected();
        }
        self.connection = "disconnected";
        self.synchronized = false;
    }
    pub fn apply(&mut self, payload: &[u8], now: u64) -> bool {
        if self.connection != "connected" {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(payload) else {
            self.synchronized = false;
            return false;
        };
        let Some(report) = value.get("print").and_then(Value::as_object) else {
            return false;
        };
        if report.get("command").and_then(Value::as_str) != Some("push_status") {
            return false;
        }
        let full = match report.get("msg") {
            None => true,
            Some(value) if value.as_u64() == Some(0) => true,
            Some(value) if value.as_u64() == Some(1) => false,
            _ => {
                self.synchronized = false;
                return false;
            }
        };
        if !self.fresh(now) {
            self.synchronized = false;
        }
        if full {
            self.print = PrintStatus::default();
            self.ams = None;
            self.tray_bits = None;
            self.synchronized = true;
        } else if !self.synchronized {
            return false;
        }
        update(&mut self.print.state, report, "gcode_state", string);
        update(&mut self.print.percent, report, "mc_percent", percent);
        update(
            &mut self.print.remaining_minutes,
            report,
            "mc_remaining_time",
            |v| number(v).and_then(|v| u32::try_from(v).ok()),
        );
        update(&mut self.print.error, report, "print_error", number);
        update(&mut self.print.job_id, report, "subtask_id", string);
        update(&mut self.print.file, report, "gcode_file", string);
        update(&mut self.print.name, report, "subtask_name", string);
        if let Some(ams) = report.get("ams") {
            if let Some(ams) = ams.as_object() {
                self.update_ams(ams);
            } else {
                self.ams = None;
                self.tray_bits = None;
            }
        }
        self.synchronized = self.print.state.is_some() && self.print.error.is_some();
        self.updated_at = Some(now);
        true
    }

    fn fresh(&self, now: u64) -> bool {
        self.updated_at
            .and_then(|time| now.checked_sub(time))
            .is_some_and(|age| age < MAX_AGE_SECS)
    }

    pub fn status(&self, now: u64) -> Status {
        let fresh = self.fresh(now);
        let connection = match self.connection {
            "connected" if self.updated_at.is_some() && !fresh => "stale",
            "connected" if !self.synchronized => "synchronizing",
            other => other,
        };
        let synchronized = connection == "connected" && self.synchronized && fresh;
        Status {
            start: self.start.clone(),
            connection,
            synchronized,
            updated_at: self.updated_at,
            ready_to_print: synchronized
                && matches!(self.print.state.as_deref(), Some("IDLE" | "FINISH"))
                && self.print.error == Some(0),
            print: self.print.clone(),
            ams: self.ams.clone(),
        }
    }

    fn update_ams(&mut self, report: &Map<String, Value>) {
        let ams = self.ams.get_or_insert_with(AmsStatus::default);
        update(&mut ams.current_tray, report, "tray_now", |v| {
            number(v)
                .filter(|&v| v < 16 || v == 254 || v == 255)
                .and_then(|v| u16::try_from(v).ok())
        });
        update(&mut self.tray_bits, report, "tray_exist_bits", bits);
        if let Some(units) = report.get("ams").and_then(Value::as_array) {
            if units.is_empty() {
                ams.units.clear();
            }
            for raw in units {
                let Some(id) = raw.get("id").and_then(slot) else {
                    continue;
                };
                let Some(raw) = raw.as_object() else {
                    continue;
                };
                if !ams.units.iter().any(|unit| unit.id == id) {
                    ams.units.push(Unit {
                        id,
                        ..Unit::default()
                    });
                }
                let unit = ams.units.iter_mut().find(|unit| unit.id == id).unwrap();
                update(&mut unit.humidity, raw, "humidity", |v| {
                    number(v)
                        .filter(|&v| v <= 5)
                        .and_then(|v| u8::try_from(v).ok())
                });
                if let Some(trays) = raw.get("tray").and_then(Value::as_array) {
                    if trays.is_empty() {
                        unit.trays.clear();
                    }
                    for raw in trays {
                        let Some(id) = raw.get("id").and_then(slot) else {
                            continue;
                        };
                        let Some(raw) = raw.as_object() else {
                            continue;
                        };
                        if !unit.trays.iter().any(|tray| tray.id == id) {
                            unit.trays.push(Tray {
                                id,
                                ..Tray::default()
                            });
                        }
                        let tray = unit.trays.iter_mut().find(|tray| tray.id == id).unwrap();
                        if raw.len() == 1 {
                            if !report.contains_key("tray_exist_bits")
                                && let Some(mask) = &mut self.tray_bits
                            {
                                *mask &= !(1 << (unit.id * 4 + id));
                            }
                            *tray = Tray {
                                id,
                                present: Some(false),
                                ..Tray::default()
                            };
                        }
                        update(&mut tray.material, raw, "tray_type", string);
                        update(&mut tray.color, raw, "tray_color", |v| {
                            v.as_str()
                                .filter(|s| {
                                    s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit())
                                })
                                .map(str::to_owned)
                        });
                        update(&mut tray.remaining_percent, raw, "remain", percent);
                    }
                    unit.trays.sort_by_key(|tray| tray.id);
                }
            }
        }
        if let Some(value) = report.get("ams_exist_bits") {
            if let Some(mask) = bits(value) {
                ams.units.retain(|unit| mask & (1 << unit.id) != 0);
            } else {
                ams.units.clear();
            }
        }
        for unit in &mut ams.units {
            for tray in &mut unit.trays {
                if let Some(mask) = self.tray_bits {
                    tray.present = Some(mask & (1 << (unit.id * 4 + tray.id)) != 0);
                    if tray.present == Some(false) {
                        *tray = Tray {
                            id: tray.id,
                            present: Some(false),
                            ..Tray::default()
                        };
                    }
                }
            }
        }
        ams.units.sort_by_key(|unit| unit.id);
    }
}

fn update<T>(
    target: &mut Option<T>,
    report: &Map<String, Value>,
    key: &str,
    parse: impl FnOnce(&Value) -> Option<T>,
) {
    if let Some(value) = report.get(key) {
        *target = parse(value);
    }
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn percent(value: &Value) -> Option<u8> {
    number(value)
        .filter(|&v| v <= 100)
        .and_then(|v| u8::try_from(v).ok())
}
fn slot(value: &Value) -> Option<u8> {
    number(value)
        .filter(|&v| v < 4)
        .and_then(|v| u8::try_from(v).ok())
}
fn bits(value: &Value) -> Option<u16> {
    u16::from_str_radix(value.as_str()?, 16).ok()
}
fn string(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn push(state: &mut State, value: Value, now: u64) -> bool {
        state.apply(
            &serde_json::to_vec(&Value::Object(Map::from_iter([(
                "print".to_owned(),
                value,
            )])))
            .unwrap(),
            now,
        )
    }

    fn full() -> Value {
        serde_json::from_str::<Value>(include_str!("../tests/fixtures/p1_status.json")).unwrap()["print"].clone()
    }

    #[test]
    fn full_sync_and_partial_print_ams_updates_do_not_invent_idle_or_forget_fields() {
        let mut state = State::new(true);
        assert_eq!(state.status(100).connection, "connecting");
        state.connected();
        assert_eq!(state.status(100).connection, "synchronizing");
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"gcode_state":"IDLE","print_error":0}),
            100,
        );
        assert!(!state.status(100).ready_to_print);
        assert!(push(&mut state, full(), 101));
        assert!(state.status(101).ready_to_print);
        let trays = &state.status(101).ams.unwrap().units[0].trays;
        assert_eq!(trays[0].present, Some(true));
        assert_eq!(trays[2].present, Some(false));
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"ams":{"ams":[{"id":"0","tray":[{"id":"1"}]}]}}),
            101,
        );
        assert_eq!(
            state.status(101).ams.unwrap().units[0].trays[1].present,
            Some(false)
        );
        push(&mut state, full(), 101);
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"gcode_state":"RUNNING","mc_percent":"25","mc_remaining_time":"10",
            "subtask_id":"job-1", "gcode_file":"plate.gcode.3mf",
            "ams":{"tray_now":"0","ams":[{"id":"0","tray":[{"id":"0","remain":60}]}]}}),
            102,
        );
        let status = state.status(102);
        assert!(!status.ready_to_print);
        assert_eq!(status.print.state.as_deref(), Some("RUNNING"));
        assert_eq!(status.print.percent, Some(25));
        assert_eq!(status.print.remaining_minutes, Some(10));
        let ams = status.ams.unwrap();
        assert_eq!(ams.current_tray, Some(0));
        assert_eq!(ams.units[0].trays[0].material.as_deref(), Some("PLA"));
        assert_eq!(ams.units[0].trays[0].remaining_percent, Some(60));
        assert_eq!(ams.units[0].trays[1].material.as_deref(), Some("PETG"));
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"ams":{"tray_exist_bits":"2","ams":[{"id":"0","tray":[{"id":"0"}]}]}}),
            103,
        );
        let tray = &state.status(103).ams.unwrap().units[0].trays[0];
        assert_eq!(tray.present, Some(false));
        assert_eq!(tray.material, None);
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"ams":{"ams_exist_bits":"0"}}),
            104,
        );
        assert!(state.status(104).ams.unwrap().units.is_empty());
    }

    #[test]
    fn stale_disconnect_reconnect_and_bad_reports_never_authorize_printing() {
        let mut state = State::new(true);
        state.connected();
        push(&mut state, full(), 100);
        assert!(state.status(100).ready_to_print);
        assert_eq!(state.status(160).connection, "stale");
        assert!(!state.status(160).ready_to_print);
        assert!(!state.status(99).ready_to_print);
        state.disconnected();
        assert_eq!(state.status(101).connection, "disconnected");
        assert!(!state.status(101).ready_to_print);
        state.connected();
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"mc_percent":35}),
            102,
        );
        assert_eq!(state.status(102).print.state, None);
        assert!(!state.status(102).synchronized);
        push(&mut state, full(), 103);
        for raw in ["PAUSE", "PREPARE", "RUNNING", "FAILED", "FUTURE_STATE"] {
            push(
                &mut state,
                json!({"command":"push_status","msg":1,"gcode_state":raw}),
                104,
            );
            assert!(!state.status(104).ready_to_print);
        }
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"gcode_state":"FINISH","print_error":42}),
            105,
        );
        assert!(!state.status(105).ready_to_print);
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"print_error":0}),
            106,
        );
        assert!(state.status(106).ready_to_print);
        push(
            &mut state,
            json!({"command":"project_file","result":"success","gcode_state":"RUNNING","access_code":"test-secret"}),
            107,
        );
        assert_eq!(state.status(107).print.state.as_deref(), Some("FINISH"));
        assert!(
            !serde_json::to_string(&state.status(107))
                .unwrap()
                .contains("test-secret")
        );
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"print_error":"invalid","mc_percent":500}),
            108,
        );
        assert_eq!(state.status(108).print.error, None);
        assert_eq!(state.status(108).print.percent, None);
        assert!(!state.status(108).synchronized);
        assert!(!state.status(108).ready_to_print);
        assert!(!state.apply(b"broken", 109));
        assert!(!state.status(109).synchronized);
        let mut lan = full();
        lan.as_object_mut().unwrap().remove("msg");
        push(&mut state, lan, 110);
        assert!(state.status(110).ready_to_print);
        push(
            &mut state,
            json!({"command":"push_status","msg":0,"gcode_state":"IDLE"}),
            111,
        );
        assert!(state.status(111).ams.is_none());
        assert!(!state.status(111).ready_to_print);
    }
}
