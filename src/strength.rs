use crate::plates::{Error, Result};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// OrcaSlicer 2.4.2 PrintConfig.cpp: sparse_infill_pattern enum_values.
pub const PATTERNS: &[&str] = &[
    "rectilinear",
    "alignedrectilinear",
    "zigzag",
    "crosszag",
    "lockedzag",
    "line",
    "grid",
    "triangles",
    "tri-hexagon",
    "cubic",
    "adaptivecubic",
    "quartercubic",
    "supportcubic",
    "lightning",
    "honeycomb",
    "3dhoneycomb",
    "lateral-honeycomb",
    "lateral-lattice",
    "crosshatch",
    "tpmsd",
    "tpmsfk",
    "gyroid",
    "concentric",
    "hilbertcurve",
    "archimedeanchords",
    "octagramspiral",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Strength {
    pub sparse_infill_pattern: Option<String>,
    pub sparse_infill_density: Option<f64>,
    pub wall_loops: Option<u32>,
}
impl Strength {
    pub fn validate(&self) -> Result<()> {
        if self
            .sparse_infill_pattern
            .as_deref()
            .is_some_and(|p| !PATTERNS.contains(&p))
        {
            return Err(Error::Invalid(
                "Unknown infill pattern for OrcaSlicer 2.4.2",
            ));
        }
        if self
            .sparse_infill_density
            .is_some_and(|d| !d.is_finite() || !(0.0..=100.0).contains(&d))
        {
            return Err(Error::Invalid("Infill density must be 0–100 percent"));
        }
        if self.wall_loops.is_some_and(|w| w > 1000) {
            return Err(Error::Invalid(
                "Wall loops must be an integer from 0 to 1000",
            ));
        }
        Ok(())
    }
    // Always apply to the original flattened process, never to a previous override.
    pub fn apply(&self, process: &mut Map<String, Value>) -> Result<()> {
        self.validate()?;
        if let Some(pattern) = &self.sparse_infill_pattern {
            process.insert("sparse_infill_pattern".into(), pattern.clone().into());
        }
        if let Some(density) = self.sparse_infill_density {
            process.insert("sparse_infill_density".into(), format!("{density}%").into());
        }
        if let Some(walls) = self.wall_loops {
            let scale = f64::from(walls) / 2.0;
            for (key, default) in [("top_shell_layers", 4.0), ("bottom_shell_layers", 3.0)] {
                let layers = (number(process, key, default)? * scale).ceil();
                process.insert(key.into(), layers.to_string().into());
            }
            for key in ["top_shell_thickness", "bottom_shell_thickness"] {
                if process.contains_key(key) {
                    let thickness = number(process, key, 0.0)? * scale;
                    process.insert(key.into(), thickness.to_string().into());
                }
            }
            process.insert("wall_loops".into(), walls.to_string().into());
        }
        Ok(())
    }
}

fn number(process: &Map<String, Value>, key: &str, default: f64) -> Result<f64> {
    let Some(value) = process.get(key) else {
        return Ok(default);
    };
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_f64())
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or(Error::Invalid("Process profile has an invalid shell value"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn base() -> Map<String, Value> {
        json!({"sparse_infill_pattern":"crosshatch", "sparse_infill_density":"15%", "wall_loops":"2",
        "top_shell_layers":"5", "bottom_shell_layers":"3", "top_shell_thickness":"1", "bottom_shell_thickness":"0",
        "layer_height":"0.2", "outer_wall_speed":"200"}).as_object().unwrap().clone()
    }
    #[test]
    fn walls_scale_original_shells_without_changing_unrelated_settings() {
        let original = base();
        let mut inherited = original.clone();
        Strength::default().apply(&mut inherited).unwrap();
        assert_eq!(inherited, original);
        for (walls, top, bottom, thickness) in [
            (2, 5, 3, "1"),
            (4, 10, 6, "2"),
            (3, 8, 5, "1.5"),
            (2, 5, 3, "1"),
            (0, 0, 0, "0"),
        ] {
            let mut profile = original.clone();
            Strength {
                sparse_infill_pattern: Some("adaptivecubic".into()),
                sparse_infill_density: Some(22.5),
                wall_loops: Some(walls),
            }
            .apply(&mut profile)
            .unwrap();
            assert_eq!(profile["sparse_infill_pattern"], "adaptivecubic");
            assert_eq!(profile["sparse_infill_density"], "22.5%");
            assert_eq!(profile["wall_loops"], walls.to_string());
            assert_eq!(profile["top_shell_layers"], top.to_string());
            assert_eq!(profile["bottom_shell_layers"], bottom.to_string());
            assert_eq!(profile["top_shell_thickness"], thickness);
            assert_eq!(profile["bottom_shell_thickness"], "0");
            assert_eq!(profile["layer_height"], "0.2");
            assert_eq!(profile["outer_wall_speed"], "200");
        }
        let mut different = original.clone();
        different.insert("top_shell_layers".into(), json!("7"));
        different.insert("bottom_shell_layers".into(), json!("4"));
        different.insert("top_shell_thickness".into(), json!("0.9"));
        Strength {
            wall_loops: Some(3),
            ..Default::default()
        }
        .apply(&mut different)
        .unwrap();
        assert_eq!(different["top_shell_layers"], "11");
        assert_eq!(different["bottom_shell_layers"], "6");
        assert_eq!(different["top_shell_thickness"], "1.35");
        assert_eq!(different["sparse_infill_pattern"], "crosshatch");
    }
    #[test]
    fn sparse_values_obey_the_supported_cli_domain() {
        for density in [0., 15., 22.5, 100.] {
            for pattern in [
                "adaptivecubic",
                "crosshatch",
                "gyroid",
                "lockedzag",
                "tpmsd",
                "tpmsfk",
                "rectilinear",
            ] {
                let value = Strength {
                    sparse_infill_pattern: Some(pattern.into()),
                    sparse_infill_density: Some(density),
                    wall_loops: Some(2),
                };
                value.validate().unwrap();
                let mut p = base();
                value.apply(&mut p).unwrap();
                assert_eq!(p["sparse_infill_pattern"], pattern);
                assert_eq!(p["sparse_infill_density"], format!("{density}%"));
            }
        }
        for density in [-1., 100.1, f64::NAN, f64::INFINITY] {
            assert!(
                Strength {
                    sparse_infill_density: Some(density),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for pattern in ["unknown", "Gyroid", "adaptivecubic;cmd"] {
            assert!(
                Strength {
                    sparse_infill_pattern: Some(pattern.into()),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Strength {
                wall_loops: Some(1001),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(serde_json::from_value::<Strength>(json!({"wall_loops":2.5})).is_err());
        assert!(serde_json::from_value::<Strength>(json!({"wall_loops":-1})).is_err());
    }
}
