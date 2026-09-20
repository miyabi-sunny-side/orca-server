use crate::plates::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const PRINTER: &str = "Bambu Lab P1S 0.4 nozzle";
pub const BEDS: [&str; 4] = [
    "Textured PEI Plate",
    "Cool Plate",
    "Engineering Plate",
    "High Temp Plate",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub machine: String,
    pub process: String,
    pub filament: String,
    pub bed: String,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            machine: PRINTER.into(),
            process: "0.20mm Standard @BBL X1C".into(),
            filament: "Generic PLA High Speed @BBL X1C".into(),
            bed: BEDS[0].into(),
        }
    }
}

pub struct Profiles {
    machine: BTreeMap<String, Value>,
    process: BTreeMap<String, Value>,
    filament: BTreeMap<String, Value>,
}

impl Profiles {
    pub fn load(root: &Path) -> Result<Self> {
        let load = |category: &str| -> Result<BTreeMap<String, Value>> {
            let mut profiles = BTreeMap::new();
            let mut directories = vec![root.join(category)];
            while let Some(directory) = directories.pop() {
                for entry in fs::read_dir(directory)? {
                    let entry = entry?;
                    let path = entry.path();
                    if entry.file_type()?.is_dir() {
                        directories.push(path);
                    } else if path.extension().is_some_and(|e| e == "json") {
                        let value: Value = serde_json::from_slice(&fs::read(&path)?)
                            .map_err(std::io::Error::other)?;
                        if let Some(name) = value["name"].as_str() {
                            profiles.insert(name.to_owned(), value);
                        }
                    }
                }
            }
            Ok(profiles)
        };
        Ok(Self {
            machine: load("machine")?,
            process: load("process")?,
            filament: load("filament")?,
        })
    }

    pub fn machine(&self, key: &str) -> Result<Map<String, Value>> {
        let profile = flatten(&self.machine, key)?;
        if profile.get("instantiation").and_then(Value::as_str) != Some("true")
            || profile
                .get("nozzle_diameter")
                .and_then(Value::as_array)
                .is_none_or(|v| v.len() != 1)
        {
            return Err(Error::Invalid(
                "Select a supported single-nozzle machine profile",
            ));
        }
        Ok(profile)
    }

    pub fn machines(&self) -> Value {
        Value::Array(self.machine.keys().filter_map(|key| {
            let p = self.machine(key).ok()?;
            Some(serde_json::json!({"key":key,"model":p.get("printer_model"),"nozzle_diameter":p.get("nozzle_diameter").and_then(|v| v.get(0))}))
        }).collect())
    }

    pub fn choices_for(&self, key: &str) -> Result<Value> {
        let machine = self.machine(key)?;
        let names = |profiles: &BTreeMap<String, Value>| -> Vec<String> {
            profiles
                .keys()
                .filter(|name| selectable(profiles, name, key).is_ok())
                .cloned()
                .collect()
        };
        let processes = names(&self.process);
        let filaments = names(&self.filament);
        let process = machine
            .get("default_print_profile")
            .and_then(Value::as_str)
            .filter(|s| processes.iter().any(|v| v == s))
            .ok_or(Error::Invalid(
                "Machine default process profile is unavailable",
            ))?;
        let legacy = Selection::default().filament;
        let filament = if key == PRINTER && filaments.contains(&legacy) {
            legacy
        } else {
            machine
                .get("default_filament_profile")
                .and_then(|v| v.get(0))
                .and_then(Value::as_str)
                .filter(|s| filaments.iter().any(|v| v == s))
                .ok_or(Error::Invalid(
                    "Machine default filament profile is unavailable",
                ))?
                .to_owned()
        };
        Ok(
            serde_json::json!({"version":"2.4.2","printer":key,"processes":processes,"filaments":filaments,"beds":BEDS,
            "defaults":Selection { machine:key.into(), process:process.into(), filament, bed:BEDS[0].into() }}),
        )
    }

    pub fn validate_process(&self, machine: &str, process: &str) -> Result<()> {
        self.machine(machine)?;
        selectable(&self.process, process, machine)?;
        Ok(())
    }

    pub(crate) fn resolve_filament(
        &self,
        setting: &crate::filament::SettingData,
        material: &str,
    ) -> Result<Map<String, Value>> {
        setting.overrides_json.validate()?;
        self.machine(&setting.machine_profile_key)?;
        let mut profile = selectable(
            &self.filament,
            &setting.base_profile_key,
            &setting.machine_profile_key,
        )?;
        let base = profile
            .get("filament_type")
            .and_then(|v| v.get(0))
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("Base profile has no material type"))?;
        if material != base
            && material
                .strip_suffix("-CF")
                .or_else(|| material.strip_suffix("-GF"))
                != Some(base)
        {
            return Err(Error::Invalid(
                "Base profile material differs from the catalog material",
            ));
        }
        for (key, value) in [
            (
                "nozzle_temperature_initial_layer",
                setting.overrides_json.nozzle_temperature_initial_layer,
            ),
            (
                "nozzle_temperature",
                setting.overrides_json.nozzle_temperature,
            ),
        ] {
            if let Some(value) = value {
                profile.insert(key.into(), serde_json::json!([value.to_string()]));
            }
        }
        Ok(profile)
    }

    pub fn write(&self, selection: &Selection, directory: &Path) -> Result<()> {
        if !BEDS.contains(&selection.bed.as_str()) {
            return Err(Error::Invalid("Unknown bed type"));
        }
        for (filename, profile) in [
            ("printer.json", self.machine(&selection.machine)?),
            (
                "process.json",
                selectable(&self.process, &selection.process, &selection.machine)?,
            ),
            (
                "filament.json",
                selectable(&self.filament, &selection.filament, &selection.machine)?,
            ),
        ] {
            serde_json::to_writer(fs::File::create(directory.join(filename))?, &profile)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

fn selectable(
    profiles: &BTreeMap<String, Value>,
    name: &str,
    machine: &str,
) -> Result<Map<String, Value>> {
    let profile = flatten(profiles, name)?;
    if profile.get("instantiation").and_then(Value::as_str) != Some("true")
        || !profile
            .get("compatible_printers")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|v| v.as_str() == Some(machine)))
    {
        return Err(Error::Invalid(
            "Profile is not compatible with the selected machine and nozzle",
        ));
    }
    Ok(profile)
}

fn flatten(profiles: &BTreeMap<String, Value>, name: &str) -> Result<Map<String, Value>> {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = name;
    loop {
        if !seen.insert(current) {
            return Err(Error::Invalid("Cyclic profile inheritance"));
        }
        let value = profiles
            .get(current)
            .and_then(Value::as_object)
            .ok_or(Error::Invalid("Unknown profile or parent"))?;
        chain.push(value);
        match value.get("inherits") {
            None => break,
            Some(Value::String(parent)) if parent.is_empty() => break,
            Some(Value::String(parent)) => current = parent,
            _ => return Err(Error::Invalid("Invalid profile inheritance")),
        }
    }
    let mut merged = Map::new();
    for value in chain.into_iter().rev() {
        merged.extend(value.clone());
    }
    merged.remove("inherits");
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn material_settings_resolve_only_compatible_profiles_and_typed_deltas() {
        let profiles = Profiles {
            machine: BTreeMap::from([(
                PRINTER.into(),
                json!({"instantiation":"true","nozzle_diameter":["0.4"]}),
            )]),
            process: BTreeMap::new(),
            filament: BTreeMap::from([
                (
                    "base".into(),
                    json!({"instantiation":"false","filament_type":["PETG"],"nozzle_temperature_initial_layer":["255"],"nozzle_temperature":["255"],"required_nozzle_HRC":["0"]}),
                ),
                (
                    "petg".into(),
                    json!({"inherits":"base","instantiation":"true","compatible_printers":[PRINTER]}),
                ),
            ]),
        };
        let mut setting = crate::filament::SettingData {
            machine_profile_key: PRINTER.into(),
            base_profile_key: "petg".into(),
            overrides_json: crate::filament::Overrides::default(),
        };
        assert_eq!(
            profiles.resolve_filament(&setting, "PETG-GF").unwrap()["nozzle_temperature"],
            json!(["255"])
        );
        setting.overrides_json.nozzle_temperature_initial_layer = Some(250);
        setting.overrides_json.nozzle_temperature = Some(240);
        let resolved = profiles.resolve_filament(&setting, "PETG-GF").unwrap();
        assert_eq!(resolved["nozzle_temperature_initial_layer"], json!(["250"]));
        assert_eq!(resolved["nozzle_temperature"], json!(["240"]));
        assert_eq!(resolved["required_nozzle_HRC"], json!(["0"]));
        assert!(profiles.resolve_filament(&setting, "PLA").is_err());
        setting.machine_profile_key = "Bambu Lab P1S 0.2 nozzle".into();
        assert!(profiles.resolve_filament(&setting, "PETG-GF").is_err());
        setting.machine_profile_key = PRINTER.into();
        setting.base_profile_key = "base".into();
        assert!(profiles.resolve_filament(&setting, "PETG").is_err());
    }
    #[test]
    fn machine_choices_and_defaults_never_cross_nozzle_profiles() {
        let a1 = "Bambu Lab A1 mini 0.2 nozzle";
        let machine = |name: &str, diameter: &str, process: &str, filament: &str| {
            json!({
                "name":name,"instantiation":"true","printer_model":name,
                "nozzle_diameter":[diameter],"default_print_profile":process,
                "default_filament_profile":[filament],"printable_area":["0x0","180x0","180x180","0x180"]
            })
        };
        let preset = |name: &str, printer: &str| json!({"name":name,"instantiation":"true","compatible_printers":[printer]});
        let profiles = Profiles {
            machine: BTreeMap::from([
                (
                    PRINTER.into(),
                    machine(PRINTER, "0.4", "p1-process", "p1-pla"),
                ),
                (a1.into(), machine(a1, "0.2", "a1-process", "a1-pla")),
            ]),
            process: BTreeMap::from([
                ("p1-process".into(), preset("p1-process", PRINTER)),
                ("a1-process".into(), preset("a1-process", a1)),
            ]),
            filament: BTreeMap::from([
                ("p1-pla".into(), preset("p1-pla", PRINTER)),
                ("a1-pla".into(), preset("a1-pla", a1)),
            ]),
        };
        let choices = profiles.choices_for(a1).unwrap();
        assert_eq!(choices["processes"], json!(["a1-process"]));
        assert_eq!(choices["filaments"], json!(["a1-pla"]));
        assert_eq!(choices["defaults"]["machine"], a1);
        assert_eq!(
            profiles.machine(a1).unwrap()["nozzle_diameter"],
            json!(["0.2"])
        );
        assert!(profiles.choices_for("missing").is_err());
        let mut selection: Selection = serde_json::from_value(choices["defaults"].clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        profiles.write(&selection, dir.path()).unwrap();
        selection.process = "p1-process".into();
        assert!(profiles.write(&selection, dir.path()).is_err());
        selection.process = "a1-process".into();
        selection.filament = "p1-pla".into();
        assert!(profiles.write(&selection, dir.path()).is_err());
    }

    #[test]
    fn loads_nested_profiles_and_excludes_incompatible_or_base_presets() {
        let root = tempfile::tempdir().unwrap();
        for category in ["machine", "process", "filament/nested"] {
            fs::create_dir_all(root.path().join(category)).unwrap();
        }
        let directory = root.path().join("filament");
        fs::write(
            directory.join("base.json"),
            json!({"name":"base", "instantiation":"false", "compatible_printers":[PRINTER]})
                .to_string(),
        )
        .unwrap();
        fs::write(
            directory.join("nested/selected.json"),
            json!({"name":"selected", "inherits":"base", "instantiation":"true"}).to_string(),
        )
        .unwrap();
        fs::write(directory.join("other.json"), json!({"name":"other", "instantiation":"true", "compatible_printers":["different printer"]}).to_string()).unwrap();
        let profiles = Profiles::load(root.path()).unwrap();
        assert!(selectable(&profiles.filament, "selected", PRINTER).is_ok());
        assert!(selectable(&profiles.filament, "base", PRINTER).is_err());
        assert!(selectable(&profiles.filament, "other", PRINTER).is_err());
    }

    #[test]
    fn inherits_parent_values_but_child_arrays_and_scalars_replace_them() {
        let profiles = BTreeMap::from([
            (
                "base".into(),
                json!({"name":"base","layer_height":"0.2","colors":["red","blue"],"walls":"2"}),
            ),
            (
                "middle".into(),
                json!({"name":"middle","inherits":"base","walls":"3"}),
            ),
            (
                "selected".into(),
                json!({"name":"selected","inherits":"middle","colors":["green"]}),
            ),
        ]);
        assert_eq!(
            Value::Object(flatten(&profiles, "selected").unwrap()),
            json!({"name":"selected","layer_height":"0.2","colors":["green"],"walls":"3"})
        );
        assert!(flatten(&profiles, "missing").is_err());
        let cyclic = BTreeMap::from([
            ("a".into(), json!({"inherits":"b"})),
            ("b".into(), json!({"inherits":"a"})),
        ]);
        assert!(flatten(&cyclic, "a").is_err());
    }
}
