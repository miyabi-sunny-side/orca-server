use crate::scenarios::strength_support as settings;
#[test]
fn strength_settings() {
    settings::strength_settings(false);
}
#[test]
fn support_interface() {
    settings::support_interface(false);
}
#[test]
fn legacy_support() {
    settings::legacy_support();
}
