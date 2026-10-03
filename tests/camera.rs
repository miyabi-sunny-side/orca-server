mod common;
use common::peers::{SECRET, config, tls};
use common::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

/// One camera connection answered by `reply` after checking the login, as the P1S does.
fn camera(rig: &Rig, reply: Vec<u8>) -> (u16, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = config(
        &rig.root.path().join("trusted.pem"),
        &rig.root.path().join("trusted.key"),
    );
    let (logins, received) = mpsc::channel();
    std::thread::spawn(move || {
        for raw in listener.incoming() {
            let Ok(mut stream) = tls(raw.unwrap(), &config) else {
                continue;
            };
            let mut login = vec![0u8; 80];
            if stream.read_exact(&mut login).is_ok() {
                logins.send(login).unwrap();
                let _ = stream.write_all(&reply);
                let _ = stream.flush();
            }
        }
    });
    (port, received)
}
fn frame(image: &[u8]) -> Vec<u8> {
    let mut frame = u32::try_from(image.len()).unwrap().to_le_bytes().to_vec();
    frame.extend([0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    frame.extend(image);
    frame
}

#[test]
fn the_camera_returns_one_jpeg_and_failures_name_no_secret() {
    let mut rig = Rig::new("camera");
    let jpeg = [0xff, 0xd8, 0xff, 0xe0, 7, 7, 7, 0xff, 0xd9];
    let (port, logins) = camera(&rig, frame(&jpeg));
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
    let (port, _) = camera(&rig, frame(b"not a jpeg"));
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
