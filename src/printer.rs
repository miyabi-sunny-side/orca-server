use crate::printer_state::State;
use axum::{Json, Router, routing::get};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, SubscribeReasonCode, Transport};
use rustls::pki_types::pem::PemObject;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    io::Read,
    net::IpAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

#[derive(Debug)]
struct PinnedCertificate(CertificateDer<'static>);
impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if cert == &self.0 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
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

pub struct Config {
    ip: IpAddr,
    port: u16,
    serial: String,
    access_code: String,
    certificate: PathBuf,
}

impl Config {
    fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        let ip = get("P1_IP");
        let serial = get("P1_SERIAL");
        let access_code = get("P1_ACCESS_CODE");
        let certificate = get("P1_TLS_CERT");
        let port = get("P1_MQTT_PORT");
        if [&ip, &serial, &access_code, &certificate, &port]
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
        Ok(Some(Self {
            ip,
            port,
            serial,
            access_code,
            certificate,
        }))
    }

    /// Read printer-only settings; absence leaves printer integration disabled.
    /// # Errors
    /// Rejects incomplete settings or malformed values without echoing credentials.
    pub fn from_env() -> Result<Option<Self>, &'static str> {
        let mut values = std::collections::BTreeMap::new();
        for name in [
            "P1_IP",
            "P1_SERIAL",
            "P1_ACCESS_CODE",
            "P1_TLS_CERT",
            "P1_MQTT_PORT",
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
    fn options(&self) -> Result<MqttOptions, &'static str> {
        let file = std::fs::File::open(&self.certificate).map_err(|_| "Cannot read P1_TLS_CERT")?;
        let mut pem = Vec::new();
        file.take(65_537)
            .read_to_end(&mut pem)
            .map_err(|_| "Cannot read P1_TLS_CERT")?;
        if pem.len() > 65_536 {
            return Err("P1_TLS_CERT exceeds 64 KiB");
        }
        let certificates = CertificateDer::pem_slice_iter(&pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "P1_TLS_CERT must be a PEM certificate")?;
        if certificates.len() != 1 {
            return Err("P1_TLS_CERT must contain exactly one printer certificate");
        }
        rustls::server::ParsedCertificate::try_from(&certificates[0])
            .map_err(|_| "P1_TLS_CERT contains an invalid certificate")?;
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|_| "Cannot configure printer TLS")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCertificate(certificates[0].clone())))
        .with_no_client_auth();
        let mut options = MqttOptions::new(
            format!("orca-server-{}", uuid::Uuid::new_v4()),
            self.ip.to_string(),
            self.port,
        );
        options.set_credentials("bblp", &self.access_code);
        options.set_transport(Transport::tls_with_config(tls.into()));
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

/// Start the configured printer connection and expose its current observation.
/// # Errors
/// Rejects a missing, oversized, or malformed pinned certificate.
pub fn router(config: Option<Config>) -> Result<Router, &'static str> {
    let state = Arc::new(Mutex::new(State::new(config.is_some())));
    if let Some(config) = config {
        let options = config.options()?;
        tokio::spawn(run(options, config.serial, state.clone()));
    }
    Ok(Router::new().route(
        "/api/printer/status",
        get(move || {
            let state = state.clone();
            async move { Json(state.lock().await.status(now())) }
        }),
    ))
}

fn request_snapshot(client: &AsyncClient, topic: &str) -> Result<(), rumqttc::ClientError> {
    client.try_publish(
        topic,
        QoS::AtMostOnce,
        false,
        br#"{"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}"#
            .to_vec(),
    )
}

async fn run(options: MqttOptions, serial: String, state: Arc<Mutex<State>>) {
    let report = format!("device/{serial}/report");
    let request = format!("device/{serial}/request");
    loop {
        // New queues on reconnect: requests from a lost connection are never replayed.
        let (client, mut events) = AsyncClient::new(options.clone(), 8);
        events.network_options.set_connection_timeout(5);
        let mut refresh = tokio::time::interval(Duration::from_mins(5));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut subscribed = false;
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
                        state.lock().await.apply(&message.payload, now());
                    }
                    Ok(_) => {},
                    Err(_) => break, // Never log library errors or packets: they may contain credentials.
                },
                _ = refresh.tick() => {
                    if subscribed && !state.lock().await.status(now()).synchronized && request_snapshot(&client, &request).is_err() { break; }
                }
            }
        }
        state.lock().await.disconnected();
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

    #[test]
    fn malformed_certificate_is_a_startup_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.pem");
        std::fs::write(
            &file,
            "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let config = Config {
            ip: "127.0.0.1".parse().unwrap(),
            port: 8883,
            serial: "TESTSERIAL".into(),
            access_code: "test-only-secret".into(),
            certificate: file,
        };
        assert!(config.options().is_err());
    }

    #[test]
    fn only_the_explicitly_pinned_certificate_is_accepted() {
        let trusted = CertificateDer::from(vec![1, 2, 3]);
        let pinned = PinnedCertificate(trusted.clone());
        let name = ServerName::try_from("127.0.0.1").unwrap();
        assert!(
            pinned
                .verify_server_cert(&trusted, &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        assert!(
            pinned
                .verify_server_cert(
                    &CertificateDer::from(vec![4, 5, 6]),
                    &[],
                    &name,
                    &[],
                    UnixTime::now()
                )
                .is_err()
        );
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
        assert_eq!(config.certificate, PathBuf::from("/tmp/printer.pem"));
        for (key, bad) in [
            ("P1_IP", "bad"),
            ("P1_SERIAL", "../+/test-secret"),
            ("P1_ACCESS_CODE", ""),
            ("P1_TLS_CERT", ""),
            ("P1_MQTT_PORT", "0"),
            ("P1_MQTT_PORT", "65536"),
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
