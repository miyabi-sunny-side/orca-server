use crate::{
    artifacts,
    plate_api::blocking,
    plates::{Error, Plate, Result, Store},
    profiles::{Profiles, Selection},
};
use axum::{
    Json, Router,
    extract::{Path as UrlPath, Query, State},
    routing::{get, post},
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

    /// Arranges and slices a snapshot, publishing both artifacts only on success.
    ///
    /// # Errors
    /// Rejects concurrent work, invalid settings, CLI failures and stale revisions.
    pub async fn slice(&self, store: Store, id: String) -> Result<Plate> {
        let _permit = self
            .slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Conflict("OrcaSlicer is busy; retry after the current job"))?;
        let profiles = self.profiles.clone();
        let source = store.clone();
        let (mut plate, selection, scratch) = blocking(move || {
            let plate = source.get(&id)?;
            let selection: Selection = serde_json::from_value(
                plate
                    .settings
                    .get("slicer")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            )
            .map_err(|_| Error::Invalid("Invalid slicer settings"))?;
            let scratch = tempfile::tempdir()?;
            profiles.write(&selection, scratch.path())?;
            for (index, model) in plate.models.iter().enumerate() {
                std::fs::write(
                    scratch.path().join(format!("{index}.stl")),
                    source.read_file(&id, &model.path)?,
                )?;
            }
            Ok((plate, selection, scratch))
        })
        .await?;
        tracing::info!(plate = %plate.id, revision = %plate.revision, "arranging plate");
        self.run(
            scratch.path(),
            arrange_args(&selection.bed, plate.models.len()),
            &plate.id,
            "arrange",
        )
        .await?;
        let path = scratch.path().to_owned();
        let count = plate.models.len();
        let settings = selection.clone();
        blocking(move || artifacts::validate(&path.join("project.3mf"), count, &settings, false))
            .await?;
        tracing::info!(plate = %plate.id, "slicing saved project");
        self.run(scratch.path(), slice_args(), &plate.id, "slice")
            .await?;
        plate.settings["slicer"] =
            serde_json::to_value(&selection).map_err(std::io::Error::other)?;
        blocking(move || {
            artifacts::validate(
                &scratch.path().join("print.gcode.3mf"),
                count,
                &selection,
                true,
            )?;
            let saved = store.save_artifacts(&plate, scratch.path())?;
            tracing::info!(plate = %saved.id, revision = %saved.revision, "slice saved");
            Ok(saved)
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

pub(crate) fn router(store: Store, slicer: Option<Slicer>) -> Router {
    Router::new()
        .route("/api/slicer/profiles", get(choices))
        .route("/api/plates/{id}/slice", post(slice))
        .with_state((store, slicer))
}
#[derive(serde::Deserialize)]
struct ProfileQuery {
    machine: Option<String>,
}

async fn choices(
    State((_, slicer)): State<(Store, Option<Slicer>)>,
    Query(query): Query<ProfileQuery>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        slicer
            .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
            .profiles
            .choices_for(query.machine.as_deref().unwrap_or(crate::profiles::PRINTER))?,
    ))
}
async fn slice(
    State((store, slicer)): State<(Store, Option<Slicer>)>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Plate>> {
    slicer
        .ok_or(Error::Unavailable("OrcaSlicer is not configured"))?
        .slice(store, id)
        .await
        .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cli_exit_timeout_and_busy_leave_the_saved_plate_readable() {
        use crate::{
            plates::{Input, ModelInput},
            profiles::PRINTER,
        };
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
        let store = Store::open(root.path().join("plates")).unwrap();
        let before = store
            .save(
                None,
                Input {
                    name: "saved".into(),
                    models: vec![ModelInput {
                        name: "cube.stl".into(),
                        source: None,
                        data: include_bytes!("../tests/fixtures/cube.stl").to_vec(),
                    }],
                    settings: serde_json::json!({}),
                },
            )
            .unwrap();
        let permit = slicer.slot.acquire().await.unwrap();
        assert!(matches!(
            slicer.slice(store.clone(), before.id.clone()).await,
            Err(Error::Conflict(_))
        ));
        drop(permit);
        std::fs::write(&binary, "#!/bin/sh\nexit 7\n").unwrap();
        assert!(matches!(
            slicer.slice(store.clone(), before.id.clone()).await,
            Err(Error::Upstream(_))
        ));
        std::fs::write(&binary, "#!/bin/sh\nexec sleep 30\n").unwrap();
        assert!(matches!(
            slicer.slice(store.clone(), before.id.clone()).await,
            Err(Error::Timeout)
        ));
        assert_eq!(slicer.slot.available_permits(), 1);
        assert_eq!(store.get(&before.id).unwrap(), before);
        std::fs::write(&binary, "#!/bin/sh\nprintf 'OrcaSlicer-2.4.1:\\n'\n").unwrap();
        assert!(matches!(
            Slicer::new(appdir, Duration::from_secs(1)).await,
            Err(Error::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn disabled_slicer_and_cross_origin_requests_have_explicit_errors() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let root = tempfile::tempdir().unwrap();
        let app = crate::app_with_store(Store::open(root.path()).unwrap());
        for (method, path) in [
            ("GET", "/api/slicer/profiles"),
            ("POST", "/api/plates/missing/slice"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/plates/missing/slice")
                    .header("Origin", "https://elsewhere.invalid")
                    .header("Host", "localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
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
