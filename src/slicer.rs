use crate::{
    artifacts,
    plate_api::blocking,
    plates::{Error, Result},
    profiles::{Profiles, Selection},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::Semaphore,
};

#[derive(Clone)]
pub struct Slicer {
    binary: PathBuf,
    pub(crate) profiles: Arc<Profiles>,
    timeout: Duration,
    // ponytail: one CLI job per server; add a bounded queue only when needed.
    slot: Arc<Semaphore>,
}

impl Slicer {
    /// Loads profiles and checks the executable from an extracted `OrcaSlicer` 2.4.2 `AppImage`.
    ///
    /// # Errors
    /// Rejects missing files, incompatible versions and an unusable CLI.
    pub async fn new(appdir: PathBuf, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() || timeout > Duration::from_hours(1) {
            return Err(Error::Invalid("OrcaSlicer timeout must be 1–3600 seconds"));
        }
        let appdir = std::fs::canonicalize(appdir)?;
        let profiles = Profiles::load(&appdir.join("resources/profiles/BBL"))?;
        let scratch = tempfile::tempdir()?;
        let slicer = Self {
            binary: appdir.join("AppRun"),
            profiles: Arc::new(profiles),
            timeout,
            slot: Arc::new(Semaphore::new(1)),
        };
        let help = slicer
            .run(scratch.path(), vec!["--help".into()], "startup", "version")
            .await?;
        if !help.lines().any(|line| line.trim() == "OrcaSlicer-2.4.2:") {
            return Err(Error::Unavailable("OrcaSlicer 2.4.2 is required"));
        }
        // Fail at startup if the bundled defaults cannot be resolved.
        slicer
            .profiles
            .write(&Selection::default(), scratch.path())?;
        Ok(slicer)
    }

    /// Arrange and slice the frozen inputs and resolved profiles in an execution directory.
    /// # Errors
    /// Rejects CLI failures, timeouts and incomplete or mismatched outputs.
    pub(crate) async fn slice(
        &self,
        directory: PathBuf,
        count: usize,
        selection: Selection,
        id: &str,
    ) -> Result<()> {
        let _permit = self
            .slot
            .acquire()
            .await
            .map_err(|_| Error::Unavailable("Slicer stopped"))?;
        self.run(
            &directory,
            arrange_args(&selection.bed, count),
            id,
            "arrange",
        )
        .await?;
        let path = directory.clone();
        let settings = selection.clone();
        blocking(move || artifacts::validate(&path.join("project.3mf"), count, &settings, false))
            .await?;
        self.run(&directory, slice_args(), id, "slice").await?;
        blocking(move || {
            artifacts::validate(&directory.join("print.gcode.3mf"), count, &selection, true)
        })
        .await
    }

    async fn run(
        &self,
        directory: &Path,
        args: Vec<OsString>,
        plate: &str,
        phase: &str,
    ) -> Result<String> {
        let mut child = Command::new(&self.binary)
            .args(&args)
            .current_dir(directory)
            .env_remove("APPDIR")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                tracing::error!(%plate, %phase, %error, "cannot start OrcaSlicer");
                Error::Unavailable("Cannot start OrcaSlicer; check server logs")
            })?;
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let result = tokio::time::timeout(self.timeout, async {
            tokio::join!(
                child.wait(),
                drain(stdout, plate, phase),
                drain(stderr, plate, phase)
            )
        })
        .await;
        let Ok((status, stdout, stderr)) = result else {
            child.kill().await?;
            tracing::error!(%plate, %phase, "OrcaSlicer timed out and was killed");
            return Err(Error::Timeout);
        };
        let status = status?;
        let stdout = stdout?;
        let _ = stderr?;
        tracing::info!(%plate, %phase, %status, "OrcaSlicer exited");
        if !status.success() {
            return Err(Error::Upstream("OrcaSlicer failed; check server logs"));
        }
        Ok(stdout)
    }
}

async fn drain(
    mut stream: impl AsyncRead + Unpin,
    plate: &str,
    phase: &str,
) -> std::io::Result<String> {
    let mut captured = Vec::new();
    let mut chunk = [0; 4096];
    let mut logged = 0;
    loop {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        let keep = count.min((64usize * 1024).saturating_sub(captured.len()));
        captured.extend_from_slice(&chunk[..keep]);
        if logged < 512 * 1024 {
            let count = count.min(512 * 1024 - logged);
            if phase == "version" {
                tracing::debug!(output = %String::from_utf8_lossy(&chunk[..count]), "OrcaSlicer version help");
            } else {
                tracing::info!(%plate, %phase, output = %String::from_utf8_lossy(&chunk[..count]), "OrcaSlicer output");
            }
            logged += count;
            if logged >= 512 * 1024 {
                tracing::warn!(%plate, %phase, "further CLI output suppressed");
            }
        }
    }
    Ok(String::from_utf8_lossy(&captured).into_owned())
}

fn arrange_args(bed: &str, models: usize) -> Vec<OsString> {
    let mut args: Vec<_> = [
        "--datadir",
        "data",
        "--load-settings",
        "printer.json;process.json",
        "--load-filaments",
        "filament.json",
        "--curr-bed-type",
        bed,
        "--arrange",
        "1",
        "--export-3mf",
        "project.3mf",
        "--outputdir",
        ".",
    ]
    .map(OsString::from)
    .into();
    args.extend((0..models).map(|index| OsString::from(format!("{index}.stl"))));
    args
}
fn slice_args() -> Vec<OsString> {
    [
        "--datadir",
        "data",
        "--slice",
        "1",
        "--export-3mf",
        "print.gcode.3mf",
        "--outputdir",
        ".",
        "project.3mf",
    ]
    .map(OsString::from)
    .into()
}

pub(crate) fn router(slicer: Option<Slicer>) -> Router {
    Router::new()
        .route("/api/slicer/profiles", get(choices))
        .route("/api/slicer/process", get(process))
        .with_state(slicer)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessQuery {
    machine: String,
    process: String,
    sparse_infill_pattern: Option<String>,
    sparse_infill_density: Option<f64>,
    wall_loops: Option<u32>,
}
async fn process(
    State(slicer): State<Option<Slicer>>,
    Query(query): Query<ProcessQuery>,
) -> Result<Json<serde_json::Map<String, serde_json::Value>>> {
    let mut values = slicer
        .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
        .profiles
        .resolve_process(
            &query.machine,
            &query.process,
            &crate::strength::Strength {
                sparse_infill_pattern: query.sparse_infill_pattern,
                sparse_infill_density: query.sparse_infill_density,
                wall_loops: query.wall_loops,
            },
        )?;
    values.retain(|key, _| {
        [
            "sparse_infill_pattern",
            "sparse_infill_density",
            "wall_loops",
            "top_shell_layers",
            "bottom_shell_layers",
            "top_shell_thickness",
            "bottom_shell_thickness",
        ]
        .contains(&key.as_str())
    });
    Ok(Json(values))
}
#[derive(serde::Deserialize)]
struct ProfileQuery {
    machine: Option<String>,
}
async fn choices(
    State(slicer): State<Option<Slicer>>,
    Query(query): Query<ProfileQuery>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        slicer
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
            .profiles
            .choices_for(query.machine.as_deref().unwrap_or(crate::profiles::PRINTER))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cli_exit_timeout_and_busy_leave_the_saved_plate_readable() {
        use crate::profiles::PRINTER;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let appdir = root.path().join("app");
        let defaults = Selection::default();
        for (category, name) in [
            ("machine", PRINTER),
            ("process", &defaults.process),
            ("filament", &defaults.filament),
        ] {
            let directory = appdir.join("resources/profiles/BBL").join(category);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("profile.json"), serde_json::json!({"name":name, "instantiation":"true", "nozzle_diameter":["0.4"], "compatible_printers":[PRINTER]}).to_string()).unwrap();
        }
        let binary = appdir.join("AppRun");
        std::fs::write(&binary, "#!/bin/sh\nprintf 'OrcaSlicer-2.4.2:\\n'\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let slicer = Slicer::new(appdir.clone(), Duration::from_millis(100))
            .await
            .unwrap();
        let directory = root.path().join("execution");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(&binary, "#!/bin/sh\nexit 7\n").unwrap();
        assert!(matches!(
            slicer
                .slice(directory.clone(), 1, defaults.clone(), "test")
                .await,
            Err(Error::Upstream(_))
        ));
        std::fs::write(&binary, "#!/bin/sh\nexec sleep 30\n").unwrap();
        assert!(matches!(
            slicer
                .slice(directory.clone(), 1, defaults.clone(), "test")
                .await,
            Err(Error::Timeout)
        ));
        assert_eq!(slicer.slot.available_permits(), 1);
        std::fs::write(&binary, "#!/bin/sh\nprintf 'OrcaSlicer-2.4.1:\\n'\n").unwrap();
        assert!(matches!(
            Slicer::new(appdir, Duration::from_secs(1)).await,
            Err(Error::Unavailable(_))
        ));
    }

    #[test]
    fn independent_inputs_are_arranged_and_reslice_uses_only_saved_project() {
        let args = arrange_args("Textured PEI Plate", 2);
        let args: Vec<_> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|a| a == ["--arrange", "1"]));
        assert!(
            args.windows(2)
                .any(|a| a == ["--curr-bed-type", "Textured PEI Plate"])
        );
        assert_eq!(&args[args.len() - 2..], ["0.stl", "1.stl"]);
        assert!(!args.contains(&"--assemble"));
        let slice = slice_args();
        assert_eq!(
            slice,
            [
                "--datadir",
                "data",
                "--slice",
                "1",
                "--export-3mf",
                "print.gcode.3mf",
                "--outputdir",
                ".",
                "project.3mf"
            ]
            .map(OsString::from)
        );
    }
}
