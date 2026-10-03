//! Bambu LAN MQTT controls as `BambuStudio` sends them, and the replies that answer them.
//!
//! Sources: `BambuStudio` 77b9dd9 `DeviceManager.cpp` (`command_*`), `DeviceCore/DevLampCtrl.cpp`,
//! `DeviceCore/DevFan.cpp`, `DeviceCore/DevPrintOptions.cpp`.

use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One operator control. Values are validated before a message is built.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Pause,
    Resume,
    Stop,
    /// 1 silent, 2 standard, 3 sport, 4 ludicrous.
    Speed {
        level: u8,
    },
    Light {
        on: bool,
    },
    NozzleTemperature {
        celsius: u16,
    },
    BedTemperature {
        celsius: u16,
    },
    Fan {
        fan: Fan,
        percent: u8,
    },
    /// Load an AMS tray (0–15) or the external spool (254), heating the nozzle to `celsius`.
    Load {
        tray: u8,
        celsius: u16,
    },
    Unload {
        celsius: u16,
    },
    /// Continue, retry or acknowledge after an AMS or filament error.
    Ams {
        step: AmsStep,
    },
    AutoRecovery {
        on: bool,
    },
    Sound {
        on: bool,
    },
    AmsReading {
        on_insert: bool,
        on_power_up: bool,
        remain: bool,
    },
    Recording {
        on: bool,
    },
    Timelapse {
        on: bool,
    },
    Calibrate {
        bed_leveling: bool,
        vibration: bool,
        motor_noise: bool,
    },
    SkipObjects {
        objects: Vec<u32>,
    },
    ClearError {
        code: u32,
    },
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fan {
    Part,
    Aux,
    Chamber,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AmsStep {
    Resume,
    Reset,
    Done,
}

/// Where a control is sent and which reply answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub payload: Value,
    pub section: &'static str,
    pub command: &'static str,
}

impl Control {
    /// The MQTT message, or why the values cannot be sent.
    #[allow(clippy::too_many_lines)] // One table of every supported control.
    pub fn message(&self, sequence: &str) -> Result<Message, &'static str> {
        let print =
            |command: &'static str, fields: Value| build("print", command, fields, sequence);
        let gcode = |line: String| print("gcode_line", json!({"param": line}));
        let filament = |ams_id: u8, slot_id: u8, target: u8, celsius: u16| {
            if !(150..=300).contains(&celsius) {
                return Err("Filament temperature must be 150-300 °C");
            }
            Ok(print(
                "ams_change_filament",
                json!({"ams_id":ams_id,"slot_id":slot_id,"target":target,"curr_temp":celsius,"tar_temp":celsius}),
            ))
        };
        let toggle = |on: bool| if on { "enable" } else { "disable" };
        Ok(match self {
            Self::Pause => print("pause", json!({"param":""})),
            Self::Resume => print("resume", json!({"param":""})),
            Self::Stop => print("stop", json!({"param":""})),
            Self::Speed { level } => {
                if !(1..=4).contains(level) {
                    return Err("Speed level must be 1-4");
                }
                print("print_speed", json!({"param":level.to_string()}))
            }
            Self::Light { on } => build(
                "system",
                "ledctrl",
                json!({"led_node":"chamber_light","led_mode":if *on {"on"} else {"off"},
                    "led_on_time":500,"led_off_time":500,"loop_times":1,"interval_time":1000}),
                sequence,
            ),
            Self::NozzleTemperature { celsius } => {
                if *celsius > 300 {
                    return Err("Nozzle temperature must be 0-300 °C");
                }
                gcode(format!("M104 S{celsius}\n"))
            }
            Self::BedTemperature { celsius } => {
                if *celsius > 120 {
                    return Err("Bed temperature must be 0-120 °C");
                }
                gcode(format!("M140 S{celsius}\n"))
            }
            Self::Fan { fan, percent } => {
                if *percent > 100 {
                    return Err("Fan speed must be 0-100%");
                }
                let index = match fan {
                    Fan::Part => 1,
                    Fan::Aux => 2,
                    Fan::Chamber => 3,
                };
                // M106 takes 0-255; round half up so 50% is 128.
                let value = (u32::from(*percent) * 255 + 50) / 100;
                gcode(format!("M106 P{index} S{value}\n"))
            }
            Self::Load { tray, celsius } => match tray {
                0..=15 => filament(tray / 4, tray % 4, *tray, *celsius)?,
                254 => filament(254, 0, 254, *celsius)?,
                _ => return Err("Tray must be an AMS slot 0-15 or the external spool 254"),
            },
            Self::Unload { celsius } => filament(255, 255, 255, *celsius)?,
            Self::Ams { step } => print(
                "ams_control",
                json!({"param":match step {
                    AmsStep::Resume => "resume",
                    AmsStep::Reset => "reset",
                    AmsStep::Done => "done",
                }}),
            ),
            Self::AutoRecovery { on } => print(
                "print_option",
                json!({"option":i32::from(*on),"auto_recovery":on}),
            ),
            Self::Sound { on } => print("print_option", json!({"sound_enable":on})),
            Self::AmsReading {
                on_insert,
                on_power_up,
                remain,
            } => print(
                "ams_user_setting",
                json!({"ams_id":-1,"startup_read_option":on_power_up,"tray_read_option":on_insert,"calibrate_remain_flag":remain}),
            ),
            Self::Recording { on } => build(
                "camera",
                "ipcam_record_set",
                json!({"control":toggle(*on)}),
                sequence,
            ),
            Self::Timelapse { on } => build(
                "camera",
                "ipcam_timelapse",
                json!({"control":toggle(*on)}),
                sequence,
            ),
            Self::Calibrate {
                bed_leveling,
                vibration,
                motor_noise,
            } => {
                let option = u8::from(*motor_noise) << 3
                    | u8::from(*vibration) << 2
                    | u8::from(*bed_leveling) << 1;
                if option == 0 {
                    return Err("Select at least one calibration");
                }
                print("calibration", json!({"option":option}))
            }
            Self::SkipObjects { objects } => {
                if objects.is_empty() {
                    return Err("Select at least one object to skip");
                }
                print("skip_objects", json!({"obj_list":objects}))
            }
            Self::ClearError { code } => print(
                "clean_print_error",
                json!({"print_error":code,"subtask_id":""}),
            ),
            Self::Version => build("info", "get_version", json!({}), sequence),
        })
    }
}

fn build(section: &'static str, command: &'static str, fields: Value, sequence: &str) -> Message {
    let mut body = fields;
    body["command"] = command.into();
    body["sequence_id"] = sequence.into();
    Message {
        payload: json!({ section: body }),
        section,
        command,
    }
}

/// The printer's answer to `message` within one report, if this report carries it.
/// `Ok(())` is success; `Err` holds the printer's reason (or `None`).
pub fn reply(report: &Value, message: &Message) -> Option<Result<(), Option<String>>> {
    let body = report.get(message.section)?;
    let sequence = match &body["sequence_id"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if body["command"] != message.command
        || Some(sequence.as_str()) != message.payload[message.section]["sequence_id"].as_str()
    {
        return None;
    }
    match body["result"].as_str() {
        Some(r) if r.eq_ignore_ascii_case("success") => Some(Ok(())),
        Some(_) => Some(Err(body["reason"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().filter(|c| !c.is_control()).take(200).collect()))),
        // get_version answers with its modules and no result field.
        None if message.command == "get_version" => Some(Ok(())),
        None => None,
    }
}

/// Start options a person chooses per plate; defaults follow `BambuStudio`'s print dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // One flag per `project_file` option.
pub struct StartOptions {
    pub bed_leveling: bool,
    pub flow_calibration: bool,
    pub timelapse: bool,
    pub vibration_calibration: bool,
}

/// Deserialize `null` as the default value.
pub(crate) fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            bed_leveling: true,
            flow_calibration: true,
            timelapse: true,
            vibration_calibration: false,
        }
    }
}

/// Whether a reported job name or file is this attempt. P1S firmware may report the file name
/// (`orca-<id>.gcode.3mf`) as `subtask_name` and omit `gcode_file`.
pub fn names_attempt(name: Option<&str>, file: Option<&str>, id: &str) -> bool {
    let job = format!("orca-{id}");
    let path = format!("{job}.gcode.3mf");
    let name = name.filter(|s| !s.is_empty());
    let file = file.filter(|s| !s.is_empty());
    (name.is_some() || file.is_some())
        && name.is_none_or(|s| s == job || s == path)
        && file.is_none_or(|s| s == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value)] // Tests read better with values.
    fn sent(control: Control) -> Message {
        control.message("7").unwrap()
    }

    #[test]
    fn job_controls_match_bambu_studio() {
        for (control, command) in [
            (Control::Pause, "pause"),
            (Control::Resume, "resume"),
            (Control::Stop, "stop"),
        ] {
            let m = sent(control);
            assert_eq!(
                m.payload,
                json!({"print":{"command":command,"param":"","sequence_id":"7"}})
            );
            assert_eq!((m.section, m.command), ("print", command));
        }
        assert_eq!(
            sent(Control::Speed { level: 3 }).payload,
            json!({"print":{"command":"print_speed","param":"3","sequence_id":"7"}})
        );
        for level in [0, 5] {
            assert!(Control::Speed { level }.message("7").is_err());
        }
        assert_eq!(
            sent(Control::SkipObjects {
                objects: vec![3, 9]
            })
            .payload,
            json!({"print":{"command":"skip_objects","obj_list":[3,9],"sequence_id":"7"}})
        );
        assert!(
            Control::SkipObjects { objects: vec![] }
                .message("7")
                .is_err()
        );
        assert_eq!(
            sent(Control::ClearError { code: 0x0300_400C }).payload,
            json!({"print":{"command":"clean_print_error","print_error":0x0300_400C_u32,"subtask_id":"","sequence_id":"7"}})
        );
    }

    #[test]
    fn hardware_controls_use_gcode_or_system_messages_with_bounds() {
        let light = sent(Control::Light { on: true });
        assert_eq!(
            light.payload,
            json!({"system":{"command":"ledctrl","led_node":"chamber_light","led_mode":"on",
                "led_on_time":500,"led_off_time":500,"loop_times":1,"interval_time":1000,"sequence_id":"7"}})
        );
        assert_eq!((light.section, light.command), ("system", "ledctrl"));
        assert_eq!(
            sent(Control::Light { on: false }).payload["system"]["led_mode"],
            "off"
        );
        let gcode = |c| sent(c).payload["print"]["param"].clone();
        assert_eq!(
            gcode(Control::NozzleTemperature { celsius: 220 }),
            "M104 S220\n"
        );
        assert_eq!(gcode(Control::BedTemperature { celsius: 60 }), "M140 S60\n");
        assert_eq!(
            gcode(Control::Fan {
                fan: Fan::Part,
                percent: 100
            }),
            "M106 P1 S255\n"
        );
        assert_eq!(
            gcode(Control::Fan {
                fan: Fan::Aux,
                percent: 50
            }),
            "M106 P2 S128\n"
        );
        assert_eq!(
            gcode(Control::Fan {
                fan: Fan::Chamber,
                percent: 0
            }),
            "M106 P3 S0\n"
        );
        let m = sent(Control::BedTemperature { celsius: 0 });
        assert_eq!((m.section, m.command), ("print", "gcode_line"));
        for bad in [
            Control::NozzleTemperature { celsius: 301 },
            Control::BedTemperature { celsius: 121 },
            Control::Fan {
                fan: Fan::Part,
                percent: 101,
            },
        ] {
            assert!(bad.message("7").is_err(), "{bad:?}");
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One table of controls.
    fn filament_and_option_controls_match_bambu_studio() {
        assert_eq!(
            sent(Control::Load {
                tray: 6,
                celsius: 220
            })
            .payload,
            json!({"print":{"command":"ams_change_filament","ams_id":1,"slot_id":2,"target":6,
                "curr_temp":220,"tar_temp":220,"sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::Load {
                tray: 254,
                celsius: 250
            })
            .payload,
            json!({"print":{"command":"ams_change_filament","ams_id":254,"slot_id":0,"target":254,
                "curr_temp":250,"tar_temp":250,"sequence_id":"7"}})
        );
        assert!(
            Control::Load {
                tray: 16,
                celsius: 220
            }
            .message("7")
            .is_err()
        );
        assert!(
            Control::Load {
                tray: 0,
                celsius: 149
            }
            .message("7")
            .is_err()
        );
        assert!(Control::Unload { celsius: 301 }.message("7").is_err());
        assert_eq!(
            sent(Control::Unload { celsius: 220 }).payload,
            json!({"print":{"command":"ams_change_filament","ams_id":255,"slot_id":255,"target":255,
                "curr_temp":220,"tar_temp":220,"sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::Ams {
                step: AmsStep::Resume
            })
            .payload,
            json!({"print":{"command":"ams_control","param":"resume","sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::AutoRecovery { on: true }).payload,
            json!({"print":{"command":"print_option","option":1,"auto_recovery":true,"sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::Sound { on: false }).payload,
            json!({"print":{"command":"print_option","sound_enable":false,"sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::AmsReading {
                on_insert: true,
                on_power_up: false,
                remain: true
            })
            .payload,
            json!({"print":{"command":"ams_user_setting","ams_id":-1,"startup_read_option":false,
                "tray_read_option":true,"calibrate_remain_flag":true,"sequence_id":"7"}})
        );
        let record = sent(Control::Recording { on: true });
        assert_eq!(
            record.payload,
            json!({"camera":{"command":"ipcam_record_set","control":"enable","sequence_id":"7"}})
        );
        assert_eq!(
            (record.section, record.command),
            ("camera", "ipcam_record_set")
        );
        assert_eq!(
            sent(Control::Timelapse { on: false }).payload,
            json!({"camera":{"command":"ipcam_timelapse","control":"disable","sequence_id":"7"}})
        );
        assert_eq!(
            sent(Control::Calibrate {
                bed_leveling: true,
                vibration: true,
                motor_noise: false
            })
            .payload,
            json!({"print":{"command":"calibration","option":6,"sequence_id":"7"}})
        );
        assert!(
            Control::Calibrate {
                bed_leveling: false,
                vibration: false,
                motor_noise: false
            }
            .message("7")
            .is_err()
        );
        let version = sent(Control::Version);
        assert_eq!(
            version.payload,
            json!({"info":{"command":"get_version","sequence_id":"7"}})
        );
        assert_eq!((version.section, version.command), ("info", "get_version"));
    }

    #[test]
    fn controls_parse_from_api_input() {
        let c: Control =
            serde_json::from_value(json!({"action":"fan","fan":"chamber","percent":40})).unwrap();
        assert_eq!(
            c,
            Control::Fan {
                fan: Fan::Chamber,
                percent: 40
            }
        );
        assert!(
            serde_json::from_value::<Control>(json!({"action":"gcode","line":"M999"})).is_err()
        );
    }

    #[test]
    fn replies_match_section_command_and_sequence() {
        let m = sent(Control::Pause);
        assert_eq!(
            reply(
                &json!({"print":{"command":"pause","sequence_id":"7","result":"success"}}),
                &m
            ),
            Some(Ok(()))
        );
        assert_eq!(
            reply(
                &json!({"print":{"command":"pause","sequence_id":7,"result":"SUCCESS"}}),
                &m
            ),
            Some(Ok(()))
        );
        assert_eq!(
            reply(
                &json!({"print":{"command":"pause","sequence_id":"7","result":"fail","reason":"not printing"}}),
                &m
            ),
            Some(Err(Some("not printing".into())))
        );
        assert_eq!(
            reply(
                &json!({"print":{"command":"pause","sequence_id":"7","result":"failed"}}),
                &m
            ),
            Some(Err(None))
        );
        for other in [
            json!({"print":{"command":"pause","sequence_id":"8","result":"success"}}),
            json!({"print":{"command":"resume","sequence_id":"7","result":"success"}}),
            json!({"system":{"command":"pause","sequence_id":"7","result":"success"}}),
            json!({"print":{"command":"push_status","msg":1}}),
            json!({"print":{"command":"pause","sequence_id":"7"}}),
        ] {
            assert_eq!(reply(&other, &m), None, "{other}");
        }
        let light = sent(Control::Light { on: true });
        assert_eq!(
            reply(
                &json!({"system":{"command":"ledctrl","sequence_id":"7","result":"success"}}),
                &light
            ),
            Some(Ok(()))
        );
        // get_version answers with its module list instead of a result field.
        let version = sent(Control::Version);
        assert_eq!(
            reply(
                &json!({"info":{"command":"get_version","sequence_id":"7","module":[]}}),
                &version
            ),
            Some(Ok(()))
        );
    }

    #[test]
    fn start_options_default_to_bambu_studio_for_a_single_nozzle_printer() {
        // SelectMachineDialog: saved value, else "auto", else "on"; P1S offers no auto mode.
        // PrintJob::set_print_config is called with vibration calibration false.
        let d = StartOptions::default();
        assert!(d.bed_leveling && d.flow_calibration && d.timelapse);
        assert!(!d.vibration_calibration);
        let partial: StartOptions = serde_json::from_value(json!({"timelapse":false})).unwrap();
        assert!(!partial.timelapse && partial.bed_leveling);
        assert!(serde_json::from_value::<StartOptions>(json!({"layer_inspect":true})).is_err());
    }

    #[test]
    fn attempt_names_accept_the_file_name_as_job_name() {
        let id = "95e1af65-01a4-4ff3-bb6f-c7dc01b654b0";
        let name = format!("orca-{id}");
        let file = format!("orca-{id}.gcode.3mf");
        assert!(names_attempt(Some(&name), None, id));
        assert!(names_attempt(None, Some(&file), id));
        assert!(
            names_attempt(Some(&file), None, id),
            "production 2026-10-01"
        );
        assert!(names_attempt(Some(&name), Some(&file), id));
        assert!(names_attempt(Some(""), Some(&file), id));
        assert!(!names_attempt(None, None, id));
        assert!(!names_attempt(Some(""), Some(""), id));
        assert!(!names_attempt(Some("orca-other"), None, id));
        assert!(!names_attempt(
            Some(&name),
            Some("orca-other.gcode.3mf"),
            id
        ));
        assert!(!names_attempt(Some(&format!("{name}x")), None, id));
        assert!(!names_attempt(Some(&format!("{file}.bak")), None, id));
    }
}

/// Merge a differential report into the last full one, as `BambuStudio` does: objects merge
/// recursively, every other value (arrays included) is replaced.
pub fn merge(document: &mut serde_json::Map<String, Value>, diff: &serde_json::Map<String, Value>) {
    for (key, value) in diff {
        match (document.get_mut(key), value) {
            (Some(Value::Object(old)), Value::Object(new)) => merge(old, new),
            _ => {
                document.insert(key.clone(), value.clone());
            }
        }
    }
}

/// What the printer reports now, for people: temperatures, progress, fans, light, camera,
/// HMS codes and supported options. Unknown or unreported values are `null`.
pub fn live(document: &serde_json::Map<String, Value>) -> Value {
    let get = |key: &str| document.get(key).unwrap_or(&Value::Null);
    let number = |key: &str| {
        let v = get(key);
        v.as_f64().or_else(|| v.as_str()?.parse().ok())
    };
    // Reported values are small; saturation is the intended behavior for nonsense.
    #[allow(clippy::cast_possible_truncation)]
    let integer = |key: &str| number(key).map(|n| n.round() as i64);
    // Fan speeds are reported as gears 0-15 (BambuStudio DevFan).
    let fan = |key: &str| {
        integer(key)
            .filter(|g| (0..=15).contains(g))
            .map(|g| (g * 100 + 7) / 15)
    };
    let flag = integer("home_flag");
    let bit = |n: u32| flag.map(|f| f >> n & 1 == 1);
    let light = get("lights_report").as_array().and_then(|nodes| {
        nodes
            .iter()
            .find(|n| n["node"] == "chamber_light")
            .and_then(|n| n["mode"].as_str())
            .map(|mode| mode != "off")
    });
    let ipcam = |key: &str| get("ipcam")[key].as_str().map(|v| v == "enable");
    let hms: Vec<String> = get("hms")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let (attr, code) = (h["attr"].as_u64()?, h["code"].as_u64()?);
            Some(format!(
                "{:04X}_{:04X}_{:04X}_{:04X}",
                attr >> 16 & 0xFFFF,
                attr & 0xFFFF,
                code >> 16 & 0xFFFF,
                code & 0xFFFF
            ))
        })
        .collect();
    json!({
        "temperatures": {
            "nozzle": number("nozzle_temper"),
            "nozzle_target": number("nozzle_target_temper"),
            "bed": number("bed_temper"),
            "bed_target": number("bed_target_temper"),
            "chamber": number("chamber_temper"),
        },
        "layer": {"current": integer("layer_num"), "total": integer("total_layer_num")},
        "speed": {"level": integer("spd_lvl"), "percent": integer("spd_mag")},
        "fans": {
            "part": fan("cooling_fan_speed"),
            "aux": fan("big_fan1_speed"),
            "chamber": fan("big_fan2_speed"),
            "heatbreak": fan("heatbreak_fan_speed"),
        },
        "light": light,
        "camera": {"recording": ipcam("ipcam_record"), "timelapse": ipcam("timelapse")},
        "sdcard": get("sdcard").as_bool(),
        "wifi_signal": get("wifi_signal").as_str(),
        "stage": integer("stg_cur"),
        "skipped_objects": get("s_obj").as_array().map(|ids| ids.iter().filter_map(Value::as_u64).collect::<Vec<_>>()),
        "hms": hms,
        // BambuStudio parse_home_flag / DevPrintOptions: a value bit is meaningful only when supported.
        "options": {
            "auto_recovery": bit(4),
            "sound": bit(18).and_then(|supported| supported.then(|| bit(17)).flatten()),
            "remain_detection": bit(7),
            "motor_noise_calibration": bit(21),
        },
    })
}

#[cfg(test)]
mod live_tests {
    use super::*;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("object expected")
        };
        map
    }

    #[test]
    fn diffs_merge_objects_and_replace_arrays() {
        let mut doc = map(
            json!({"nozzle_temper":27,"ipcam":{"timelapse":"disable","ipcam_record":"enable"},
            "lights_report":[{"node":"chamber_light","mode":"on"}],"hms":[{"attr":1,"code":2}]}),
        );
        merge(
            &mut doc,
            &map(json!({"nozzle_temper":210.5,"ipcam":{"timelapse":"enable"},
            "lights_report":[{"node":"chamber_light","mode":"off"}],"hms":[]})),
        );
        assert_eq!(
            Value::Object(doc),
            json!({"nozzle_temper":210.5,"ipcam":{"timelapse":"enable","ipcam_record":"enable"},
                "lights_report":[{"node":"chamber_light","mode":"off"}],"hms":[]})
        );
    }

    #[test]
    fn live_view_reads_a_production_p1s_report() {
        // Values from the production P1S full report recorded on 2026-10-03.
        let doc = map(
            json!({"nozzle_temper":27,"nozzle_target_temper":0,"bed_temper":24.03125,
            "bed_target_temper":0,"chamber_temper":5,"layer_num":0,"total_layer_num":337,
            "spd_lvl":2,"spd_mag":100,"cooling_fan_speed":"0","big_fan1_speed":"15","big_fan2_speed":"8",
            "heatbreak_fan_speed":"0","lights_report":[{"mode":"on","node":"chamber_light"}],
            "ipcam":{"ipcam_record":"enable","timelapse":"disable"},"sdcard":true,"wifi_signal":"-44dBm",
            "home_flag":24_331_536,"mc_print_stage":"1","stg_cur":0,"print_type":"idle",
            "hms":[{"attr":0x0300_0D00_u32,"code":0x0001_0004}]}),
        );
        let live = live(&doc);
        assert_eq!(
            live["temperatures"],
            json!({"nozzle":27.0,"nozzle_target":0.0,"bed":24.03125,"bed_target":0.0,"chamber":5.0})
        );
        assert_eq!(live["layer"], json!({"current":0,"total":337}));
        assert_eq!(live["speed"], json!({"level":2,"percent":100}));
        assert_eq!(
            live["fans"],
            json!({"part":0,"aux":100,"chamber":53,"heatbreak":0})
        );
        assert_eq!(live["light"], true);
        assert_eq!(live["camera"], json!({"recording":true,"timelapse":false}));
        assert_eq!(live["sdcard"], true);
        assert_eq!(live["wifi_signal"], "-44dBm");
        assert_eq!(live["stage"], 0);
        assert_eq!(live["hms"], json!(["0300_0D00_0001_0004"]));
        // home_flag: auto recovery bit 4, sound bit 17 with support bit 18, motor noise bit 21.
        assert_eq!(
            live["options"],
            json!({"auto_recovery":true,"sound":null,"remain_detection":false,"motor_noise_calibration":true})
        );
    }

    #[test]
    fn missing_or_malformed_values_are_null() {
        let live = live(&map(
            json!({"nozzle_temper":"hot","lights_report":[],"hms":[{"attr":"x"}]}),
        ));
        assert!(live["temperatures"]["nozzle"].is_null());
        assert!(live["light"].is_null());
        assert_eq!(live["hms"], json!([]));
        assert!(live["fans"]["part"].is_null());
        assert!(live["options"]["auto_recovery"].is_null());
    }
}
