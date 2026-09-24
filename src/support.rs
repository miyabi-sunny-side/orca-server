use crate::plates::{Error, Result};
use serde_json::{Map, Value, json};

fn number(value: &Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_f64())
        .filter(|n| n.is_finite())
}

pub(crate) fn configure(
    process: &mut Map<String, Value>,
    enabled: bool,
    distinct: bool,
) -> Result<()> {
    process.insert(
        "enable_support".into(),
        if enabled { "1" } else { "0" }.into(),
    );
    process.insert("enforce_support_layers".into(), "0".into());
    if !enabled {
        return Ok(());
    }
    if !matches!(
        process.get("support_type").and_then(Value::as_str),
        Some("normal(auto)" | "tree(auto)")
    ) {
        process.insert("support_type".into(), "normal(auto)".into());
    }
    process.insert("support_filament".into(), "1".into());
    process.insert(
        "support_interface_filament".into(),
        if distinct { "2" } else { "1" }.into(),
    );
    process.insert("support_interface_not_for_body".into(), "1".into());
    // Orca 2.4.2 defaults: three top layers; -1 uses the top count for bottom interfaces.
    for (key, default, minimum) in [
        ("support_interface_top_layers", 3.0, 0.0),
        ("support_interface_bottom_layers", -1.0, -1.0),
    ] {
        let value = match process.get(key) {
            Some(v) => number(v)
                .filter(|n| *n >= minimum && n.fract() == 0.0)
                .ok_or(Error::Invalid(
                    "Process profile has invalid support interface layers",
                ))?,
            None => default,
        };
        process.insert(
            key.into(),
            if value == 0.0 { default } else { value }
                .to_string()
                .into(),
        );
    }
    if distinct {
        configure_materials(process, 2)?;
    }
    Ok(())
}

pub(crate) fn configure_materials(process: &mut Map<String, Value>, count: usize) -> Result<()> {
    if !(1..=3).contains(&count) {
        return Err(Error::Invalid("Expected one to three print materials"));
    }
    if count > 1 {
        process.insert("enable_prime_tower".into(), "1".into());
        for key in [
            "flush_into_infill",
            "flush_into_objects",
            "flush_into_support",
        ] {
            process.insert(key.into(), "0".into());
        }
        // Explicit volumes prevent the CLI's colour-based calculation from erasing a resin change.
        // PrintConfig.cpp 2.4.2 defaults: 280 mm³ between tools, multiplied by 0.3.
        let table = process
            .get("flush_volumes_matrix")
            .and_then(Value::as_array);
        let side = table.map_or(0, |v| v.len().isqrt());
        let volume = |index| {
            table
                .filter(|t| side >= 2 && side * side == t.len())
                .and_then(|t| t.get(index))
                .and_then(number)
                .filter(|n| *n > 0.0)
                .unwrap_or(280.0)
                .to_string()
        };
        let matrix = Value::Array(
            (0..count)
                .flat_map(|row| (0..count).map(move |column| (row, column)))
                .map(|(row, column)| {
                    Value::String(if row == column {
                        "0".into()
                    } else {
                        volume(row * side + column)
                    })
                })
                .collect(),
        );
        process.insert("flush_volumes_matrix".into(), matrix);
        if !process
            .get("flush_multiplier")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty() && a.iter().all(|v| number(v).is_some_and(|n| n > 0.0)))
        {
            process.insert("flush_multiplier".into(), json!(["0.3"]));
        }
    }
    Ok(())
}

pub(crate) fn material_profile(
    mut profile: Map<String, Value>,
    filament: &crate::filament::Filament,
    main: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>> {
    filament.data.validate()?;
    profile.insert(
        "filament_colour".into(),
        json!([format!("#{}", &filament.data.color[..6])]),
    );
    if let Some(main) = main {
        let name = profile
            .get("name")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("Material profile has no name"))?;
        profile.insert(
            "name".into(),
            format!("{name} / material {}", filament.id).into(),
        );
        for prefix in ["cool_plate", "eng_plate", "hot_plate", "textured_plate"] {
            for suffix in ["temp", "temp_initial_layer"] {
                let key = format!("{prefix}_{suffix}");
                if let Some(value) = main.get(&key) {
                    profile.insert(key, value.clone());
                }
            }
        }
    }
    Ok(profile)
}

pub(crate) fn check_materials(settings: &Value, profiles: &[Map<String, Value>]) -> Result<()> {
    let invalid =
        || Error::Invalid("3MF did not preserve the ordered material profiles, types and colours");
    if !(1..=3).contains(&profiles.len()) {
        return Err(invalid());
    }
    for (output, input) in [
        ("filament_settings_id", "name"),
        ("filament_type", "filament_type"),
        ("filament_colour", "filament_colour"),
    ] {
        // Pre-interface executions froze no colour; keep their single-material retry compatible.
        if input == "filament_colour" && profiles.len() == 1 && !profiles[0].contains_key(input) {
            continue;
        }
        let actual = settings
            .get(output)
            .and_then(Value::as_array)
            .filter(|a| a.len() == profiles.len())
            .ok_or_else(invalid)?;
        for (value, profile) in actual.iter().zip(profiles) {
            let expected = profile
                .get(input)
                .and_then(|v| if input == "name" { Some(v) } else { v.get(0) })
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            let value = value.as_str().ok_or_else(invalid)?;
            if if input == "filament_colour" {
                !value.eq_ignore_ascii_case(expected)
            } else {
                value != expected
            } {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_roles_enable_purge_without_support_and_three_materials_keep_positive_pairs() {
        let mut process = Map::new();
        configure_materials(&mut process, 3).unwrap();
        assert_eq!(process["enable_prime_tower"], "1");
        let table = process["flush_volumes_matrix"].as_array().unwrap();
        assert_eq!(table.len(), 9);
        for (i, value) in table.iter().enumerate() {
            assert_eq!(number(value).unwrap() > 0., i / 3 != i % 3);
        }
        assert!(!process.contains_key("enable_support"));
    }
    use serde_json::json;

    #[test]
    fn interface_keeps_its_nozzle_settings_and_uses_main_bed_without_mutating_catalog() {
        let main =
            json!({"name":"PLA", "cool_plate_temp":["35"], "cool_plate_temp_initial_layer":["40"]})
                .as_object()
                .unwrap()
                .clone();
        let original = json!({"name":"PETG", "filament_type":["PETG"],"nozzle_temperature":["250"], "cool_plate_temp":["0"], "cool_plate_temp_initial_layer":["0"]}).as_object().unwrap().clone();
        let filament: crate::filament::Filament = serde_json::from_value(json!({"id":"petg-black", "name":"black", "vendor":"fixture", "material":"PETG-GF", "color":"000000FF", "bambu_filament_id":null})).unwrap();
        let secondary = material_profile(original.clone(), &filament, Some(&main)).unwrap();
        assert_eq!(secondary["name"], "PETG / material petg-black");
        assert_eq!(
            secondary["nozzle_temperature"],
            original["nozzle_temperature"]
        );
        assert_eq!(secondary["filament_type"], original["filament_type"]);
        assert_eq!(secondary["filament_colour"], json!(["#000000"]));
        assert_eq!(secondary["cool_plate_temp"], main["cool_plate_temp"]);
        assert_eq!(
            secondary["cool_plate_temp_initial_layer"],
            main["cool_plate_temp_initial_layer"]
        );
        let primary = material_profile(original.clone(), &filament, None).unwrap();
        assert_eq!(primary["cool_plate_temp"], original["cool_plate_temp"]);
        assert_eq!(primary["name"], original["name"]);
        let mut invalid = filament;
        invalid.data.color = "x".into();
        assert!(material_profile(original, &invalid, None).is_err());
    }

    #[test]
    fn generated_profiles_must_preserve_every_material_in_order_even_for_same_pla() {
        let profiles = [json!({"name":"PLA white","filament_type":["PLA"],"filament_colour":["#FFFFFF"]}), json!({"name":"PLA black / interface b","filament_type":["PLA"],"filament_colour":["#000000"]})].map(|v| v.as_object().unwrap().clone());
        let settings = json!({"filament_settings_id":["PLA white","PLA black / interface b"],"filament_type":["PLA","PLA"],"filament_colour":["#FFFFFF","#000000"]});
        check_materials(&settings, &profiles).unwrap();
        for (key, wrong) in [
            (
                "filament_settings_id",
                json!(["PLA black / interface b", "PLA white"]),
            ),
            ("filament_type", json!(["PLA", "PETG"])),
            ("filament_colour", json!(["#000000", "#FFFFFF"])),
            ("filament_type", json!(["PLA"])),
            (
                "filament_settings_id",
                json!(["PLA white", "PLA black / interface b", "extra"]),
            ),
        ] {
            let mut changed = settings.clone();
            changed[key] = wrong;
            assert!(check_materials(&changed, &profiles).is_err(), "{key}");
        }
        assert!(check_materials(&settings, &[]).is_err());
        assert!(
            check_materials(
                &settings,
                &[
                    profiles[0].clone(),
                    profiles[1].clone(),
                    profiles[0].clone()
                ]
            )
            .is_err()
        );
    }

    fn process() -> Map<String, Value> {
        json!({"enable_support":"1", "enforce_support_layers":"8", "support_type":"tree(manual)",
            "support_interface_top_layers":"0", "support_interface_bottom_layers":"0",
            "support_top_z_distance":"0.2", "support_bottom_z_distance":"0.2", "support_interface_spacing":"0.5",
            "wall_loops":"4", "brim_type":"no_brim", "enable_prime_tower":"0"}).as_object().unwrap().clone()
    }

    #[test]
    fn off_is_explicit_and_on_keeps_gaps_and_uses_main_for_support_body() {
        let mut off = process();
        configure(&mut off, false, true).unwrap();
        assert_eq!(off["enable_support"], "0");
        assert_eq!(off["enforce_support_layers"], "0");
        assert_eq!(off["enable_prime_tower"], "0");
        let mut on = process();
        configure(&mut on, true, false).unwrap();
        assert_eq!(on["enable_support"], "1");
        assert_eq!(on["enforce_support_layers"], "0");
        assert_eq!(on["support_type"], "normal(auto)");
        assert_eq!(on["support_filament"], "1");
        assert_eq!(on["support_interface_filament"], "1");
        assert_eq!(on["support_interface_top_layers"], "3");
        assert_eq!(on["support_interface_bottom_layers"], "-1");
        for key in [
            "support_top_z_distance",
            "support_bottom_z_distance",
            "support_interface_spacing",
            "wall_loops",
            "brim_type",
            "enable_prime_tower",
        ] {
            assert_eq!(on[key], process()[key]);
        }
        on.insert("support_type".into(), json!("tree(auto)"));
        on.insert("support_interface_top_layers".into(), json!("4"));
        configure(&mut on, true, false).unwrap();
        assert_eq!(on["support_type"], "tree(auto)");
        assert_eq!(on["support_interface_top_layers"], "4");
    }

    #[test]
    fn distinct_ids_use_second_interface_and_nonzero_purge_outside_the_model() {
        let mut p = process();
        p.insert("flush_volumes_matrix".into(), json!(["0", "0", "0", "0"]));
        p.insert("flush_multiplier".into(), json!(["0"]));
        configure(&mut p, true, true).unwrap();
        assert_eq!(p["support_filament"], "1");
        assert_eq!(p["support_interface_filament"], "2");
        assert_eq!(p["support_interface_not_for_body"], "1");
        assert_eq!(p["enable_prime_tower"], "1");
        for key in [
            "flush_into_infill",
            "flush_into_objects",
            "flush_into_support",
        ] {
            assert_eq!(p[key], "0");
        }
        assert_eq!(p["flush_volumes_matrix"], json!(["0", "280", "280", "0"]));
        assert_eq!(p["flush_multiplier"], json!(["0.3"]));
        p.insert(
            "flush_volumes_matrix".into(),
            json!(["0", "350", "450", "0"]),
        );
        p.insert("flush_multiplier".into(), json!(["1"]));
        configure(&mut p, true, true).unwrap();
        assert_eq!(p["flush_volumes_matrix"], json!(["0", "350", "450", "0"]));
        assert_eq!(p["flush_multiplier"], json!(["1"]));
    }

    #[test]
    fn invalid_layers_are_rejected_and_larger_purge_tables_keep_the_first_pair() {
        for value in [json!("-2"), json!("1.5"), json!("NaN"), Value::Null] {
            let mut p = process();
            p.insert("support_interface_top_layers".into(), value);
            assert!(configure(&mut p, true, false).is_err());
        }
        let mut p = process();
        p.insert(
            "flush_volumes_matrix".into(),
            json!([
                "0", "300", "400", "500", "600", "0", "300", "300", "300", "300", "0", "300",
                "300", "300", "300", "0"
            ]),
        );
        configure(&mut p, true, true).unwrap();
        assert_eq!(p["flush_volumes_matrix"], json!(["0", "300", "600", "0"]));
    }
}
