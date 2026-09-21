use crate::{
    plates::{Error, Result},
    print_start::{self, Attempt, Phase},
    printer_state::State,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, SubscribeReasonCode, Transport};
use rustls::pki_types::pem::PemObject;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime},
};
use std::{
    io::Read,
    net::IpAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, mpsc, oneshot};
use x509_cert::der::{Decode, Encode};

#[derive(Debug)]
struct PinnedCertificate {
    certificate: CertificateDer<'static>,
    public_key: SubjectPublicKeyInfoDer<'static>,
}

impl PinnedCertificate {
    fn new(certificate: CertificateDer<'static>) -> std::result::Result<Self, &'static str> {
        const INVALID: &str = "P1_TLS_CERT contains an invalid certificate";
        let public_key =
            if let Ok(parsed) = rustls::server::ParsedCertificate::try_from(&certificate) {
                parsed.subject_public_key_info()
            } else {
                // WebPKI's end-entity parser only accepts v3. Parse the pinned v1
                // certificate without changing its DER or inventing a trust chain.
                let parsed = x509_cert::Certificate::from_der(&certificate).map_err(|_| INVALID)?;
                let tbs = parsed.tbs_certificate();
                if tbs.version() != x509_cert::Version::V1
                    || tbs.extensions().is_some()
                    || tbs.issuer_unique_id().is_some()
                    || tbs.subject_unique_id().is_some()
                    || tbs.signature() != parsed.signature_algorithm()
                    || parsed.signature().unused_bits() != 0
                {
                    return Err(INVALID);
                }
                tbs.subject_public_key_info()
                    .to_der()
                    .map_err(|_| INVALID)?
                    .into()
            };
        webpki::RawPublicKeyEntity::try_from(&public_key).map_err(|_| INVALID)?;
        Ok(Self {
            certificate,
            public_key,
        })
    }

    fn check_pin(&self, cert: &CertificateDer<'_>) -> std::result::Result<(), rustls::Error> {
        if cert == &self.certificate {
            Ok(())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        self.check_pin(cert)?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.check_pin(cert)?;
        let algorithms = rustls::crypto::ring::default_provider().signature_verification_algorithms;
        let candidates = algorithms
            .mapping
            .iter()
            .find(|(scheme, _)| *scheme == signature.scheme)
            .ok_or(rustls::Error::PeerMisbehaved(
                rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
            ))?
            .1;
        let key = webpki::RawPublicKeyEntity::try_from(&self.public_key).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        // TLS 1.2 schemes can map to several key algorithms (e.g. ECDSA curves).
        // Verify the actual handshake signature with the key from the pinned DER.
        if candidates.iter().any(|alg| {
            key.verify_signature(*alg, message, signature.signature())
                .is_ok()
        }) {
            Ok(HandshakeSignatureValid::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature,
            ))
        }
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.check_pin(cert)?;
        rustls::crypto::verify_tls13_signature_with_raw_key(
            message,
            &self.public_key,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Clone)]
enum Certificate {
    File(PathBuf),
    Pem(Vec<u8>),
}
impl Certificate {
    fn bytes(&self) -> std::result::Result<Vec<u8>, &'static str> {
        let bytes = match self {
            Self::File(path) => {
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(|_| "Cannot read P1_TLS_CERT")?
                    .take(65_537)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Cannot read P1_TLS_CERT")?;
                bytes
            }
            Self::Pem(bytes) => bytes.clone(),
        };
        if bytes.len() > 65_536 {
            return Err("Printer certificate exceeds 64 KiB");
        }
        Ok(bytes)
    }
    #[cfg(test)]
    fn path(&self) -> &std::path::Path {
        match self {
            Self::File(path) => path,
            Self::Pem(_) => panic!("file fixture expected"),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    ip: IpAddr,
    port: u16,
    ftps_port: u16,
    start_timeout: u64,
    serial: String,
    access_code: String,
    certificate: Certificate,
    machine: String,
    nozzle_diameter: String,
    nozzle_material: String,
}

impl Config {
    fn parse(
        get: impl Fn(&str) -> Option<String>,
    ) -> std::result::Result<Option<Self>, &'static str> {
        let ip = get("P1_IP");
        let serial = get("P1_SERIAL");
        let access_code = get("P1_ACCESS_CODE");
        let certificate = get("P1_TLS_CERT");
        let port = get("P1_MQTT_PORT");
        let ftps_port = get("P1_FTPS_PORT");
        let start_timeout = get("P1_START_TIMEOUT_SECS");
        if [
            &ip,
            &serial,
            &access_code,
            &certificate,
            &port,
            &ftps_port,
            &start_timeout,
        ]
        .iter()
        .all(|v| v.is_none())
        {
            return Ok(None);
        }
        let ip = ip
            .and_then(|v| v.parse().ok())
            .ok_or("P1_IP must be an IP address")?;
        let serial = serial
            .filter(|s| {
                !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .ok_or("P1_SERIAL must contain 1..64 ASCII letters or digits")?;
        let access_code = access_code
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .ok_or("P1_ACCESS_CODE is missing or invalid")?;
        let certificate = certificate
            .filter(|s| !s.is_empty())
            .ok_or("P1_TLS_CERT must name the printer certificate PEM file")?
            .into();
        let port = port
            .unwrap_or_else(|| "8883".into())
            .parse::<u16>()
            .ok()
            .filter(|&v| v > 0)
            .ok_or("P1_MQTT_PORT must be 1..65535")?;
        let ftps_port = ftps_port
            .unwrap_or_else(|| "990".into())
            .parse::<u16>()
            .ok()
            .filter(|&v| v > 0)
            .ok_or("P1_FTPS_PORT must be 1..65535")?;
        let start_timeout = start_timeout
            .unwrap_or_else(|| "600".into())
            .parse::<u64>()
            .ok()
            .filter(|&v| (1..=3600).contains(&v))
            .ok_or("P1_START_TIMEOUT_SECS must be 1..3600")?;
        Ok(Some(Self {
            ip,
            port,
            ftps_port,
            start_timeout,
            serial,
            access_code,
            certificate: Certificate::File(certificate),
            machine: crate::profiles::PRINTER.into(),
            nozzle_diameter: "0.4".into(),
            nozzle_material: "unknown".into(),
        }))
    }

    /// Read printer-only settings; absence leaves printer integration disabled.
    /// # Errors
    /// Rejects incomplete settings or malformed values without echoing credentials.
    pub fn from_env() -> std::result::Result<Option<Self>, &'static str> {
        let mut values = std::collections::BTreeMap::new();
        for name in [
            "P1_IP",
            "P1_SERIAL",
            "P1_ACCESS_CODE",
            "P1_TLS_CERT",
            "P1_MQTT_PORT",
            "P1_FTPS_PORT",
            "P1_START_TIMEOUT_SECS",
        ] {
            match std::env::var(name) {
                Ok(value) => {
                    values.insert(name, value);
                }
                Err(std::env::VarError::NotPresent) => {}
                Err(_) => return Err("P1 settings must be UTF-8"),
            }
        }
        Self::parse(|name| values.get(name).cloned())
    }
}

impl Config {
    fn tls(&self) -> std::result::Result<Arc<rustls::ClientConfig>, &'static str> {
        let pem = self.certificate.bytes()?;
        let certificates = CertificateDer::pem_slice_iter(&pem)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| "P1_TLS_CERT must be a PEM certificate")?;
        if certificates.len() != 1 {
            return Err("P1_TLS_CERT must contain exactly one printer certificate");
        }
        let pinned = PinnedCertificate::new(certificates[0].clone())?;
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|_| "Cannot configure printer TLS")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(pinned))
        .with_no_client_auth();
        Ok(Arc::new(tls))
    }
    pub(crate) fn for_settings(
        settings: &crate::database::Settings,
        diameter: &str,
    ) -> Result<Self> {
        let values = std::collections::BTreeMap::from([
            ("P1_IP", settings.host.clone()),
            ("P1_SERIAL", settings.serial.clone()),
            ("P1_ACCESS_CODE", settings.access_code.clone()),
            ("P1_TLS_CERT", "inline".into()),
            ("P1_MQTT_PORT", settings.mqtt_port.to_string()),
            ("P1_FTPS_PORT", settings.ftps_port.to_string()),
            (
                "P1_START_TIMEOUT_SECS",
                settings.start_timeout_secs.to_string(),
            ),
        ]);
        let mut config = Self::parse(|key| values.get(key).cloned())
            .map_err(Error::Invalid)?
            .expect("settings provided");
        config.certificate = Certificate::Pem(settings.tls_certificate.as_bytes().to_vec());
        config.machine.clone_from(&settings.machine_profile_key);
        diameter.clone_into(&mut config.nozzle_diameter);
        config.nozzle_material.clone_from(&settings.nozzle_material);
        config.tls().map_err(Error::Invalid)?;
        Ok(config)
    }

    pub(crate) fn import(self) -> Result<crate::database::Settings> {
        self.tls().map_err(Error::Invalid)?;
        let pem = String::from_utf8(self.certificate.bytes().map_err(Error::Invalid)?)
            .map_err(|_| Error::Invalid("Certificate PEM must be UTF-8"))?;
        Ok(crate::database::Settings {
            name: "P1S".into(),
            host: self.ip.to_string(),
            serial: self.serial,
            access_code: self.access_code,
            tls_certificate: pem,
            machine_profile_key: self.machine,
            default_process_profile_key: crate::profiles::Selection::default().process,
            bed_type: crate::profiles::BEDS[0].into(),
            nozzle_material: self.nozzle_material,
            mqtt_port: self.port,
            ftps_port: self.ftps_port,
            start_timeout_secs: u16::try_from(self.start_timeout).expect("validated timeout"),
        })
    }

    fn options(&self) -> std::result::Result<MqttOptions, &'static str> {
        let mut options = MqttOptions::new(
            format!("orca-server-{}", uuid::Uuid::new_v4()),
            self.ip.to_string(),
            self.port,
        );
        options.set_credentials("bblp", &self.access_code);
        options.set_transport(Transport::tls_with_config(
            rumqttc::TlsConfiguration::Rustls(self.tls()?),
        ));
        options.set_keep_alive(Duration::from_secs(15));
        options.set_clean_session(true);
        options.set_max_packet_size(1024 * 1024, 64 * 1024);
        Ok(options)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct Observation(tokio::task::AbortHandle);
impl Drop for Observation {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Request {
    Start(String, u64),
    AutoRefill {
        enabled: bool,
        epoch: u64,
        response: oneshot::Sender<Result<()>>,
    },
}
#[derive(Clone)]
pub struct Printer {
    state: Arc<Mutex<State>>,
    config: Option<Config>,
    starts: mpsc::Sender<Request>,
    _observation: Option<Arc<Observation>>,
    inventory: Option<(crate::database::Database, crate::database::Device)>,
}

impl Printer {
    /// Start the configured observation loop. Missing settings disable printing.
    /// # Errors
    /// Rejects an invalid pinned certificate.
    pub(crate) fn new(
        config: Option<Config>,
        inventory: Option<(crate::database::Database, crate::database::Device)>,
    ) -> std::result::Result<Self, &'static str> {
        let mut initial = State::new(config.is_some());
        if let Some((db, device)) = &inventory {
            initial.start = db
                .restore_attempt(&device.id)
                .map_err(|_| "Cannot restore print state")?;
        }
        let state = Arc::new(Mutex::new(initial));
        let (starts, receiver) = mpsc::channel(1);
        let observation = if let Some(config) = &config {
            Some(Arc::new(Observation(
                tokio::spawn(run(
                    config.options()?,
                    config.clone(),
                    state.clone(),
                    receiver,
                    inventory.clone(),
                ))
                .abort_handle(),
            )))
        } else {
            None
        };
        Ok(Self {
            state,
            config,
            starts,
            _observation: observation,
            inventory,
        })
    }

    pub async fn status(&self) -> crate::printer_state::Status {
        self.state.lock().await.status(now())
    }

    // Keep report updates and AMS edits outside the queue admission transaction.
    pub(crate) async fn observed_status(
        &self,
    ) -> Result<(
        tokio::sync::MutexGuard<'_, State>,
        crate::printer_state::Status,
    )> {
        let state = self.state.lock().await;
        let status = state.status(now());
        if let Some((db, device)) = self.inventory.clone() {
            let observed = state.status(now());
            crate::plate_api::blocking(move || db.observe_ams(&device, &observed)).await?;
        }
        Ok((state, status))
    }

    pub(crate) async fn ams_inventory(
        &self,
        change: Option<crate::ams::Change>,
    ) -> Result<(bool, Vec<crate::ams::AmsSlot>)> {
        let state = self.state.lock().await;
        let status = state.status(now());
        let current = status.synchronized;
        if change.as_ref().is_some_and(|c| match c {
            crate::ams::Change::Mapping(_, m) => m.filament_id.is_some(),
            crate::ams::Change::Priority(_) => true,
        }) && !current
        {
            return Err(Error::Conflict(
                "Wait for a complete, current printer report before mapping",
            ));
        }
        let (db, device) = self.inventory.clone().ok_or(Error::NotFound)?;
        let slots = crate::plate_api::blocking(move || {
            db.observe_ams(&device, &status)?;
            match change {
                Some(crate::ams::Change::Mapping(id, m)) => {
                    db.map_slot(&device.id, &id, m.revision, m.filament_id.as_deref())?;
                }
                Some(crate::ams::Change::Priority(p)) => db.prioritize_slots(
                    &device.id,
                    &p.filament_id,
                    &device.settings.machine_profile_key,
                    &p.order,
                )?,
                None => {}
            }
            db.ams_slots(&device.id)
        })
        .await?;
        Ok((current, slots))
    }

    pub(crate) async fn resolve_material(
        &self,
        filament: String,
    ) -> Result<Vec<crate::ams::AmsSlot>> {
        let state = self.state.lock().await;
        let status = state.status(now());
        if !status.synchronized {
            return Err(Error::Conflict("Wait for a current printer report"));
        }
        let (db, device) = self.inventory.clone().ok_or(Error::NotFound)?;
        crate::plate_api::blocking(move || {
            db.observe_ams(&device, &status)?;
            db.resolve_slots(&device.id, &filament, &device.settings.machine_profile_key)
        })
        .await
    }
    pub(crate) async fn set_auto_refill(&self, enabled: bool) -> Result<()> {
        let state = self.state.lock().await;
        let status = state.status(now());
        if !status.synchronized || status.auto_refill.supported != Some(true) {
            return Err(Error::Conflict(
                "Auto refill support is not confirmed by the current printer",
            ));
        }
        let (response, receiver) = oneshot::channel();
        self.starts
            .try_send(Request::AutoRefill {
                enabled,
                epoch: state.epoch,
                response,
            })
            .map_err(|_| Error::Conflict("Printer command is busy"))?;
        drop(state);
        tokio::time::timeout(Duration::from_secs(3), receiver)
            .await
            .map_err(|_| {
                Error::Unavailable(
                    "Setting request timed out; inspect the reported state before retrying",
                )
            })?
            .map_err(|_| Error::Unavailable("Printer connection closed"))?
    }

    pub(crate) async fn forget_retired(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let (Some((db, device)), Some(attempt)) = (&self.inventory, &state.start) {
            let active:bool=db.connection()?.query_row("SELECT EXISTS(SELECT 1 FROM print_jobs WHERE printer_id=?1 AND attempt_id=?2 AND state NOT IN ('completed','cancelled'))",rusqlite::params![device.id,attempt.id],|r|r.get(0))?;
            if !active {
                state.start = None;
            }
        }
        Ok(())
    }

    /// Upload a durably reserved execution. The MQTT path persists the send barrier separately.
    pub(crate) async fn start(&self, attempt: Attempt, bytes: Vec<u8>) -> Result<Attempt> {
        let config = self
            .config
            .clone()
            .ok_or(Error::Unavailable("Printer is not configured"))?;
        let (db, device) = self
            .inventory
            .as_ref()
            .ok_or(Error::Unavailable("Printer registry is unavailable"))?;
        let mut state = self.state.lock().await;
        let status = state.status(now());
        print_start::check_nozzle(&status, &config.nozzle_diameter, &config.nozzle_material)?;
        db.check_attempt(device, &attempt, &status)?;
        if state.start.as_ref().is_some_and(|a| {
            matches!(
                a.phase,
                Phase::Uploading | Phase::AwaitingConfirmation | Phase::Accepted | Phase::Printing
            )
        }) {
            return Err(Error::Conflict("A start request is still active"));
        }
        db.persist_attempt(&device.id, &attempt)?;
        let epoch = state.epoch;
        state.start = Some(attempt.clone());
        drop(state);
        let printer = self.clone();
        let transfer = attempt.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_mins(5),
                upload(&config, &transfer.filename(), &bytes),
            )
            .await;
            let mut state = printer.state.lock().await;
            let Some(start) = state.start.as_mut().filter(|s| s.id == transfer.id) else {
                return;
            };
            if !matches!(result, Ok(Ok(()))) {
                start.fail(
                    Phase::UploadFailed,
                    "FTPS transfer failed or timed out; no start command was sent",
                );
            } else if printer
                .starts
                .try_send(Request::Start(transfer.id, epoch))
                .is_err()
            {
                start.fail(
                    Phase::NotSent,
                    "Printer command channel unavailable; no start command was sent",
                );
            }
            persist(printer.inventory.as_ref(), state.start.as_ref());
        });
        Ok(attempt)
    }
}

fn persist(
    inventory: Option<&(crate::database::Database, crate::database::Device)>,
    attempt: Option<&Attempt>,
) {
    if let (Some((db, device)), Some(attempt)) = (inventory, attempt)
        && db.persist_attempt(&device.id, attempt).is_err()
    {
        tracing::warn!("Print observation could not be saved; retrying on the next report or tick");
    }
}

async fn upload(config: &Config, name: &str, bytes: &[u8]) -> std::result::Result<(), ()> {
    use suppaftp::{
        tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream},
        types::FileType,
    };
    // One TLS config per upload shares the control session with its data connection.
    let connector = AsyncRustlsConnector::from(suppaftp::tokio_rustls::TlsConnector::from(
        config.tls().map_err(|_| ())?,
    ));
    let mut ftp = AsyncRustlsFtpStream::connect_secure_implicit(
        (config.ip, config.ftps_port),
        connector,
        &config.ip.to_string(),
    )
    .await
    .map_err(|_| ())?;
    // Keep PASV's dynamic port, but never follow its address to another host.
    ftp.set_passive_nat_workaround(true);
    if config.ip.is_ipv6() {
        ftp.set_mode(suppaftp::Mode::ExtendedPassive);
    }
    ftp.login("bblp", &config.access_code)
        .await
        .map_err(|_| ())?;
    ftp.custom_command("PBSZ 0", &[suppaftp::Status::CommandOk])
        .await
        .map_err(|_| ())?;
    ftp.custom_command("PROT P", &[suppaftp::Status::CommandOk])
        .await
        .map_err(|_| ())?;
    ftp.transfer_type(FileType::Binary).await.map_err(|_| ())?;
    let count = ftp
        .put_file(name, &mut std::io::Cursor::new(bytes))
        .await
        .map_err(|_| ())?;
    if count != bytes.len() as u64 {
        return Err(());
    }
    // A final positive transfer reply is authoritative; QUIT failure cannot undo it.
    let _ = tokio::time::timeout(Duration::from_secs(2), ftp.quit()).await;
    Ok(())
}

fn request_snapshot(
    client: &AsyncClient,
    topic: &str,
) -> std::result::Result<(), rumqttc::ClientError> {
    client.try_publish(
        topic,
        QoS::AtMostOnce,
        false,
        br#"{"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}"#
            .to_vec(),
    )
}

fn queue_refill(client: &AsyncClient, topic: &str, enabled: bool) -> Result<()> {
    let sequence = (uuid::Uuid::new_v4().as_u128() % 2_000_000_000 + 1).to_string();
    let payload = serde_json::json!({"print":{"command":"print_option","sequence_id":sequence,"auto_switch_filament":enabled}});
    client
        .try_publish(topic, QoS::AtMostOnce, false, payload.to_string())
        .map_err(|_| Error::Unavailable("Setting request was not queued"))
}

async fn run(
    options: MqttOptions,
    config: Config,
    state: Arc<Mutex<State>>,
    mut starts: mpsc::Receiver<Request>,
    inventory: Option<(crate::database::Database, crate::database::Device)>,
) {
    let serial = &config.serial;
    let report = format!("device/{serial}/report");
    let request = format!("device/{serial}/request");
    loop {
        // New queues on reconnect: requests from a lost connection are never replayed.
        let (client, mut events) = AsyncClient::new(options.clone(), 8);
        events.network_options.set_connection_timeout(5);
        let mut refresh = tokio::time::interval(Duration::from_mins(5));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut subscribed = false;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                event = events.poll() => match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        state.lock().await.connected();
                        if client.try_subscribe(&report, QoS::AtMostOnce).is_err() { break; }
                    }
                    Ok(Event::Incoming(Incoming::SubAck(ack))) => {
                        if ack.return_codes.len() != 1 || !matches!(ack.return_codes[0], SubscribeReasonCode::Success(_)) { break; }
                        subscribed = true;
                        if request_snapshot(&client, &request).is_err() { break; }
                        refresh.reset();
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) if subscribed && message.topic == report && !message.retain => {
                        let mut state = state.lock().await;
                        let applied=state.apply(&message.payload, now());
                        let status = state.status(now());
                        if applied && let Some((db,device))=&inventory {
                            let db=db.clone();let device=device.clone();let observed=state.status(now());
                            if crate::plate_api::blocking(move || db.observe_ams(&device,&observed)).await.is_err() {
                                tracing::warn!("AMS observation could not be saved; inventory reads will retry");
                            }
                        }
                        if let Ok(value) = serde_json::from_slice(&message.payload)
                            && let Some(start) = &mut state.start { start.observe(&value, &status); }
                        persist(inventory.as_ref(),state.start.as_ref());
                    }
                    Ok(_) => {},
                    Err(_) => break, // Never log library errors or packets: they may contain credentials.
                },
                Some(command) = starts.recv() => {
                    let mut state = state.lock().await;
                    let status=state.status(now());
                    let (id,epoch)=match command {
                        Request::Start(id,epoch) => (id,epoch),
                        Request::AutoRefill{enabled,epoch,response} => {
                            let result=if response.is_closed() || !subscribed || state.epoch!=epoch || !status.synchronized || status.auto_refill.supported!=Some(true) {
                                Err(Error::Conflict("Printer connection or auto refill support changed"))
                            } else {
                                queue_refill(&client,&request,enabled)
                            };
                            let _=response.send(result);
                            continue;
                        },
                    };
                    let same_connection = state.epoch == epoch && subscribed;
                    if let Some(start) = state.start.as_mut().filter(|s| s.id == id && s.phase == Phase::Uploading) {
                        let admitted=inventory.as_ref().is_some_and(|(db,device)|db.check_attempt(device,start,&status).is_ok());
                        if !same_connection || !admitted || print_start::check_nozzle(&status,&config.nozzle_diameter,&config.nozzle_material).is_err() {
                            start.fail(Phase::NotSent,"Printer status or selected AMS changed during transfer; no start command was sent");
                        } else {
                            // Commit BEFORE enqueueing MQTT. A crash in either side of this barrier is never replayed.
                            start.sent(now());
                            let saved=inventory.as_ref().is_some_and(|(db,device)|db.persist_attempt(&device.id,start).is_ok());
                            if !saved {
                                start.fail(Phase::NotSent,"Cannot persist the start request; no start command was sent");
                            } else if client.try_publish(&request,QoS::AtMostOnce,false,start.command().to_string()).is_err() {
                                start.fail(Phase::NotSent,"MQTT publish was not queued; no start command was sent");
                            }
                        }
                        persist(inventory.as_ref(),Some(start));
                    }
                }
                _ = tick.tick() => {
                    let mut state = state.lock().await;
                    let synchronized = state.status(now()).synchronized;
                    if let Some(start) = &mut state.start {
                        let pending = matches!(start.phase, Phase::AwaitingConfirmation | Phase::Accepted | Phase::Printing);
                        start.tick(now(), config.start_timeout);
                        if !synchronized { start.disconnected(); }
                        persist(inventory.as_ref(),Some(start));
                        // Drop this client's queue so timed-out writes cannot be sent later.
                        if pending && start.phase == Phase::Unknown { break; }
                    }
                }
                _ = refresh.tick() => {
                    if subscribed && !state.lock().await.status(now()).synchronized && request_snapshot(&client, &request).is_err() { break; }
                }
            }
        }
        {
            let mut state = state.lock().await;
            state.disconnected();
            persist(inventory.as_ref(), state.start.as_ref());
        }
        tracing::warn!(
            "P1 MQTT disconnected; check address, access code and pinned certificate; retrying"
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn fixture(dir: &std::path::Path, name: &str, version: &str) -> Config {
        let pem = dir.join(format!("{name}.pem"));
        let key = dir.join(format!("{name}.key"));
        let version = if version == "v1" {
            // OpenSSL < 3.2 defaults to v1 without this option.
            let help = std::process::Command::new("openssl")
                .args(["req", "-help"])
                .output()
                .unwrap();
            if String::from_utf8_lossy(&help.stderr).contains("-x509v1") {
                vec!["-x509v1"]
            } else {
                vec![]
            }
        } else {
            vec!["-addext", "subjectAltName=DNS:isolated-printer"]
        };
        let output = std::process::Command::new("openssl")
            .args([
                "req",
                "-config",
                "/dev/null",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=isolated-printer",
            ])
            .args(version)
            .arg("-out")
            .arg(&pem)
            .arg("-keyout")
            .arg(key)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "synthetic certificate generation failed"
        );
        Config {
            ip: "127.0.0.1".parse().unwrap(),
            port: 8883,
            ftps_port: 990,
            start_timeout: 600,
            serial: "TESTSERIAL".into(),
            access_code: "test-only-secret".into(),
            certificate: Certificate::File(pem),
            machine: crate::profiles::PRINTER.into(),
            nozzle_diameter: "0.4".into(),
            nozzle_material: "unknown".into(),
        }
    }

    fn handshake(
        config: &Config,
        cert: &std::path::Path,
        key: &std::path::Path,
    ) -> std::result::Result<(), rustls::Error> {
        fn transfer(
            from: &mut rustls::Connection,
            to: &mut rustls::Connection,
        ) -> std::result::Result<(), rustls::Error> {
            let mut wire = Vec::new();
            from.write_tls(&mut wire).unwrap();
            to.read_tls(&mut std::io::Cursor::new(wire)).unwrap();
            to.process_new_packets()?;
            Ok(())
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cert = CertificateDer::from_pem_file(cert).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::from_pem_file(key).unwrap();
        // Supplying another key deliberately produces an invalid handshake signature.
        let certified = rustls::sign::CertifiedKey::new(
            vec![cert],
            provider.key_provider.load_private_key(key).unwrap(),
        );
        let server = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)));
        let mut server =
            rustls::Connection::Server(rustls::ServerConnection::new(Arc::new(server))?);
        let mut client = rustls::Connection::Client(rustls::ClientConnection::new(
            config.tls().expect("app TLS configuration"),
            ServerName::from(config.ip),
        )?);
        for _ in 0..8 {
            transfer(&mut client, &mut server)?;
            transfer(&mut server, &mut client)?;
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
        }
        panic!("TLS handshake did not finish");
    }

    #[test]
    fn pinned_tls_accepts_v1_and_v3_but_rejects_other_certificates_and_signing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let other = fixture(dir.path(), "other", "v1");
        for version in ["v1", "v3"] {
            let config = fixture(dir.path(), version, version);
            let key = config.certificate.path().with_extension("key");
            let der = CertificateDer::from_pem_file(config.certificate.path()).unwrap();
            let parsed = x509_cert::Certificate::from_der(&der).unwrap();
            assert_eq!(
                parsed.tbs_certificate().version(),
                if version == "v1" {
                    x509_cert::Version::V1
                } else {
                    x509_cert::Version::V3
                }
            );
            assert!(
                config.options().is_ok(),
                "MQTT configuration must accept {version}"
            );
            handshake(&config, config.certificate.path(), &key).unwrap();
            // Even a different certificate for the same key must fail DER pinning.
            let renewed = dir.path().join("renewed.pem");
            assert!(
                std::process::Command::new("openssl")
                    .args(["x509", "-in"])
                    .arg(config.certificate.path())
                    .args(["-set_serial", "1", "-signkey"])
                    .arg(&key)
                    .arg("-out")
                    .arg(&renewed)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            assert!(matches!(
                handshake(&config, &renewed, &key),
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure
                ))
            ));
            assert!(matches!(
                handshake(
                    &config,
                    other.certificate.path(),
                    &other.certificate.path().with_extension("key")
                ),
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure
                ))
            ));
            assert!(matches!(
                handshake(
                    &config,
                    config.certificate.path(),
                    &other.certificate.path().with_extension("key")
                ),
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::BadSignature
                ))
            ));
        }
    }

    #[test]
    fn malformed_certificate_is_a_startup_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = fixture(dir.path(), "invalid", "v1");
        let pem = std::fs::read_to_string(config.certificate.path()).unwrap();
        let der = CertificateDer::from_pem_slice(pem.as_bytes()).unwrap();
        for bad in [
            String::new(),
            "not PEM".into(),
            pem.repeat(2),
            " ".repeat(65_537),
            "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n".into(),
            "-----BEGIN CERTIFICATE-----\n!invalid!\n-----END CERTIFICATE-----\n".into(),
        ] {
            std::fs::write(config.certificate.path(), bad).unwrap();
            assert!(config.options().is_err());
            assert!(config.tls().is_err());
        }
        for length in [0, 1, der.len() / 2, der.len() - 1] {
            assert!(PinnedCertificate::new(der[..length].to_vec().into()).is_err());
        }
        let mut trailing = der.to_vec();
        trailing.push(0);
        assert!(PinnedCertificate::new(trailing.into()).is_err());
    }

    #[test]
    fn configuration_is_optional_but_partial_or_invalid_credentials_fail_without_echoing_values() {
        assert!(Config::parse(|_| None).unwrap().is_none());
        let good = BTreeMap::from([
            ("P1_IP", "127.0.0.1"),
            ("P1_SERIAL", "TESTP1SERIAL"),
            ("P1_ACCESS_CODE", "test-secret"),
            ("P1_TLS_CERT", "/tmp/printer.pem"),
        ]);
        let config = Config::parse(|key| good.get(key).map(|s| (*s).to_owned()))
            .unwrap()
            .unwrap();
        assert_eq!(config.ip.to_string(), "127.0.0.1");
        assert_eq!(config.port, 8883);
        assert_eq!(config.serial, "TESTP1SERIAL");
        assert_eq!(config.access_code, "test-secret");
        assert_eq!(
            config.certificate.path(),
            std::path::Path::new("/tmp/printer.pem")
        );
        for (key, bad) in [
            ("P1_IP", "bad"),
            ("P1_SERIAL", "../+/test-secret"),
            ("P1_ACCESS_CODE", ""),
            ("P1_TLS_CERT", ""),
            ("P1_MQTT_PORT", "0"),
            ("P1_MQTT_PORT", "65536"),
            ("P1_FTPS_PORT", "0"),
            ("P1_START_TIMEOUT_SECS", "0"),
            ("P1_START_TIMEOUT_SECS", "3601"),
        ] {
            let mut fields = good.clone();
            fields.insert(key, bad);
            let error = Config::parse(|name| fields.get(name).map(|s| (*s).to_owned()))
                .err()
                .unwrap();
            assert!(error.contains(key));
            assert!(!error.contains("test-secret"));
        }
        for key in good.keys() {
            let mut fields = good.clone();
            fields.remove(key);
            assert!(Config::parse(|name| fields.get(name).map(|s| (*s).to_owned())).is_err());
        }
    }
}
