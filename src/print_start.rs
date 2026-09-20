use crate::{
    plates::{Error, Result},
    printer_state::Status,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Uploading,
    AwaitingConfirmation,
    Accepted,
    Printing,
    Finished,
    Rejected,
    UploadFailed,
    NotSent,
    Unknown,
    Resolved,
}

#[derive(Clone, Serialize)]
pub struct Attempt {
    pub id: String,
    pub plate_id: String,
    pub revision: String,
    pub ams_slot: u8,
    pub phase: Phase,
    pub message: Option<&'static str>,
    #[serde(skip)]
    pub material: String,
    #[serde(skip)]
    sequence: String,
    #[serde(skip)]
    sent_at: Option<u64>,
    #[serde(skip)]
    observed_printing: bool,
}

pub fn check_nozzle(status: &Status, diameter: &str, material: &str) -> Result<()> {
    if status
        .nozzle_diameter
        .as_deref()
        .is_some_and(|v| v != diameter)
        || (material != "unknown"
            && status
                .nozzle_material
                .as_deref()
                .is_some_and(|v| v != material))
    {
        return Err(Error::Conflict(
            "Printer reports a different nozzle; check the installed nozzle and registry",
        ));
    }
    Ok(())
}

pub fn check_ready(status: &Status, slot: u8, material: &str) -> Result<()> {
    if !status.ready_to_print {
        return Err(Error::Conflict("Printer is not ready"));
    }
    let tray = status
        .ams
        .as_ref()
        .and_then(|ams| ams.units.iter().find(|unit| unit.id == slot / 4))
        .and_then(|unit| unit.trays.iter().find(|tray| tray.id == slot % 4));
    if slot >= 16
        || !tray.is_some_and(|tray| {
            tray.present == Some(true) && tray.material.as_deref() == Some(material)
        })
    {
        return Err(Error::Conflict(
            "Selected AMS tray is absent, unknown or has a different material",
        ));
    }
    Ok(())
}

impl Attempt {
    pub fn new(plate_id: String, revision: String, ams_slot: u8, material: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            plate_id,
            revision,
            ams_slot,
            phase: Phase::Uploading,
            message: None,
            material,
            sequence: (uuid::Uuid::new_v4().as_u128() % 2_000_000_000 + 1).to_string(),
            sent_at: None,
            observed_printing: false,
        }
    }
    pub fn name(&self) -> String {
        format!("orca-{}", self.id)
    }
    pub fn filename(&self) -> String {
        format!("{}.gcode.3mf", self.name())
    }
    pub fn command(&self) -> Value {
        json!({"print":{
            "command":"project_file", "sequence_id":self.sequence,
            "param":"Metadata/plate_1.gcode", "url":format!("ftp:///{}", self.filename()),
            "file":self.filename(), "subtask_name":self.name(),
            "project_id":"0", "profile_id":"0", "task_id":"0", "subtask_id":"0", "md5":"",
            "use_ams":true, "ams_mapping":[self.ams_slot], "bed_type":"auto",
            "timelapse":false, "bed_leveling":true, "flow_cali":false,
            "vibration_cali":true, "layer_inspect":false
        }})
    }
    pub fn blocks_start(&self) -> bool {
        matches!(
            self.phase,
            Phase::Uploading
                | Phase::AwaitingConfirmation
                | Phase::Accepted
                | Phase::Printing
                | Phase::Unknown
        )
    }
    pub fn sent(&mut self, now: u64) {
        self.phase = Phase::AwaitingConfirmation;
        self.sent_at = Some(now);
    }
    pub fn fail(&mut self, phase: Phase, message: &'static str) {
        self.phase = phase;
        self.message = Some(message);
    }
    pub fn disconnected(&mut self) {
        if matches!(
            self.phase,
            Phase::AwaitingConfirmation | Phase::Accepted | Phase::Printing
        ) {
            self.fail(
                Phase::Unknown,
                "Printer disconnected; check the printer before another start",
            );
        }
    }
    pub fn tick(&mut self, now: u64, timeout: u64) {
        if matches!(self.phase, Phase::AwaitingConfirmation | Phase::Accepted)
            && self
                .sent_at
                .is_some_and(|sent| now.checked_sub(sent).is_none_or(|age| age >= timeout))
        {
            self.fail(
                Phase::Unknown,
                "Start confirmation timed out; the command will not be resent",
            );
        }
    }
    pub fn resolve(&mut self, status: &Status) -> Result<()> {
        if self.phase != Phase::Unknown || !status.ready_to_print {
            return Err(Error::Conflict(
                "Only an unknown start with a ready printer can be resolved",
            ));
        }
        self.phase = Phase::Resolved;
        self.message = None;
        Ok(())
    }
    pub fn observe(&mut self, value: &Value, status: &Status) {
        if !matches!(
            self.phase,
            Phase::AwaitingConfirmation | Phase::Accepted | Phase::Printing | Phase::Unknown
        ) {
            return;
        }
        let report = &value["print"];
        if report["command"] == "project_file" && !self.observed_printing {
            let sequence = report["sequence_id"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| report["sequence_id"].as_u64().map(|v| v.to_string()));
            if sequence.as_deref() == Some(&self.sequence) {
                match report["result"].as_str() {
                    Some("success" | "SUCCESS") if self.phase != Phase::Unknown => {
                        self.phase = Phase::Accepted;
                    }
                    Some("fail" | "FAIL") => {
                        self.fail(Phase::Rejected, "Printer rejected the start request");
                    }
                    _ => {}
                }
            }
        }
        if report["command"] != "push_status" || !status.synchronized {
            return;
        }
        let matches = status.print.name.as_deref() == Some(&self.name())
            || status.print.file.as_deref() == Some(&self.filename());
        if !matches {
            return;
        }
        if status.print.error != Some(0) {
            self.fail(
                Phase::Unknown,
                "Printer reported an error; inspect the printer",
            );
            return;
        }
        match status.print.state.as_deref() {
            Some("RUNNING") => {
                self.observed_printing = true;
                self.phase = Phase::Printing;
                self.message = None;
            }
            Some("PREPARE") if self.phase != Phase::Printing => self.phase = Phase::Accepted,
            Some("FINISH") if self.observed_printing => self.phase = Phase::Finished,
            Some("FAILED" | "PAUSE") => {
                self.fail(Phase::Unknown, "Print stopped; inspect the printer");
            }
            Some("IDLE") if self.observed_printing => self.fail(
                Phase::Unknown,
                "Print ended without a completion report; inspect the printer",
            ),
            _ => {}
        }
    }
}

#[cfg(test)]
fn material(bytes: &[u8]) -> Result<String> {
    material_for(bytes, crate::profiles::PRINTER)
}

pub fn material_for(bytes: &[u8], machine: &str) -> Result<String> {
    use std::io::{Cursor, Read};
    let invalid = || {
        Error::Invalid(
            "Print must contain one matching printer plate and one material at filament index 1",
        )
    };
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive
        .file_names()
        .filter(|n| {
            n.starts_with("Metadata/plate_")
                && std::path::Path::new(n)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"))
        })
        .count()
        != 1
        || archive
            .by_name("Metadata/plate_1.gcode")
            .map_err(|_| invalid())?
            .size()
            == 0
    {
        return Err(invalid());
    }
    let mut read = |name: &str| -> Result<String> {
        let mut text = String::new();
        archive
            .by_name(name)
            .map_err(|_| invalid())?
            .take(1_048_577)
            .read_to_string(&mut text)
            .map_err(|_| invalid())?;
        if text.len() > 1_048_576 {
            return Err(invalid());
        }
        Ok(text)
    };
    let settings: Value =
        serde_json::from_str(&read("Metadata/project_settings.config")?).map_err(|_| invalid())?;
    let xml = read("Metadata/slice_info.config")?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| invalid())?;
    let plates: Vec<_> = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("plate"))
        .collect();
    if plates.len() != 1
        || !plates[0].children().any(|n| {
            n.has_tag_name("metadata")
                && n.attribute("key") == Some("index")
                && n.attribute("value") == Some("1")
        })
    {
        return Err(invalid());
    }
    let filaments: Vec<_> = plates[0]
        .children()
        .filter(|n| n.has_tag_name("filament"))
        .collect();
    if filaments.len() != 1
        || filaments[0].attribute("id") != Some("1")
        || settings["printer_settings_id"] != machine
    {
        return Err(invalid());
    }
    let material = filaments[0]
        .attribute("type")
        .filter(|s| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
        .ok_or_else(invalid)?;
    if settings["filament_type"] != json!([material]) {
        return Err(invalid());
    }
    Ok(material.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer_state::State;
    use serde_json::json;

    fn ready() -> Status {
        let mut state = State::new(true);
        state.connected();
        state.apply(include_bytes!("../tests/fixtures/p1_status.json"), 10);
        state.status(10)
    }

    #[test]
    fn reported_nozzle_must_match_but_absent_readings_are_not_invented() {
        let mut status = ready();
        assert!(check_nozzle(&status, "0.4", "unknown").is_ok());
        status.nozzle_diameter = Some("0.2".into());
        assert!(check_nozzle(&status, "0.4", "unknown").is_err());
        assert!(check_nozzle(&status, "0.2", "unknown").is_ok());
        status.nozzle_material = Some("stainless_steel".into());
        assert!(check_nozzle(&status, "0.2", "hardened_steel").is_err());
        assert!(check_nozzle(&status, "0.2", "stainless_steel").is_ok());
    }

    #[test]
    fn start_requires_fresh_idle_printer_and_present_selected_tray() {
        let mut status = ready();
        assert!(check_ready(&status, 0, "PLA").is_ok());
        for slot in [1, 4, 16, 255] {
            assert!(check_ready(&status, slot, "PLA").is_err());
        }
        assert!(check_ready(&status, 0, "PETG").is_err());
        status.ready_to_print = false;
        assert!(check_ready(&status, 0, "PLA").is_err());
    }

    #[test]
    fn single_material_maps_to_the_selected_physical_tray() {
        let a = Attempt::new("plate".into(), "revision".into(), 3, "PLA".into());
        let command = a.command();
        assert_eq!(command["print"]["ams_mapping"], json!([3]));
        assert_eq!(command["print"]["param"], "Metadata/plate_1.gcode");
        assert_eq!(command["print"]["url"], format!("ftp:///{}", a.filename()));
        assert_eq!(command["print"]["use_ams"], true);
    }

    #[test]
    fn publish_and_ack_are_not_printing_and_unrelated_reports_cannot_confirm() {
        let mut a = Attempt::new("plate".into(), "revision".into(), 0, "PLA".into());
        a.sent(10);
        assert_eq!(a.phase, Phase::AwaitingConfirmation);
        a.observe(
            &json!({"print":{"command":"project_file","sequence_id":"wrong","result":"success"}}),
            &ready(),
        );
        assert_eq!(a.phase, Phase::AwaitingConfirmation);
        a.observe(&json!({"print":{"command":"project_file","sequence_id":a.sequence,"result":"success"}}), &ready());
        assert_eq!(a.phase, Phase::Accepted);
        let mut status = ready();
        status.print.state = Some("RUNNING".into());
        status.print.name = Some("someone-else".into());
        a.observe(&json!({"print":{"command":"push_status"}}), &status);
        assert_eq!(a.phase, Phase::Accepted);
        status.print.name = Some(a.name());
        a.observe(&json!({"print":{"command":"push_status"}}), &status);
        assert_eq!(a.phase, Phase::Printing);
        a.disconnected();
        a.observe(
            &json!({"print":{"command":"project_file","sequence_id":a.sequence,"result":"fail"}}),
            &status,
        );
        assert_eq!(a.phase, Phase::Unknown);
        status.print.state = Some("FINISH".into());
        a.observe(&json!({"print":{"command":"push_status"}}), &status);
        assert_eq!(a.phase, Phase::Finished);
        assert!(!a.blocks_start());
    }

    #[test]
    fn an_interrupted_print_never_becomes_successful_completion() {
        let mut status = ready();
        let mut a = Attempt::new("plate".into(), "revision".into(), 0, "PLA".into());
        a.sent(10);
        status.print.name = Some(a.name());
        status.print.state = Some("RUNNING".into());
        a.observe(&json!({"print":{"command":"push_status"}}), &status);
        for state in ["PAUSE", "IDLE", "FAILED"] {
            let mut stopped = a.clone();
            status.print.state = Some(state.into());
            stopped.observe(&json!({"print":{"command":"push_status"}}), &status);
            assert_eq!(stopped.phase, Phase::Unknown);
            assert!(stopped.blocks_start());
        }
    }

    #[test]
    fn loss_or_timeout_remains_unknown_until_explicit_resolution_or_matching_report() {
        let mut a = Attempt::new("plate".into(), "revision".into(), 0, "PLA".into());
        a.sent(10);
        a.tick(15, 5);
        assert_eq!(a.phase, Phase::Unknown);
        assert!(a.blocks_start());
        assert!(a.resolve(&ready()).is_ok());
        assert_eq!(a.phase, Phase::Resolved);
        let mut a = Attempt::new("p".into(), "r".into(), 0, "PLA".into());
        a.disconnected();
        assert_eq!(a.phase, Phase::Uploading);
        a.sent(1);
        a.disconnected();
        assert_eq!(a.phase, Phase::Unknown);
        let mut busy = ready();
        busy.ready_to_print = false;
        assert!(a.resolve(&busy).is_err());
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use std::io::{Cursor, Write};
    fn archive(filaments: &str, types: &Value) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in [
            (
                "Metadata/slice_info.config",
                format!(
                    "<config><plate><metadata key=\"index\" value=\"1\"/>{filaments}</plate></config>"
                ),
            ),
            (
                "Metadata/project_settings.config",
                json!({"printer_settings_id":crate::profiles::PRINTER,"filament_type":types})
                    .to_string(),
            ),
            ("Metadata/plate_1.gcode", "G28".into()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn refuses_artifacts_for_another_machine_or_nozzle() {
        let bytes = archive("<filament id=\"1\" type=\"PLA\"/>", &json!(["PLA"]));
        assert!(material_for(&bytes, "Bambu Lab A1 mini 0.2 nozzle").is_err());
        assert_eq!(
            material_for(&bytes, crate::profiles::PRINTER).unwrap(),
            "PLA"
        );
    }
    #[test]
    fn reads_one_material_and_rejects_ambiguous_or_invalid_archives() {
        assert_eq!(
            material(&archive(
                "<filament id=\"1\" type=\"PLA\"/>",
                &json!(["PLA"])
            ))
            .unwrap(),
            "PLA"
        );
        for (xml, types) in [
            ("<filament id=\"2\" type=\"PLA\"/>", json!(["PLA"])),
            (
                "<filament id=\"1\"/><filament id=\"2\"/>",
                json!(["PLA", "PETG"]),
            ),
            ("<filament id=\"1\" type=\"PETG\"/>", json!(["PLA"])),
        ] {
            assert!(material(&archive(xml, &types)).is_err());
        }
        assert!(material(b"broken zip").is_err());
    }
}
