use crate::scenarios::cli as scenario;
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
fn printer_fit() {
    scenario::printer_fit(&appdir());
}
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn strength() {
    scenario::strength(&appdir());
}
use crate::scenarios::import as import_scenario;
use crate::scenarios::support_cli as support_scenario;
#[test]
#[ignore = "requires official Orca 2.4.2"]
fn support() {
    support_scenario::support(crate::common::Rig::with_options(
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
