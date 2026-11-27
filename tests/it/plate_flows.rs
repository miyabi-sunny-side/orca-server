use crate::scenarios::plates;
#[test]
fn plate_admission() {
    plates::plate_admission();
}
#[test]
fn creation_defaults() {
    plates::creation_defaults();
}
#[test]
fn recover_default_process() {
    plates::recover_default_process();
}

#[test]
fn filament_picker() {
    plates::filament_picker();
}

#[test]
fn plate_duplication() {
    plates::plate_duplication();
}

#[test]
fn plate_archive() {
    plates::plate_archive();
}
