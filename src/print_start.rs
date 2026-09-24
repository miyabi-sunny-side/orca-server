use crate::{
    plates::{Error, Result},
    printer_state::Status,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct MaterialSlot {
    pub ams_slot: u8,
    pub material: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub plate_id: String,
    pub job_id: String,
    pub ams_slot: u8,
    pub phase: Phase,
    pub message: Option<String>,
    pub material: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<MaterialSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<MaterialSlot>,
    /// UTC seconds at the first matching FINISH observation, absent on old attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    sequence: String,
    sent_at: Option<u64>,
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

impl Attempt {
    pub fn new(plate_id: String, job_id: String, ams_slot: u8, material: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            plate_id,
            job_id,
            ams_slot,
            phase: Phase::Uploading,
            message: None,
            material,
            interface: None,
            secondary: None,
            completed_at: None,
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
        let mapping: Vec<_> = std::iter::once(self.ams_slot)
            .chain(self.secondary.iter().map(|i| i.ams_slot))
            .chain(self.interface.iter().map(|i| i.ams_slot))
            .collect();
        json!({"print":{
            "command":"project_file", "sequence_id":self.sequence,
            "param":"Metadata/plate_1.gcode", "url":format!("ftp:///{}", self.filename()),
            "file":self.filename(), "subtask_name":self.name(),
            "project_id":"0", "profile_id":"0", "task_id":"0", "subtask_id":"0", "md5":"",
            "use_ams":true, "ams_mapping":mapping, "bed_type":"auto",
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
    pub(crate) fn was_sent(&self) -> bool {
        self.sent_at.is_some()
    }
    pub fn fail(&mut self, phase: Phase, message: &'static str) {
        self.phase = phase;
        self.message = Some(message.into());
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
            Some("FINISH") if self.observed_printing => {
                self.phase = Phase::Finished;
                self.completed_at = status.updated_at;
            }
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

#[cfg(test)]
fn material_for(bytes: &[u8], machine: &str) -> Result<String> {
    let profile =
        json!({"name":"material 0","filament_type":["PLA"],"filament_colour":["#FFFFFF"]})
            .as_object()
            .expect("fixture")
            .clone();
    materials_for(bytes, machine, &[profile]).map(|mut materials| materials.remove(0))
}

pub(crate) fn materials_for(
    bytes: &[u8],
    machine: &str,
    profiles: &[serde_json::Map<String, Value>],
) -> Result<Vec<String>> {
    use std::io::{Cursor, Read};
    let invalid = || {
        Error::Invalid(
            "Print must contain one matching printer plate and valid planned material indices",
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
    crate::support::check_materials(&settings, profiles)?;
    let xml = read("Metadata/slice_info.config")?;
    if settings["printer_settings_id"] != machine {
        return Err(invalid());
    }
    check_used_materials(&xml, profiles)?;
    Ok(profiles
        .iter()
        .map(|p| {
            p["filament_type"][0]
                .as_str()
                .expect("validated material type")
                .to_owned()
        })
        .collect())
}

fn check_used_materials(xml: &str, profiles: &[serde_json::Map<String, Value>]) -> Result<()> {
    let invalid = || Error::Invalid("Slice contains invalid or unplanned material indices");
    let doc = roxmltree::Document::parse(xml).map_err(|_| invalid())?;
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
    if filaments.is_empty() || filaments.len() > profiles.len() {
        return Err(invalid());
    }
    let mut used = std::collections::BTreeSet::new();
    for filament in filaments {
        let id = filament.attribute("id").ok_or_else(invalid)?;
        let index = id
            .parse::<usize>()
            .ok()
            .filter(|i| (1..=profiles.len()).contains(i) && i.to_string() == id)
            .ok_or_else(invalid)?;
        if !used.insert(index) {
            return Err(invalid());
        }
        let material = filament
            .attribute("type")
            .filter(|s| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
            .ok_or_else(invalid)?;
        if profiles[index - 1]["filament_type"][0] != material {
            return Err(invalid());
        }
        if let Some(colour) = filament
            .attribute("color")
            .filter(|_| profiles.len() > 1 || profiles[index - 1].contains_key("filament_colour"))
        {
            if !profiles[index - 1]["filament_colour"][0]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(colour))
            {
                return Err(invalid());
            }
        } else if profiles.len() > 1 {
            return Err(invalid());
        }
    }
    if !used.contains(&1) {
        return Err(invalid());
    }
    Ok(())
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
    fn single_material_maps_to_the_selected_physical_tray() {
        let a = Attempt::new("plate".into(), "revision".into(), 3, "PLA".into());
        let command = a.command();
        assert_eq!(command["print"]["ams_mapping"], json!([3]));
        assert_eq!(command["print"]["param"], "Metadata/plate_1.gcode");
        assert_eq!(command["print"]["url"], format!("ftp:///{}", a.filename()));
        assert_eq!(command["print"]["use_ams"], true);
    }

    #[test]
    fn interface_mapping_is_ordered_and_old_attempts_keep_one_slot() {
        let original = Attempt::new("p".into(), "j".into(), 3, "PLA".into());
        let mut raw = json!(original);
        raw["interface"] = json!({"ams_slot":1,"material":"PETG"});
        let mut attempt: Attempt = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(attempt.command()["print"]["ams_mapping"], json!([3, 1]));
        attempt.sent(10);
        attempt.tick(20, 5);
        assert_eq!(attempt.phase, Phase::Unknown);
        assert!(attempt.blocks_start());
        raw.as_object_mut().unwrap().remove("interface");
        let old: Attempt = serde_json::from_value(raw).unwrap();
        assert_eq!(old.command()["print"]["ams_mapping"], json!([3]));
    }

    #[test]
    fn role_and_interface_slots_preserve_three_material_order() {
        let mut attempt = Attempt::new("plate".into(), "job".into(), 3, "PLA".into());
        attempt.secondary = Some(MaterialSlot {
            ams_slot: 0,
            material: "PLA".into(),
        });
        attempt.interface = Some(MaterialSlot {
            ams_slot: 2,
            material: "PETG".into(),
        });
        assert_eq!(attempt.command()["print"]["ams_mapping"], json!([3, 0, 2]));
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
    fn completion_time_is_the_first_matching_finish_observation() {
        let mut status = ready();
        let mut attempt = Attempt::new("plate".into(), "job".into(), 0, "PLA".into());
        attempt.sent(10);
        status.print.name = Some(attempt.name());
        status.print.state = Some("FINISH".into());
        status.updated_at = Some(20);
        let report = json!({"print":{"command":"push_status"}});
        attempt.observe(&report, &status);
        assert!(
            json!(attempt)["completed_at"].is_null(),
            "FINISH without real printing is not completion"
        );
        status.print.state = Some("RUNNING".into());
        attempt.observe(&report, &status);
        status.print.state = Some("FINISH".into());
        status.updated_at = Some(30);
        attempt.observe(&report, &status);
        assert_eq!(json!(attempt)["completed_at"], 30);
        let mut restored: Attempt = serde_json::from_value(json!(attempt)).unwrap();
        status.updated_at = Some(90);
        restored.observe(&report, &status);
        assert_eq!(
            json!(restored)["completed_at"],
            30,
            "repeated finish after restart must keep the first time"
        );
        let mut old = json!(restored);
        old.as_object_mut().unwrap().remove("completed_at");
        let mut legacy: Attempt = serde_json::from_value(old).unwrap();
        legacy.observe(&report, &status);
        assert!(
            json!(legacy)["completed_at"].is_null(),
            "old finished attempts have no known completion time"
        );
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
    #[test]
    fn legacy_single_material_without_frozen_colour_keeps_type_and_profile_checks() {
        let bytes = archive(
            r##"<filament id="1" type="PLA" color="#00AE42"/>"##,
            &json!(["PLA"]),
        );
        let profile = json!({"name":"material 0","filament_type":["PLA"]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            materials_for(
                &bytes,
                crate::profiles::PRINTER,
                std::slice::from_ref(&profile)
            )
            .unwrap(),
            vec!["PLA"]
        );
        let mut wrong = profile.clone();
        wrong.insert("name".into(), json!("wrong"));
        assert!(materials_for(&bytes, crate::profiles::PRINTER, &[wrong]).is_err());
        let two = archive(
            r##"<filament id="1" type="PLA" color="#00AE42"/><filament id="2" type="PLA" color="#00AE42"/>"##,
            &json!(["PLA", "PLA"]),
        );
        let mut second = profile.clone();
        second.insert("name".into(), json!("material 1"));
        assert!(materials_for(&two, crate::profiles::PRINTER, &[profile, second]).is_err());
    }
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
                json!({"printer_settings_id":crate::profiles::PRINTER,"filament_type":types,"filament_settings_id":types.as_array().unwrap().iter().enumerate().map(|(i,_)| format!("material {i}")).collect::<Vec<_>>(),"filament_colour":types.as_array().unwrap().iter().map(|_| "#FFFFFF").collect::<Vec<_>>()})
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
    fn two_material_archives_validate_all_indices_and_allow_an_unused_interface() {
        let profiles = [
            json!({"name":"material 0","filament_type":["PLA"],"filament_colour":["#FFFFFF"]}),
            json!({"name":"material 1","filament_type":["PETG"],"filament_colour":["#FFFFFF"]}),
        ]
        .map(|v| v.as_object().unwrap().clone());
        for xml in [
            r##"<filament id="1" type="PLA" color="#FFFFFF"/><filament id="2" type="PETG" color="#FFFFFF"/>"##,
            r##"<filament id="1" type="PLA" color="#FFFFFF"/>"##,
        ] {
            let bytes = archive(xml, &json!(["PLA", "PETG"]));
            assert_eq!(
                materials_for(&bytes, crate::profiles::PRINTER, &profiles).unwrap(),
                vec!["PLA", "PETG"]
            );
            assert!(
                materials_for(
                    &bytes,
                    crate::profiles::PRINTER,
                    &[profiles[1].clone(), profiles[0].clone()]
                )
                .is_err()
            );
        }
        for xml in [
            r##"<filament id="2" type="PETG" color="#FFFFFF"/>"##,
            r##"<filament id="1" type="PLA" color="#FFFFFF"/><filament id="1" type="PLA" color="#FFFFFF"/>"##,
            r##"<filament id="1" type="PLA" color="#FFFFFF"/><filament id="3" type="PETG" color="#FFFFFF"/>"##,
            r##"<filament id="1" type="PLA" color="#FFFFFF"/><filament id="2" type="PLA" color="#FFFFFF"/>"##,
            r##"<filament id="1" type="PLA" color="#FFFFFF"/><filament id="2" type="PETG" color="#000000"/>"##,
        ] {
            assert!(
                materials_for(
                    &archive(xml, &json!(["PLA", "PETG"])),
                    crate::profiles::PRINTER,
                    &profiles
                )
                .is_err()
            );
        }
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
