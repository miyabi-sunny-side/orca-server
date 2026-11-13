#![allow(dead_code)] // Artifact checks are shared by separate test targets.
use crate::common::*;
use serde_json::Value;
use std::{fs, path::PathBuf};
pub fn transform(vertex: [f64; 3], text: Option<&str>) -> [f64; 3] {
    let matrix: Vec<f64> = text
        .unwrap_or("1 0 0 0 1 0 0 0 1 0 0 0")
        .split_whitespace()
        .map(|v| v.parse().expect("transform matrix entry is a number"))
        .collect();
    assert_eq!(matrix.len(), 12, "3MF transform has 12 entries");
    std::array::from_fn(|i| {
        (0..3).map(|j| vertex[j] * matrix[j * 3 + i]).sum::<f64>() + matrix[9 + i]
    })
}
pub fn boxes(bytes: &[u8]) -> Vec<[[f64; 2]; 3]> {
    fn vertices(bytes: &[u8], path: &str, id: &str) -> Vec<[f64; 3]> {
        let text = String::from_utf8(zip_read(bytes, path)).expect("3MF model is UTF-8");
        let xml = roxmltree::Document::parse(&text).expect("3MF model is XML");
        let object = xml
            .descendants()
            .find(|n| n.has_tag_name("object") && n.attribute("id") == Some(id))
            .expect("referenced 3MF object exists");
        let mut points = Vec::new();
        for node in object.descendants() {
            if node.has_tag_name("vertex") {
                points.push(["x", "y", "z"].map(|key| {
                    node.attribute(key)
                        .expect("vertex has the coordinate")
                        .parse()
                        .expect("vertex coordinate is a number")
                }));
            }
            if node.has_tag_name("component") {
                let target = node
                    .attribute((
                        "http://schemas.microsoft.com/3dmanufacturing/production/2015/06",
                        "path",
                    ))
                    .unwrap_or(path)
                    .trim_start_matches('/');
                points.extend(
                    vertices(
                        bytes,
                        target,
                        node.attribute("objectid")
                            .expect("component has an objectid"),
                    )
                    .into_iter()
                    .map(|v| transform(v, node.attribute("transform"))),
                );
            }
        }
        points
    }
    let text = String::from_utf8(zip_read(bytes, "3D/3dmodel.model")).expect("3MF model is UTF-8");
    let xml = roxmltree::Document::parse(&text).expect("3MF model is XML");
    let mut result = Vec::new();
    for item in xml
        .descendants()
        .filter(|n| n.has_tag_name("item") && n.parent().is_some_and(|p| p.has_tag_name("build")))
    {
        let points: Vec<_> = vertices(
            bytes,
            "3D/3dmodel.model",
            item.attribute("objectid")
                .expect("build item has an objectid"),
        )
        .into_iter()
        .map(|v| transform(v, item.attribute("transform")))
        .collect();
        assert!(!points.is_empty(), "build item has vertices");
        result.push(std::array::from_fn(|i| {
            [
                points.iter().map(|v| v[i]).fold(f64::INFINITY, f64::min),
                points
                    .iter()
                    .map(|v| v[i])
                    .fold(f64::NEG_INFINITY, f64::max),
            ]
        }));
    }
    result.sort_by(|a, b| a.partial_cmp(b).expect("box bounds are comparable"));
    result
}
pub fn geometry(project: &[u8], printed: &[u8], size: f64, height: f64) {
    let before = boxes(project);
    let after = boxes(printed);
    assert_eq!(before.len(), 2, "project has two build items: {before:?}");
    assert_eq!(
        after.len(),
        2,
        "printed artifact has two build items: {after:?}"
    );
    for b in &before {
        for ([low, high], limit) in b.iter().zip([size, size, height]) {
            assert!(0. <= *low && low < high && *high <= limit, "{before:?}");
        }
    }
    assert!(
        (0..2).any(|a| before[0][a][1] <= before[1][a][0] || before[1][a][1] <= before[0][a][0]),
        "{before:?}"
    );
    for (a, b) in before
        .iter()
        .flatten()
        .flatten()
        .zip(after.iter().flatten().flatten())
    {
        assert!(
            (a - b).abs() < 0.001,
            "printed geometry keeps project position: {a} vs {b}"
        );
    }
}
pub fn prediction(bytes: &[u8]) -> u64 {
    let text = String::from_utf8(zip_read(bytes, "Metadata/slice_info.config"))
        .expect("slice info is UTF-8");
    let xml = roxmltree::Document::parse(&text).expect("slice info is XML");
    xml.descendants()
        .find(|n| n.has_tag_name("metadata") && n.attribute("key") == Some("prediction"))
        .expect("slice info has a prediction")
        .attribute("value")
        .expect("prediction has a value")
        .parse()
        .expect("prediction is an integer")
}
pub fn estimate(rig: &Rig, job: &Value) -> (Value, PathBuf, Vec<u8>) {
    until(
        || {
            let estimate = rig.waiting(job)["estimate"].clone();
            assert_ne!(estimate["state"], "failed", "{estimate}");
            estimate["state"] == "ready"
        },
        120,
    );
    let snapshot: Value =
        serde_json::from_str(&rig.stored(job, "estimate_json")).expect("stored estimate is JSON");
    let path = rig.output.join(format!("slice-{}", id(&snapshot)));
    fs::create_dir_all(&path).expect("slice output directory is creatable");
    let bytes = rig.cached_artifact(job, "gcode");
    fs::write(path.join("print.gcode.3mf"), &bytes).expect("gcode artifact is writable");
    fs::write(
        path.join("project.3mf"),
        rig.cached_artifact(job, "project"),
    )
    .expect("project artifact is writable");
    for (name, profile) in snapshot["input"]["settings"]["profiles"]
        .as_object()
        .expect("estimate settings have a profiles object")
    {
        write_json(&path.join(name), profile);
    }
    (snapshot, path, bytes)
}
pub fn parameter(command: &str, key: char) -> Option<f64> {
    command
        .split_whitespace()
        .skip(1)
        .find_map(|word| word.strip_prefix(key).and_then(|v| v.parse().ok()))
}
pub fn motion(command: &str) -> bool {
    matches!(
        command.split_whitespace().next(),
        Some("G0" | "G1" | "G2" | "G3")
    )
}
