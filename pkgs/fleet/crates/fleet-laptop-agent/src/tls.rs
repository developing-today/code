//! Broker TLS Phase 1 on the client side of the laptop-agent
//! endpoint (BT-4 slice 2, #14402; design of record
//! doc/2026-11-24T13-59-26Z_design_broker-tls §§4–6), mirroring the
//! queue client's posture from BT-3:
//!
//! - **M1 — additive, default off.** The Tailscale leg validates only
//!   when `LAPTOP_AGENT_TLS_TRUST_STORE` names a BT-2 trust store.
//!   Unset, the leg speaks plaintext Phase 0 exactly as before (the
//!   daemon dual-accepts, slice 1).
//! - **Fail closed.** With a trust store, the daemon's leaf must chain
//!   to the store's active classical anchors and name the dialled
//!   address; anything else refuses the connection before a request
//!   byte is written. A store that cannot be read refuses the leg too.
//!   Neither ever falls back to plaintext; the ladder moves on to its
//!   next leg instead.
//! - **Named break-glass.** `LAPTOP_AGENT_TLS_BREAK_GLASS=laptop-agent`
//!   skips validation, still encrypts, and logs on every use. Any other
//!   non-empty value is a config error, so a typo never silently turns
//!   validation off.
//!
//! The certificate is identity; the token is still the authorisation.

use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fleet_cert::chain::TrustSelection;
use fleet_cert::trust::active_anchors;
use fleet_proxy::http::HttpConn;
use fleet_proxy::tunnel::tls_over_tcp;
use rustls::ClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// The trust store variable (a BT-2 store directory).
pub const TRUST_STORE_VAR: &str = "LAPTOP_AGENT_TLS_TRUST_STORE";
/// The break-glass variable and the one value that enables it.
pub const BREAK_GLASS_VAR: &str = "LAPTOP_AGENT_TLS_BREAK_GLASS";
pub const BREAK_GLASS_VALUE: &str = "laptop-agent";

/// How long the TLS handshake may take once the TCP leg is up.
pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);

/// The client's posture toward the daemon endpoint.
pub enum AgentTls {
    /// Phase 0: plaintext, no validation (neither variable set).
    Plain,
    /// Phase 1: TLS validated against the trust store's anchors.
    Validate {
        config: Arc<ClientConfig>,
        version: u32,
        content_sha256: String,
    },
    /// Break-glass: TLS, no validation, logged on every use.
    BreakGlass { config: Arc<ClientConfig> },
}

impl AgentTls {
    /// Resolve the posture from the environment.
    pub fn from_env() -> Result<AgentTls, String> {
        Self::from_vars(
            std::env::var(BREAK_GLASS_VAR).ok().as_deref(),
            std::env::var(TRUST_STORE_VAR).ok().as_deref(),
        )
    }

    /// The posture for these variable values: break-glass wins over a
    /// trust store, which wins over Phase 0.
    pub fn from_vars(break_glass: Option<&str>, store: Option<&str>) -> Result<AgentTls, String> {
        if let Some(v) = break_glass.map(str::trim).filter(|v| !v.is_empty()) {
            if v == BREAK_GLASS_VALUE {
                return Ok(Self::break_glass());
            }
            return Err(format!(
                "{BREAK_GLASS_VAR}={v:?} is not the named break-glass override; \
                 set it to \"{BREAK_GLASS_VALUE}\" to skip laptop-agent TLS validation"
            ));
        }
        match store.map(str::trim).filter(|s| !s.is_empty()) {
            Some(store) => Self::validate_from_store(Path::new(store)),
            None => Ok(Self::Plain),
        }
    }

    /// Validate against the active classical anchors of the BT-2
    /// trust store at `store` (re-verified against the recorded
    /// content hash on every read; drift fails closed).
    pub fn validate_from_store(store: &Path) -> Result<AgentTls, String> {
        let (bundle, anchors_pem) = active_anchors(store, TrustSelection::Classical)
            .map_err(|e| format!("laptop-agent TLS trust store {}: {e}", store.display()))?;
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(&anchors_pem) {
            let cert = cert.map_err(|e| {
                format!(
                    "laptop-agent TLS trust store {}: cannot parse anchor PEM: {e}",
                    store.display()
                )
            })?;
            roots.add(cert).map_err(|e| {
                format!(
                    "laptop-agent TLS trust store {}: anchor is not a usable trust root: {e}",
                    store.display()
                )
            })?;
        }
        if roots.is_empty() {
            return Err(format!(
                "laptop-agent TLS trust store {}: the active bundle holds no classical anchors",
                store.display()
            ));
        }
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS protocol versions: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self::Validate {
            config: Arc::new(config),
            version: bundle.version,
            content_sha256: bundle.content_sha256,
        })
    }

    /// Encrypt, validate nothing.
    pub fn break_glass() -> AgentTls {
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("rustls default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
            .with_no_client_auth();
        Self::BreakGlass {
            config: Arc::new(config),
        }
    }

    /// The trust bundle version validated against, if any.
    pub fn bundle_version(&self) -> Option<u32> {
        match self {
            Self::Validate { version, .. } => Some(*version),
            _ => None,
        }
    }

    /// An HTTP connection over `tcp` per the posture. `host` is the
    /// dialled address, which the daemon's leaf must name.
    pub fn connect(&self, tcp: TcpStream, host: &str) -> std::io::Result<HttpConn> {
        let config = match self {
            Self::Plain => return HttpConn::new(tcp),
            Self::Validate { config, .. } => config,
            Self::BreakGlass { config } => {
                eprintln!(
                    "laptop-agent: BREAK-GLASS: {BREAK_GLASS_VAR}={BREAK_GLASS_VALUE} — \
                     connecting to {host} WITHOUT validating the daemon certificate"
                );
                config
            }
        };
        match tls_over_tcp(tcp, Arc::clone(config), host, HANDSHAKE_DEADLINE) {
            Ok(stream) => HttpConn::from_tls(stream),
            Err(e) => Err(std::io::Error::new(
                e.kind(),
                match self {
                    Self::Validate {
                        version,
                        content_sha256,
                        ..
                    } => format!(
                        "laptop-agent TLS validation failed for {host} against trust bundle \
                         version {version} (content {content_sha256}): {e} — connection \
                         refused, fail closed; if the daemon leaf was re-issued, roll the \
                         trust bundle forward, or use the named break-glass \
                         {BREAK_GLASS_VAR}={BREAK_GLASS_VALUE}"
                    ),
                    _ => format!("laptop-agent TLS handshake with {host} failed: {e}"),
                },
            )),
        }
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The break-glass verifier: accepts any server certificate. Used
/// only behind the named override.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    //! A synthetic estate (fleet-cert root set → Base issuer →
    //! `broker-server` leaf from an on-host key + CSR → BT-2 trust
    //! bundle staged, acknowledged and activated), a loopback TLS
    //! server presenting the leaf, and the real client connect path.
    use super::AgentTls;
    use fleet_cert::algorithm::AlgorithmId;
    use fleet_cert::bundle::{load_bundle, read_artifact};
    use fleet_cert::engine;
    use fleet_cert::graph::CrossSigningMode;
    use fleet_cert::model::{
        IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
    };
    use fleet_cert::timeparse::{CalendarDuration, Rfc3339};
    use fleet_cert::trust::{
        TrustExportRequest, acknowledge, activate, export_trust_bundle, stage_bundle,
    };
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const NB: &str = "2026-01-01T00:00:00Z";
    const LEAF_NB: &str = "2026-11-01T00:00:00Z";
    const AT: &str = "2026-11-20T00:00:00Z";
    const DAEMON_IP: &str = "100.99.113.103";
    const TOKEN: &str = "test-token";

    fn ts(s: &str) -> Rfc3339 {
        Rfc3339::parse(s).unwrap()
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "fleet-laptop-agent-tls-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Estate {
        store: PathBuf,
        chain_pem: Vec<u8>,
        key_pem: String,
    }

    fn build_estate(tmp: &TempDir, tag: &str, org: &str) -> Estate {
        let root_dir = tmp.path().join(format!("{tag}-roots"));
        engine::create_root_set(
            &RootCreateRequest {
                organization: org.into(),
                country: "US".into(),
                algorithms: vec![AlgorithmId::P256],
                generation: 1,
                partition: "A".into(),
                cross_signing: CrossSigningMode::Both,
                not_before: ts(NB),
                not_after: ts("2046-01-01T00:00:00Z"),
                allow_legacy_root_strength: false,
                private_store: None,
            },
            &root_dir,
        )
        .unwrap();
        let issuer_dir = tmp.path().join(format!("{tag}-issuer"));
        engine::create_issuer_set(
            &IssuerCreateRequest {
                root_bundle: root_dir.clone(),
                root_signer: SignerSource {
                    key_dir: root_dir.join("private/keys"),
                },
                name: "Base".into(),
                purpose: IssuerPurpose::General,
                ou: None,
                slots: vec![(AlgorithmId::P256, "a".to_string())],
                cross_signing: CrossSigningMode::RootsOnly,
                not_before: ts(NB),
                not_after: ts("2031-01-01T00:00:00Z"),
                allow_legacy_root_strength: false,
                permitted_dns: vec![],
                private_store: None,
            },
            &issuer_dir,
        )
        .unwrap();

        // The laptop's own key + CSR: the key never leaves the host.
        let host_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let csr_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        let csr = csr_params.serialize_request(&host_key).unwrap();
        let csr_path = tmp.path().join(format!("{tag}-daemon.csr.pem"));
        std::fs::write(&csr_path, csr.pem().unwrap()).unwrap();

        let leaf_dir = tmp.path().join(format!("{tag}-leaf"));
        engine::issue_leaf(
            &LeafIssueRequest {
                issuer_bundle: issuer_dir.clone(),
                issuer_signer: SignerSource {
                    key_dir: issuer_dir.join("private/keys"),
                },
                profile: "broker-server".into(),
                name: "laptop-agent".into(),
                common_name: None,
                dns_names: vec!["laptop-agent.internal".into()],
                ip_addresses: vec![DAEMON_IP.parse().unwrap()],
                csr: Some(csr_path),
                key_algorithm: None,
                issuer_algorithm: Some(AlgorithmId::P256),
                issuer_partition: None,
                not_before: ts(LEAF_NB),
                not_after: CalendarDuration::days(90).add_to(ts(LEAF_NB)).unwrap(),
                private_store: None,
            },
            &leaf_dir,
        )
        .unwrap();

        let leaf_bundle = load_bundle(&leaf_dir).unwrap();
        let leaf_record = leaf_bundle
            .manifest
            .certificates
            .iter()
            .find(|r| r.role == "leaf")
            .unwrap();
        let mut chain_pem = read_artifact(&leaf_bundle, &leaf_record.cert_path).unwrap();
        let issuer_bundle = load_bundle(&issuer_dir).unwrap();
        for record in &issuer_bundle.manifest.certificates {
            if record.role == "issuer" {
                chain_pem.extend(read_artifact(&issuer_bundle, &record.cert_path).unwrap());
            }
        }

        let bundle_dir = tmp.path().join(format!("{tag}-trust-bundle"));
        export_trust_bundle(
            &TrustExportRequest {
                root_bundles: vec![root_dir],
                version: 1,
                name: "Fleet Trust Bundle".into(),
                at: ts(AT),
            },
            &bundle_dir,
        )
        .unwrap();
        let store = tmp.path().join(format!("{tag}-store"));
        stage_bundle(&store, &bundle_dir).unwrap();
        acknowledge(&store, None, ts(AT)).unwrap();
        activate(&store, None).unwrap();

        Estate {
            store,
            chain_pem,
            key_pem: host_key.serialize_pem(),
        }
    }

    /// Read one request head (the client sends GETs with no body).
    fn read_head(stream: &mut impl Read) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => break,
            }
        }
        String::from_utf8_lossy(&head).to_string()
    }

    fn respond(stream: &mut impl Write, head: &str) {
        let (status, body) = if head
            .lines()
            .any(|l| l.eq_ignore_ascii_case(&format!("X-Agent-Token: {TOKEN}")))
        {
            ("200 OK", "{\"ok\":true}")
        } else {
            ("401 Unauthorized", "{\"error\":\"bad token\"}")
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.flush();
    }

    /// A loopback daemon stand-in presenting the estate's leaf; it
    /// answers each connection once, in the daemon's reply shape.
    fn serve_tls(estate: &Estate) -> std::net::SocketAddr {
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&estate.chain_pem)
            .collect::<Result<_, _>>()
            .unwrap();
        let key = PrivateKeyDer::from_pem_slice(estate.key_pem.as_bytes()).unwrap();
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let config = Arc::clone(&config);
                std::thread::spawn(move || {
                    let conn = rustls::ServerConnection::new(config).unwrap();
                    let mut tls = rustls::StreamOwned::new(conn, stream);
                    let head = read_head(&mut tls);
                    if head.is_empty() {
                        return;
                    }
                    respond(&mut tls, &head);
                    tls.conn.send_close_notify();
                    let _ = tls.conn.write_tls(&mut tls.sock);
                });
            }
        });
        addr
    }

    /// One GET /health over the posture, as the Tailscale leg sends it.
    fn health(addr: std::net::SocketAddr, mode: &AgentTls, token: &str) -> std::io::Result<u16> {
        let tcp = TcpStream::connect(addr)?;
        let mut conn = mode.connect(tcp, DAEMON_IP)?;
        conn.request_with_extra_headers("GET", "/health", DAEMON_IP, token, None, &[])
            .map(|(status, _)| status)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    #[test]
    fn validated_leg_round_trips_and_keeps_the_token_gate() {
        let tmp = TempDir::new("roundtrip");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let addr = serve_tls(&estate);
        let mode = AgentTls::validate_from_store(&estate.store).unwrap();
        assert_eq!(mode.bundle_version(), Some(1));
        assert_eq!(health(addr, &mode, TOKEN).unwrap(), 200);
        assert_eq!(health(addr, &mode, "wrong-token").unwrap(), 401);
    }

    #[test]
    fn an_untrusted_daemon_leaf_is_refused_fail_closed() {
        let tmp = TempDir::new("untrusted");
        let estate_a = build_estate(&tmp, "a", "Test Org A LLC");
        let estate_b = build_estate(&tmp, "b", "Test Org B LLC");
        let addr = serve_tls(&estate_a);
        let mode = AgentTls::validate_from_store(&estate_b.store).unwrap();
        let err = health(addr, &mode, TOKEN).unwrap_err().to_string();
        assert!(
            err.contains("TLS validation failed") && err.contains("fail closed"),
            "{err}"
        );
        assert!(err.contains("trust bundle version 1"), "{err}");
    }

    #[test]
    fn a_leaf_for_another_address_is_refused() {
        let tmp = TempDir::new("name");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let addr = serve_tls(&estate);
        let mode = AgentTls::validate_from_store(&estate.store).unwrap();
        let tcp = TcpStream::connect(addr).unwrap();
        let err = match mode.connect(tcp, "100.99.113.104") {
            Ok(_) => panic!("a leaf naming another address was accepted"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("TLS validation failed"), "{err}");
    }

    #[test]
    fn break_glass_connects_without_validation() {
        let tmp = TempDir::new("breakglass");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let addr = serve_tls(&estate);
        let mode = AgentTls::from_vars(Some("laptop-agent"), None).unwrap();
        assert_eq!(mode.bundle_version(), None);
        assert_eq!(health(addr, &mode, TOKEN).unwrap(), 200);
    }

    #[test]
    fn a_validating_client_refuses_a_plaintext_daemon() {
        let tmp = TempDir::new("plaindaemon");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let mode = AgentTls::validate_from_store(&estate.store).unwrap();
        let err = health(addr, &mode, TOKEN).unwrap_err().to_string();
        assert!(err.contains("TLS validation failed"), "{err}");
    }

    #[test]
    fn plain_posture_speaks_phase0_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let head = read_head(&mut stream);
                respond(&mut stream, &head);
            }
        });
        let mode = AgentTls::from_vars(None, None).unwrap();
        assert!(matches!(mode, AgentTls::Plain));
        assert_eq!(health(addr, &mode, TOKEN).unwrap(), 200);
    }

    #[test]
    fn env_values_resolve_by_precedence_and_typos_are_errors() {
        let tmp = TempDir::new("vars");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let store = estate.store.to_str().unwrap();
        assert!(matches!(
            AgentTls::from_vars(Some(" "), Some("")).unwrap(),
            AgentTls::Plain
        ));
        assert!(matches!(
            AgentTls::from_vars(None, Some(store)).unwrap(),
            AgentTls::Validate { version: 1, .. }
        ));
        assert!(matches!(
            AgentTls::from_vars(Some("laptop-agent"), Some(store)).unwrap(),
            AgentTls::BreakGlass { .. }
        ));
        let typo = match AgentTls::from_vars(Some("queue-broker"), Some(store)) {
            Err(e) => e,
            Ok(_) => panic!("a wrong break-glass name was accepted"),
        };
        assert!(typo.contains("LAPTOP_AGENT_TLS_BREAK_GLASS"), "{typo}");
        let missing = match AgentTls::from_vars(None, Some("/nonexistent/store")) {
            Err(e) => e,
            Ok(_) => panic!("an unreadable store was accepted"),
        };
        assert!(
            missing.contains("laptop-agent TLS trust store"),
            "{missing}"
        );
    }
}
