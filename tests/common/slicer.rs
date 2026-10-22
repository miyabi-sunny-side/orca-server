//! Test-only Orca substitute. Records real process inputs and rewrites a known print artifact.
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

#[allow(clippy::too_many_lines)] // Keep this end-to-end observation sequence together.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|v| v == "--help") {
        println!("OrcaSlicer-2.4.2:");
        return;
    }
    let executable = std::env::current_exe().expect("current executable path is available");
    let control = executable
        .parent()
        .expect("fixture CLI lives in a directory")
        .parent()
        .expect("fixture directory has a parent control directory");
    let root = std::env::current_dir().expect("working directory is available");
    let profiles: BTreeMap<String, Value> = ["printer", "process", "filament", "interface"]
        .into_iter()
        .filter_map(|name| {
            let path = root.join(format!("{name}.json"));
            path.exists().then(|| {
                (
                    name.to_owned(),
                    serde_json::from_slice(&fs::read(path).expect("profile file is readable"))
                        .expect("profile file is JSON"),
                )
            })
        })
        .collect();
    let materials: Vec<&Value> = ["filament", "interface"]
        .iter()
        .filter_map(|n| profiles.get(*n))
        .collect();
    let mut inputs: Vec<_> = fs::read_dir(&root)
        .expect("slicer working directory is readable")
        .map(|v| v.expect("directory entry is readable").path())
        .filter(|p| p.extension().is_some_and(|v| v == "stl"))
        .collect();
    inputs.sort();
    let hashes: Vec<_> = inputs
        .iter()
        .map(|p| {
            let output = Command::new("sha256sum")
                .arg(p)
                .output()
                .expect("sha256sum runs");
            assert!(
                output.status.success(),
                "sha256sum failed for {}",
                p.display()
            );
            String::from_utf8(output.stdout)
                .expect("sha256sum output is UTF-8")
                .split_whitespace()
                .next()
                .expect("sha256sum prints a hash")
                .to_owned()
        })
        .collect();
    let trace = json!({"directory":root,"arguments":args,"profiles":profiles,"inputs":hashes});
    // One append write prevents concurrent estimates from interleaving trace records.
    let mut line = serde_json::to_vec(&trace).expect("trace is serializable");
    line.push(b'\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(control.join("cli.jsonl"))
        .expect("CLI trace log opens for append")
        .write_all(&line)
        .expect("CLI trace record is written");
    let deadline = Instant::now() + Duration::from_mins(2);
    while control.join("cli-hold").exists() {
        assert!(Instant::now() < deadline, "fixture CLI hold timed out");
        thread::sleep(Duration::from_millis(20));
    }
    if control.join("cli-fail").exists() {
        std::process::exit(3);
    }
    let argument = |name: &str| {
        args.iter()
            .position(|v| v == name)
            .map(|i| args[i + 1].as_str())
    };
    let output = argument("--export-3mf").expect("output");
    let bed = argument("--curr-bed-type").map_or_else(
        || {
            let mut archive = ZipArchive::new(fs::File::open("project.3mf").expect("project.3mf opens")).expect("project.3mf is a zip archive");
            let mut text = String::new();
            archive
                .by_name("Metadata/project_settings.config")
                .expect("project.3mf contains project settings")
                .read_to_string(&mut text)
                .expect("project settings are UTF-8 text");
            serde_json::from_str::<Value>(&text).expect("project settings are JSON")["curr_bed_type"]
                .as_str()
                .expect("project settings declare curr_bed_type")
                .to_owned()
        },
        str::to_owned,
    );
    let mut source = ZipArchive::new(Cursor::new(include_bytes!(
        "../fixtures/p1_print.gcode.3mf"
    )))
    .expect("embedded print fixture is a zip archive");
    let mut target =
        ZipWriter::new(fs::File::create(Path::new(output)).expect("output artifact is creatable"));
    for i in 0..source.len() {
        let mut entry = source.by_index(i).expect("fixture zip entry is readable");
        let name = entry.name().to_owned();
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .expect("fixture zip entry is decompressible");
        if name == "Metadata/project_settings.config" {
            let mut settings: Value =
                serde_json::from_slice(&data).expect("fixture project settings are JSON");
            settings["printer_settings_id"] = profiles["printer"]["name"].clone();
            settings["print_settings_id"] = profiles["process"]["name"].clone();
            settings["filament_settings_id"] =
                json!(materials.iter().map(|p| &p["name"]).collect::<Vec<_>>());
            settings["curr_bed_type"] = json!(bed);
            for key in [
                "filament_type",
                "filament_colour",
                "nozzle_temperature",
                "nozzle_temperature_initial_layer",
            ] {
                settings[key] = json!(
                    materials
                        .iter()
                        .flat_map(|p| p
                            .get(key)
                            .cloned()
                            .unwrap_or(json!(["#FFFFFF"]))
                            .as_array()
                            .expect("material profile value is an array")
                            .clone())
                        .collect::<Vec<_>>()
                );
            }
            data = serde_json::to_vec(&settings).expect("settings are serializable");
        } else if name == "Metadata/slice_info.config" {
            let text = String::from_utf8(data).expect("slice info is UTF-8");
            let doc = roxmltree::Document::parse(&text).expect("slice info is XML");
            let plate = doc
                .descendants()
                .find(|n| n.has_tag_name("plate"))
                .expect("slice info has a plate");
            let mut edits: Vec<_> = plate
                .children()
                .filter(|n| n.has_tag_name("filament"))
                .map(|n| n.range())
                .collect();
            let insertion = plate.range().end - "</plate>".len();
            let mut modified = text.clone();
            let mut filaments = String::new();
            for (i, p) in materials.iter().enumerate() {
                let material = p["filament_type"][0]
                    .as_str()
                    .expect("material profile has filament_type");
                let color = p["filament_colour"][0].as_str().unwrap_or("#FFFFFF");
                write!(
                    filaments,
                    "<filament id=\"{}\" type=\"{}\" color=\"{}\" used_for_object=\"{}\"/>",
                    i + 1,
                    xml(material),
                    xml(color),
                    i == 0
                )
                .expect("writing to a String cannot fail");
            }
            modified.insert_str(insertion, &filaments);
            edits.sort_by_key(|r| r.start);
            for range in edits.into_iter().rev() {
                modified.replace_range(range, "");
            }
            data = modified.into_bytes();
        }
        target
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("output zip entry starts");
        target
            .write_all(&data)
            .expect("output zip entry is written");
    }
    target.finish().expect("output zip is finalized");
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
