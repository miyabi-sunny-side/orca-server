use crate::plates::{Error, MAX_UPLOAD, Result};
use roxmltree::{Document, Node};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};
type Bounds = [[f64; 2]; 3];
type Transform = [f64; 12];
type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

#[derive(Debug, Serialize)]
pub struct ModelBounds {
    pub index: usize,
    pub bounds: Bounds,
}

fn invalid() -> Error {
    Error::Invalid("Saved layout could not be read")
}
fn text(archive: &mut Archive<'_>, path: &str, budget: &mut u64) -> Result<String> {
    let file = archive
        .by_name(path.trim_start_matches('/'))
        .map_err(|_| invalid())?;
    let mut text = String::new();
    file.take(*budget + 1)
        .read_to_string(&mut text)
        .map_err(|_| invalid())?;
    *budget = budget.checked_sub(text.len() as u64).ok_or_else(invalid)?;
    Ok(text)
}
fn transform(node: Node<'_, '_>) -> Result<Transform> {
    let Some(value) = node.attribute("transform").filter(|s| !s.is_empty()) else {
        return Ok([1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);
    };
    let values = value
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    if values.iter().any(|v| !v.is_finite()) {
        return Err(invalid());
    }
    values.try_into().map_err(|_| invalid())
}
fn vertices(
    archive: &mut Archive<'_>,
    path: &str,
    id: &str,
    chain: &mut Vec<Transform>,
    bounds: &mut Bounds,
    budget: &mut u64,
) -> Result<()> {
    if chain.len() > 16 {
        return Err(invalid());
    }
    let xml = text(archive, path, budget)?;
    let document = Document::parse(&xml).map_err(|_| invalid())?;
    let object = document
        .descendants()
        .find(|n| n.has_tag_name("object") && n.attribute("id") == Some(id))
        .ok_or_else(invalid)?;
    for vertex in object.descendants().filter(|n| n.has_tag_name("vertex")) {
        let mut point = [0.; 3];
        for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
            point[index] = vertex
                .attribute(axis)
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
        }
        for matrix in chain.iter().rev() {
            point = std::array::from_fn(|axis| {
                point
                    .iter()
                    .enumerate()
                    .map(|(i, v)| v * matrix[i * 3 + axis])
                    .sum::<f64>()
                    + matrix[9 + axis]
            });
        }
        for (axis, value) in point.into_iter().enumerate() {
            if !value.is_finite() {
                return Err(invalid());
            }
            bounds[axis][0] = bounds[axis][0].min(value);
            bounds[axis][1] = bounds[axis][1].max(value);
        }
    }
    for component in object.descendants().filter(|n| n.has_tag_name("component")) {
        let target = component
            .attributes()
            .find(|a| a.name() == "path")
            .map_or(path, |a| a.value());
        chain.push(transform(component)?);
        vertices(
            archive,
            target,
            component.attribute("objectid").ok_or_else(invalid)?,
            chain,
            bounds,
            budget,
        )?;
        chain.pop();
    }
    Ok(())
}

pub fn layout(bytes: &[u8]) -> Result<Vec<ModelBounds>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    let mut budget = MAX_UPLOAD as u64;
    let metadata = text(&mut archive, "Metadata/model_settings.config", &mut budget)?;
    let document = Document::parse(&metadata).map_err(|_| invalid())?;
    let mut names = BTreeMap::new();
    for object in document
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("object"))
    {
        let name = object
            .children()
            .find(|n| n.has_tag_name("metadata") && n.attribute("key") == Some("name"))
            .and_then(|n| n.attribute("value"))
            .ok_or_else(invalid)?;
        let index: usize = name
            .strip_suffix(".stl")
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        if index >= 64 {
            return Err(invalid());
        }
        names.insert(object.attribute("id").ok_or_else(invalid)?, index);
    }
    let root = text(&mut archive, "3D/3dmodel.model", &mut budget)?;
    let document = Document::parse(&root).map_err(|_| invalid())?;
    let mut models = Vec::new();
    for item in document.descendants().filter(|n| n.has_tag_name("item")) {
        let id = item.attribute("objectid").ok_or_else(invalid)?;
        let index = *names.get(id).ok_or_else(invalid)?;
        let mut bounds = [[f64::INFINITY, f64::NEG_INFINITY]; 3];
        vertices(
            &mut archive,
            "3D/3dmodel.model",
            id,
            &mut vec![transform(item)?],
            &mut bounds,
            &mut budget,
        )?;
        if bounds.iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid());
        }
        models.push(ModelBounds { index, bounds });
    }
    if models.is_empty() || models.len() > 64 {
        return Err(invalid());
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn project(transform: &str, component: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let root = format!(
            r#"<model><resources><object id="2"><components><component path="/3D/part.model" objectid="1" transform="{component}"/></components></object></resources><build><item objectid="2" transform="{transform}"/></build></model>"#
        );
        for (name, text) in [
            ("3D/3dmodel.model", root.as_str()),
            (
                "3D/part.model",
                r#"<model><resources><object id="1"><mesh><vertices><vertex x="-10" y="-10" z="-10"/><vertex x="10" y="10" z="10"/></vertices></mesh></object></resources></model>"#,
            ),
            (
                "Metadata/model_settings.config",
                r#"<config><object id="2"><metadata key="name" value="0.stl"/></object></config>"#,
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn composes_build_and_component_transforms_before_bounding_vertices() {
        let preview = layout(&project(
            "1 0 0 0 1 0 0 0 1 118 107 0",
            "1 0 0 0 1 0 0 0 1 10 10 10",
        ))
        .unwrap();
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].index, 0);
        assert_eq!(preview[0].bounds, [[118., 138.], [107., 127.], [0., 20.]]);
        let rotated = layout(&project(
            "0 1 0 -1 0 0 0 0 1 50 50 0",
            "1 0 0 0 1 0 0 0 1 10 10 10",
        ))
        .unwrap();
        assert_eq!(rotated[0].bounds, [[30., 50.], [50., 70.], [0., 20.]]);
        assert!(layout(&project("NaN 0 0 0 1 0 0 0 1 0 0 0", "")).is_err());
        assert!(layout(b"broken archive").is_err());
    }
}
