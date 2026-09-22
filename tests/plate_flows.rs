mod common;
#[path = "scenarios/plates.rs"]
mod plates;
#[test]
fn plate_admission() {
    plates::plate_admission(false);
}
#[test]
fn creation_defaults() {
    plates::creation_defaults(false);
}
#[test]
fn recover_default_process() {
    plates::recover_default_process();
}

#[test]
fn filament_picker() {
    plates::filament_picker(false);
}
