mod common;
use common::*;
use std::{
    fs,
    net::TcpListener,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
};
#[test]
fn panic_reaps_owned_processes_and_releases_ports_and_store() {
    let mut root = PathBuf::new();
    let mut ports = Vec::new();
    let mut processes = Vec::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut rig = Rig::new("panic-cleanup");
        root = rig.root.path().to_owned();
        ports.extend([
            rig.broker.port,
            rig.ftp.port,
            reqwest::Url::parse(&rig.scad.base).unwrap().port().unwrap(),
            reqwest::Url::parse(&rig.base).unwrap().port().unwrap(),
        ]);
        rig.launch();
        rig.seed();
        rig.hold(true);
        rig.add(3);
        until(|| !rig.traces().is_empty(), 10);
        let pid = rig.process.as_ref().unwrap().id();
        processes.push(pid);
        for task in fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
            let children = fs::read_to_string(task.unwrap().path().join("children")).unwrap();
            processes.extend(
                children
                    .split_whitespace()
                    .map(|p| p.parse::<u32>().unwrap()),
            );
        }
        assert!(processes.len() > 1, "held CLI child must exist");
        panic!("intentional fixture panic");
    }));
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"intentional fixture panic")
    );
    assert!(!root.exists());
    for port in ports {
        TcpListener::bind(("127.0.0.1", port)).expect("owned listener leaked after panic");
    }
    until(
        || {
            processes
                .iter()
                .all(|pid| !PathBuf::from(format!("/proc/{pid}")).exists())
        },
        10,
    );
}
