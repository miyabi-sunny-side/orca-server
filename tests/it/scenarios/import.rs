use crate::common::*;
use serde_json::Value;
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
#[allow(clippy::format_collect)]
pub fn model_fixture(painted: bool, multiple: bool) -> Vec<u8> {
    let points = [
        (0, 0, 0),
        (20, 0, 0),
        (20, 20, 0),
        (0, 20, 0),
        (0, 0, 20),
        (20, 0, 20),
        (20, 20, 20),
        (0, 20, 20),
    ];
    let faces = [
        (0, 2, 1),
        (0, 3, 2),
        (4, 5, 6),
        (4, 6, 7),
        (0, 1, 5),
        (0, 5, 4),
        (1, 2, 6),
        (1, 6, 5),
        (2, 3, 7),
        (2, 7, 6),
        (3, 0, 4),
        (3, 4, 7),
    ];
    let vertices = points
        .iter()
        .map(|(x, y, z)| format!("<vertex x=\"{x}\" y=\"{y}\" z=\"{z}\"/>"))
        .collect::<String>();
    let triangles = faces
        .iter()
        .enumerate()
        .map(|(i, (a, b, c))| {
            format!(
                "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"{}/>",
                if painted && i == 0 {
                    " paint_color=\"0C\""
                } else {
                    ""
                }
            )
        })
        .collect::<String>();
    let core = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
    let production = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
    let part = format!(
        "<model unit=\"millimeter\" xmlns=\"{core}\"><resources><object id=\"1\"><mesh><vertices>{vertices}</vertices><triangles>{triangles}</triangles></mesh></object></resources></model>"
    );
    let components = "<component p:path=\"/3D/Objects/part.model\" objectid=\"1\"/><component p:path=\"/3D/Objects/part.model\" objectid=\"1\" transform=\"0.05 0 0 0 0.25 0 0 0 0.05 22 1 0\"/>";
    let build = format!(
        "<item objectid=\"10\"/>{}",
        if multiple {
            "<item objectid=\"10\" transform=\"1 0 0 0 1 0 0 0 1 300 0 0\"/>"
        } else {
            ""
        }
    );
    let model = format!(
        "<model unit=\"millimeter\" xmlns=\"{core}\" xmlns:p=\"{production}\" requiredextensions=\"p\"><resources><object id=\"10\"><components>{components}</components></object></resources><build>{build}</build></model>"
    );
    let plates=(0..if multiple{2}else{1}).map(|i|format!("<plate><metadata key=\"plater_id\" value=\"{}\"/><metadata key=\"plater_name\" value=\"寸法確認 {}\"/><model_instance><metadata key=\"object_id\" value=\"10\"/><metadata key=\"instance_id\" value=\"{i}\"/></model_instance></plate>",i+1,i+1)).collect::<String>();
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [
        (
            "[Content_Types].xml",
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"start\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\" Target=\"/3D/3dmodel.model\"/></Relationships>",
        ),
        ("3D/3dmodel.model", &model),
        ("3D/Objects/part.model", &part),
        (
            "Metadata/model_settings.config",
            &format!("<config>{plates}</config>"),
        ),
        (
            "Metadata/uninterpreted-extension.xml",
            "<vendor material=\"preserve exactly\"/>",
        ),
        (
            "Metadata/plate_1.gcode",
            "UNTRUSTED SOURCE GCODE - MUST NOT RUN",
        ),
    ] {
        archive
            .start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("in-memory zip accepts a new file");
        archive
            .write_all(data.as_bytes())
            .expect("in-memory zip accepts data");
    }
    archive
        .finish()
        .expect("in-memory zip finishes")
        .into_inner()
}
#[allow(clippy::too_many_lines)]
pub fn model_import(appdir: Option<&Path>, browser: bool) {
    let mut rig = Rig::with_options("model-import", "v3", appdir);
    rig.launch();
    rig.seed();
    rig.stop(false);
    rig.env.remove("SCAD_LIVE_URL");
    rig.launch();
    rig.idle();
    let fixtures = rig.output.join("fixtures");
    fs::create_dir_all(&fixtures).expect("fixture directory is creatable");
    let single = model_fixture(false, false);
    let painted = model_fixture(true, true);
    let multi = model_fixture(false, true);
    for (name, data) in [
        ("single.3mf", &single),
        ("painted.3mf", &painted),
        ("multiple.3mf", &multi),
        ("cube.stl", &fixture("cube.stl")),
        ("bad.3mf", &b"broken zip".to_vec()),
    ] {
        fs::write(fixtures.join(name), data).expect("fixture directory is writable");
    }
    let files = [("multiple.3mf", multi.clone())];
    let info = rig.multipart_json("/api/plates/file-info", &files, &[], 200);
    assert_eq!(array(&info).len(), 2, "file info lists both plates");
    assert!(
        info[1]["print_reason"].is_null(),
        "second plate is printable: {info}"
    );
    let derived = rig.multipart("/api/plates/file-preview", &files, &[("plate", "1")], 200);
    let mesh = stl_io::read_stl(&mut Cursor::new(&derived)).expect("preview is STL");
    assert_eq!(mesh.faces.len(), 24, "preview holds both components");
    let low: Vec<f32> = (0..3)
        .map(|axis| {
            mesh.vertices
                .iter()
                .map(|v| v[axis])
                .fold(f32::INFINITY, f32::min)
        })
        .collect();
    let high: Vec<f32> = (0..3)
        .map(|axis| {
            mesh.vertices
                .iter()
                .map(|v| v[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect();
    assert_eq!(
        low,
        [300.0, 0.0, 0.0],
        "preview lower bound is the second build item"
    );
    assert_eq!(
        high,
        [323.0, 20.0, 20.0],
        "preview upper bound is the second build item"
    );
    let imported = rig.multipart_json(
        "/api/plates/files",
        &files,
        &[
            ("plate", "1"),
            ("name", "Imported geometry"),
            ("quantities", "[2]"),
        ],
        201,
    );
    assert_eq!(
        imported["conditions"],
        rig.get("/api/default-settings")["conditions"],
        "import uses the default conditions"
    );
    assert_eq!(
        imported["imported"]["selection"]["items"][0]["build_index"], 1,
        "import selects the requested plate's build item"
    );
    assert_eq!(
        rig.bytes(&format!("/api/plates/{}/original", id(&imported))),
        multi,
        "original file is kept byte for byte"
    );
    let multicolor = rig.multipart_json(
        "/api/plates/files",
        &[("painted.3mf", painted.clone())],
        &[("plate", "0"), ("name", "Preserved colors")],
        201,
    );
    let q = rig.get(&format!(
        "/api/queue?printer_id=p1&plate_id={}",
        id(&multicolor)
    ));
    assert_eq!(
        q["admission"]["allowed"], false,
        "multicolor plate is not admitted"
    );
    assert!(
        q["admission"]["reason"]
            .as_str()
            .expect("refusal has a reason")
            .contains("多色"),
        "refusal names multicolor: {q}"
    );
    rig.send(serde_json::json!({"type":"add","plate_id":multicolor["id"],"plate_version":multicolor["version"]}),409);
    let before = rig.get("/api/plates");
    for source in [&imported, &multicolor] {
        let copy = rig.post(
            &format!("/api/plates/{}/duplicate", id(source)),
            &serde_json::json!({"name":"Copied import"}),
            201,
        );
        assert_eq!(
            copy["conditions"], source["conditions"],
            "copy keeps conditions"
        );
        assert_eq!(
            copy["imported"]["selection"], source["imported"]["selection"],
            "copy keeps the import selection"
        );
        assert_eq!(
            copy["imported"]["model_id"], copy["models"][0]["id"],
            "copy's import points at its own model"
        );
        assert_ne!(
            copy["models"][0]["id"], source["models"][0]["id"],
            "copy has a new model"
        );
        assert_eq!(
            rig.bytes(&format!("/api/plates/{}/original", id(&copy))),
            rig.bytes(&format!("/api/plates/{}/original", id(source))),
            "copy keeps the original file"
        );
        assert_eq!(
            rig.bytes(&format!(
                "/api/plates/{}/files/{}",
                id(&copy),
                id(&copy["models"][0])
            )),
            rig.bytes(&format!(
                "/api/plates/{}/files/{}",
                id(source),
                id(&source["models"][0])
            )),
            "copy keeps the model file"
        );
        rig.request("DELETE", &format!("/api/plates/{}", id(&copy)), None, 204);
        assert_eq!(
            rig.get(&format!("/api/plates/{}", id(source))),
            *source,
            "deleting the copy leaves the source"
        );
    }
    rig.multipart(
        "/api/plates/files",
        &[("bad.3mf", b"invalid".to_vec())],
        &[("name", "Invalid")],
        400,
    );
    assert_eq!(
        rig.get("/api/plates"),
        before,
        "invalid upload creates no plate"
    );
    rig.stop(false);
    rig.launch();
    rig.idle();
    assert_eq!(
        rig.bytes(&format!("/api/plates/{}/original", id(&multicolor))),
        painted,
        "original survives a restart"
    );
    let job=array(&rig.send(serde_json::json!({"type":"add","plate_id":imported["id"],"plate_version":imported["version"]}),200)["waiting"]).last().expect("added job is waiting").clone();
    until(
        || {
            let estimate = rig.waiting(&job)["estimate"].clone();
            assert_ne!(estimate["state"], "failed", "{estimate}");
            estimate["state"] == "ready"
        },
        120,
    );
    let saved: Value =
        serde_json::from_str(&rig.stored(&job, "estimate_json")).expect("stored estimate is JSON");
    let process = &saved["input"]["settings"]["profiles"]["process.json"];
    assert_eq!(process["enable_support"], "0", "import disables support");
    assert_eq!(process["brim_type"], "no_brim", "import disables brim");
    assert!(
        !saved.to_string().contains("UNTRUSTED SOURCE GCODE"),
        "estimate input excludes source G-code"
    );
    if appdir.is_some() {
        let bundle = rig.cached_artifact(&job, "gcode");
        fs::write(rig.output.join("resliced.gcode.3mf"), &bundle)
            .expect("output directory is writable");
        assert!(
            !String::from_utf8(zip_read(&bundle, "Metadata/plate_1.gcode"))
                .expect("G-code is UTF-8")
                .contains("UNTRUSTED SOURCE GCODE"),
            "resliced bundle excludes source G-code"
        );
        let files = [("official.3mf", bundle)];
        let produced = rig.multipart_json("/api/plates/file-info", &files, &[], 200);
        assert_eq!(array(&produced).len(), 1, "official bundle has one plate");
        assert!(
            produced[0]["print_reason"].is_null(),
            "official bundle is printable: {produced}"
        );
        let roundtrip = rig.multipart("/api/plates/file-preview", &files, &[], 200);
        assert_eq!(
            stl_io::read_stl(&mut Cursor::new(&roundtrip))
                .expect("preview is STL")
                .faces
                .len(),
            48,
            "official bundle preview holds the model"
        );
        fs::write(rig.output.join("resliced-preview.stl"), roundtrip)
            .expect("output directory is writable");
    }
    if browser {
        rig.browser(
            "E2E_IMPORT_CONTEXT",
            &serde_json::json!({"fixtures":fixtures}),
            None,
        );
    }
    assert!(
        rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty(),
        "import does not print"
    );
}
