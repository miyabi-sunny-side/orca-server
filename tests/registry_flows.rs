mod common;
#[path = "scenarios/registry.rs"]
mod scenario;
#[test]
#[ignore = "requires official Orca 2.4.2; full local verification runs this target"]
fn independent_printers() {
    let appdir =
        std::path::PathBuf::from(std::env::var_os("ORCA_APPDIR").expect("ORCA_APPDIR required"));
    scenario::registry(Some(&appdir), false);
}
#[test]
fn filament_ams() {
    scenario::filament_ams(None, false);
}
