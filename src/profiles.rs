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
    pub process: String,
    pub filament: String,
    pub bed: String,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
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

    pub fn choices(&self) -> Value {
        let names = |profiles: &BTreeMap<String, Value>| -> Vec<String> {
            profiles
                .keys()
                .filter(|name| selectable(profiles, name).is_ok())
                .cloned()
                .collect()
        };
        serde_json::json!({"version":"2.4.2", "printer":PRINTER, "processes":names(&self.process), "filaments":names(&self.filament), "beds":BEDS, "defaults":Selection::default()})
    }

    pub fn write(&self, selection: &Selection, directory: &Path) -> Result<()> {
        if !BEDS.contains(&selection.bed.as_str()) {
            return Err(Error::Invalid("Unknown bed type"));
        }
        for (filename, profile) in [
            ("printer.json", flatten(&self.machine, PRINTER)?),
            (
                "process.json",
                selectable(&self.process, &selection.process)?,
            ),
            (
                "filament.json",
                selectable(&self.filament, &selection.filament)?,
            ),
        ] {
            serde_json::to_writer(fs::File::create(directory.join(filename))?, &profile)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

fn selectable(profiles: &BTreeMap<String, Value>, name: &str) -> Result<Map<String, Value>> {
    let profile = flatten(profiles, name)?;
    if profile.get("instantiation").and_then(Value::as_str) != Some("true")
        || !profile
            .get("compatible_printers")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|v| v.as_str() == Some(PRINTER)))
    {
        return Err(Error::Invalid("Profile is not compatible with P1S 0.4 mm"));
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
        assert_eq!(profiles.choices()["filaments"], json!(["selected"]));
        assert!(selectable(&profiles.filament, "base").is_err());
        assert!(selectable(&profiles.filament, "other").is_err());
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
