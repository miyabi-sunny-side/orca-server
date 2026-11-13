//! Every integration test in one binary. Each test owns its store, ports and processes, so they
//! all run in parallel, and the shared fixtures are compiled and linked once.
#[path = "common/artifact.rs"]
mod artifact;
mod common;
mod scenarios {
    pub mod ams_nozzle;
    pub mod cli;
    pub mod compact;
    pub mod import;
    pub mod plates;
    pub mod registry;
    pub mod strength_support;
    pub mod support_cli;
}

mod a1_fleet;
mod ams_nozzle;
mod ams_refresh;
mod artifact_transform;
mod browser;
mod camera;
mod cleanup;
mod codes;
mod compact_queue;
mod container;
mod controls;
mod diagnostics;
mod external_spool;
mod history;
mod import_flow;
mod journal;
mod material_roles;
mod material_settings;
mod mcp;
mod model_files;
mod official_cli;
mod plate_flows;
mod plate_slices;
mod plates;
mod port;
mod protocols;
mod queue_lifecycle;
mod registry_flows;
mod release_notes;
mod storage;
