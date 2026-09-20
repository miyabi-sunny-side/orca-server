use crate::{
    plates::{Error, Result},
    printer_state::Tray,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilamentData {
    pub name: String,
    pub vendor: String,
    pub material: String,
    pub color: String,
    pub bambu_filament_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Filament {
    pub id: String,
    #[serde(flatten)]
    pub data: FilamentData,
}
impl FilamentData {
    pub fn validate(&self) -> Result<()> {
        for text in [&self.name, &self.vendor, &self.material] {
            if text.trim().is_empty() || text.len() > 160 || text.chars().any(char::is_control) {
                return Err(Error::Invalid(
                    "Name, vendor and material must contain 1..160 bytes without control characters",
                ));
            }
        }
        if self.color.len() != 8 || !self.color.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::Invalid(
                "Color must be eight hexadecimal RGBA digits",
            ));
        }
        if self.bambu_filament_id.as_ref().is_some_and(|id| {
            id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        }) {
            return Err(Error::Invalid("Invalid Bambu filament ID"));
        }
        Ok(())
    }
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Overrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nozzle_temperature_initial_layer: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nozzle_temperature: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_temperature_initial_layer: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_temperature: Option<u16>,
}
impl Overrides {
    pub fn validate(&self) -> Result<()> {
        if [
            self.nozzle_temperature,
            self.nozzle_temperature_initial_layer,
        ]
        .into_iter()
        .flatten()
        .any(|v| !(120..=350).contains(&v))
        {
            return Err(Error::Invalid(
                "Explicit nozzle temperatures must be integers from 120 to 350 Celsius",
            ));
        }
        if [self.bed_temperature_initial_layer, self.bed_temperature]
            .into_iter()
            .flatten()
            .any(|v| v > 120)
        {
            return Err(Error::Invalid(
                "Explicit bed temperatures must be integers from 0 to 120 Celsius",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingData {
    pub machine_profile_key: String,
    pub base_profile_key: String,
    #[serde(default)]
    pub overrides_json: Overrides,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Setting {
    pub id: String,
    pub filament_id: String,
    #[serde(flatten)]
    pub data: SettingData,
}

pub(crate) fn candidates(tray: &Tray, filaments: &[Filament]) -> Vec<String> {
    if tray.present != Some(true)
        || tray.tag_uid.is_none()
        || tray.profile_id.is_none()
        || tray.color.is_none()
        || tray.material.is_none()
    {
        return vec![];
    }
    filaments
        .iter()
        .filter(|f| {
            f.data.bambu_filament_id == tray.profile_id
                && Some(&f.data.material) == tray.material.as_ref()
                && tray
                    .color
                    .as_ref()
                    .is_some_and(|color| color.eq_ignore_ascii_case(&f.data.color))
        })
        .map(|f| f.id.clone())
        .collect()
}
pub(crate) fn same_identity(a: &Tray, b: &Tray) -> bool {
    a.present == b.present
        && a.material == b.material
        && a.color == b.color
        && a.profile_id == b.profile_id
        && a.tag_uid == b.tag_uid
        && a.brand == b.brand
        && a.temperature_min == b.temperature_min
        && a.temperature_max == b.temperature_max
}

pub(crate) fn nozzle_fit(
    material: &str,
    diameter: &str,
    nozzle_material: &str,
    required_hrc: Option<u16>,
) -> &'static str {
    let abrasive = material.ends_with("-CF") || material.ends_with("-GF");
    let Some(diameter) = diameter
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.)
    else {
        return "unknown";
    };
    if abrasive && diameter < 0.4 {
        return "unsupported";
    }
    if nozzle_material == "unknown" {
        return "unknown";
    }
    if (abrasive || required_hrc.is_some_and(|v| v > 20)) && nozzle_material != "hardened_steel" {
        return "unsupported";
    }
    if required_hrc.is_some_and(|v| v > 55) {
        return "unsupported";
    }
    if (abrasive && diameter < 0.6) || required_hrc.is_none() {
        return "unknown";
    }
    "supported"
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn item(id: &str, color: &str) -> Filament {
        Filament {
            id: id.into(),
            data: FilamentData {
                name: "PLA Matte".into(),
                vendor: "Bambu Lab".into(),
                material: "PLA".into(),
                color: color.into(),
                bambu_filament_id: Some("GFA01".into()),
            },
        }
    }
    #[test]
    fn automatic_match_requires_unambiguous_reported_identity() {
        let black = item("black", "000000FF");
        let white = item("white", "FFFFFFFF");
        let mut tray = crate::printer_state::Tray {
            present: Some(true),
            material: Some("PLA".into()),
            profile_id: Some("GFA01".into()),
            color: Some("000000FF".into()),
            tag_uid: Some("TESTTAG".into()),
            ..Default::default()
        };
        assert_eq!(
            candidates(&tray, &[black.clone(), white.clone()]),
            vec!["black"]
        );
        assert_eq!(
            candidates(&tray, &[black.clone(), item("ambiguous", "000000FF")]).len(),
            2
        );
        tray.tag_uid = None;
        assert!(candidates(&tray, std::slice::from_ref(&black)).is_empty());
        tray.tag_uid = Some("OTHER".into());
        tray.color = None;
        assert!(candidates(&tray, std::slice::from_ref(&black)).is_empty());
        tray.color = Some("FFFFFFFF".into());
        assert_eq!(candidates(&tray, &[black, white]), vec!["white"]);
    }
    #[test]
    fn identity_changes_invalidate_mapping_but_consumption_does_not() {
        let mut a = crate::printer_state::Tray {
            present: Some(true),
            material: Some("PETG".into()),
            color: Some("FFFFFFFF".into()),
            temperature_max: Some(260),
            ..Default::default()
        };
        let mut b = a.clone();
        b.remaining_percent = Some(10);
        b.last_seen_at = Some(50);
        assert!(same_identity(&a, &b));
        b.temperature_max = Some(240);
        assert!(!same_identity(&a, &b));
        b = a.clone();
        b.tag_uid = Some("NEW".into());
        assert!(!same_identity(&a, &b));
        a.present = Some(false);
        assert!(!same_identity(&a, &b));
    }
    #[test]
    fn material_and_explicit_temperature_overrides_are_bounded() {
        let mut material = item("gf", "FFFFFFFF").data;
        material.material = "PETG-GF".into();
        assert!(material.validate().is_ok());
        assert_ne!(material.material, "PETG");
        material.color = "red; background:url(x)".into();
        assert!(material.validate().is_err());
        let o: Overrides = serde_json::from_value(
            json!({"nozzle_temperature_initial_layer":250,"nozzle_temperature":240}),
        )
        .unwrap();
        assert!(o.validate().is_ok());
        assert!(serde_json::from_value::<Overrides>(json!({"filament_start_gcode":"G1"})).is_err());
        assert!(serde_json::from_value::<Overrides>(json!({"nozzle_temperature":240.5})).is_err());
        assert!(
            Overrides {
                nozzle_temperature: Some(900),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn bed_temperature_overrides_distinguish_zero_and_absence() {
        let empty: Overrides = serde_json::from_value(json!({})).unwrap();
        assert_eq!(serde_json::to_value(empty).unwrap(), json!({}));
        for value in [0, 65, 120] {
            let input = json!({"bed_temperature_initial_layer":value,"bed_temperature":value});
            let parsed: Overrides = serde_json::from_value(input.clone()).unwrap();
            assert!(parsed.validate().is_ok());
            assert_eq!(serde_json::to_value(parsed).unwrap(), input);
        }
        for value in [json!(-1), json!(121), json!(65.5), json!("65")] {
            for key in ["bed_temperature_initial_layer", "bed_temperature"] {
                let parsed = serde_json::from_value::<Overrides>(json!({key:value}));
                assert!(parsed.map_or(true, |p| p.validate().is_err()));
            }
        }
    }
    #[test]
    fn abrasive_material_needs_a_suitable_known_nozzle() {
        assert_eq!(
            nozzle_fit("PETG-GF", "0.2", "hardened_steel", Some(0)),
            "unsupported"
        );
        assert_eq!(
            nozzle_fit("PETG-GF", "0.4", "stainless_steel", Some(0)),
            "unsupported"
        );
        assert_eq!(nozzle_fit("PETG-GF", "0.4", "unknown", Some(0)), "unknown");
        assert_eq!(
            nozzle_fit("PETG-GF", "0.6", "hardened_steel", Some(40)),
            "supported"
        );
        assert_eq!(
            nozzle_fit("PLA", "0.2", "stainless_steel", Some(0)),
            "supported"
        );
        assert_eq!(nozzle_fit("PLA", "0.4", "unknown", None), "unknown");
        assert_eq!(
            nozzle_fit("PLA", "0.4", "hardened_steel", Some(60)),
            "unsupported"
        );
        assert_eq!(
            nozzle_fit("PETG-GF", "0.4", "hardened_steel", Some(0)),
            "unknown"
        );
    }
}
