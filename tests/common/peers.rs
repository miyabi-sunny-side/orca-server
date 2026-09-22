//! Loopback-only protocol peers. Synthetic certificates and isolated request queues.
use super::wire::{packet, read_packet};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    sign::{CertifiedKey, SingleCertAndKey},
};
use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub const SERIAL: &str = "TESTP1SERIAL";
pub const SECRET: &str = "isolated-test-access-code";
type Tls = StreamOwned<ServerConnection, TcpStream>;
pub fn certificate(root: &Path, name: &str, version: &str) {
    let mut command = Command::new("openssl");
    command.args(["req", "-config", "/dev/null", "-x509", "-newkey"]);
    if version == "v1" {
        command.arg("rsa:2048");
        let help = Command::new("openssl")
            .args(["req", "-help"])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&help.stderr).contains("-x509v1") {
            command.arg("-x509v1");
        }
    } else {
        command.args([
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-addext",
            "subjectAltName=DNS:isolated-printer",
        ]);
    }
    let output = command
        .args([
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=isolated-printer",
            "-keyout",
        ])
        .arg(root.join(format!("{name}.key")))
        .arg("-out")
        .arg(root.join(format!("{name}.pem")))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "openssl: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info = Command::new("openssl")
        .args(["x509", "-in"])
        .arg(root.join(format!("{name}.pem")))
        .args(["-noout", "-text"])
        .output()
        .unwrap();
    assert!(info.status.success());
    assert!(
        String::from_utf8_lossy(&info.stdout).contains(&format!("Version: {} (", &version[1..]))
    );
}
pub fn config(cert: &Path, key: &Path) -> Arc<ServerConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let certs = CertificateDer::pem_file_iter(cert)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::from_pem_file(key).unwrap())
        .unwrap();
    // Synthetic v1 peers need the same signed handshake as a printer, without WebPKI's v3-only certificate parser.
    let resolver = SingleCertAndKey::from(CertifiedKey::new(certs, key));
    Arc::new(
        ServerConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver)),
    )
}
pub fn tls(raw: TcpStream, config: &Arc<ServerConfig>) -> io::Result<Tls> {
    raw.set_read_timeout(Some(Duration::from_secs(2)))?;
    raw.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut stream = StreamOwned::new(ServerConnection::new(config.clone()).unwrap(), raw);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    Ok(stream)
}
fn normal_end(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
    )
}
#[derive(Default)]
pub struct Records {
    pub requests: Vec<Value>,
    pub prints: Vec<Value>,
    pub options: Vec<Value>,
    pub uploads: Vec<String>,
    pub contents: Vec<Vec<u8>>,
    pub errors: Vec<String>,
}
pub enum Action {
    Disconnect,
    InvalidLogin,
    Report(Vec<u8>, bool, String),
    Fail,
    Wait,
}
pub struct Peer {
    pub port: u16,
    pub records: Arc<Mutex<Records>>,
    pub actions: Sender<Action>,
    pub tls_failures: Arc<AtomicUsize>,
    pub received: Arc<AtomicBool>,
    pub allow_options: Arc<AtomicBool>,
    pub gate: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    serial: String,
}
impl Peer {
    pub fn broker(root: &Path, name: &str, serial: &str) -> Self {
        Self::start(root, name, serial, false)
    }
    pub fn ftps(root: &Path, name: &str) -> Self {
        Self::start(root, name, SERIAL, true)
    }
    fn start(root: &Path, name: &str, serial: &str, ftp: bool) -> Self {
        let config = config(
            &root.join(format!("{name}.pem")),
            &root.join(format!("{name}.key")),
        );
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let (actions, rx) = mpsc::channel();
        let mut peer = Self {
            port: socket.local_addr().unwrap().port(),
            records: Arc::default(),
            actions,
            tls_failures: Arc::default(),
            received: Arc::default(),
            allow_options: Arc::default(),
            gate: Arc::default(),
            stopped: Arc::default(),
            thread: None,
            serial: serial.to_owned(),
        };
        let records = peer.records.clone();
        let stopped = peer.stopped.clone();
        let failures = peer.tls_failures.clone();
        let received = peer.received.clone();
        let gate = peer.gate.clone();
        let serial = serial.to_owned();
        let allow_options = peer.allow_options.clone();
        peer.thread = Some(thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match socket.accept() {
                    Ok((raw, _)) => {
                        let Ok(mut stream) = tls(raw, &config) else {
                            failures.fetch_add(1, Ordering::SeqCst);
                            continue;
                        };
                        let outcome = if ftp {
                            ftp_session(
                                &mut stream,
                                &config,
                                &rx,
                                &records,
                                &stopped,
                                &received,
                                &gate,
                            )
                        } else {
                            mqtt_session(
                                &mut stream,
                                &serial,
                                &rx,
                                &records,
                                &stopped,
                                &allow_options,
                            )
                        };
                        if let Err(e) = outcome
                            && !normal_end(&e)
                        {
                            records.lock().unwrap().errors.push(e.to_string());
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => {
                        records.lock().unwrap().errors.push(e.to_string());
                        break;
                    }
                }
            }
        }));
        peer
    }
    pub fn send(&self, value: &Value) {
        self.report(serde_json::to_vec(value).unwrap(), false, None);
    }
    pub fn report(&self, bytes: Vec<u8>, retained: bool, topic: Option<&str>) {
        self.actions
            .send(Action::Report(
                bytes,
                retained,
                topic.map_or_else(|| format!("device/{}/report", self.serial), str::to_owned),
            ))
            .unwrap();
    }
    pub fn action(&self, action: Action) {
        self.actions.send(action).unwrap();
    }
    pub fn requests(&self) -> Vec<Value> {
        self.records.lock().unwrap().requests.clone()
    }
    pub fn prints(&self) -> Vec<Value> {
        self.records.lock().unwrap().prints.clone()
    }
    pub fn uploads(&self) -> Vec<String> {
        self.records.lock().unwrap().uploads.clone()
    }
    pub fn contents(&self) -> Vec<Vec<u8>> {
        self.records.lock().unwrap().contents.clone()
    }
    pub fn release(&self) {
        self.gate.store(true, Ordering::SeqCst);
    }
    pub fn reset_gate(&self) {
        self.gate.store(false, Ordering::SeqCst);
        self.received.store(false, Ordering::SeqCst);
    }
    pub fn check(&self) {
        let seen = self.records.lock().unwrap();
        assert!(seen.errors.is_empty(), "{:?}", seen.errors);
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.release();
        if let Some(handle) = self.thread.take() {
            let joined = handle.join();
            if !thread::panicking() {
                assert!(joined.is_ok(), "protocol peer panicked");
                self.check();
            }
        }
    }
}
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}
fn mqtt_session(
    peer: &mut Tls,
    serial: &str,
    actions: &Receiver<Action>,
    records: &Mutex<Records>,
    stopped: &AtomicBool,
    allow_options: &AtomicBool,
) -> io::Result<()> {
    let (header, login) = read_packet(peer)?;
    if header != 0x10
        || !login.windows(4).any(|w| w == b"bblp")
        || !login.windows(SECRET.len()).any(|w| w == SECRET.as_bytes())
        || login.get(7).is_none_or(|v| v & 2 == 0)
    {
        return Err(invalid("unexpected MQTT login"));
    }
    peer.write_all(&[0x20, 2, 0, 0])?;
    let (header, body) = read_packet(peer)?;
    let report = format!("device/{serial}/report");
    if header != 0x82
        || body.len() < 2
        || !body.windows(report.len()).any(|w| w == report.as_bytes())
    {
        return Err(invalid("unexpected MQTT subscription"));
    }
    peer.write_all(&[0x90, 3, body[0], body[1], 0])?;
    peer.sock
        .set_read_timeout(Some(Duration::from_millis(50)))?;
    while !stopped.load(Ordering::SeqCst) {
        match actions.try_recv() {
            Ok(Action::Disconnect) => return Ok(()),
            Ok(Action::InvalidLogin) => peer.write_all(&packet(0x10, &login))?,
            Ok(Action::Report(value, retained, topic)) => {
                let mut body = u16::try_from(topic.len()).unwrap().to_be_bytes().to_vec();
                body.extend(topic.as_bytes());
                body.extend(value);
                peer.write_all(&packet(if retained { 0x31 } else { 0x30 }, &body))?;
            }
            _ => {}
        }
        let (header, body) = match read_packet(peer) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            other => other?,
        };
        if header == 0xc0 {
            peer.write_all(&[0xd0, 0])?;
            continue;
        }
        if header != 0x30 || body.len() < 2 {
            return Err(invalid("only non-retained QoS0 request allowed"));
        }
        let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
        if body.get(2..2 + length) != Some(format!("device/{serial}/request").as_bytes()) {
            return Err(invalid("request topic mismatch"));
        }
        let value: Value =
            serde_json::from_slice(&body[2 + length..]).map_err(|_| invalid("request JSON"))?;
        let mut seen = records.lock().unwrap();
        if value.get("pushing").is_some() {
            if value["pushing"]["command"] != "pushall"
                || value["pushing"]["version"] != 1
                || value["pushing"]["push_target"] != 1
            {
                return Err(invalid("invalid pushall"));
            }
            seen.requests.push(value);
        } else if value["print"]["command"] == "print_option"
            && allow_options.load(Ordering::SeqCst)
        {
            let fields = value["print"]
                .as_object()
                .ok_or_else(|| invalid("invalid print option"))?;
            if fields.len() != 3
                || !fields.contains_key("sequence_id")
                || !value["print"]["auto_switch_filament"].is_boolean()
            {
                return Err(invalid("invalid print option fields"));
            }
            seen.options.push(value);
        } else {
            if value["print"]["command"] != "project_file" {
                return Err(invalid("unexpected print command"));
            }
            seen.prints.push(value["print"].clone());
        }
    }
    Ok(())
}
fn ftp_session(
    peer: &mut Tls,
    config: &Arc<ServerConfig>,
    actions: &Receiver<Action>,
    records: &Mutex<Records>,
    stopped: &AtomicBool,
    received: &AtomicBool,
    gate: &AtomicBool,
) -> io::Result<()> {
    peer.write_all(b"220 isolated peer ready\r\n")?;
    let mut reader = BufReader::new(peer);
    let mut listener = None;
    while !stopped.load(Ordering::SeqCst) {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if line.len() > 4096 {
            return Err(invalid("oversized FTP command"));
        }
        let (command, arg) = line
            .trim_end_matches(['\r', '\n'])
            .split_once(' ')
            .unwrap_or((line.trim(), ""));
        let reply: &[u8] = match command {
            "USER" if arg == "bblp" => b"331 password required\r\n",
            "PASS" if arg == SECRET => b"230 logged in\r\n",
            "PBSZ" if arg == "0" => b"200 ok\r\n",
            "PROT" if arg == "P" => b"200 ok\r\n",
            "TYPE" if arg == "I" => b"200 ok\r\n",
            "PASV" => {
                let socket = TcpListener::bind("127.0.0.1:0")?;
                socket.set_nonblocking(true)?;
                let port = socket.local_addr()?.port();
                listener = Some(socket);
                reader.get_mut().write_all(
                    format!(
                        "227 Passive (203,0,113,22,{},{})\r\n",
                        port / 256,
                        port % 256
                    )
                    .as_bytes(),
                )?;
                continue;
            }
            "STOR" => {
                if !arg.starts_with("orca-") || !arg.ends_with(".gcode.3mf") || arg.contains('/') {
                    return Err(invalid("unexpected upload name"));
                }
                reader.get_mut().write_all(b"150 send data\r\n")?;
                let listener = listener
                    .as_ref()
                    .ok_or_else(|| invalid("no passive listener"))?;
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let raw = loop {
                    match listener.accept() {
                        Ok((raw, _)) => break raw,
                        Err(e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline
                                && !stopped.load(Ordering::SeqCst) =>
                        {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => return Err(e),
                    }
                };
                let mut data = tls(raw, config)?;
                if data.conn.handshake_kind() != Some(rustls::HandshakeKind::Resumed) {
                    return Err(invalid("FTPS data TLS session was not reused"));
                }
                let mut content = Vec::new();
                data.read_to_end(&mut content)?;
                {
                    let mut seen = records.lock().unwrap();
                    seen.contents.push(content);
                    seen.uploads.push(arg.to_owned());
                }
                let action = actions.try_recv().ok();
                if matches!(action, Some(Action::Wait)) {
                    received.store(true, Ordering::SeqCst);
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    while !gate.load(Ordering::SeqCst) && !stopped.load(Ordering::SeqCst) {
                        if std::time::Instant::now() > deadline {
                            return Err(invalid("FTPS gate timed out"));
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                if matches!(action, Some(Action::Fail)) {
                    reader
                        .get_mut()
                        .write_all(format!("550 {SECRET}\r\n").as_bytes())?;
                    continue;
                }
                b"226 transfer complete\r\n"
            }
            "QUIT" => {
                reader.get_mut().write_all(b"221 bye\r\n")?;
                return Ok(());
            }
            _ => return Err(invalid("unexpected FTP command")),
        };
        reader.get_mut().write_all(reply)?;
    }
    Ok(())
}
