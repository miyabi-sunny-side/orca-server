use serde::{Deserialize, Serialize};
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

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Tray {
    pub id: u8,
    pub present: Option<bool>,
    pub material: Option<String>,
    pub color: Option<String>,
    pub remaining_percent: Option<u8>,
    pub profile_id: Option<String>,
    pub brand: Option<String>,
    pub tag_uid: Option<String>,
    pub temperature_min: Option<u16>,
    pub temperature_max: Option<u16>,
    pub last_seen_at: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
pub struct Unit {
    pub id: u8,
    pub humidity: Option<u8>,
    pub trays: Vec<Tray>,
}

#[derive(Clone, Default, Serialize)]
pub struct AmsStatus {
    pub detect_on_insert: Option<bool>,
    pub detect_on_power_up: Option<bool>,
    pub current_tray: Option<u16>,
    pub units: Vec<Unit>,
}

#[derive(Clone, Default, Serialize)]
pub struct AutoRefill {
    pub supported: Option<bool>,
    pub enabled: Option<bool>,
    pub groups: Option<Vec<u16>>,
}
impl AutoRefill {
    fn update(&mut self, report: &Map<String, Value>) {
        // BambuStudio DeviceManager: print_option / home_flag bit 10 / filam_bak.
        update(
            &mut self.supported,
            report,
            "support_filament_backup",
            Value::as_bool,
        );
        update(&mut self.enabled, report, "home_flag", |v| {
            number(v).map(|n| n & (1 << 10) != 0)
        });
        update(&mut self.groups, report, "filam_bak", |v| {
            v.as_array()?
                .iter()
                .map(|n| u16::try_from(n.as_u64()?).ok())
                .collect()
        });
    }
    pub fn peers(&self, tray: u8) -> Option<Vec<u8>> {
        let bit = 1u16.checked_shl(u32::from(tray))?;
        let groups = self.groups.as_ref()?;
        Some(
            (0..16)
                .filter(|&peer| {
                    peer != tray
                        && groups
                            .iter()
                            .any(|&mask| mask & bit != 0 && mask & (1 << peer) != 0)
                })
                .collect(),
        )
    }
}

#[derive(Serialize)]
pub struct Status {
    pub nozzle_diameter: Option<String>,
    pub nozzle_material: Option<String>,
    pub start: Option<crate::print_start::Attempt>,
    pub connection: &'static str,
    pub synchronized: bool,
    pub updated_at: Option<u64>,
    pub ready_to_print: bool,
    pub print: PrintStatus,
    pub ams: Option<AmsStatus>,
    pub auto_refill: AutoRefill,
}

impl Status {
    pub(crate) fn matches_attempt(&self, id: &str) -> bool {
        let name = self.print.name.as_deref().filter(|s| !s.is_empty());
        let file = self.print.file.as_deref().filter(|s| !s.is_empty());
        (name.is_some() || file.is_some())
            && name.is_none_or(|s| s == format!("orca-{id}"))
            && file.is_none_or(|s| s == format!("orca-{id}.gcode.3mf"))
    }

    pub(crate) fn check_stopped(&self, id: &str) -> crate::plates::Result<()> {
        use crate::plates::Error;
        if self.connection != "connected" || !self.synchronized {
            return Err(Error::Conflict(
                "Wait for a fresh synchronized printer report",
            ));
        }
        if self.print.error != Some(0) {
            return Err(Error::Conflict("Clear the printer error before recovery"));
        }
        match self.print.state.as_deref() {
            Some("FAILED") => {}
            Some("RUNNING" | "PREPARE") => {
                return Err(Error::Conflict("Printer is still printing or preparing"));
            }
            Some("PAUSE") => {
                return Err(Error::Conflict(
                    "Printer is paused; stop the print before recovery",
                ));
            }
            _ => {
                return Err(Error::Conflict(
                    "Stopped print is not confirmed by the current report",
                ));
            }
        }
        if !self.matches_attempt(id) {
            return Err(Error::Conflict(
                "Printer report does not match the recovery target",
            ));
        }
        Ok(())
    }
}

/// Repeated identical warnings are summarized at this interval.
pub const REPEAT_LOG_SECS: u64 = 600;

/// Connection history for diagnostics and throttled logs. Holds safe categories, never raw errors.
#[derive(Clone, Default, Serialize)]
pub struct Link {
    pub connected_at: Option<u64>,
    pub disconnected_at: Option<u64>,
    pub disconnect_reason: Option<&'static str>,
    /// Lost or failed connections since the last synchronized report.
    pub failures: u32,
    /// First synchronized report on the current connection.
    pub synchronized_at: Option<u64>,
    /// Latest full report on the current connection.
    pub snapshot_at: Option<u64>,
    #[serde(skip)]
    logged_at: Option<u64>,
}
impl Link {
    pub fn connected(&mut self, now: u64) {
        self.connected_at = Some(now);
        self.synchronized_at = None;
        self.snapshot_at = None;
    }
    /// Record a lost or failed connection; true when it should be logged.
    pub fn failed(&mut self, reason: &'static str, now: u64) -> bool {
        let log = self.failures == 0
            || self.disconnect_reason != Some(reason)
            || self
                .logged_at
                .is_none_or(|at| now.saturating_sub(at) >= REPEAT_LOG_SECS);
        self.failures += 1;
        self.disconnected_at = Some(now);
        self.disconnect_reason = Some(reason);
        if log {
            self.logged_at = Some(now);
        }
        log
    }
    /// The first synchronized report on this connection returns the failures it recovered from.
    pub fn synchronized(&mut self, now: u64) -> Option<u32> {
        if self.synchronized_at.is_some() {
            return None;
        }
        self.synchronized_at = Some(now);
        Some(std::mem::take(&mut self.failures))
    }
}

#[derive(Clone, Serialize)]
pub struct SaveFailure {
    pub at: u64,
    pub operation: &'static str,
    pub kind: &'static str,
}
/// Why memory and the database may differ: the latest failed save and whether it still fails.
#[derive(Clone, Default, Serialize)]
pub struct Saves {
    pub last_failure: Option<SaveFailure>,
    pub failing: bool,
    pub failures: u32,
}
impl Saves {
    /// Record a failed save; true when it is new or its operation or kind changed.
    pub fn failed(&mut self, operation: &'static str, kind: &'static str, now: u64) -> bool {
        let log = !self.failing
            || self
                .last_failure
                .as_ref()
                .is_none_or(|f| (f.operation, f.kind) != (operation, kind));
        self.last_failure = Some(SaveFailure {
            at: now,
            operation,
            kind,
        });
        self.failing = true;
        self.failures += 1;
        log
    }
    /// A successful save of the failing operation returns how many failures preceded it.
    pub fn saved(&mut self, operation: &str) -> Option<u32> {
        if !self.failing
            || self
                .last_failure
                .as_ref()
                .is_some_and(|f| f.operation != operation)
        {
            return None;
        }
        self.failing = false;
        Some(std::mem::take(&mut self.failures))
    }
}

pub struct State {
    /// Reports offered to `apply` since start, accepted or not.
    pub reports: u64,
    pub ignored: Option<(u64, &'static str)>,
    pub ignored_count: u64,
    /// Accepted reports that carried the `ams` key.
    pub ams_reports: u64,
    pub ams_report_at: Option<u64>,
    /// Latest full-report request: time, trigger and whether it was queued.
    pub snapshot_request: Option<(u64, &'static str, bool)>,
    /// Latest AMS exist/reading/read-done bits as reported (log projection).
    pub ams_bits: Option<Value>,
    pub link: Link,
    pub saves: Saves,
    /// Latest FTPS failure: time and the stage that failed.
    pub upload_failure: Option<(u64, &'static str)>,
    nozzle_diameter: Option<String>,
    nozzle_material: Option<String>,
    pub start: Option<crate::print_start::Attempt>,
    pub epoch: u64,
    connection: &'static str,
    synchronized: bool,
    updated_at: Option<u64>,
    print: PrintStatus,
    ams: Option<AmsStatus>,
    auto_refill: AutoRefill,
    tray_bits: Option<u16>,
}

impl State {
    pub fn new(configured: bool) -> Self {
        Self {
            reports: 0,
            ignored: None,
            ignored_count: 0,
            ams_reports: 0,
            ams_report_at: None,
            snapshot_request: None,
            ams_bits: None,
            link: Link::default(),
            saves: Saves::default(),
            upload_failure: None,
            nozzle_diameter: None,
            nozzle_material: None,
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
            auto_refill: AutoRefill::default(),
            tray_bits: None,
        }
    }
    pub fn connected(&mut self) {
        let start = self.start.take();
        let epoch = self.epoch.wrapping_add(1);
        let mut link = std::mem::take(&mut self.link);
        link.snapshot_at = None;
        link.synchronized_at = None;
        let saves = std::mem::take(&mut self.saves);
        let upload_failure = self.upload_failure.take();
        let counts = (
            self.reports,
            self.ignored,
            self.ignored_count,
            self.ams_reports,
            self.ams_report_at,
            self.snapshot_request,
            self.ams_bits.take(),
        );
        *self = Self::new(true);
        (
            self.reports,
            self.ignored,
            self.ignored_count,
            self.ams_reports,
            self.ams_report_at,
            self.snapshot_request,
            self.ams_bits,
        ) = counts;
        self.link = link;
        self.saves = saves;
        self.upload_failure = upload_failure;
        self.start = start;
        self.epoch = epoch;
        self.connection = "connected";
    }
    pub fn disconnected(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(start) = &mut self.start {
            start.disconnected();
            start.lost_connection();
        }
        self.connection = "disconnected";
        self.synchronized = false;
    }
    /// Apply one report; every report is counted and the last ignore reason is kept.
    pub fn apply(&mut self, payload: &[u8], now: u64) -> bool {
        self.reports += 1;
        match self.accept(payload, now) {
            Ok(()) => true,
            Err(reason) => {
                self.ignored = Some((now, reason));
                self.ignored_count += 1;
                false
            }
        }
    }
    fn accept(&mut self, payload: &[u8], now: u64) -> std::result::Result<(), &'static str> {
        if self.connection != "connected" {
            return Err("disconnected");
        }
        let Ok(value) = serde_json::from_slice::<Value>(payload) else {
            self.synchronized = false;
            return Err("invalid_json");
        };
        let Some(report) = value.get("print").and_then(Value::as_object) else {
            return Err("not_print");
        };
        if report.get("command").and_then(Value::as_str) != Some("push_status") {
            return Err("not_push_status");
        }
        let full = match report.get("msg") {
            None => true,
            Some(value) if value.as_u64() == Some(0) => true,
            Some(value) if value.as_u64() == Some(1) => false,
            _ => {
                self.synchronized = false;
                return Err("invalid_msg");
            }
        };
        if !self.fresh(now) {
            self.synchronized = false;
        }
        if full {
            self.nozzle_diameter = None;
            self.nozzle_material = None;
            self.print = PrintStatus::default();
            self.ams = None;
            self.auto_refill = AutoRefill::default();
            self.tray_bits = None;
            self.synchronized = true;
            self.link.snapshot_at = Some(now);
        } else if !self.synchronized {
            return Err("unsynchronized_diff");
        }
        if report.contains_key("ams") {
            self.ams_reports += 1;
            self.ams_report_at = Some(now);
        }
        update(&mut self.nozzle_diameter, report, "nozzle_diameter", string);
        self.auto_refill.update(report);
        update(&mut self.nozzle_material, report, "nozzle_type", string);
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
                self.update_ams(ams, now);
            } else {
                self.ams = None;
                self.tray_bits = None;
            }
        }
        self.synchronized = self.print.state.is_some() && self.print.error.is_some();
        self.updated_at = Some(now);
        // An empty target must be reported, not merely omitted, to count as a snapshot.
        if full
            && ["subtask_name", "gcode_file"]
                .iter()
                .all(|k| report.contains_key(*k))
        {
            let status = self.status(now);
            if let Some(start) = &mut self.start {
                start.resynchronized(&status);
            }
        }
        Ok(())
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
            nozzle_diameter: self.nozzle_diameter.clone(),
            nozzle_material: self.nozzle_material.clone(),
            start: self.start.clone(),
            connection,
            synchronized,
            updated_at: self.updated_at,
            ready_to_print: synchronized
                && matches!(self.print.state.as_deref(), Some("IDLE" | "FINISH"))
                && self.print.error == Some(0),
            print: self.print.clone(),
            ams: self.ams.clone(),
            auto_refill: self.auto_refill.clone(),
        }
    }

    fn update_ams(&mut self, report: &Map<String, Value>, now: u64) {
        let ams = self.ams.get_or_insert_with(AmsStatus::default);
        update(
            &mut ams.detect_on_insert,
            report,
            "insert_flag",
            Value::as_bool,
        );
        update(
            &mut ams.detect_on_power_up,
            report,
            "power_on_flag",
            Value::as_bool,
        );
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
                let Some(id) = raw
                    .get("id")
                    .and_then(|v| number(v).and_then(|n| u8::try_from(n).ok()))
                else {
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
                                *mask &= !tray_bit(unit.id, id).unwrap_or(0);
                            }
                            *tray = Tray {
                                id,
                                present: Some(false),
                                ..Tray::default()
                            };
                        }
                        tray.update(raw, now);
                    }
                    unit.trays.sort_by_key(|tray| tray.id);
                }
            }
        }
        if let Some(value) = report.get("ams_exist_bits") {
            if let Some(mask) = bits(value) {
                ams.units.retain(|unit| {
                    1u16.checked_shl(u32::from(unit.id))
                        .is_none_or(|bit| mask & bit != 0)
                });
            } else {
                ams.units.clear();
            }
        }
        ams.update_presence(self.tray_bits, now);
        ams.units.sort_by_key(|unit| unit.id);
    }
}

impl AmsStatus {
    fn update_presence(&mut self, bits: Option<u16>, now: u64) {
        for unit in &mut self.units {
            for tray in &mut unit.trays {
                if let Some((mask, bit)) = bits.zip(tray_bit(unit.id, tray.id)) {
                    let present = mask & bit != 0;
                    if tray.present != Some(present) {
                        tray.last_seen_at = Some(now);
                    }
                    tray.present = Some(present);
                    if tray.present == Some(false) {
                        *tray = Tray {
                            id: tray.id,
                            present: Some(false),
                            last_seen_at: tray.last_seen_at,
                            ..Tray::default()
                        };
                    }
                }
            }
        }
    }
}

impl Tray {
    fn update(&mut self, raw: &Map<String, Value>, now: u64) {
        update(&mut self.material, raw, "tray_type", string);
        update(&mut self.profile_id, raw, "tray_info_idx", string);
        update(&mut self.brand, raw, "tray_sub_brands", string);
        update(&mut self.tag_uid, raw, "tag_uid", |v| {
            string(v).filter(|s| s.chars().any(|c| c != '0'))
        });
        update(
            &mut self.temperature_min,
            raw,
            "nozzle_temp_min",
            temperature,
        );
        update(
            &mut self.temperature_max,
            raw,
            "nozzle_temp_max",
            temperature,
        );
        update(&mut self.color, raw, "tray_color", |v| {
            v.as_str()
                .filter(|s| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_owned)
        });
        update(&mut self.remaining_percent, raw, "remain", percent);
        self.last_seen_at = Some(now);
    }
}

/// A short hex/decimal field as reported, or `"invalid"`; never arbitrary text.
fn short_code(value: &Value) -> Value {
    match value {
        Value::Number(n) => Value::Number(n.clone()),
        Value::String(s)
            if (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Value::String(s.clone())
        }
        _ => Value::from("invalid"),
    }
}
fn colour(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
}
fn tray_summary(raw: &Value) -> Value {
    let Some(tray) = raw.as_object() else {
        return Value::from("invalid");
    };
    let id = tray.get("id").and_then(number);
    if tray.len() == 1 && id.is_some() {
        return serde_json::json!({"id":id,"kind":"id_only"});
    }
    let material = tray.get("tray_type").and_then(string);
    serde_json::json!({
        "id": id,
        "kind": if material.is_some() { "loaded" } else { "empty" },
        "material": material,
        "color": tray.get("tray_color").and_then(colour),
        "profile": tray.get("tray_info_idx").and_then(string),
        "remain": tray.get("remain").and_then(number),
    })
}

/// What a report said about the AMS, for logs: allow-listed fields and tray shapes only.
/// `None` for reports that neither carry `ams` nor replace it as a full report.
pub fn ams_summary(value: &Value) -> Option<Value> {
    let report = value.get("print")?.as_object()?;
    if report.get("command").and_then(Value::as_str) != Some("push_status") {
        return None;
    }
    let msg = report.get("msg");
    let full = msg.is_none_or(|m| m.as_u64() == Some(0));
    let ams = report.get("ams");
    if ams.is_none() && !full {
        return None;
    }
    let mut summary = serde_json::json!({
        "msg": msg.map(|m| if m.is_u64() { m.clone() } else { Value::from("invalid") }),
        "full": full,
    });
    let Some(ams) = ams else {
        summary["ams"] = Value::from("absent");
        return Some(summary);
    };
    let Some(ams) = ams.as_object() else {
        summary["ams"] = Value::from("invalid");
        return Some(summary);
    };
    summary["ams"] = Value::from("object");
    for (key, name) in [
        ("tray_exist_bits", "exist_bits"),
        ("tray_reading_bits", "reading_bits"),
        ("tray_read_done_bits", "read_done_bits"),
        ("ams_exist_bits", "ams_exist_bits"),
        ("tray_now", "tray_now"),
    ] {
        if let Some(value) = ams.get(key) {
            summary[name] = short_code(value);
        }
    }
    if let Some(units) = ams.get("ams") {
        summary["units"] = units.as_array().map_or(Value::from("invalid"), |units| {
            units
                .iter()
                .map(|unit| {
                    let trays = match unit.get("tray") {
                        None => Value::from("omitted"),
                        Some(trays) => trays.as_array().map_or(Value::from("invalid"), |t| {
                            t.iter().map(tray_summary).collect()
                        }),
                    };
                    serde_json::json!({"id": unit.get("id").and_then(number), "trays": trays})
                })
                .collect()
        });
    }
    Some(summary)
}

fn slot_view(tray: Option<&Tray>) -> Value {
    let state = match tray.map(|t| t.present) {
        None => "absent",
        Some(None) => "unknown",
        Some(Some(true)) => "present",
        Some(Some(false)) => "empty",
    };
    serde_json::json!({
        "state": state,
        "material": tray.and_then(|t| t.material.clone()),
        "color": tray.and_then(|t| t.color.clone()),
        "profile": tray.and_then(|t| t.profile_id.clone()),
    })
}

/// Slots whose presence, material, colour, profile or tag identity changed between two states.
/// Identity changes are shown as a flag; tag values are never included.
pub fn ams_changes(before: Option<&AmsStatus>, after: Option<&AmsStatus>) -> Vec<Value> {
    let trays = |ams: Option<&AmsStatus>| -> std::collections::BTreeMap<(u8, u8), Tray> {
        ams.into_iter()
            .flat_map(|a| &a.units)
            .flat_map(|u| u.trays.iter().map(move |t| ((u.id, t.id), t.clone())))
            .collect()
    };
    let (old, new) = (trays(before), trays(after));
    let keys: std::collections::BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    keys.into_iter()
        .filter_map(|key| {
            let (b, a) = (old.get(&key), new.get(&key));
            let (bv, av) = (slot_view(b), slot_view(a));
            let identity_changed = b.and_then(|t| t.tag_uid.as_ref()) != a.and_then(|t| t.tag_uid.as_ref());
            (bv != av || identity_changed).then(|| {
                serde_json::json!({"unit": key.0, "tray": key.1, "before": bv, "after": av, "identity_changed": identity_changed})
            })
        })
        .collect()
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
fn tray_bit(unit: u8, tray: u8) -> Option<u16> {
    1u16.checked_shl(u32::from(unit) * 4 + u32::from(tray))
}
fn temperature(value: &Value) -> Option<u16> {
    number(value)
        .filter(|v| (1..=500).contains(v))
        .and_then(|v| u16::try_from(v).ok())
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

    #[test]
    fn native_refill_reports_distinguish_support_state_and_device_groups() {
        let mut state = State::new(true);
        state.connected();
        state.apply(
            br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}"#,
            1,
        );
        let r = state.status(1).auto_refill;
        assert_eq!(r.supported, None);
        assert_eq!(r.enabled, None);
        assert_eq!(r.groups, None);
        state.apply(br#"{"print":{"command":"push_status","msg":1,"support_filament_backup":false,"home_flag":0,"filam_bak":[]}}"#,2);
        let r = state.status(2).auto_refill;
        assert_eq!(r.supported, Some(false));
        assert_eq!(r.enabled, Some(false));
        assert_eq!(r.peers(0), Some(vec![]));
        state.apply(br#"{"print":{"command":"push_status","msg":1,"support_filament_backup":true,"home_flag":1024,"filam_bak":[3,12]}}"#,3);
        let r = state.status(3).auto_refill;
        assert_eq!(r.supported, Some(true));
        assert_eq!(r.enabled, Some(true));
        assert_eq!(r.peers(0), Some(vec![1]));
        assert_eq!(r.peers(2), Some(vec![3]));
        state.apply(
            br#"{"print":{"command":"push_status","msg":1,"mc_percent":5}}"#,
            4,
        );
        assert_eq!(state.status(4).auto_refill.peers(1), Some(vec![0]));
        state.apply(
            br#"{"print":{"command":"push_status","msg":1,"home_flag":"bad","filam_bak":["bad"]}}"#,
            5,
        );
        assert_eq!(state.status(5).auto_refill.enabled, None);
        assert_eq!(state.status(5).auto_refill.groups, None);
        state.connected();
        assert_eq!(state.status(6).auto_refill.supported, None);
    }

    #[test]
    fn ams_identity_and_temperatures_merge_without_inventing_unknown_values() {
        let mut state = State::new(true);
        state.connected();
        assert!(push(
            &mut state,
            json!({"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,
            "ams":{"insert_flag":true,"power_on_flag":true,"tray_exist_bits":"3","ams":[{"id":"0","tray":[
                {"id":"0","tray_type":"PLA","tray_sub_brands":"PLA Matte","tray_info_idx":"GFA01","tray_color":"000000FF","tag_uid":"0123456789AB","nozzle_temp_min":"190","nozzle_temp_max":"230","remain":-1},
                {"id":"1","tray_type":"PETG","tray_info_idx":"PTEST001","tray_color":"FFFFFFFF","tag_uid":"000000000000","nozzle_temp_min":"220","nozzle_temp_max":"260","remain":60}
            ]}]}}),
            10
        ));
        let ams = state.status(10).ams.unwrap();
        assert_eq!(ams.detect_on_insert, Some(true));
        assert_eq!(ams.detect_on_power_up, Some(true));
        let tray = &ams.units[0].trays[0];
        assert_eq!(tray.profile_id.as_deref(), Some("GFA01"));
        assert_eq!(tray.brand.as_deref(), Some("PLA Matte"));
        assert_eq!(tray.tag_uid.as_deref(), Some("0123456789AB"));
        assert_eq!(tray.temperature_min, Some(190));
        assert_eq!(tray.temperature_max, Some(230));
        assert_eq!(tray.remaining_percent, None);
        assert_eq!(ams.units[0].trays[1].tag_uid, None);
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"ams":{"ams":[{"id":"0","tray":[{"id":"0","remain":50}]}]}}),
            11,
        );
        let tray = &state.status(11).ams.unwrap().units[0].trays[0];
        assert_eq!(tray.tag_uid.as_deref(), Some("0123456789AB"));
        assert_eq!(tray.temperature_max, Some(230));
        assert_eq!(tray.remaining_percent, Some(50));
        push(
            &mut state,
            json!({"command":"push_status","msg":1,"ams":{"ams":[{"id":"0","tray":[{"id":"0"}]}]}}),
            12,
        );
        let tray = &state.status(12).ams.unwrap().units[0].trays[0];
        assert_eq!(tray.present, Some(false));
        assert!(
            tray.tag_uid.is_none() && tray.profile_id.is_none() && tray.temperature_max.is_none()
        );
        state.disconnected();
        assert!(!state.status(13).synchronized);
        state.connected();
        assert!(state.status(14).ams.is_none());
    }

    #[test]
    fn ams_unit_identity_is_not_a_global_four_slot_index() {
        let mut state = State::new(true);
        state.connected();
        push(
            &mut state,
            json!({"command":"push_status","gcode_state":"IDLE","print_error":0,
            "ams":{"ams":[{"id":"0","tray":[{"id":"0","tray_type":"PLA"}]},{"id":"128","tray":[{"id":"0","tray_type":"PETG"}]}]}}),
            1,
        );
        let ams = state.status(1).ams.unwrap();
        assert_eq!(ams.units.len(), 2);
        assert_eq!(ams.units[1].id, 128);
        assert_eq!(ams.units[1].trays[0].material.as_deref(), Some("PETG"));
        assert_eq!(ams.units[1].trays[0].present, None);
    }

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

#[cfg(test)]
mod resync_tests {
    use super::*;
    use crate::print_start::{Attempt, Phase};

    const IDLE: &[u8] = br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,"subtask_name":"","gcode_file":""}}"#;

    fn uncertain() -> State {
        let mut state = State::new(true);
        state.connected();
        let mut attempt = Attempt::new("plate".into(), "job".into(), 0, "PLA".into());
        attempt.sent(1);
        state.start = Some(attempt);
        state.apply(IDLE, 2);
        state.start.as_mut().unwrap().tick(10, 5);
        state
    }

    fn phase(state: &State) -> Phase {
        state.start.as_ref().unwrap().phase
    }

    #[test]
    fn only_a_full_idle_snapshot_on_a_new_connection_resolves_an_uncertain_start() {
        let mut state = uncertain();
        assert_eq!(phase(&state), Phase::Unknown);
        state.apply(IDLE, 11);
        assert_eq!(phase(&state), Phase::Unknown, "same connection");
        state.disconnected();
        state.connected();
        state.apply(
            br#"{"print":{"command":"push_status","msg":1,"gcode_state":"IDLE","print_error":0,"subtask_name":"","gcode_file":""}}"#,
            12,
        );
        assert_eq!(phase(&state), Phase::Unknown, "a diff is not a snapshot");
        state.apply(
            br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}"#,
            13,
        );
        assert_eq!(
            phase(&state),
            Phase::Unknown,
            "missing identity fields are not an empty target"
        );
        state.apply(IDLE, 14);
        assert_eq!(phase(&state), Phase::Resolved);
    }

    #[test]
    fn a_restart_counts_as_a_lost_connection() {
        let mut state = State::new(true);
        let mut attempt = Attempt::new("plate".into(), "job".into(), 0, "PLA".into());
        attempt.sent(1);
        attempt.disconnected();
        attempt.lost_connection();
        let mut restored: Attempt = serde_json::from_value(serde_json::json!(attempt)).unwrap();
        restored.message = None;
        state.start = Some(restored);
        state.connected();
        state.apply(IDLE, 2);
        assert_eq!(phase(&state), Phase::Resolved);
    }
}

#[cfg(test)]
mod configuration_tests {
    use super::*;
    #[test]
    fn observed_nozzle_changes_remain_distinct_from_unreported_settings() {
        let mut state = State::new(true);
        state.connected();
        assert!(state.status(1).nozzle_diameter.is_none());
        state.apply(br#"{"print":{"command":"push_status","gcode_state":"IDLE","print_error":0,"nozzle_diameter":"0.2","nozzle_type":"stainless_steel"}}"#,1);
        assert_eq!(state.status(1).nozzle_diameter.as_deref(), Some("0.2"));
        assert_eq!(
            state.status(1).nozzle_material.as_deref(),
            Some("stainless_steel")
        );
        state.apply(
            br#"{"print":{"command":"push_status","msg":1,"nozzle_diameter":"0.4"}}"#,
            2,
        );
        assert_eq!(state.status(2).nozzle_diameter.as_deref(), Some("0.4"));
        state.disconnected();
        state.connected();
        assert!(state.status(3).nozzle_diameter.is_none());
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn repeated_failures_are_logged_first_on_change_and_every_ten_minutes() {
        let mut link = Link::default();
        assert!(link.failed("refused", 100));
        assert!(!link.failed("refused", 105));
        assert!(!link.failed("refused", 699));
        assert!(link.failed("refused", 700), "periodic summary");
        assert!(link.failed("auth", 705), "reason changed");
        assert_eq!(link.failures, 5);
        assert_eq!(
            (link.disconnected_at, link.disconnect_reason),
            (Some(705), Some("auth"))
        );
        link.connected(710);
        assert_eq!(
            link.synchronized(711),
            Some(5),
            "recovered after 5 failures"
        );
        assert_eq!(link.synchronized(712), None, "only the first snapshot");
        assert_eq!(link.synchronized_at, Some(711));
        assert!(link.failed("closed", 800), "a new outage is logged again");
        link.connected(805);
        assert_eq!(link.synchronized_at, None);
        assert_eq!(link.synchronized(806), Some(1));
    }

    #[test]
    fn save_failures_are_logged_once_per_kind_and_recovery_is_reported() {
        let mut saves = Saves::default();
        assert_eq!(saves.saved("attempt"), None);
        assert!(saves.failed("attempt", "database", 10));
        assert!(!saves.failed("attempt", "database", 11));
        assert!(saves.failed("attempt", "conflict", 12));
        assert_eq!(
            saves.saved("ams"),
            None,
            "another operation does not clear it"
        );
        assert!(saves.failing);
        assert_eq!(saves.saved("attempt"), Some(3));
        assert!(!saves.failing);
        let last = saves.last_failure.as_ref().unwrap();
        assert_eq!(
            (last.at, last.operation, last.kind),
            (12, "attempt", "conflict")
        );
    }

    #[test]
    fn connection_history_survives_reconnects_and_tracks_full_snapshots() {
        let mut state = State::new(true);
        state.link.failed("timeout", 1);
        state.connected();
        state.link.connected(2);
        assert_eq!(state.link.disconnect_reason, Some("timeout"));
        state.apply(
            br#"{"print":{"command":"push_status","msg":1,"gcode_state":"IDLE","print_error":0}}"#,
            3,
        );
        assert_eq!(state.link.snapshot_at, None, "a diff is not a snapshot");
        state.apply(
            br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}"#,
            4,
        );
        assert_eq!(state.link.snapshot_at, Some(4));
        state.connected();
        assert_eq!(state.link.snapshot_at, None);
        assert_eq!(state.link.connected_at, Some(2));
    }
}

#[cfg(test)]
mod ams_trace_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_summary_keeps_allowed_fields_and_tray_shapes_without_identifiers() {
        let report = json!({"print":{"command":"push_status","msg":0,"ams":{
            "tray_exist_bits":"b","tray_reading_bits":"0","tray_read_done_bits":"b","ams_exist_bits":"1","tray_now":"255",
            "ams":[{"id":"0","humidity":"3","tray":[
                {"id":"0","tray_type":"PLA","tray_color":"000000FF","tray_info_idx":"GFA01","remain":80,"tag_uid":"SECRET-TAG","tray_uuid":"SECRET-UUID","tray_sub_brands":"PLA Matte"},
                {"id":"1"},
                {"id":"2","tray_type":"","tray_color":"00000000"},
                "broken"
            ]},{"id":"1"}]}}});
        let s = ams_summary(&report).unwrap();
        assert_eq!(s["full"], true);
        assert_eq!(s["msg"], 0);
        assert_eq!(s["ams"], "object");
        assert_eq!(
            (&s["exist_bits"], &s["reading_bits"], &s["read_done_bits"]),
            (&json!("b"), &json!("0"), &json!("b"))
        );
        let trays = &s["units"][0]["trays"];
        assert_eq!(
            trays[0],
            json!({"id":0,"kind":"loaded","material":"PLA","color":"000000FF","profile":"GFA01","remain":80})
        );
        assert_eq!(trays[1], json!({"id":1,"kind":"id_only"}));
        assert_eq!(trays[2]["kind"], "empty");
        assert_eq!(trays[3], "invalid");
        assert_eq!(s["units"][1]["trays"], "omitted");
        let text = s.to_string();
        assert!(!text.contains("SECRET") && !text.contains("Matte"));
    }

    #[test]
    fn report_summary_marks_absent_or_invalid_ams_and_skips_unrelated_diffs() {
        let full = |ams: Option<Value>| {
            let mut r = json!({"print":{"command":"push_status","gcode_state":"IDLE"}});
            if let Some(ams) = ams {
                r["print"]["ams"] = ams;
            }
            r
        };
        let s = ams_summary(&full(None)).unwrap();
        assert_eq!(
            (&s["full"], &s["msg"], &s["ams"]),
            (&json!(true), &Value::Null, &json!("absent"))
        );
        assert_eq!(
            ams_summary(&full(Some(json!("x")))).unwrap()["ams"],
            "invalid"
        );
        let bad = ams_summary(&full(Some(json!({"tray_exist_bits":"zz","ams":"x"})))).unwrap();
        assert_eq!(
            (&bad["exist_bits"], &bad["units"]),
            (&json!("invalid"), &json!("invalid"))
        );
        let diff = json!({"print":{"command":"push_status","msg":1,"mc_percent":5}});
        assert!(ams_summary(&diff).is_none());
        let odd = json!({"print":{"command":"push_status","msg":"two","ams":{}}});
        assert_eq!(ams_summary(&odd).unwrap()["msg"], "invalid");
        assert!(ams_summary(&json!({"print":{"command":"project_file","ams":{}}})).is_none());
    }

    fn state_with(report: &Value) -> Option<AmsStatus> {
        let mut state = State::new(true);
        state.connected();
        state.apply(report.to_string().as_bytes(), 1);
        state.status(1).ams
    }

    #[test]
    fn applied_changes_report_state_and_identity_changes_but_not_remaining() {
        let report = |trays: Value, bits: &str| {
            json!({"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0,
                "ams":{"tray_exist_bits":bits,"ams":[{"id":"0","tray":trays}]}}})
        };
        let before = state_with(&report(
            json!([{"id":"0","tray_type":"PLA","tray_color":"000000FF","tag_uid":"AAAA","remain":80},{"id":"1","tray_type":"PLA","tray_color":"FFFFFFFF"}]),
            "3",
        ));
        let same = state_with(&report(
            json!([{"id":"0","tray_type":"PLA","tray_color":"000000FF","tag_uid":"AAAA","remain":60},{"id":"1","tray_type":"PLA","tray_color":"FFFFFFFF"}]),
            "3",
        ));
        assert!(ams_changes(before.as_ref(), same.as_ref()).is_empty());
        let after = state_with(&report(
            json!([{"id":"0","tray_type":"PLA","tray_color":"000000FF","tag_uid":"BBBB"},{"id":"1"}]),
            "1",
        ));
        let changes = ams_changes(before.as_ref(), after.as_ref());
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["tray"], 0);
        assert_eq!(changes[0]["identity_changed"], true);
        assert_eq!(changes[1]["before"]["state"], "present");
        assert_eq!(
            changes[1]["after"],
            json!({"state":"empty","material":null,"color":null,"profile":null})
        );
        let cleared = ams_changes(before.as_ref(), None);
        assert_eq!(cleared.len(), 2);
        assert_eq!(cleared[0]["after"]["state"], "absent");
        assert!(!serde_json::to_string(&changes).unwrap().contains("BBBB"));
    }

    #[test]
    fn ignored_reports_keep_their_reason_and_all_reports_are_counted() {
        let mut state = State::new(true);
        assert!(!state.apply(b"{}", 1));
        assert_eq!(state.ignored, Some((1, "disconnected")));
        state.connected();
        for (payload, reason) in [
            (&b"broken"[..], "invalid_json"),
            (br#"{"info":{}}"#, "not_print"),
            (
                br#"{"print":{"command":"project_file"}}"#,
                "not_push_status",
            ),
            (
                br#"{"print":{"command":"push_status","msg":"x"}}"#,
                "invalid_msg",
            ),
            (
                br#"{"print":{"command":"push_status","msg":1,"ams":{}}}"#,
                "unsynchronized_diff",
            ),
        ] {
            assert!(!state.apply(payload, 2));
            assert_eq!(state.ignored, Some((2, reason)));
        }
        assert!(state.apply(
            br#"{"print":{"command":"push_status","msg":0,"gcode_state":"IDLE","print_error":0}}"#,
            3
        ));
        assert_eq!(state.reports, 7);
        assert_eq!(state.ignored_count, 6);
    }
}
