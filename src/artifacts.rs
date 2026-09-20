use crate::{
    plates::{Error, MAX_UPLOAD, Result},
    profiles::Selection,
};
use roxmltree::{Document, Node};
use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

fn metadata<'a>(node: Node<'a, 'a>, key: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name("metadata") && n.attribute("key") == Some(key))
        .and_then(|n| n.attribute("value"))
}

fn check_layout(xml: &str, models: usize) -> Result<()> {
    let invalid = || Error::Invalid("Models must fit together on one plate");
    let document = Document::parse(xml).map_err(|_| invalid())?;
    let root = document.root_element();
    let plates: Vec<_> = root
        .children()
        .filter(|n| n.has_tag_name("plate"))
        .collect();
    if plates.len() != 1 || metadata(plates[0], "plater_id") != Some("1") {
        return Err(invalid());
    }
    let objects: BTreeSet<_> = root
        .children()
        .filter(|n| n.has_tag_name("object"))
        .filter_map(|n| n.attribute("id"))
        .collect();
    let instances: Vec<_> = plates[0]
        .children()
        .filter(|n| n.has_tag_name("model_instance"))
        .collect();
    let assigned: BTreeSet<_> = instances
        .iter()
        .filter_map(|n| metadata(*n, "object_id"))
        .collect();
    if objects.len() != models || instances.len() != models || assigned != objects {
        return Err(invalid());
    }
    Ok(())
}

fn check_slice(xml: &str, models: usize) -> Result<()> {
    let invalid = || Error::Invalid("OrcaSlicer did not slice every model on the plate");
    let document = Document::parse(xml).map_err(|_| invalid())?;
    let root = document.root_element();
    if !root.descendants().any(|n| {
        n.has_tag_name("header_item")
            && n.attribute("key") == Some("OrcaSlicer-Version")
            && n.attribute("value") == Some("2.4.2")
    }) {
        return Err(invalid());
    }
    let plates: Vec<_> = root
        .children()
        .filter(|n| n.has_tag_name("plate"))
        .collect();
    if plates.len() != 1 || metadata(plates[0], "outside") != Some("false") {
        return Err(invalid());
    }
    let objects: Vec<_> = plates[0]
        .children()
        .filter(|n| n.has_tag_name("object"))
        .collect();
    if objects.len() != models
        || objects
            .iter()
            .any(|n| n.attribute("skipped") != Some("false"))
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn validate(path: &Path, models: usize, selection: &Selection, sliced: bool) -> Result<()> {
    if path
        .metadata()
        .map_err(|_| Error::Upstream("Missing 3MF output"))?
        .len()
        > MAX_UPLOAD as u64
    {
        return Err(Error::Invalid("Slicer artifact exceeds 64 MiB"));
    }
    let mut archive = zip::ZipArchive::new(File::open(path)?)
        .map_err(|_| Error::Upstream("Invalid 3MF archive"))?;
    let mut read = |name: &str| -> Result<String> {
        let file = archive
            .by_name(name)
            .map_err(|_| Error::Upstream("Missing 3MF metadata"))?;
        let mut text = String::new();
        file.take(4 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        if text.len() > 4 * 1024 * 1024 {
            return Err(Error::Upstream("3MF metadata too large"));
        }
        Ok(text)
    };
    check_layout(&read("Metadata/model_settings.config")?, models)?;
    let settings: serde_json::Value =
        serde_json::from_str(&read("Metadata/project_settings.config")?)
            .map_err(|_| Error::Upstream("Invalid 3MF settings"))?;
    if settings["printer_settings_id"] != selection.machine
        || settings["print_settings_id"] != selection.process
        || settings["filament_settings_id"] != serde_json::json!([selection.filament])
        || settings["curr_bed_type"] != selection.bed
    {
        return Err(Error::Upstream("3MF did not preserve selected profiles"));
    }
    if sliced {
        check_slice(&read("Metadata/slice_info.config")?, models)?;
        let mut gcode = archive
            .by_name("Metadata/plate_1.gcode")
            .map_err(|_| Error::Upstream("Missing print G-code"))?;
        let mut first = [0; 1];
        gcode
            .read_exact(&mut first)
            .map_err(|_| Error::Upstream("Empty print G-code"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"<config><object id="2"/><object id="4"/><plate><metadata key="plater_id" value="1"/><model_instance><metadata key="object_id" value="2"/></model_instance><model_instance><metadata key="object_id" value="4"/></model_instance></plate></config>"#;
    const SLICE: &str = r#"<config><header><header_item key="OrcaSlicer-Version" value="2.4.2"/></header><plate><metadata key="outside" value="false"/><object skipped="false"/><object skipped="false"/></plate></config>"#;

    #[test]
    fn rejects_missing_models_extra_plates_and_incomplete_slices() {
        assert!(check_layout(LAYOUT, 2).is_ok());
        assert!(check_layout(LAYOUT, 3).is_err());
        assert!(check_layout(&LAYOUT.replace("</config>", "<plate/></config>"), 2).is_err());
        assert!(check_layout(&LAYOUT.replace("value=\"4\"", "value=\"2\""), 2).is_err());
        assert!(check_slice(SLICE, 2).is_ok());
        assert!(check_slice(&SLICE.replace("skipped=\"false\"", "skipped=\"true\""), 2).is_err());
        assert!(check_slice(&SLICE.replace("value=\"false\"", "value=\"true\""), 2).is_err());
        assert!(check_slice(&SLICE.replace("2.4.2", "2.4.1"), 2).is_err());
        assert!(check_slice("broken XML", 2).is_err());
    }
}
