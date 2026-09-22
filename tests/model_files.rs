#![allow(clippy::format_collect, clippy::too_many_lines)] // Small XML fixture and one atomic import/recovery scenario.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use orca_server::plates::Store;
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use tower::ServiceExt;

fn three_mf(painted: bool) -> Vec<u8> {
    let mesh = stl_io::read_stl(&mut Cursor::new(include_bytes!("fixtures/cube.stl"))).unwrap();
    let vertices = mesh
        .vertices
        .iter()
        .map(|v| format!(r#"<vertex x="{}" y="{}" z="{}"/>"#, v[0], v[1], v[2]))
        .collect::<String>();
    let triangles = mesh
        .faces
        .iter()
        .map(|f| {
            format!(
                r#"<triangle v1="{}" v2="{}" v3="{}" {}/>"#,
                f.vertices[0],
                f.vertices[1],
                f.vertices[2],
                if painted { r#"paint_color="0C""# } else { "" }
            )
        })
        .collect::<String>();
    let model = format!(
        r#"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices>{vertices}</vertices><triangles>{triangles}</triangles></mesh></object></resources><build><item objectid="1"/><item objectid="1" transform="1 0 0 0 1 0 0 0 1 300 0 0"/></build></model>"#
    );
    let settings = r#"<config><plate><metadata key="plater_name" value="Cube A"/><metadata key="plater_id" value="1"/><model_instance><metadata key="object_id" value="1"/><metadata key="instance_id" value="0"/></model_instance></plate><plate><metadata key="plater_name" value="Cube B"/><metadata key="plater_id" value="2"/><model_instance><metadata key="object_id" value="1"/><metadata key="instance_id" value="1"/></model_instance></plate></config>"#;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in [
        ("3D/3dmodel.model", model.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="start" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/></Relationships>"#,
        ),
        ("Metadata/model_settings.config", settings),
        (
            "Metadata/vendor-extension.xml",
            "<preserve arbitrary='true'/>",
        ),
        ("Metadata/plate_1.gcode", "UNTRUSTED GCODE - NEVER RUN"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn upload(path: &str, files: &[(&str, &[u8])], values: &[(&str, &str)]) -> Request<Body> {
    let mut body = Vec::new();
    for (name, value) in values {
        write!(
            body,
            "--orca\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .unwrap();
    }
    for (name, bytes) in files {
        write!(body,"--orca\r\nContent-Disposition: form-data; name=\"models\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").unwrap();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--orca--\r\n");
    Request::post(path)
        .header("content-type", "multipart/form-data; boundary=orca")
        .body(Body::from(body))
        .unwrap()
}
async fn json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn external_files_save_atomically_with_exact_original_and_selected_mapping() {
    let root = tempfile::tempdir().unwrap();
    let app = orca_server::app_with_store(Store::open(root.path()).unwrap());
    let source = three_mf(true);
    let files = [("Painted.3mf", source.as_slice())];
    let info = app
        .clone()
        .oneshot(upload("/api/plates/file-info", &files, &[]))
        .await
        .unwrap();
    assert_eq!(info.status(), StatusCode::OK);
    let info = json(info).await;
    assert_eq!(info[1]["name"], "Cube B");
    assert!(info[1]["print_reason"].is_string());
    let preview = app
        .clone()
        .oneshot(upload(
            "/api/plates/file-preview",
            &files,
            &[("plate", "1")],
        ))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let derived = to_bytes(preview.into_body(), usize::MAX).await.unwrap();
    let mesh = stl_io::read_stl(&mut Cursor::new(&derived)).unwrap();
    assert_eq!(mesh.faces.len(), 12);
    assert!(mesh.vertices.iter().all(|v| v[0] >= 300.0 && v[0] <= 320.0));
    let saved = app
        .clone()
        .oneshot(upload(
            "/api/plates/files",
            &files,
            &[("plate", "1"), ("name", "Imported"), ("quantities", "[2]")],
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::CREATED);
    let saved = json(saved).await;
    assert_eq!(saved["models"][0]["quantity"], 2);
    assert_eq!(saved["imported"]["selection"]["items"][0]["build_index"], 1);
    assert_eq!(saved["imported"]["file_name"], "Painted.3mf");
    let id = saved["id"].as_str().unwrap();
    for (path, expected) in [
        (format!("/api/plates/{id}/original"), source.as_slice()),
        (
            format!(
                "/api/plates/{id}/files/{}",
                saved["models"][0]["id"].as_str().unwrap()
            ),
            derived.as_ref(),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            expected
        );
    }
    let edit = json!({"name":"Edited","version":saved["version"],"conditions":saved["conditions"],"models":saved["models"]});
    let response = app
        .clone()
        .oneshot(
            Request::put(format!("/api/plates/{id}"))
                .header("content-type", "application/json")
                .body(Body::from(edit.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["imported"], saved["imported"]);
    let store = Store::open(root.path()).unwrap();
    let reloaded = serde_json::to_value(store.get(id).unwrap()).unwrap();
    assert_eq!(reloaded["imported"], saved["imported"]);
    for (files, values) in [
        (files.to_vec(), vec![("plate", "99"), ("name", "Invalid")]),
        (
            vec![("broken.3mf", b"invalid".as_slice())],
            vec![("name", "Invalid")],
        ),
        (
            vec![
                ("valid.stl", include_bytes!("fixtures/cube.stl").as_slice()),
                ("broken.stl", b"invalid".as_slice()),
            ],
            vec![("name", "Invalid")],
        ),
        (
            files.to_vec(),
            vec![("plate", "0"), ("name", "Invalid"), ("quantities", "[0]")],
        ),
    ] {
        let response = app
            .clone()
            .oneshot(upload("/api/plates/files", &files, &values))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(store.list("").unwrap().len(), 1);
    }
    let stls = [
        ("first.stl", include_bytes!("fixtures/cube.stl").as_slice()),
        ("second.stl", include_bytes!("fixtures/cube.stl").as_slice()),
    ];
    let saved = app
        .oneshot(upload(
            "/api/plates/files",
            &stls,
            &[("name", "Two STLs"), ("quantities", "[2,3]")],
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::CREATED);
    let saved = json(saved).await;
    assert_eq!(saved["models"][0]["quantity"], 2);
    assert_eq!(saved["models"][1]["quantity"], 3);
    assert!(saved.get("imported").is_none());
}
