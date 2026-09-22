#[path = "common/artifact.rs"]
mod artifact;
mod common;
#[path = "scenarios/cli.rs"]
mod scenario;
fn appdir() -> std::path::PathBuf {
    std::env::var_os("ORCA_APPDIR")
        .expect("ORCA_APPDIR must point to official Orca 2.4.2 AppDir")
        .into()
}
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn layout() {
    scenario::layout(&appdir());
}
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn estimates() {
    scenario::estimates(&appdir());
}
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn strength() {
    scenario::strength(&appdir());
}
#[path = "scenarios/import.rs"]
mod import_scenario;
#[path = "scenarios/support_cli.rs"]
mod support_scenario;
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn support() {
    support_scenario::support(common::Rig::with_options(
        "official-support",
        "v3",
        Some(&appdir()),
    ));
}
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn imported_geometry() {
    import_scenario::model_import(Some(&appdir()), false);
}
