mod common;
use common::peers::{SECRET, camera, frame};
use common::*;

#[test]
fn the_camera_returns_one_jpeg_and_failures_name_no_secret() {
    let mut rig = Rig::new("camera");
    let jpeg = [0xff, 0xd8, 0xff, 0xe0, 7, 7, 7, 0xff, 0xd9];
    let (port, logins) = camera(rig.root.path(), frame(&jpeg));
    rig.env.insert("P1_CAMERA_PORT".into(), port.to_string());
    rig.launch();
    let response = rig
        .http
        .get(format!("{}/api/printers/p1/camera", rig.base))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.bytes().unwrap().as_ref(), jpeg);
    let login = logins.recv().unwrap();
    assert_eq!(&login[16..20], b"bblp");
    assert_eq!(&login[48..48 + SECRET.len()], SECRET.as_bytes());

    rig.stop(false);
    let (port, _) = camera(rig.root.path(), frame(b"not a jpeg"));
    rig.db()
        .execute("UPDATE printers SET camera_port=?1", [port])
        .unwrap();
    rig.launch();
    let failed = rig.request("GET", "/api/printers/p1/camera", None, 502);
    assert!(!failed.to_string().contains(SECRET));
    let journal = rig.get("/api/journal?kind=camera");
    assert_eq!(array(&journal["entries"]).len(), 2);
    assert_eq!(journal["entries"][1]["body"]["failed_stage"], "frame");
    assert!(!journal.to_string().contains(SECRET));
    rig.request("GET", "/api/printers/missing/camera", None, 404);
    rig.check();
}
