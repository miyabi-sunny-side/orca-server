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
    camera_port: u16,
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
        let camera_port = get("P1_CAMERA_PORT");
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
        let camera_port = camera_port
            .unwrap_or_else(|| "6000".into())
            .parse::<u16>()
            .ok()
            .filter(|&v| v > 0)
            .ok_or("P1_CAMERA_PORT must be 1..65535")?;
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
            camera_port,
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
            "P1_CAMERA_PORT",
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
            ("P1_CAMERA_PORT", settings.camera_port.to_string()),
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
            camera_port: self.camera_port,
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

/// How long a manual refresh waits for the printer's full report.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn now() -> u64 {
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
    /// Ask the printer for a full report; answers the connection epoch and report count at sending.
    Snapshot {
        response: oneshot::Sender<Result<(u64, u64)>>,
    },
    /// Publish an operator control and answer with the printer's reply to it.
    Control {
        message: crate::bambu::Message,
        response: oneshot::Sender<Result<Reply>>,
    },
}
/// What became of one published control.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Reply {
    /// `success`, `rejected`, or `none` when no answer arrived in time or the connection ended.
    pub reply: &'static str,
    pub reason: Option<String>,
}
/// How long a published control waits for the printer's answer.
const REPLY_TIMEOUT: u64 = 10;
type Pending = Vec<(crate::bambu::Message, oneshot::Sender<Result<Reply>>, u64)>;

/// Answer every pending control that `report` replies to.
fn answer(pending: &mut Pending, report: &serde_json::Value) {
    let mut index = 0;
    while index < pending.len() {
        if let Some(result) = crate::bambu::reply(report, &pending[index].0) {
            let (_, response, _) = pending.swap_remove(index);
            let _ = response.send(Ok(match result {
                Ok(()) => Reply {
                    reply: "success",
                    reason: None,
                },
                Err(reason) => Reply {
                    reply: "rejected",
                    reason,
                },
            }));
        } else {
            index += 1;
        }
    }
}
/// End controls without an answer: those older than the timeout, or all of them (`now` = MAX).
fn expire(pending: &mut Pending, now: u64, reason: &str) {
    let mut index = 0;
    while index < pending.len() {
        if now.saturating_sub(pending[index].2) >= REPLY_TIMEOUT {
            let (_, response, _) = pending.swap_remove(index);
            let _ = response.send(Ok(Reply {
                reply: "none",
                reason: Some(reason.into()),
            }));
        } else {
            index += 1;
        }
    }
}
pub(crate) struct Diagnostics {
    pub status: crate::printer_state::Status,
    pub epoch: u64,
    pub link: crate::printer_state::Link,
    pub saves: crate::printer_state::Saves,
    pub upload_failure: Option<(u64, &'static str)>,
    /// Report counters, the latest AMS bits and the latest full-report request.
    pub observation: serde_json::Value,
}

#[derive(Clone)]
pub struct Printer {
    state: Arc<Mutex<State>>,
    config: Option<Config>,
    starts: mpsc::Sender<Request>,
    /// One manual full-report request at a time per printer.
    refreshing: Arc<Mutex<()>>,
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
            crate::journal::secret(&config.access_code);
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
            refreshing: Arc::default(),
            _observation: observation,
            inventory,
        })
    }

    /// Ask the printer itself for a full report and wait until a newer full AMS report from the
    /// same connection has been applied and saved. Sending alone is not completion.
    pub(crate) async fn refresh_ams(&self) -> Result<crate::printer_state::FullAms> {
        use crate::printer_state::{Refresh, refresh_state};
        let _one = self
            .refreshing
            .try_lock()
            .map_err(|_| Error::Conflict("An AMS refresh is already in progress"))?;
        if self.config.is_none() {
            return Err(Error::Upstream("Printer connection is not configured"));
        }
        let (response, receiver) = oneshot::channel();
        self.starts
            .try_send(Request::Snapshot { response })
            .map_err(|_| Error::Conflict("Printer command is busy; retry"))?;
        let (epoch, floor) = tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| Error::Upstream("Printer connection did not accept the request"))?
            .map_err(|_| Error::Upstream("Printer connection closed"))??;
        let deadline = tokio::time::Instant::now() + REFRESH_TIMEOUT;
        loop {
            let seen = self.state.lock().await.full_ams_seen.clone();
            let notified = seen.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let outcome = {
                let state = self.state.lock().await;
                refresh_state(state.epoch, state.full_ams.as_ref(), epoch, floor)
            };
            let printer = self.inventory.as_ref().map_or("", |(_, d)| d.id.as_str());
            match outcome {
                Refresh::Done(full) => {
                    tracing::info!(
                        printer,
                        epoch,
                        report = full.report,
                        reading = full.reading,
                        "Manual AMS refresh completed"
                    );
                    return Ok(full);
                }
                Refresh::Failed(reason) => {
                    tracing::warn!(printer, epoch, reason, "Manual AMS refresh failed");
                    return Err(if reason == "save" {
                        Error::Unavailable(
                            "The full report arrived but could not be saved; check server storage",
                        )
                    } else {
                        Error::Upstream(
                            "Printer connection changed before the full report arrived; retry",
                        )
                    });
                }
                Refresh::Pending => {}
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                // A disconnect also ends the wait; report it rather than a timeout.
                let state = self.state.lock().await;
                if let Refresh::Pending =
                    refresh_state(state.epoch, state.full_ams.as_ref(), epoch, floor)
                {
                    tracing::warn!(printer, epoch, "Manual AMS refresh timed out");
                    return Err(Error::NoReply(
                        "The printer did not send a full report in time",
                    ));
                }
            }
        }
    }

    pub async fn status(&self) -> crate::printer_state::Status {
        self.state.lock().await.status(now())
    }

    /// One in-memory snapshot for diagnostics. Unlike `observed_status`, it writes nothing.
    pub(crate) async fn diagnostics(&self) -> Diagnostics {
        let state = self.state.lock().await;
        Diagnostics {
            status: state.status(now()),
            epoch: state.epoch,
            link: state.link.clone(),
            saves: state.saves.clone(),
            upload_failure: state.upload_failure,
            observation: serde_json::json!({
                "reports": state.reports,
                "ignored": state.ignored_count,
                "last_ignored": state.ignored.map(|(at, reason)| serde_json::json!({"at":at,"reason":reason})),
                "ams_reports": state.ams_reports,
                "ams_report_at": state.ams_report_at,
                "ams_bits": state.ams_bits,
                "snapshot_request": state.snapshot_request.map(|(at, trigger, sent)| serde_json::json!({"at":at,"trigger":trigger,"sent":sent})),
            }),
        }
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
            let (observed, report) = (state.status(now()), state.reports);
            crate::plate_api::blocking(move || observe(&db, &device, &observed, "status", report))
                .await?;
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
        let report = state.reports;
        // Rows of an unplugged AMS stay stored for when it returns; list only reported units.
        let units: Option<Vec<u8>> = current.then(|| {
            status
                .ams
                .iter()
                .flat_map(|ams| &ams.units)
                .map(|unit| unit.id)
                .collect()
        });
        let mut slots = crate::plate_api::blocking(move || {
            observe(&db, &device, &status, "inventory", report)?;
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
        if let Some(units) = units {
            slots.retain(|slot| units.contains(&slot.ams_id));
        }
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
        let report = state.reports;
        crate::plate_api::blocking(move || {
            observe(&db, &device, &status, "resolve", report)?;
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

    /// Publish one operator control on the current connection and wait for the printer's answer.
    /// Sending is not completion: the reply and later reports tell what the printer did.
    pub(crate) async fn control(&self, control: &crate::bambu::Control) -> Result<Reply> {
        let config = self
            .config
            .as_ref()
            .ok_or(Error::Unavailable("Printer is not configured"))?;
        // The A1 series moves the bed in Y; every other supported model is CoreXY.
        let core_xy = !config.machine.contains(" A1");
        let sequence = (uuid::Uuid::new_v4().as_u128() % 2_000_000_000 + 1).to_string();
        let message = control
            .clone()
            .oriented(core_xy)
            .message(&sequence)
            .map_err(Error::Invalid)?;
        if !self.state.lock().await.status(now()).synchronized {
            return Err(Error::Conflict(
                "Wait for a fresh synchronized printer report",
            ));
        }
        let (response, receiver) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_secs(3),
            self.starts.send(Request::Control { message, response }),
        )
        .await
        .map_err(|_| Error::Conflict("Printer command is busy"))?
        .map_err(|_| Error::Unavailable("Printer connection closed"))?;
        tokio::time::timeout(Duration::from_secs(REPLY_TIMEOUT + 5), receiver)
            .await
            .map_err(|_| Error::Unavailable("Printer connection closed"))?
            .map_err(|_| Error::Unavailable("Printer connection closed"))?
    }

    /// Run one storage operation on its own FTPS login, never during a print transfer.
    async fn storage<T>(
        &self,
        event: serde_json::Value,
        operation: impl AsyncFnOnce(&mut Ftps) -> std::result::Result<T, &'static str>,
    ) -> Result<T> {
        let config = self
            .config
            .clone()
            .ok_or(Error::Unavailable("Printer is not configured"))?;
        if self
            .state
            .lock()
            .await
            .start
            .as_ref()
            .is_some_and(|a| a.phase == Phase::Uploading)
        {
            return Err(Error::Conflict(
                "A print file is being transferred; try again after it starts",
            ));
        }
        let result = tokio::time::timeout(Duration::from_mins(1), async {
            let mut ftp = ftps(&config).await?;
            let value = operation(&mut ftp).await?;
            let _ = tokio::time::timeout(Duration::from_secs(2), ftp.quit()).await;
            Ok(value)
        })
        .await
        .unwrap_or(Err("timeout"));
        let mut body = event;
        body["failed_stage"] = serde_json::json!(result.as_ref().err());
        let printer = self.inventory.as_ref().map_or("", |(_, d)| d.id.as_str());
        crate::journal::event(printer, None, "ftps", body);
        result.map_err(|stage| match stage {
            "not_found" => Error::NotFound,
            "too_large" => Error::Invalid("File is larger than 512 MiB"),
            _ => Error::Upstream(
                "Printer storage could not be read; check the printer's SD card and LAN",
            ),
        })
    }

    /// Entries of one storage directory.
    pub(crate) async fn files(&self, dir: &str) -> Result<Vec<serde_json::Value>> {
        let dir = crate::sdcard::path(dir).map_err(Error::Invalid)?;
        let listed = dir.clone();
        self.storage(
            serde_json::json!({"event":"list","path":dir}),
            async move |ftp| {
                let lines = ftp.list(Some(&listed)).await.map_err(|_| "list")?;
                Ok(lines
                    .iter()
                    .filter_map(|line| crate::sdcard::entry(line, &listed))
                    .collect())
            },
        )
        .await
    }

    /// The bytes of one stored file, up to [`crate::sdcard::MAX_DOWNLOAD`].
    pub(crate) async fn download(&self, path: &str) -> Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let path = crate::sdcard::path(path).map_err(Error::Invalid)?;
        let file = path.clone();
        self.storage(
            serde_json::json!({"event":"download","path":path}),
            async move |ftp| {
                let stream = ftp.retr_as_stream(&file).await.map_err(|_| "not_found")?;
                let mut bytes = Vec::new();
                let mut limited = stream.take(crate::sdcard::MAX_DOWNLOAD as u64 + 1);
                limited
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| "transfer")?;
                if bytes.len() > crate::sdcard::MAX_DOWNLOAD {
                    return Err("too_large");
                }
                limited
                    .into_inner()
                    .finish()
                    .await
                    .map_err(|_| "transfer")?;
                Ok(bytes)
            },
        )
        .await
    }

    /// Delete one stored file.
    pub(crate) async fn delete_file(&self, path: &str) -> Result<()> {
        let path = crate::sdcard::path(path).map_err(Error::Invalid)?;
        let file = path.clone();
        self.storage(
            serde_json::json!({"event":"delete","path":path}),
            async move |ftp| ftp.rm(&file).await.map_err(|_| "not_found"),
        )
        .await
    }

    /// One JPEG from the printer camera, within 10 seconds.
    pub(crate) async fn snapshot(&self) -> Result<Vec<u8>> {
        let config = self
            .config
            .clone()
            .ok_or(Error::Unavailable("Printer is not configured"))?;
        let tls = config
            .tls()
            .map_err(|_| Error::Upstream("Camera TLS failed"))?;
        let printer = self.inventory.as_ref().map_or("", |(_, d)| d.id.as_str());
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            crate::camera::snapshot(tls, config.ip, config.camera_port, &config.access_code),
        )
        .await
        .unwrap_or(Err("timeout"));
        crate::journal::event(
            printer,
            None,
            "camera",
            serde_json::json!({"bytes":result.as_ref().ok().map(Vec::len),"failed_stage":result.as_ref().err()}),
        );
        result.map_err(|stage| match stage {
            "timeout" => Error::NoReply("Camera did not send an image within 10 seconds"),
            _ => Error::Upstream("Camera image could not be read; check the printer's LAN camera"),
        })
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
                Phase::Uploading
                    | Phase::AwaitingConfirmation
                    | Phase::Accepted
                    | Phase::Printing
                    | Phase::Paused
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
        let id = device.id.clone();
        tokio::spawn(async move {
            let file = transfer.filename();
            let ftps = |body| crate::journal::event(&id, Some(epoch), "ftps", body);
            ftps(serde_json::json!({"event":"upload","file":file,"bytes":bytes.len()}));
            let result =
                tokio::time::timeout(Duration::from_mins(5), upload(&config, &file, &bytes)).await;
            let mut state = printer.state.lock().await;
            let Some(start) = state.start.as_mut().filter(|s| s.id == transfer.id) else {
                return;
            };
            let failed = match result {
                Ok(Ok(())) => None,
                Ok(Err(step)) => Some(step),
                Err(_) => Some("timeout"),
            };
            ftps(serde_json::json!({"event":"result","file":file,"failed_stage":failed}));
            if failed.is_some() {
                start.fail(
                    Phase::UploadFailed,
                    "FTPS transfer failed or timed out; no start command was sent",
                );
            } else if printer
                .starts
                .try_send(Request::Start(transfer.id.clone(), epoch))
                .is_err()
            {
                start.fail(
                    Phase::NotSent,
                    "Printer command channel unavailable; no start command was sent",
                );
            }
            if let Some(step) = failed {
                state.upload_failure = Some((now(), step));
                tracing::warn!(
                    printer = printer.inventory.as_ref().map_or("", |(_, d)| d.id.as_str()),
                    job = %transfer.job_id,
                    attempt = %transfer.id,
                    stage = step,
                    "FTPS transfer failed; no start command was sent"
                );
            }
            persist(printer.inventory.as_ref(), &mut state);
        });
        Ok(attempt)
    }
}

/// Apply one printer report to the state, the AMS inventory and the current attempt.
async fn on_report(
    printer: &str,
    state: &mut State,
    inventory: Option<&(crate::database::Database, crate::database::Device)>,
    payload: &[u8],
) {
    crate::journal::report(printer, state.epoch, payload);
    let value = serde_json::from_slice::<serde_json::Value>(payload).ok();
    let summary = value.as_ref().and_then(crate::printer_state::ams_summary);
    let before = summary.as_ref().and_then(|_| state.status(now()).ams);
    let ignored_before = state.ignored.map(|(_, reason)| reason);
    let applied = state.apply(payload, now());
    let status = state.status(now());
    let report = state.reports;
    if applied
        && status.synchronized
        && let Some(failures) = state.link.synchronized(now())
    {
        tracing::info!(
            printer,
            epoch = state.epoch,
            report,
            failures,
            state = status.print.state.as_deref().unwrap_or(""),
            error = status.print.error.map(print_start::error_code),
            "P1 status synchronized"
        );
    }
    if let Some(summary) = &summary {
        log_ams_report(
            printer,
            state,
            summary,
            before.as_ref(),
            applied,
            ignored_before,
        );
    }
    let mut saved = true;
    if applied && let Some((db, device)) = inventory {
        let (db, device, observed) = (db.clone(), device.clone(), state.status(now()));
        match crate::plate_api::blocking(move || observe(&db, &device, &observed, "mqtt", report))
            .await
        {
            Ok(()) => {
                if let Some(failures) = state.saves.saved("ams") {
                    tracing::info!(printer, report, failures, "AMS observation saved again");
                }
            }
            Err(error) => {
                saved = false;
                if state.saves.failed("ams", save_kind(&error), now()) {
                    tracing::warn!(
                        printer,
                        report,
                        kind = save_kind(&error),
                        "AMS observation could not be saved; inventory reads will retry"
                    );
                }
            }
        }
    }
    if applied
        && let Some(summary) = summary
            .as_ref()
            .filter(|s| s["full"] == true && s["ams"] == "object")
    {
        state.full_ams = Some(crate::printer_state::FullAms {
            report,
            epoch: state.epoch,
            at: now(),
            saved,
            reading: crate::printer_state::still_reading(summary),
        });
        state.full_ams_seen.notify_waiters();
    }
    if let Some(value) = &value
        && let Some(start) = &mut state.start
    {
        start.observe(value, &status);
    }
    persist(inventory, state);
}

/// Log what an AMS-bearing (or AMS-replacing full) report said and what it changed in memory.
/// Unchanged diffs go to debug; full reports, changes, new bit patterns and ignore reasons to info.
fn log_ams_report(
    printer: &str,
    state: &mut State,
    summary: &serde_json::Value,
    before: Option<&crate::printer_state::AmsStatus>,
    applied: bool,
    ignored_before: Option<&'static str>,
) {
    let report = state.reports;
    if !applied {
        let reason = state.ignored.map_or("", |(_, reason)| reason);
        if ignored_before == Some(reason) {
            tracing::debug!(
                printer,
                epoch = state.epoch,
                report,
                reason,
                "AMS report ignored"
            );
        } else {
            tracing::info!(printer, epoch = state.epoch, report, reason, summary = %summary, "AMS report ignored");
        }
        return;
    }
    let changes = crate::printer_state::ams_changes(before, state.status(now()).ams.as_ref());
    let bits = serde_json::json!({"exist":summary["exist_bits"],"reading":summary["reading_bits"],"read_done":summary["read_done_bits"]});
    let new_bits = summary["ams"] == "object"
        && bits
            .as_object()
            .is_some_and(|b| b.values().any(|v| !v.is_null()))
        && state.ams_bits.as_ref() != Some(&bits);
    if new_bits {
        state.ams_bits = Some(bits);
    }
    let changes = serde_json::Value::from(changes);
    if summary["full"] == true || new_bits || changes.as_array().is_some_and(|c| !c.is_empty()) {
        tracing::info!(printer, epoch = state.epoch, report, ams_reports = state.ams_reports, summary = %summary, changes = %changes, "AMS report applied");
    } else {
        tracing::debug!(printer, epoch = state.epoch, report, summary = %summary, "AMS report applied without change");
    }
}

/// Save an observation and log what the committed transaction changed, tied to its report.
fn observe(
    db: &crate::database::Database,
    device: &crate::database::Device,
    status: &crate::printer_state::Status,
    source: &'static str,
    report: u64,
) -> Result<()> {
    match db.observe_ams(device, status) {
        Ok(saved) if !saved.changes.is_empty() => {
            tracing::info!(printer = %device.id, source, report, report_at = status.updated_at, saved = saved.saved, changes = %serde_json::json!(saved.changes), "AMS assignment changed");
            Ok(())
        }
        Ok(saved) => {
            if saved.saved > 0 {
                tracing::debug!(printer = %device.id, source, report, saved = saved.saved, "AMS observation saved");
            }
            Ok(())
        }
        Err(error) => {
            // The MQTT path logs through its failure counter; request paths return the error.
            if source != "mqtt" {
                tracing::warn!(printer = %device.id, source, report, kind = save_kind(&error), "AMS observation could not be saved");
            }
            Err(error)
        }
    }
}

/// Record a lost connection; repeated identical failures are summarized.
fn connection_ended(
    printer: &str,
    state: &mut State,
    reason: &'static str,
    detail: Option<&str>,
    inventory: Option<&(crate::database::Database, crate::database::Device)>,
    last_phase: &mut Option<(String, Phase)>,
) {
    let epoch = state.epoch;
    crate::journal::event(
        printer,
        Some(epoch),
        "connection",
        serde_json::json!({"event":"disconnected","reason":reason,"detail":detail}),
    );
    state.disconnected();
    if state.link.failed(reason, now()) {
        tracing::warn!(
            printer,
            epoch,
            reason,
            failures = state.link.failures,
            "P1 MQTT disconnected; retrying every 5 seconds (repeats are summarized every 10 minutes)"
        );
    }
    persist(inventory, state);
    log_phase(printer, last_phase, state);
}

/// A manual full-report request on the current connection: its epoch and report count at sending.
async fn manual_snapshot(
    printer: &str,
    state: &Mutex<State>,
    client: &AsyncClient,
    topic: &str,
    subscribed: bool,
) -> Result<(u64, u64)> {
    if !subscribed {
        return Err(Error::Upstream(
            "Printer is not connected; check its power and network",
        ));
    }
    let marks = {
        let s = state.lock().await;
        (s.epoch, s.reports)
    };
    if snapshot(printer, state, client, topic, "manual").await {
        Ok(marks)
    } else {
        Err(Error::Upstream(
            "Full report request was not sent; check the printer connection",
        ))
    }
}

/// Ask for a full report and record why. Replies carry no reliable link to a request.
async fn snapshot(
    printer: &str,
    state: &Mutex<State>,
    client: &AsyncClient,
    topic: &str,
    trigger: &'static str,
) -> bool {
    let sent = request_snapshot(client, topic).is_ok();
    let mut state = state.lock().await;
    crate::journal::request(printer, state.epoch, SNAPSHOT, sent);
    state.snapshot_request = Some((now(), trigger, sent));
    tracing::info!(
        printer,
        epoch = state.epoch,
        report = state.reports,
        trigger,
        sent,
        "Full printer report requested"
    );
    sent
}

/// Send a transferred start once, and only on the connection it was admitted for.
fn send_start(
    state: &mut State,
    id: &str,
    same_connection: bool,
    client: &AsyncClient,
    request: &str,
    config: &Config,
    inventory: Option<&(crate::database::Database, crate::database::Device)>,
) {
    let status = state.status(now());
    let state_epoch = state.epoch;
    let Some(start) = state
        .start
        .as_mut()
        .filter(|s| s.id == id && s.phase == Phase::Uploading)
    else {
        return;
    };
    let admitted =
        inventory.is_some_and(|(db, device)| db.check_attempt(device, start, &status).is_ok());
    if !same_connection
        || !admitted
        || print_start::check_nozzle(&status, &config.nozzle_diameter, &config.nozzle_material)
            .is_err()
    {
        start.fail(
            Phase::NotSent,
            "Printer status or selected AMS changed during transfer; no start command was sent",
        );
    } else {
        // Commit BEFORE enqueueing MQTT. A crash in either side of this barrier is never replayed.
        start.sent(now());
        let saved =
            inventory.is_some_and(|(db, device)| db.persist_attempt(&device.id, start).is_ok());
        if saved {
            let command = start.command().to_string();
            let queued = client
                .try_publish(request, QoS::AtMostOnce, false, command.clone())
                .is_ok();
            let printer = inventory.map_or("", |(_, device)| device.id.as_str());
            crate::journal::request(printer, state_epoch, command.as_bytes(), queued);
            if !queued {
                start.fail(
                    Phase::NotSent,
                    "MQTT publish was not queued; no start command was sent",
                );
            }
        } else {
            start.fail(
                Phase::NotSent,
                "Cannot persist the start request; no start command was sent",
            );
        }
    }
    persist(inventory, state);
}

/// Save the observed attempt. A failure and its recovery are logged once, not on every retry.
fn persist(
    inventory: Option<&(crate::database::Database, crate::database::Device)>,
    state: &mut State,
) {
    let (Some((db, device)), Some(attempt)) = (inventory, state.start.as_ref()) else {
        return;
    };
    match db.persist_attempt(&device.id, attempt) {
        Ok(()) => {
            if let Some(failures) = state.saves.saved("attempt") {
                tracing::info!(printer = %device.id, job = %attempt.job_id, attempt = %attempt.id, failures, "Print observation saved again");
            }
        }
        Err(error) => {
            let kind = save_kind(&error);
            if state.saves.failed("attempt", kind, now()) {
                tracing::warn!(printer = %device.id, job = %attempt.job_id, attempt = %attempt.id, kind, "Print observation could not be saved; retrying on the next report or tick");
            }
        }
    }
}

fn log_phase(printer: &str, last: &mut Option<(String, Phase)>, state: &State) {
    if let (Some((from, to)), Some(a)) = (transition(last, state.start.as_ref()), &state.start) {
        tracing::info!(
            printer,
            epoch = state.epoch,
            job = %a.job_id,
            attempt = %a.id,
            from = ?from,
            to = ?to,
            reason = a.message.as_deref().unwrap_or(""),
            "Print attempt phase changed"
        );
    }
}

type Ftps = suppaftp::tokio::AsyncRustlsFtpStream;

/// Log in over implicit FTPS; an error names the failed stage, never the server's reply.
async fn ftps(config: &Config) -> std::result::Result<Ftps, &'static str> {
    use suppaftp::{tokio::AsyncRustlsConnector, types::FileType};
    // One TLS config per upload shares the control session with its data connection.
    let connector = AsyncRustlsConnector::from(suppaftp::tokio_rustls::TlsConnector::from(
        config.tls().map_err(|_| "tls")?,
    ));
    let mut ftp = Ftps::connect_secure_implicit(
        (config.ip, config.ftps_port),
        connector,
        &config.ip.to_string(),
    )
    .await
    .map_err(|_| "connect")?;
    // Keep PASV's dynamic port, but never follow its address to another host.
    ftp.set_passive_nat_workaround(true);
    if config.ip.is_ipv6() {
        ftp.set_mode(suppaftp::Mode::ExtendedPassive);
    }
    ftp.login("bblp", &config.access_code)
        .await
        .map_err(|_| "login")?;
    ftp.custom_command("PBSZ 0", &[suppaftp::Status::CommandOk])
        .await
        .map_err(|_| "protection")?;
    ftp.custom_command("PROT P", &[suppaftp::Status::CommandOk])
        .await
        .map_err(|_| "protection")?;
    ftp.transfer_type(FileType::Binary)
        .await
        .map_err(|_| "binary_mode")?;
    Ok(ftp)
}

/// Upload the print file; an error names the failed stage, never the server's reply.
async fn upload(
    config: &Config,
    name: &str,
    bytes: &[u8],
) -> std::result::Result<(), &'static str> {
    let mut ftp = ftps(config).await?;
    let count = ftp
        .put_file(name, &mut std::io::Cursor::new(bytes))
        .await
        .map_err(|_| "transfer")?;
    if count != bytes.len() as u64 {
        return Err("size");
    }
    // A final positive transfer reply is authoritative; QUIT failure cannot undo it.
    let _ = tokio::time::timeout(Duration::from_secs(2), ftp.quit()).await;
    Ok(())
}

fn io_kind(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::{
        BrokenPipe, ConnectionAborted, ConnectionRefused, ConnectionReset, HostUnreachable,
        NetworkUnreachable, TimedOut, UnexpectedEof,
    };
    if error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some()
    {
        return "tls";
    }
    match error.kind() {
        TimedOut => "timeout",
        ConnectionRefused => "refused",
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => "closed",
        HostUnreachable | NetworkUnreachable => "unreachable",
        _ => "io",
    }
}

/// A safe category for logs and diagnostics; library errors can echo packets or credentials.
fn classify(error: &rumqttc::ConnectionError) -> &'static str {
    use rumqttc::{ConnectReturnCode, ConnectionError, StateError};
    match error {
        ConnectionError::Io(e)
        | ConnectionError::MqttState(
            StateError::Io(e) | StateError::Deserialization(rumqttc::mqttbytes::Error::Io(e)),
        ) => io_kind(e),
        ConnectionError::NetworkTimeout | ConnectionError::FlushTimeout => "timeout",
        ConnectionError::Tls(_) => "tls",
        ConnectionError::ConnectionRefused(
            ConnectReturnCode::NotAuthorized | ConnectReturnCode::BadUserNamePassword,
        ) => "auth",
        ConnectionError::ConnectionRefused(_) => "refused",
        ConnectionError::MqttState(StateError::AwaitPingResp) => "keepalive",
        ConnectionError::MqttState(StateError::ConnectionAborted)
        | ConnectionError::RequestsDone => "closed",
        _ => "protocol",
    }
}

fn save_kind(error: &Error) -> &'static str {
    match error {
        Error::Unavailable(_) => "database",
        Error::Conflict(_) => "conflict",
        Error::Io(_) => "io",
        _ => "other",
    }
}

/// The attempt's phase when it differs from the last one seen for the same attempt.
fn transition(
    last: &mut Option<(String, Phase)>,
    start: Option<&Attempt>,
) -> Option<(Option<Phase>, Phase)> {
    let start = start?;
    let before = last
        .as_ref()
        .filter(|(id, _)| *id == start.id)
        .map(|(_, phase)| *phase);
    if before == Some(start.phase) {
        return None;
    }
    *last = Some((start.id.clone(), start.phase));
    Some((before, start.phase))
}

const SNAPSHOT: &[u8] =
    br#"{"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}"#;

/// Ask for module versions as `BambuStudio` does on connect; the answer is kept in the status.
fn request_version(client: &AsyncClient, topic: &str, printer: &str, epoch: u64) {
    let sequence = (uuid::Uuid::new_v4().as_u128() % 2_000_000_000 + 1).to_string();
    let message = crate::bambu::Control::Version
        .message(&sequence)
        .expect("version needs no values")
        .payload
        .to_string();
    let queued = client
        .try_publish(topic, QoS::AtMostOnce, false, message.clone())
        .is_ok();
    crate::journal::request(printer, epoch, message.as_bytes(), queued);
}

fn request_snapshot(
    client: &AsyncClient,
    topic: &str,
) -> std::result::Result<(), rumqttc::ClientError> {
    client.try_publish(topic, QoS::AtMostOnce, false, SNAPSHOT.to_vec())
}

fn queue_refill(
    client: &AsyncClient,
    topic: &str,
    enabled: bool,
    printer: &str,
    epoch: u64,
) -> Result<()> {
    let sequence = (uuid::Uuid::new_v4().as_u128() % 2_000_000_000 + 1).to_string();
    let payload = serde_json::json!({"print":{"command":"print_option","sequence_id":sequence,"auto_switch_filament":enabled}}).to_string();
    let queued = client
        .try_publish(topic, QoS::AtMostOnce, false, payload.clone())
        .is_ok();
    crate::journal::request(printer, epoch, payload.as_bytes(), queued);
    queued
        .then_some(())
        .ok_or(Error::Unavailable("Setting request was not queued"))
}

/// The TLS library's own description of a failed handshake; it carries no packets or credentials.
fn tls_detail(error: &rumqttc::ConnectionError) -> Option<String> {
    use rumqttc::{ConnectionError, StateError};
    match error {
        ConnectionError::Tls(e) => Some(e.to_string()),
        ConnectionError::Io(e) | ConnectionError::MqttState(StateError::Io(e)) => e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .map(ToString::to_string),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)] // One select loop owns the connection and its requests.
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
    let printer = inventory
        .as_ref()
        .map_or(String::new(), |(_, device)| device.id.clone());
    let mut last_phase = None;
    loop {
        // New queues on reconnect: requests from a lost connection are never replayed.
        let (client, mut events) = AsyncClient::new(options.clone(), 8);
        events.network_options.set_connection_timeout(5);
        let mut refresh = tokio::time::interval(Duration::from_mins(5));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut subscribed = false;
        let mut pending: Pending = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        // Why this connection ended, as a safe category.
        let (reason, detail) = loop {
            tokio::select! {
                event = events.poll() => match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        let mut state = state.lock().await;
                        state.connected();
                        state.link.connected(now());
                        crate::journal::event(&printer, Some(state.epoch), "connection", serde_json::json!({"event":"connected"}));
                        tracing::info!(printer, epoch = state.epoch, "P1 MQTT connected; requesting a full report");
                        drop(state);
                        if client.try_subscribe(&report, QoS::AtMostOnce).is_err() { break ("subscribe", None); }
                    }
                    Ok(Event::Incoming(Incoming::SubAck(ack))) => {
                        let ok = ack.return_codes.len() == 1 && matches!(ack.return_codes[0], SubscribeReasonCode::Success(_));
                        crate::journal::event(&printer, Some(state.lock().await.epoch), "connection", serde_json::json!({"event":"subscribed","ok":ok}));
                        if !ok { break ("subscribe", None); }
                        subscribed = true;
                        if !snapshot(&printer, &state, &client, &request, "subscribed").await { break ("publish", None); }
                        request_version(&client, &request, &printer, state.lock().await.epoch);
                        refresh.reset();
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) if subscribed && message.topic == report && !message.retain => {
                        if !pending.is_empty() && let Ok(value) = serde_json::from_slice(&message.payload) {
                            answer(&mut pending, &value);
                        }
                        let mut state = state.lock().await;
                        on_report(&printer, &mut state, inventory.as_ref(), &message.payload).await;
                        log_phase(&printer, &mut last_phase, &state);
                    }
                    Ok(_) => {},
                    // Never log library errors or packets: they may contain credentials.
                    Err(error) => break (classify(&error), tls_detail(&error)),
                },
                Some(command) = starts.recv() => {
                    let command = match command {
                        Request::Snapshot { response } => {
                            let _ = response.send(manual_snapshot(&printer, &state, &client, &request, subscribed).await);
                            continue;
                        }
                        Request::Control { message, response } => {
                            if subscribed {
                                let payload = message.payload.to_string();
                                let queued = client.try_publish(&request, QoS::AtMostOnce, false, payload.clone()).is_ok();
                                crate::journal::request(&printer, state.lock().await.epoch, payload.as_bytes(), queued);
                                if queued { pending.push((message, response, now())); }
                                else { let _ = response.send(Err(Error::Unavailable("Control was not queued; check the printer connection"))); }
                            } else {
                                let _ = response.send(Err(Error::Conflict("Printer is not connected; check its power and network")));
                            }
                            continue;
                        }
                        other => other,
                    };
                    let mut state = state.lock().await;
                    let status=state.status(now());
                    let (id,epoch)=match command {
                        Request::Start(id,epoch) => (id,epoch),
                        Request::Snapshot { .. } | Request::Control { .. } => unreachable!("handled before locking"),
                        Request::AutoRefill{enabled,epoch,response} => {
                            let result=if response.is_closed() || !subscribed || state.epoch!=epoch || !status.synchronized || status.auto_refill.supported!=Some(true) {
                                Err(Error::Conflict("Printer connection or auto refill support changed"))
                            } else {
                                queue_refill(&client,&request,enabled,&printer,state.epoch)
                            };
                            let _=response.send(result);
                            continue;
                        },
                    };
                    let same_connection = state.epoch == epoch && subscribed;
                    send_start(&mut state, &id, same_connection, &client, &request, &config, inventory.as_ref());
                    log_phase(&printer, &mut last_phase, &state);
                }
                _ = tick.tick() => {
                    expire(&mut pending, now(), "No reply from the printer within 10 seconds; check the reported state");
                    let mut state = state.lock().await;
                    let synchronized = state.status(now()).synchronized;
                    let mut timed_out = false;
                    if let Some(start) = &mut state.start {
                        let pending = matches!(start.phase, Phase::AwaitingConfirmation | Phase::Accepted | Phase::Printing | Phase::Paused);
                        start.tick(now(), config.start_timeout);
                        if !synchronized { start.disconnected(); }
                        timed_out = pending && start.phase == Phase::Unknown;
                    }
                    persist(inventory.as_ref(), &mut state);
                    log_phase(&printer, &mut last_phase, &state);
                    // Drop this client's queue so timed-out writes cannot be sent later.
                    if timed_out { break (if synchronized { "start_timeout" } else { "unsynchronized" }, None); }
                }
                _ = refresh.tick() => {
                    if subscribed && !state.lock().await.status(now()).synchronized && !snapshot(&printer, &state, &client, &request, "refresh").await { break ("publish", None); }
                }
            }
        };
        expire(
            &mut pending,
            u64::MAX,
            "Connection ended before the printer replied; check the reported state",
        );
        connection_ended(
            &printer,
            &mut *state.lock().await,
            reason,
            detail.as_deref(),
            inventory.as_ref(),
            &mut last_phase,
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
            camera_port: 6000,
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

#[cfg(test)]
mod classification_tests {
    use super::*;
    use rumqttc::{ConnectReturnCode, ConnectionError, StateError};
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn connection_errors_map_to_safe_categories_without_their_text() {
        for (error, expected) in [
            (
                ConnectionError::Io(IoError::from(ErrorKind::ConnectionRefused)),
                "refused",
            ),
            (
                ConnectionError::Io(IoError::from(ErrorKind::TimedOut)),
                "timeout",
            ),
            (
                ConnectionError::Io(IoError::from(ErrorKind::ConnectionReset)),
                "closed",
            ),
            (
                ConnectionError::Io(IoError::from(ErrorKind::UnexpectedEof)),
                "closed",
            ),
            (
                ConnectionError::Io(IoError::from(ErrorKind::HostUnreachable)),
                "unreachable",
            ),
            (
                ConnectionError::Io(IoError::other("access-code-secret")),
                "io",
            ),
            (ConnectionError::NetworkTimeout, "timeout"),
            (ConnectionError::FlushTimeout, "timeout"),
            (
                ConnectionError::ConnectionRefused(ConnectReturnCode::NotAuthorized),
                "auth",
            ),
            (
                ConnectionError::ConnectionRefused(ConnectReturnCode::BadUserNamePassword),
                "auth",
            ),
            (
                ConnectionError::ConnectionRefused(ConnectReturnCode::ServiceUnavailable),
                "refused",
            ),
            (
                ConnectionError::MqttState(StateError::AwaitPingResp),
                "keepalive",
            ),
            (
                ConnectionError::MqttState(StateError::ConnectionAborted),
                "closed",
            ),
            (
                ConnectionError::MqttState(StateError::Io(IoError::from(ErrorKind::TimedOut))),
                "timeout",
            ),
            (
                ConnectionError::MqttState(StateError::WrongPacket),
                "protocol",
            ),
            (
                ConnectionError::MqttState(StateError::Deserialization(
                    rumqttc::mqttbytes::Error::Io(IoError::from(ErrorKind::UnexpectedEof)),
                )),
                "closed",
            ),
            (
                ConnectionError::Io(IoError::new(
                    ErrorKind::InvalidData,
                    rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding),
                )),
                "tls",
            ),
        ] {
            assert_eq!(classify(&error), expected, "{error:?}");
        }
    }

    #[test]
    fn phase_changes_are_reported_once_per_attempt_and_phase() {
        let mut last = None;
        let mut a = Attempt::new("p".into(), "j".into(), 0, "PLA".into());
        assert_eq!(transition(&mut last, None), None);
        assert_eq!(
            transition(&mut last, Some(&a)),
            Some((None, Phase::Uploading))
        );
        assert_eq!(transition(&mut last, Some(&a)), None);
        a.sent(1);
        assert_eq!(
            transition(&mut last, Some(&a)),
            Some((Some(Phase::Uploading), Phase::AwaitingConfirmation))
        );
        let b = Attempt::new("p".into(), "j".into(), 0, "PLA".into());
        assert_eq!(
            transition(&mut last, Some(&b)),
            Some((None, Phase::Uploading))
        );
    }

    #[test]
    fn save_errors_keep_only_their_category() {
        assert_eq!(
            save_kind(&Error::Unavailable("Database operation failed")),
            "database"
        );
        assert_eq!(
            save_kind(&Error::Conflict("Execution is no longer active")),
            "conflict"
        );
        assert_eq!(save_kind(&Error::Io(std::io::Error::other("x"))), "io");
        assert_eq!(save_kind(&Error::NotFound), "other");
    }
}
