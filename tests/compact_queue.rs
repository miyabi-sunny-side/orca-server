mod common;
#[path = "scenarios/compact.rs"]
mod scenario;
#[test]
fn compact_queue() {
    scenario::compact_queue(false);
}
