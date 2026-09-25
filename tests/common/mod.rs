//! App-owned test environment: one temporary store, process group and loopback peers per test.
#![allow(dead_code)] // Each integration target uses a different subset of these shared fixtures.
pub mod container;
pub mod legacy_schema;
pub mod peers;
pub mod wire;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::any,
};
use peers::{Peer, SECRET, SERIAL, certificate};
use serde_json::{Value, json};
use std::os::unix::process::CommandExt;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const MACHINE: &str = "Bambu Lab P1S 0.4 nozzle";
pub const PROCESS: &str = "0.20mm Standard @BBL X1C";
pub const FILAMENT: &str = "Generic PLA High Speed @BBL X1C";
pub const BED: &str = "Textured PEI Plate";
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
pub fn fixture(name: &str) -> Vec<u8> {
    fs::read(repo().join("tests/fixtures").join(name)).unwrap()
}
pub fn until(mut condition: impl FnMut() -> bool, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("condition did not become true within {seconds}s");
}
pub fn flag(path: &Path, on: bool) {
    if on {
        fs::write(path, []).unwrap();
    } else if let Err(e) = fs::remove_file(path) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
    }
}
pub fn id(value: &Value) -> &str {
    value["id"].as_str().unwrap()
}
pub fn array(value: &Value) -> &[Value] {
    value.as_array().unwrap()
}
pub fn edit(value: &Value) -> Value {
    let mut value = value.clone();
    value.as_object_mut().unwrap().remove("id");
    value.as_object_mut().unwrap().remove("imported");
    value.as_object_mut().unwrap().remove("roles");
    if let Some(models) = value.get_mut("models").and_then(Value::as_array_mut) {
        for model in models {
            model.as_object_mut().unwrap().remove("roles");
        }
    }
    value
}
pub fn printer_settings(value: &Value) -> Value {
    let mut value = value.clone();
    for key in ["id", "status", "machine", "configuration_error"] {
        value.as_object_mut().unwrap().remove(key);
    }
    value
}
pub fn merge(base: &Value, patch: &Value) -> Value {
    let mut value = base.clone();
    value
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    value
}
pub fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}
pub fn zip_read(bytes: &[u8], name: &str) -> Vec<u8> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut result = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    result
}
pub fn zip_json(bytes: &[u8], name: &str) -> Value {
    serde_json::from_slice(&zip_read(bytes, name)).unwrap()
}
pub fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
pub fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

pub struct Process {
    child: Child,
    stopped: bool,
}
impl Process {
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn spawn(command: &mut Command) -> Self {
        Self {
            child: command.process_group(0).spawn().unwrap(),
            stopped: false,
        }
    }
    pub fn stop(&mut self, kill: bool) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        // Signal the owned group, including a held fake CLI; never match processes by name.
        let signal = if kill { "-KILL" } else { "-TERM" };
        let _ = Command::new("kill")
            .args([signal, "--", &format!("-{}", self.child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", self.child.id())])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                self.child.wait().unwrap();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop(true);
    }
}

type Handler = Arc<dyn Fn(&str, &str, &[u8]) -> (u16, Vec<u8>) + Send + Sync>;
pub struct HttpPeer {
    pub base: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl HttpPeer {
    pub fn new(
        handler: impl Fn(&str, &str, &[u8]) -> (u16, Vec<u8>) + Send + Sync + 'static,
    ) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (stop, rx) = tokio::sync::oneshot::channel();
        let handler: Handler = Arc::new(handler);
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                async fn request(
                    State(handler): State<Handler>,
                    method: axum::http::Method,
                    uri: Uri,
                    body: Bytes,
                ) -> impl IntoResponse {
                    let (status, data) = handler(method.as_str(), &uri.to_string(), &body);
                    (
                        StatusCode::from_u16(status).unwrap(),
                        [("content-type", "application/json")],
                        data,
                    )
                }
                let app = Router::new().fallback(any(request)).with_state(handler);
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                    .with_graceful_shutdown(async {
                        let _ = rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self {
            base,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl Drop for HttpPeer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn fake_slicer(root: &Path) -> PathBuf {
    let app = root.join("app");
    let profiles = app.join("resources/profiles/BBL");
    for category in ["machine", "process", "filament"] {
        fs::create_dir_all(profiles.join(category)).unwrap();
    }
    let machines = [
        MACHINE,
        "Bambu Lab P1S 0.2 nozzle",
        "Bambu Lab A1 mini 0.2 nozzle",
    ];
    for (i, machine) in machines.iter().enumerate() {
        write_json(
            &profiles.join(format!("machine/{i}.json")),
            &json!({"name":machine,"instantiation":"true","nozzle_diameter":[if i==0 {"0.4"}else{"0.2"}],"printer_model":machine.split(" 0.").next().unwrap(),"default_print_profile":PROCESS,"default_filament_profile":[FILAMENT]}),
        );
    }
    write_json(
        &profiles.join("process/standard.json"),
        &json!({"name":PROCESS,"instantiation":"true","compatible_printers":machines,"brim_width":"5","brim_object_gap":"0.1","sparse_infill_pattern":"crosshatch","sparse_infill_density":"15%","wall_loops":"2","top_shell_layers":"5","bottom_shell_layers":"3","top_shell_thickness":"1","bottom_shell_thickness":"0"}),
    );
    for (i, (name, material)) in [(FILAMENT, "PLA"), ("Generic PETG", "PETG")]
        .iter()
        .enumerate()
    {
        write_json(
            &profiles.join(format!("filament/{i}.json")),
            &json!({"name":name,"instantiation":"true","compatible_printers":machines,"cool_plate_temp":["35"],"cool_plate_temp_initial_layer":["35"],"eng_plate_temp":["55"],"eng_plate_temp_initial_layer":["55"],"hot_plate_temp":["55"],"hot_plate_temp_initial_layer":["55"],"textured_plate_temp":["55"],"textured_plate_temp_initial_layer":["55"],"filament_type":[material],"nozzle_temperature":["220"],"nozzle_temperature_initial_layer":["220"],"required_nozzle_HRC":["0"]}),
        );
    }
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/fixture-slicer");
    assert!(
        binary.is_file(),
        "missing Rust fixture: run cargo build --example fixture-slicer (cargo test also builds examples)"
    );
    fs::copy(binary, app.join("AppRun")).unwrap();
    app
}

pub struct Rig {
    pub binary: PathBuf,
    pub arguments: Vec<String>,
    pub container: Option<container::Container>,
    pub root: tempfile::TempDir,
    pub store: PathBuf,
    pub output: PathBuf,
    pub broker: Peer,
    pub ftp: Peer,
    pub scad: HttpPeer,
    pub files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    pub full: Value,
    pub env: BTreeMap<String, String>,
    pub base: String,
    pub materials: Vec<Value>,
    pub plate: Value,
    pub process: Option<Process>,
    pub http: reqwest::blocking::Client,
}
impl Rig {
    pub fn new(name: &str) -> Self {
        Self::with_options(name, "v3", None)
    }
    #[allow(clippy::too_many_lines)] // Keep one isolated environment construction together.
    pub fn with_options(name: &str, version: &str, appdir: Option<&Path>) -> Self {
        let root = tempfile::Builder::new()
            .prefix("orca-rust-test-")
            .tempdir()
            .unwrap();
        let store = root.path().join("plates");
        fs::create_dir(&store).unwrap();
        let output = std::env::var_os("ORCA_TEST_OUTPUT")
            .map_or_else(|| root.path().join("output"), PathBuf::from)
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&output).unwrap();
        certificate(root.path(), "trusted", version);
        let broker = Peer::broker(root.path(), "trusted", SERIAL);
        let ftp = Peer::ftps(root.path(), "trusted");
        let files = Arc::new(Mutex::new(BTreeMap::from([(
            "parts/cube.stl".to_owned(),
            fixture("cube.stl"),
        )])));
        let content = files.clone();
        let scad = HttpPeer::new(move |_, path, _| {
            let files = content.lock().unwrap();
            if path == "/api/models" {
                return (
                    200,
                    serde_json::to_vec(&files.keys().collect::<Vec<_>>()).unwrap(),
                );
            }
            // URLs are decoded via the existing URL parser without treating '+' as a space in paths.
            let path = path
                .split('?')
                .next()
                .unwrap()
                .strip_prefix("/models/")
                .unwrap_or(path);
            let key: String =
                reqwest::Url::parse(&format!("http://localhost/?p={}", path.replace('+', "%2B")))
                    .unwrap()
                    .query_pairs()
                    .next()
                    .unwrap()
                    .1
                    .into_owned();
            files
                .get(&key)
                .map_or((404, b"not found".to_vec()), |v| (200, v.clone()))
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut env = BTreeMap::new();
        let app = appdir.map_or_else(|| fake_slicer(root.path()), Path::to_path_buf);
        for (key, value) in [
            ("PORT", port.to_string()),
            ("PLATES_DIR", store.display().to_string()),
            ("ORCA_APPDIR", app.display().to_string()),
            ("SCAD_LIVE_URL", scad.base.clone()),
            ("P1_IP", "127.0.0.1".to_owned()),
            ("P1_SERIAL", SERIAL.to_owned()),
            ("P1_ACCESS_CODE", SECRET.to_owned()),
            (
                "P1_TLS_CERT",
                root.path().join("trusted.pem").display().to_string(),
            ),
            ("P1_MQTT_PORT", broker.port.to_string()),
            ("P1_FTPS_PORT", ftp.port.to_string()),
            ("P1_START_TIMEOUT_SECS", "10".to_owned()),
        ] {
            env.insert(key.to_owned(), value);
        }
        let mut full: Value = serde_json::from_slice(&fixture("p1_status.json")).unwrap();
        full["print"]["ams"]["tray_exist_bits"] = json!("9");
        full["print"]["ams"]["ams"][0]["tray"][3]["tray_type"] = json!("PLA");
        full["print"]["ams"]["ams"][0]["tray"][3]["tray_color"] = json!("00FFFFFF");
        Self {
            binary: option_env!("CARGO_BIN_EXE_orca-server")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join("orca-server")
                }),
            arguments: vec![],
            container: None,
            root,
            store,
            output,
            broker,
            ftp,
            scad,
            files,
            full,
            env,
            base: format!("http://127.0.0.1:{port}"),
            materials: vec![],
            plate: Value::Null,
            process: None,
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .no_proxy()
                .build()
                .unwrap(),
        }
    }
    pub fn launch(&mut self) {
        assert!(self.process.is_none());
        let before = self.broker.requests().len();
        if let Some(container) = &mut self.container {
            container.launch(self.root.path(), &self.env);
        } else {
            let log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.output.join("server.log"))
                .unwrap();
            let mut command = Command::new(&self.binary);
            command.args(&self.arguments);
            command
                .current_dir(self.root.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap())
                .envs(&self.env)
                .stdout(log.try_clone().unwrap())
                .stderr(log);
            self.process = Some(Process::spawn(&mut command));
        }
        until(
            || {
                self.http
                    .get(format!("{}/healthz", self.base))
                    .send()
                    .is_ok_and(|r| r.status() == 200)
            },
            30,
        );
        if self.env.contains_key("P1_IP")
            && self.env.get("P1_MQTT_PORT") == Some(&self.broker.port.to_string())
            && self.env.get("P1_IP") == Some(&"127.0.0.1".to_owned())
        {
            until(|| self.broker.requests().len() > before, 12);
        }
    }
    pub fn stop(&mut self, kill: bool) {
        if let Some(container) = &mut self.container {
            container.stop();
        }
        if let Some(mut process) = self.process.take() {
            process.stop(kill);
        }
    }
    pub fn request(&self, method: &str, path: &str, body: Option<&Value>, expected: u16) -> Value {
        let (code, value) = self.response(method, path, body, None);
        assert_eq!(code, expected, "{path}: {value}");
        value
    }
    pub fn response(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        origin: Option<&str>,
    ) -> (u16, Value) {
        let mut request = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().unwrap();
        let code = response.status().as_u16();
        let bytes = response.bytes().unwrap();
        assert!(
            !bytes.windows(SECRET.len()).any(|v| v == SECRET.as_bytes()),
            "secret in API response"
        );
        assert!(
            !bytes
                .windows(b"BEGIN CERTIFICATE".len())
                .any(|v| v == b"BEGIN CERTIFICATE"),
            "certificate in API response"
        );
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))
        };
        (code, value)
    }
    pub fn get(&self, path: &str) -> Value {
        self.request("GET", path, None, 200)
    }
    pub fn post(&self, path: &str, body: &Value, expected: u16) -> Value {
        self.request("POST", path, Some(body), expected)
    }
    pub fn put(&self, path: &str, body: &Value, expected: u16) -> Value {
        self.request("PUT", path, Some(body), expected)
    }
    pub fn queue(&self) -> Value {
        self.get("/api/queue?printer_id=p1")
    }
    pub fn idle(&self) {
        self.broker.send(&self.full);
        until(|| self.queue()["printer"]["ready_to_print"] == true, 12);
    }
    pub fn seed(&mut self) {
        self.idle();
        for (name, color) in [("PLA 白", "FFFFFFFF"), ("PLA 青", "00FFFFFF")] {
            let material=self.post("/api/filaments",&json!({"name":name,"vendor":"Fixture","material":"PLA","color":color,"bambu_filament_id":null}),201);
            self.post(&format!("/api/filaments/{}/settings",id(&material)),&json!({"machine_profile_key":MACHINE,"base_profile_key":FILAMENT,"overrides_json":{"nozzle_temperature":215}}),201);
            self.materials.push(material);
        }
        for (slot, material) in [(0, 0), (3, 1)] {
            let slot = self.slot(slot);
            self.put(
                &format!("/api/printers/p1/ams/{}", id(&slot)),
                &json!({"revision":slot["revision"],"filament_id":self.materials[material]["id"]}),
                204,
            );
        }
        self.plate=self.post("/api/plates/import",&json!({"name":"最新のキューブ","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":2}]}),201);
    }
    pub fn slot(&self, index: u64) -> Value {
        array(&self.get("/api/printers/p1/ams")["slots"])
            .iter()
            .find(|v| v["slot_index"] == index && v["ams_id"] == 0)
            .unwrap()
            .clone()
    }
    pub fn specification(&self, slot: u64) -> Value {
        let slot = self.slot(slot);
        json!({"ams_slot_id":slot["id"],"filament_id":slot["filament_id"],"required_machine_profile_key":MACHINE,"process_profile_key":PROCESS,"bed_type":BED})
    }
    pub fn command(&self, action: Value, view: Option<&Value>) -> Value {
        let q = view.cloned().unwrap_or_else(|| self.queue());
        let mut value =
            json!({"epoch":q["epoch"],"generation":q["generation"],"request_id":q["request_id"]});
        value["action"] = action;
        value
    }
    pub fn send(&self, action: Value, expected: u16) -> Value {
        self.post(
            "/api/queue?printer_id=p1",
            &self.command(action, None),
            expected,
        )
    }
    pub fn configure(&mut self, spec: Option<Value>, plate: Option<Value>) -> Value {
        let plate = plate.unwrap_or_else(|| self.plate.clone());
        let mut plate = self.get(&format!("/api/plates/{}", id(&plate)));
        let mut spec = spec.unwrap_or_else(|| self.specification(3));
        spec.as_object_mut().unwrap().remove("ams_slot_id");
        let conditions = merge(&plate["conditions"], &spec);
        if plate["conditions"] != conditions {
            let mut data = edit(&plate);
            data["conditions"] = conditions;
            plate = self.put(&format!("/api/plates/{}", id(&plate)), &data, 200);
        }
        if self.plate["id"] == plate["id"] {
            self.plate = plate.clone();
        }
        plate
    }
    pub fn add_action(&mut self, spec: Option<Value>, plate: Option<Value>) -> Value {
        let plate = self.configure(spec, plate);
        json!({"type":"add","plate_id":plate["id"],"plate_version":plate["version"]})
    }
    pub fn add(&mut self, slot: u64) -> Value {
        let mut plate = self.plate.clone();
        if slot != 3 {
            let models: Vec<_> = array(&plate["models"]).iter().map(edit).collect();
            plate=self.post("/api/plates/import",&json!({"name":format!("{} white",plate["name"].as_str().unwrap()),"models":models}),201);
        }
        let action = self.add_action(Some(self.specification(slot)), Some(plate));
        array(&self.send(action, 200)["waiting"])
            .last()
            .unwrap()
            .clone()
    }
    pub fn next(&self, job: &Value, expected: u16) -> Value {
        let q = self.queue();
        self.post("/api/queue?printer_id=p1",&self.command(json!({"type":"next","expected_job":job["id"],"removed_job":q["current"]["id"],"cleared":true}),Some(&q)),expected)
    }
    pub fn report(&self, state: &str) {
        self.report_index(state, None);
    }
    pub fn report_index(&self, state: &str, index: Option<usize>) {
        let prints = self.broker.prints();
        let command = &prints[index.unwrap_or(prints.len() - 1)];
        let mut full = self.full.clone();
        full["print"] = merge(
            &full["print"],
            &json!({"gcode_state":state,"subtask_name":command["subtask_name"],"gcode_file":command["file"]}),
        );
        self.broker.send(&full);
    }
    pub fn phase(&self, name: &str) {
        until(|| self.queue()["current"]["state"] == name, 12);
    }
    pub fn start_phase(&self, name: &str) {
        until(|| self.queue()["printer"]["start"]["phase"] == name, 12);
    }
    pub fn traces(&self) -> Vec<Value> {
        fs::read_to_string(self.root.path().join("cli.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    pub fn hold(&self, on: bool) {
        flag(&self.root.path().join("cli-hold"), on);
    }
    pub fn waiting(&self, job: &Value) -> Value {
        array(&self.queue()["waiting"])
            .iter()
            .find(|j| j["id"] == job["id"])
            .unwrap()
            .clone()
    }
    pub fn ready(&self, job: &Value) {
        until(|| self.waiting(job)["estimate"]["state"] == "ready", 30);
        assert_eq!(self.waiting(job)["estimate"]["seconds"], 1140);
    }
    pub fn finish(&self) {
        self.report("RUNNING");
        self.phase("printing");
        self.report("FINISH");
        self.phase("awaiting_removal");
    }
    pub fn discard(&self) {
        self.send(
            json!({"type":"discard","expected_job":self.queue()["current"]["id"],"cleared":true}),
            200,
        );
    }
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.store.join("orca.sqlite3")).unwrap()
    }
    pub fn artifact(&self, name: &str) -> PathBuf {
        self.store
            .join(self.queue()["current"]["artifact_path"].as_str().unwrap())
            .join(name)
    }
    pub fn check(&self) {
        self.broker.check();
        self.ftp.check();
        let log = fs::read(self.output.join("server.log")).unwrap_or_default();
        assert!(
            !log.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
            "credential in log"
        );
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.stop(true);
        self.container.take();
        if thread::panicking() {
            let log = fs::read_to_string(self.output.join("server.log")).unwrap_or_default();
            eprintln!(
                "fixture log {}\n{}",
                self.output.display(),
                log.chars()
                    .rev()
                    .take(12000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
        } else {
            self.check();
        }
    }
}
impl Rig {
    pub fn control(&self) -> HttpPeer {
        let records = self.broker.records.clone();
        let uploads = self.ftp.records.clone();
        let actions = self.broker.actions.clone();
        let ftp_actions = self.ftp.actions.clone();
        let root = self.root.path().to_path_buf();
        let full = self.full.clone();
        HttpPeer::new(move |method, _, body| {
            if method == "POST" {
                let value: Value = serde_json::from_slice(body).unwrap();
                if let Some(on) = value["slice_hold"].as_bool() {
                    flag(&root.join("cli-hold"), on);
                } else if let Some(on) = value["slice_fail"].as_bool() {
                    flag(&root.join("cli-fail"), on);
                } else if value["fail_upload"] == true {
                    ftp_actions.send(peers::Action::Fail).unwrap();
                } else if value["disconnect"] == true {
                    actions.send(peers::Action::Disconnect).unwrap();
                } else {
                    let mut report = full.clone();
                    if let Some(state) = value["state"].as_str() {
                        let records = records.lock().unwrap();
                        let index = value["index"]
                            .as_i64()
                            .filter(|n| *n >= 0)
                            .map_or(records.prints.len() - 1, |n| usize::try_from(n).unwrap());
                        let command = &records.prints[index];
                        report["print"] = merge(
                            &report["print"],
                            &json!({"gcode_state":state,"subtask_name":command["subtask_name"],"gcode_file":command["file"]}),
                        );
                    }
                    actions
                        .send(peers::Action::Report(
                            serde_json::to_vec(&report).unwrap(),
                            false,
                            format!("device/{SERIAL}/report"),
                        ))
                        .unwrap();
                }
            }
            let records = records.lock().unwrap();
            let uploads = uploads.lock().unwrap();
            (200,serde_json::to_vec(&json!({"prints":records.prints,"uploads":uploads.uploads,"count":records.prints.len()})).unwrap())
        })
    }
}
impl Rig {
    pub fn upload(&self, name: &str, files: &[(&str, Vec<u8>)]) -> Value {
        let mut form = reqwest::blocking::multipart::Form::new().text("name", name.to_owned());
        for (name, bytes) in files {
            form = form.part(
                "models",
                reqwest::blocking::multipart::Part::bytes(bytes.clone())
                    .file_name((*name).to_owned()),
            );
        }
        let response = self
            .http
            .post(format!("{}/api/plates", self.base))
            .multipart(form)
            .send()
            .unwrap();
        assert_eq!(response.status(), 201);
        response.json().unwrap()
    }
    pub fn browser(&self, context_name: &str, context: &Value, control: Option<&HttpPeer>) {
        self.browser_env(context_name, context, control, &[]);
    }
    pub fn browser_env(
        &self,
        context_name: &str,
        context: &Value,
        control: Option<&HttpPeer>,
        extra: &[(&str, String)],
    ) {
        let mut command = Command::new("npm");
        command
            .args(["run", "test:e2e", "--", "--workers=1"])
            .current_dir(repo().join("client"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("E2E_") {
                command.env_remove(key);
            }
        }
        command
            .env("E2E_BASE_URL", &self.base)
            .env(context_name, context.to_string())
            .env("E2E_EVIDENCE_DIR", &self.output);
        if let Some(control) = control {
            command.env("E2E_PRINTER_CONTROL", &control.base);
        }
        command.envs(extra.iter().map(|(k, v)| (k, v)));
        assert!(
            command.status().unwrap().success(),
            "Chromium scenario failed: {context_name}"
        );
    }
}
impl Rig {
    pub fn edit_conditions(&mut self, changes: &Value) -> Value {
        let path = format!("/api/plates/{}", id(&self.plate));
        let mut data = edit(&self.get(&path));
        data["conditions"] = merge(&data["conditions"], changes);
        self.plate = self.put(&path, &data, 200);
        self.plate.clone()
    }
    pub fn stored(&self, job: &Value, column: &str) -> String {
        assert!(
            [
                "execution_json",
                "attempt_json",
                "artifact_path",
                "estimate_json"
            ]
            .contains(&column)
        );
        self.db()
            .query_row(
                &if column == "estimate_json" {
                    "SELECT CASE WHEN j.state='queued' THEN s.record_json ELSE e.estimate_json END FROM print_jobs j LEFT JOIN plate_slices s ON s.plate_id=j.plate_id LEFT JOIN print_executions e ON e.id=j.attempt_id WHERE j.id=?1".to_owned()
                } else {
                    format!("SELECT {column} FROM print_executions WHERE id=(SELECT attempt_id FROM print_jobs WHERE id=?1)")
                },
                [id(job)],
                |r| r.get(0),
            )
            .unwrap()
    }
    pub fn cached_artifact(&self, job: &Value, column: &str) -> Vec<u8> {
        assert!(["project", "gcode"].contains(&column));
        self.db().query_row(&format!("SELECT {column} FROM plate_slices WHERE plate_id=(SELECT plate_id FROM print_jobs WHERE id=?1)"), [id(job)], |r|r.get(0)).unwrap()
    }
    pub fn estimated(&self, job: &Value) -> Value {
        until(
            || {
                let estimate = self.waiting(job)["estimate"].clone();
                assert_ne!(estimate["state"], "failed", "{estimate}");
                estimate["state"] == "ready"
            },
            30,
        );
        serde_json::from_str(&self.stored(job, "estimate_json")).unwrap()
    }
    pub fn temperature(&self, index: usize, value: i64) {
        let fid = id(&self.materials[index]);
        let setting = self.get(&format!("/api/filaments/{fid}"))["settings"][0].clone();
        let mut data = json!({"machine_profile_key":setting["machine_profile_key"],"base_profile_key":setting["base_profile_key"],"overrides_json":setting["overrides_json"]});
        data["overrides_json"]["nozzle_temperature"] = json!(value);
        self.put(
            &format!("/api/filaments/{fid}/settings/{}", id(&setting)),
            &data,
            200,
        );
    }
    pub fn map_material(&self, slot: u64, index: usize) {
        until(|| self.slot(slot)["reported"]["present"] == true, 12);
        let slot = self.slot(slot);
        self.put(
            &format!("/api/printers/p1/ams/{}", id(&slot)),
            &json!({"revision":slot["revision"],"filament_id":self.materials[index]["id"]}),
            204,
        );
    }
}
impl Rig {
    pub fn rows(&self, sql: &str, args: &[&str]) -> Vec<Vec<rusqlite::types::Value>> {
        let db = self.db();
        let mut statement = db.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map(rusqlite::params_from_iter(args), |r| {
                (0..count).map(|i| r.get(i)).collect()
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}
impl Rig {
    pub fn multipart(
        &self,
        path: &str,
        files: &[(&str, Vec<u8>)],
        fields: &[(&str, &str)],
        expected: u16,
    ) -> Vec<u8> {
        let mut form = reqwest::blocking::multipart::Form::new();
        for (k, v) in fields {
            form = form.text((*k).to_owned(), (*v).to_owned());
        }
        for (name, data) in files {
            form = form.part(
                "models",
                reqwest::blocking::multipart::Part::bytes(data.clone())
                    .file_name((*name).to_owned())
                    .mime_str("application/octet-stream")
                    .unwrap(),
            );
        }
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .timeout(Duration::from_mins(2))
            .multipart(form)
            .send()
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().unwrap();
        assert_eq!(
            status.as_u16(),
            expected,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        bytes.to_vec()
    }
    pub fn multipart_json(
        &self,
        path: &str,
        files: &[(&str, Vec<u8>)],
        fields: &[(&str, &str)],
        expected: u16,
    ) -> Value {
        serde_json::from_slice(&self.multipart(path, files, fields, expected)).unwrap()
    }
    pub fn bytes(&self, path: &str) -> Vec<u8> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().unwrap().to_vec()
    }
}

impl Rig {
    pub fn in_container(name: &str, image: &str) -> Self {
        let mut rig = Self::new(name);
        rig.container = Some(container::Container::new(image, &rig.store, &rig.output));
        rig
    }
}
