mod common;
use common::peers::SECRET;
use common::*;
use serde_json::json;

#[test]
fn printer_storage_lists_downloads_and_deletes_files() {
    let mut rig = Rig::new("storage");
    {
        let mut records = rig.ftp.records.lock().unwrap();
        records
            .files
            .insert("/timelapse/video_1.mp4".into(), vec![1, 2, 3, 4]);
        records
            .files
            .insert("/model/part.3mf".into(), b"model".to_vec());
    }
    rig.launch();
    rig.idle();
    let root = rig.get("/api/printers/p1/files");
    let names: Vec<_> = array(&root["entries"])
        .iter()
        .map(|e| e["name"].clone())
        .collect();
    assert_eq!(names, [json!("model"), json!("timelapse")]);
    assert!(
        array(&root["entries"])
            .iter()
            .all(|e| e["directory"] == true)
    );
    let videos = rig.get("/api/printers/p1/files?path=/timelapse");
    assert_eq!(videos["entries"][0]["path"], "/timelapse/video_1.mp4");
    assert_eq!(videos["entries"][0]["size"], 4);

    let response = rig
        .http
        .get(format!(
            "{}/api/printers/p1/files/content?path=/timelapse/video_1.mp4",
            rig.base
        ))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"video_1.mp4\""
    );
    assert_eq!(response.bytes().unwrap().as_ref(), [1, 2, 3, 4]);

    rig.request(
        "DELETE",
        "/api/printers/p1/files?path=/timelapse/video_1.mp4",
        None,
        204,
    );
    assert!(
        !rig.ftp
            .records
            .lock()
            .unwrap()
            .files
            .contains_key("/timelapse/video_1.mp4")
    );
    rig.request(
        "DELETE",
        "/api/printers/p1/files?path=/timelapse/video_1.mp4",
        None,
        404,
    );
    rig.request(
        "GET",
        "/api/printers/p1/files/content?path=/missing.mp4",
        None,
        404,
    );
    for bad in ["/../x", "relative", "/a//b"] {
        rig.request(
            "GET",
            &format!("/api/printers/p1/files?path={bad}"),
            None,
            400,
        );
    }
    let journal = rig.get("/api/journal?kind=ftps");
    assert!(!journal.to_string().contains(SECRET));
    assert!(
        array(&journal["entries"])
            .iter()
            .any(|e| e["body"]["event"] == "delete")
    );
    rig.check();
}
