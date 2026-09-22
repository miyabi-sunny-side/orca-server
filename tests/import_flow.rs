mod common;
#[path = "scenarios/import.rs"]
mod scenario;
#[test]
fn model_import() {
    scenario::model_import(None, false);
}
