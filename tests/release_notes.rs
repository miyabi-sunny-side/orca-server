//! Execute the operational Python command; the tests and assertions themselves are Rust.
use std::{fs, process::Command};
fn update(path: &std::path::Path) -> std::process::Output {
    Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/packaging/release_notes.py"
        ))
        .arg(path)
        .arg("ghcr.io/example/app:0.1.0@sha256:new")
        .output()
        .unwrap()
}
#[test]
fn changes_only_leading_image_and_preserves_changelog() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.md");
    let body = "Image: `ghcr.io/example/app:0.1.0@sha256:old` · [Source](https://example.org/source)\n\n**Changes**\n- Keep `sha256:old` here.\n";
    let expected = "Image: `ghcr.io/example/app:0.1.0@sha256:new` · [Source](https://example.org/source)\n\n**Changes**\n- Keep `sha256:old` here.\n";
    fs::write(&path, body).unwrap();
    assert!(update(&path).status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert!(update(&path).status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
}
#[test]
fn rejects_unrecognized_notes_without_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.md");
    for body in [
        "",
        "Custom notes\nImage: `manual`",
        "Image: missing delimiter",
    ] {
        fs::write(&path, body).unwrap();
        let output = update(&path);
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
    }
}
