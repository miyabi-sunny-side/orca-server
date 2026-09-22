use super::*;

pub fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("Docker is required");
    assert!(
        output.status.success(),
        "Docker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
pub struct Container {
    pub image: String,
    pub name: String,
    store: PathBuf,
    output: PathBuf,
    running: bool,
    owner: String,
}
impl Container {
    pub fn new(image: &str, store: &Path, output: &Path) -> Self {
        let id = |flag: &str| {
            String::from_utf8(Command::new("id").arg(flag).output().unwrap().stdout)
                .unwrap()
                .trim()
                .to_owned()
        };
        let owner = format!("{}:{}", id("-u"), id("-g"));
        let mount = format!("{}:/data/plates", store.display());
        docker(&[
            "run",
            "--rm",
            "--user",
            "0",
            "--entrypoint",
            "chown",
            "-v",
            &mount,
            image,
            "10001:10001",
            "/data/plates",
        ]);
        Self {
            image: image.into(),
            name: format!("orca-rust-test-{}", uuid::Uuid::new_v4()),
            store: store.into(),
            output: output.into(),
            running: false,
            owner,
        }
    }
    pub fn launch(&mut self, root: &Path, env: &BTreeMap<String, String>) {
        assert!(!self.running);
        let mut args: Vec<String> = [
            "run",
            "-d",
            "--name",
            &self.name,
            "--network",
            "host",
            "-v",
            &format!("{}:/data/plates", self.store.display()),
            "-v",
            &format!("{}:/config/cert.pem:ro", root.join("trusted.pem").display()),
        ]
        .map(str::to_owned)
        .to_vec();
        for key in [
            "PORT",
            "P1_IP",
            "P1_SERIAL",
            "P1_ACCESS_CODE",
            "P1_MQTT_PORT",
            "P1_FTPS_PORT",
            "P1_START_TIMEOUT_SECS",
            "SCAD_LIVE_URL",
        ] {
            args.extend(["-e".into(), format!("{key}={}", env[key])]);
        }
        args.extend([
            "-e".into(),
            "P1_TLS_CERT=/config/cert.pem".into(),
            self.image.clone(),
        ]);
        // Own cleanup even if Docker creates the container but startup fails.
        self.running = true;
        docker(&args.iter().map(String::as_str).collect::<Vec<_>>());
    }
    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        self.running = false;
        if let Ok(output) = Command::new("docker").args(["logs", &self.name]).output() {
            use std::io::Write;
            let mut log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.output.join("server.log"))
                .unwrap();
            log.write_all(&output.stdout).unwrap();
            log.write_all(&output.stderr).unwrap();
        }
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    pub fn copy(&self, path: &str, name: &str) -> Vec<u8> {
        let output = self.output.join(name);
        docker(&[
            "cp",
            &format!("{}:{path}", self.name),
            output.to_str().unwrap(),
        ]);
        fs::read(output).unwrap()
    }
}
impl Drop for Container {
    fn drop(&mut self) {
        self.stop();
        let status = Command::new("docker")
            .args([
                "run",
                "--rm",
                "--user",
                "0",
                "--entrypoint",
                "chown",
                "-v",
                &format!("{}:/data/plates", self.store.display()),
                &self.image,
                "-R",
                &self.owner,
                "/data/plates",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !thread::panicking() {
            assert!(
                status.unwrap().success(),
                "restore owned temporary store permissions"
            );
        }
    }
}
