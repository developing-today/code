//! Broker TLS Phase 1 (BT-3, #14401; design of record
//! doc/2026-11-24T13-59-26Z_design_broker-tls §§4–6): TLS server
//! authentication on the queue broker's TCP leg, and trust-bundle
//! validation on the queue client's direct TCP leg.
//!
//! Posture, per the design's migration rules:
//! - **Certificates are component identity; the token is still
//!   the authorisation credential.** Nothing here touches token
//!   auth, and the UDS leg never takes TLS (decision D-UDS).
//! - **M1 — additive flags, permissive defaults.** The broker
//!   presents TLS only when `QUEUE_BROKER_TLS_CERT_FILE` and
//!   `QUEUE_BROKER_TLS_KEY_FILE` name a BT-1 `broker-server`
//!   leaf and its on-host key. The client validates only when
//!   `REPO_QUEUE_TLS_TRUST_STORE` names a BT-2 trust store.
//!   With neither set, both sides behave exactly as Phase 0.
//! - **M2 — the Phase 0 accept path stays.** A TLS-configured
//!   broker dual-accepts on the same port: a first byte of 0x16
//!   (a TLS handshake record) takes the TLS path; anything else
//!   is served as plaintext Phase 0, and the plaintext accept is
//!   logged so adoption is visible before any later phase makes
//!   TLS mandatory.
//! - **Fail closed on validation.** A client with a trust store
//!   refuses a broker whose leaf does not chain to the store's
//!   active anchors — no silent fallback to plaintext. The one
//!   escape is the named operator break-glass
//!   (`REPO_QUEUE_TLS_BREAK_GLASS=queue-broker`), which skips
//!   validation, still encrypts, and logs loudly on every use.

use std::io::{Read, Write};

/// A connected byte stream, plaintext or TLS — the transport
/// layer's exchanges are written against this trait object so a
/// roundtrip does not care which one it holds.
pub trait IoStream: Read + Write + Send {}

impl<T: Read + Write + Send> IoStream for T {}

/// The boxed stream the client roundtrips run over.
pub type BoxedStream = Box<dyn IoStream>;

#[cfg(feature = "client")]
pub use client::BrokerTlsClient;
#[cfg(feature = "queue-broker")]
pub use server::{BrokerStream, BrokerTlsServer};

#[cfg(feature = "queue-broker")]
mod server {
    use rustls::ServerConfig;
    use rustls::StreamOwned;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
    use rustls::server::ServerConnection;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;

    /// The broker's Phase 1 server side: one rustls server
    /// configuration built from the `broker-server` leaf chain
    /// and its on-host private key (PEM files, the shape
    /// `fleet cert leaf issue --profile broker-server` delivers
    /// and the host's own key file holds).
    pub struct BrokerTlsServer {
        config: Arc<ServerConfig>,
    }

    /// One accepted TCP connection after the Phase 1 sniff: TLS
    /// when the peer opened with a handshake record, plaintext
    /// Phase 0 otherwise.
    pub enum BrokerStream {
        Plain(TcpStream),
        Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
    }

    impl BrokerStream {
        /// Server-observed transport metadata for the HTTP adapter.
        /// The peer address comes from the accepted socket; clients cannot
        /// supply or override either the TLS classification or peer IP.
        pub(crate) fn request_transport(&self) -> crate::uds_http::RequestTransport {
            let (kind, peer) = match self {
                Self::Plain(stream) => (
                    crate::uds_http::RequestTransportKind::TcpPlain,
                    stream.peer_addr(),
                ),
                Self::Tls(stream) => (
                    crate::uds_http::RequestTransportKind::TcpTls,
                    stream.sock.peer_addr(),
                ),
            };
            crate::uds_http::RequestTransport {
                kind,
                peer_ip: peer.ok().map(|address| address.ip()),
            }
        }
    }

    impl Read for BrokerStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self {
                BrokerStream::Plain(s) => s.read(buf),
                BrokerStream::Tls(s) => s.read(buf),
            }
        }
    }

    impl Write for BrokerStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            match self {
                BrokerStream::Plain(s) => s.write(buf),
                BrokerStream::Tls(s) => s.write(buf),
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            match self {
                BrokerStream::Plain(s) => s.flush(),
                BrokerStream::Tls(s) => s.flush(),
            }
        }
    }

    impl Drop for BrokerStream {
        fn drop(&mut self) {
            // The broker's one-request-per-connection contract
            // (Connection: close) would otherwise end the TLS
            // session at the bare TCP layer, and rustls readers
            // treat that abrupt end as an error even after a
            // complete response. Send the close_notify alert on
            // the way out (best effort, never blocking on the
            // peer's answer).
            if let BrokerStream::Tls(tls) = self {
                tls.conn.send_close_notify();
                let _ = tls.conn.write_tls(&mut tls.sock);
                let _ = tls.sock.flush();
            }
        }
    }

    impl BrokerTlsServer {
        /// From the broker's environment: both
        /// `QUEUE_BROKER_TLS_CERT_FILE` and
        /// `QUEUE_BROKER_TLS_KEY_FILE` set → TLS on; neither →
        /// `Ok(None)` (Phase 0); exactly one, an unreadable
        /// file, or an unusable pair → a startup config error,
        /// named, so the broker refuses to start half-configured
        /// rather than silently serving plaintext.
        pub fn from_env() -> Result<Option<Arc<Self>>, String> {
            let cert = std::env::var("QUEUE_BROKER_TLS_CERT_FILE")
                .ok()
                .filter(|v| !v.trim().is_empty());
            let key = std::env::var("QUEUE_BROKER_TLS_KEY_FILE")
                .ok()
                .filter(|v| !v.trim().is_empty());
            match (cert, key) {
                (None, None) => Ok(None),
                (Some(cert_file), Some(key_file)) => {
                    let cert_pem = std::fs::read(&cert_file).map_err(|e| {
                        format!("QUEUE_BROKER_TLS_CERT_FILE {cert_file}: cannot read: {e}")
                    })?;
                    let key_pem = std::fs::read(&key_file).map_err(|e| {
                        format!("QUEUE_BROKER_TLS_KEY_FILE {key_file}: cannot read: {e}")
                    })?;
                    let server = Self::from_pem(&cert_pem, &key_pem).map_err(|e| {
                        format!(
                            "queue-broker TLS config (QUEUE_BROKER_TLS_CERT_FILE {cert_file}, QUEUE_BROKER_TLS_KEY_FILE {key_file}): {e}"
                        )
                    })?;
                    Ok(Some(Arc::new(server)))
                }
                (Some(_), None) => Err(
                    "QUEUE_BROKER_TLS_CERT_FILE is set but QUEUE_BROKER_TLS_KEY_FILE is not; \
                     broker TLS needs the leaf chain and its on-host key together"
                        .to_string(),
                ),
                (None, Some(_)) => Err(
                    "QUEUE_BROKER_TLS_KEY_FILE is set but QUEUE_BROKER_TLS_CERT_FILE is not; \
                     broker TLS needs the leaf chain and its on-host key together"
                        .to_string(),
                ),
            }
        }

        /// Build the server configuration from a PEM leaf chain
        /// (leaf first, then its issuers) and a PEM PKCS#8 key.
        pub fn from_pem(cert_chain_pem: &[u8], key_pem: &[u8]) -> Result<Self, String> {
            let certs: Vec<CertificateDer<'static>> =
                CertificateDer::pem_slice_iter(cert_chain_pem)
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("cannot parse certificate chain PEM: {e}"))?;
            if certs.is_empty() {
                return Err("certificate chain PEM holds no certificates".to_string());
            }
            let key = PrivateKeyDer::from_pem_slice(key_pem)
                .map_err(|e| format!("cannot parse private key PEM: {e}"))?;
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("TLS protocol versions: {e}"))?
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| format!("leaf chain + key do not form a server identity: {e}"))?;
            Ok(Self {
                config: Arc::new(config),
            })
        }

        /// Accept one connection: sniff the first byte (a TLS
        /// handshake record starts 0x16), drive the handshake to
        /// completion so a failure is logged here and drops the
        /// connection, or pass the stream through as plaintext
        /// Phase 0 with an adoption-evidence log line. `None`
        /// means the connection is dead and must be dropped.
        pub fn accept(&self, mut stream: TcpStream) -> Option<BrokerStream> {
            let mut first = [0u8; 1];
            match stream.peek(&mut first) {
                Ok(1) => {}
                Ok(_) => return None,
                Err(e) => {
                    crate::log::info(
                        "queue-broker",
                        &format!("TLS accept: cannot peek first byte: {e}"),
                    );
                    return None;
                }
            }
            if first[0] != 0x16 {
                crate::log::info(
                    "queue-broker",
                    "TLS accept: plaintext Phase 0 connection on the TLS-configured leg (dual-accept, design rule M2)",
                );
                return Some(BrokerStream::Plain(stream));
            }
            let conn = match ServerConnection::new(Arc::clone(&self.config)) {
                Ok(conn) => conn,
                Err(e) => {
                    crate::log::info(
                        "queue-broker",
                        &format!("TLS accept: cannot start server connection: {e}"),
                    );
                    return None;
                }
            };
            let mut tls = StreamOwned::new(conn, stream);
            if let Err(e) = tls.conn.complete_io(&mut tls.sock) {
                crate::log::info(
                    "queue-broker",
                    &format!("TLS accept: handshake failed: {e}"),
                );
                return None;
            }
            Some(BrokerStream::Tls(Box::new(tls)))
        }
    }
}

#[cfg(feature = "client")]
mod client {
    use super::BoxedStream;
    use fleet_cert::chain::TrustSelection;
    use fleet_cert::trust::{VersionRef, active_anchors};
    use rustls::ClientConfig;
    use rustls::StreamOwned;
    use rustls::client::ClientConnection;
    use rustls::pki_types::{CertificateDer, ServerName, pem::PemObject};
    use std::net::TcpStream;
    use std::path::Path;
    use std::sync::Arc;

    /// The client's Phase 1 posture for one direct TCP leg,
    /// resolved from the environment per connection (the trust
    /// store re-verifies on every read, so a rollover lands
    /// without a client restart — BT-2 §27.3 item 9 behaviour).
    pub enum BrokerTlsClient {
        /// No trust store configured: Phase 0 plaintext.
        Plain,
        /// Validate the broker leaf against the trust store's
        /// active classical anchors; fail closed on any error.
        Validate {
            config: Arc<ClientConfig>,
            bundle: VersionRef,
        },
        /// The named break-glass override: TLS with validation
        /// skipped, logged on every connection.
        BreakGlass { config: Arc<ClientConfig> },
    }

    /// Accepts any server certificate. Only reachable through
    /// the named break-glass override; the signature checks
    /// still run (the leaf must prove possession of its key) —
    /// what is skipped is chain validation, not the handshake.
    #[derive(Debug)]
    struct AcceptAnyServerCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    impl BrokerTlsClient {
        /// Resolve the posture from the environment:
        /// `REPO_QUEUE_TLS_BREAK_GLASS=queue-broker` (the named
        /// override; any other non-empty value is a config
        /// error, so a typo never silently disables or enables
        /// it) wins over `REPO_QUEUE_TLS_TRUST_STORE`, which
        /// wins over the Phase 0 default.
        pub fn from_env() -> Result<Self, String> {
            if let Ok(v) = std::env::var("REPO_QUEUE_TLS_BREAK_GLASS") {
                let v = v.trim();
                if v == "queue-broker" {
                    return Ok(Self::break_glass());
                }
                if !v.is_empty() {
                    return Err(format!(
                        "REPO_QUEUE_TLS_BREAK_GLASS={v:?} is not the named break-glass override; \
                         set it to \"queue-broker\" to skip broker TLS validation"
                    ));
                }
            }
            if let Ok(store) = std::env::var("REPO_QUEUE_TLS_TRUST_STORE") {
                if !store.trim().is_empty() {
                    return Self::validate_from_store(Path::new(store.trim()));
                }
            }
            Ok(Self::Plain)
        }

        /// Validate against the active anchors of the BT-2
        /// trust store at `store` (fleet-cert's store layout;
        /// `active_anchors` re-verifies the installed bundle
        /// against its recorded content hash on every read and
        /// fails closed on drift).
        pub fn validate_from_store(store: &Path) -> Result<Self, String> {
            let (bundle, anchors_pem) = active_anchors(store, TrustSelection::Classical)
                .map_err(|e| format!("queue-broker TLS trust store {}: {e}", store.display()))?;
            let mut roots = rustls::RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(&anchors_pem) {
                let cert = cert.map_err(|e| {
                    format!(
                        "queue-broker TLS trust store {}: cannot parse anchor PEM: {e}",
                        store.display()
                    )
                })?;
                roots.add(cert).map_err(|e| {
                    format!(
                        "queue-broker TLS trust store {}: anchor is not a usable trust root: {e}",
                        store.display()
                    )
                })?;
            }
            if roots.is_empty() {
                return Err(format!(
                    "queue-broker TLS trust store {}: the active bundle holds no classical anchors",
                    store.display()
                ));
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("TLS protocol versions: {e}"))?
                .with_root_certificates(roots)
                .with_no_client_auth();
            Ok(Self::Validate {
                config: Arc::new(config),
                bundle,
            })
        }

        /// The break-glass posture: encrypt, validate nothing.
        pub fn break_glass() -> Self {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("rustls default protocol versions")
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
                .with_no_client_auth();
            Self::BreakGlass {
                config: Arc::new(config),
            }
        }

        /// The acknowledged trust-bundle stamp this posture
        /// validates against (design §6: the version a client
        /// presents is the rollover health signal — recorded
        /// here for the handshake/health surfaces that consume
        /// it with BT-4/BT-5).
        pub fn bundle_version(&self) -> Option<u32> {
            match self {
                Self::Validate { bundle, .. } => Some(bundle.version),
                _ => None,
            }
        }

        /// Wrap a connected TCP stream per the posture. For
        /// `Validate`, the handshake is driven to completion
        /// here so a wrong or untrusted broker leaf is refused
        /// at connect time with a named error — never discovered
        /// mid-request, never fallen back from.
        pub fn wrap(&self, stream: TcpStream, host: &str) -> Result<BoxedStream, String> {
            let config = match self {
                Self::Plain => return Ok(Box::new(stream)),
                Self::Validate { config, .. } => config,
                Self::BreakGlass { config } => {
                    crate::log::info(
                        "queue-broker",
                        "BREAK-GLASS: REPO_QUEUE_TLS_BREAK_GLASS=queue-broker — connecting \
                         WITHOUT validating the broker certificate",
                    );
                    config
                }
            };
            let name = ServerName::try_from(host.to_string()).map_err(|e| {
                format!("queue-broker TLS: {host:?} is not a usable server name: {e}")
            })?;
            let conn = ClientConnection::new(Arc::clone(config), name)
                .map_err(|e| format!("queue-broker TLS: cannot start client connection: {e}"))?;
            let mut tls = StreamOwned::new(conn, stream);
            if let Err(e) = tls.conn.complete_io(&mut tls.sock) {
                return Err(match self {
                    Self::Validate { bundle, .. } => format!(
                        "queue-broker TLS validation failed for {host} against trust bundle \
                         version {} (content {}): {e} — connection refused, fail closed; \
                         if the broker leaf was re-issued, roll the trust bundle forward, \
                         or use the named break-glass REPO_QUEUE_TLS_BREAK_GLASS=queue-broker",
                        bundle.version, bundle.content_sha256
                    ),
                    _ => format!("queue-broker TLS handshake with {host} failed: {e}"),
                });
            }
            Ok(Box::new(tls))
        }
    }
}

#[cfg(all(test, feature = "queue-broker"))]
mod tests {
    //! The synthetic estate (design §5.2 Phase 1 acceptance):
    //! a fleet-cert root set → Base issuer → `broker-server`
    //! leaf issued from an on-host key + CSR → BT-2 trust bundle
    //! staged, acknowledged and activated — then the real
    //! `tcp_http::serve_tls` broker leg and the real client
    //! wrap, exercised over loopback.
    use super::{BrokerTlsClient, BrokerTlsServer};
    use crate::tcp_http;
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    const NB: &str = "2026-01-01T00:00:00Z";
    const LEAF_NB: &str = "2026-11-01T00:00:00Z";
    const AT: &str = "2026-11-20T00:00:00Z";
    const BROKER_IP: &str = "100.99.113.103";
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
                "fleet-queue-broker-tls-{tag}-{}-{nanos}",
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
        /// Trust store with the estate's bundle activated.
        store: PathBuf,
        /// The exported trust bundle directory (for staging
        /// stores that never activate).
        bundle_dir: PathBuf,
        /// Leaf chain PEM (leaf + issuer) for the broker.
        chain_pem: Vec<u8>,
        /// The broker host's PKCS#8 key PEM.
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

        // The broker host's own key + CSR (design §4: the key is
        // generated on the host that runs the broker and never
        // moves; the issuer only ever sees the CSR).
        let host_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let csr_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        let csr = csr_params.serialize_request(&host_key).unwrap();
        let csr_path = tmp.path().join(format!("{tag}-broker.csr.pem"));
        std::fs::write(&csr_path, csr.pem().unwrap()).unwrap();

        let leaf_dir = tmp.path().join(format!("{tag}-leaf"));
        engine::issue_leaf(
            &LeafIssueRequest {
                issuer_bundle: issuer_dir.clone(),
                issuer_signer: SignerSource {
                    key_dir: issuer_dir.join("private/keys"),
                },
                profile: "broker-server".into(),
                name: "queue-broker".into(),
                common_name: None,
                dns_names: vec!["queue-broker.internal".into()],
                ip_addresses: vec![BROKER_IP.parse().unwrap()],
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

        // The chain the broker presents: its leaf, then the
        // issuing Base cert.
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

        // BT-2: export the trust bundle from the root set and
        // run the two-phase rollover into a client store.
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
            bundle_dir,
            chain_pem,
            key_pem: host_key.serialize_pem(),
        }
    }

    /// Serve the estate's broker leg on a loopback port with
    /// the production precheck shape: the token gate runs
    /// before the handler, exactly as on the real broker.
    fn serve_estate(estate: &Estate) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(
            BrokerTlsServer::from_pem(&estate.chain_pem, estate.key_pem.as_bytes()).unwrap(),
        );
        std::thread::spawn(move || {
            tcp_http::serve_tls(
                listener,
                server,
                |head| {
                    if head.header("X-Agent-Token") == Some(TOKEN) {
                        None
                    } else {
                        Some((401, b"{\"error\":\"bad token\"}".to_vec()))
                    }
                },
                |_| (200, b"{\"ok\":true}".to_vec()),
            );
        });
        addr
    }

    /// The same synthetic TLS server with the server-observed request
    /// transport captured by the handler (the parser's default alone is
    /// not evidence that the TLS adapter propagated the handshake).
    fn serve_estate_observing_transport(
        estate: &Estate,
        observed: std::sync::mpsc::Sender<(crate::uds_http::RequestTransport, Option<String>)>,
    ) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(
            BrokerTlsServer::from_pem(&estate.chain_pem, estate.key_pem.as_bytes()).unwrap(),
        );
        std::thread::spawn(move || {
            tcp_http::serve_tls(
                listener,
                server,
                |head| {
                    if head.header("X-Agent-Token") == Some(TOKEN) {
                        None
                    } else {
                        Some((401, b"{\"error\":\"bad token\"}".to_vec()))
                    }
                },
                move |req| {
                    let version = req
                        .header("X-Fleet-Trust-Bundle-Version")
                        .map(str::to_string);
                    let _ = observed.send((req.transport, version));
                    (200, b"{\"ok\":true}".to_vec())
                },
            );
        });
        addr
    }

    /// One client roundtrip in the shape of the direct TCP leg:
    /// connect, wrap per the posture, POST with the token,
    /// read the whole response.
    fn roundtrip(
        addr: std::net::SocketAddr,
        mode: &BrokerTlsClient,
        token: &str,
    ) -> Result<String, String> {
        let mut stream = TcpStream::connect(addr)
            .map_err(|e| format!("cannot connect to queue-broker at {addr}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut stream = mode.wrap(stream, BROKER_IP)?;
        let body = b"{}";
        let bundle_version = mode
            .bundle_version()
            .map(|version| format!("X-Fleet-Trust-Bundle-Version: {version}\r\n"))
            .unwrap_or_default();
        let head = format!(
            "POST /healthz HTTP/1.1\r\nHost: {addr}\r\nX-Agent-Token: {token}\r\n{bundle_version}\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(body))
            .map_err(|e| format!("queue-broker write failed: {e}"))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("queue-broker read failed: {e}"))?;
        Ok(String::from_utf8_lossy(&raw).to_string())
    }

    #[test]
    fn validated_roundtrip_and_token_gate_over_tls() {
        let tmp = TempDir::new("roundtrip");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let addr = serve_estate(&estate);
        let mode = BrokerTlsClient::validate_from_store(&estate.store).unwrap();
        assert_eq!(mode.bundle_version(), Some(1));

        let ok = roundtrip(addr, &mode, TOKEN).unwrap();
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.ends_with("{\"ok\":true}"), "{ok}");

        // Token auth is unchanged and still required: a wrong
        // token is refused by the precheck over the TLS leg.
        // Oracle (#16583): the refusal must arrive intact on every
        // roundtrip — closing the TLS leg with the pipelined body
        // unread used to reset the connection and discard the 401
        // before the client could read it (intermittent
        // "Connection reset by peer" at this unwrap).
        for _ in 0..25 {
            let refused = roundtrip(addr, &mode, "wrong-token").unwrap();
            assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        }
    }

    #[test]
    fn validated_tls_request_preserves_server_observed_transport_and_peer() {
        // Oracle: after the real rustls handshake, the request handler sees
        // TLS provenance and the loopback peer IP; an HTTP header cannot
        // manufacture either value.
        let tmp = TempDir::new("transport-metadata");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let (tx, rx) = std::sync::mpsc::channel();
        let addr = serve_estate_observing_transport(&estate, tx);
        let mode = BrokerTlsClient::validate_from_store(&estate.store).unwrap();

        let response = roundtrip(addr, &mode, TOKEN).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let (transport, version) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("handler observes the accepted request");
        assert_eq!(
            transport.kind,
            crate::uds_http::RequestTransportKind::TcpTls
        );
        assert_eq!(transport.peer_ip, Some("127.0.0.1".parse().unwrap()));
        assert_eq!(version.as_deref(), Some("1"));
    }

    #[test]
    fn break_glass_tls_request_does_not_present_a_bundle_version() {
        // Oracle: break-glass still uses TLS, but BrokerTlsClient does not
        // present a trust-bundle version because it skipped validation.
        let tmp = TempDir::new("breakglass-version");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let (tx, rx) = std::sync::mpsc::channel();
        let addr = serve_estate_observing_transport(&estate, tx);
        let mode = BrokerTlsClient::break_glass();
        assert_eq!(mode.bundle_version(), None);

        let response = roundtrip(addr, &mode, TOKEN).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let (transport, version) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("handler observes the accepted request");
        assert_eq!(
            transport.kind,
            crate::uds_http::RequestTransportKind::TcpTls
        );
        assert_eq!(version, None);
    }

    #[test]
    fn untrusted_broker_leaf_is_refused_fail_closed() {
        let tmp = TempDir::new("untrusted");
        let estate_a = build_estate(&tmp, "a", "Test Org A LLC");
        let estate_b = build_estate(&tmp, "b", "Test Org B LLC");
        let addr = serve_estate(&estate_a);
        // The client trusts estate B's roots only; the broker
        // presents estate A's leaf.
        let mode = BrokerTlsClient::validate_from_store(&estate_b.store).unwrap();
        let err = roundtrip(addr, &mode, TOKEN).unwrap_err();
        assert!(
            err.contains("TLS validation failed") && err.contains("fail closed"),
            "{err}"
        );
    }

    #[test]
    fn break_glass_connects_without_validation() {
        let tmp = TempDir::new("breakglass");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let addr = serve_estate(&estate);
        let mode = BrokerTlsClient::break_glass();
        let ok = roundtrip(addr, &mode, TOKEN).unwrap();
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    }

    #[test]
    fn plaintext_client_still_accepted_on_tls_configured_leg() {
        // Rule M2: the Phase 0 accept path remains in the
        // binary — a pre-TLS client keeps working on the same
        // port while capable clients take the verified path.
        let tmp = TempDir::new("dual");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let (tx, rx) = std::sync::mpsc::channel();
        let addr = serve_estate_observing_transport(&estate, tx);
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .write_all(
                format!(
                    "POST /healthz HTTP/1.1\r\nHost: {addr}\r\nX-Agent-Token: {TOKEN}\r\n\
                     X-Fleet-Trust-Bundle-Version: 99\r\n\
                     Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .as_bytes(),
            )
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        let (transport, version) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("handler observes the accepted request");
        assert_eq!(
            transport.kind,
            crate::uds_http::RequestTransportKind::TcpPlain
        );
        assert_eq!(version.as_deref(), Some("99"));
    }

    #[test]
    fn validating_client_refuses_a_plaintext_broker() {
        // Fail closed cuts both ways: a client configured with
        // a trust store never silently talks plaintext to a
        // broker that has not enabled TLS.
        let tmp = TempDir::new("plainbroker");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            tcp_http::serve(listener, |_| None, |_| (200, b"{}".to_vec()));
        });
        let mode = BrokerTlsClient::validate_from_store(&estate.store).unwrap();
        let err = roundtrip(addr, &mode, TOKEN).unwrap_err();
        assert!(err.contains("TLS validation failed"), "{err}");
    }

    #[test]
    fn anchors_come_from_the_activated_bundle_only() {
        // A store whose bundle was staged but never
        // acknowledged/activated has no active anchors:
        // validation cannot be built from it (BT-2's rule that
        // rollover never activates without acknowledgements).
        let tmp = TempDir::new("staged");
        let estate = build_estate(&tmp, "a", "Test Org A LLC");
        let staged_store = tmp.path().join("staged-store");
        stage_bundle(&staged_store, &estate.bundle_dir).unwrap();
        let Err(err) = BrokerTlsClient::validate_from_store(&staged_store) else {
            panic!("a staged-only store must not yield a validating client");
        };
        assert!(err.contains("trust store"), "{err}");
    }
}
