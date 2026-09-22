mod common;
#[path = "scenarios/ams_nozzle.rs"]
mod scenarios;
#[test]
fn ams_priority() {
    scenarios::ams_priority(false);
}
#[test]
fn nozzle_material() {
    scenarios::nozzle_material();
}
