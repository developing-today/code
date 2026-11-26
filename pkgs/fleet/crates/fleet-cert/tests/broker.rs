//! Broker TLS leaves (#14399, broker-TLS ADR-0001): the
//! `broker-server`/`broker-client` profiles, the broker SAN policy,
//! CSR-only issuance from the Base issuer set, and CRL-only leaves.
//!
//! Oracles: the design of record's rules (doc/2026-11-24T13-59-26Z_
//! design_broker-tls §4), x509-parser reading the issued DER back,
//! the crate chain validator, and the system `openssl` binary when
//! present.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::load_bundle;
use fleet_cert::chain::TrustSelection;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
use fleet_cert::timeparse::{CalendarDuration, Rfc3339};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::prelude::FromDer;

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
            "fleet-cert-broker-{tag}-{}-{nanos}",
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

const NB: &str = "2026-01-01T00:00:00Z";
const BROKER_DNS: &str = "queue-broker.fleet.internal";
const BROKER_IP: &str = "100.99.113.103";

/// A P-256 root set and an A/B issuer set of the given purpose.
fn hierarchy(tmp: &TempDir, purpose: IssuerPurpose) -> (PathBuf, PathBuf) {
    let root_dir = tmp.path().join("roots");
    engine::create_root_set(
        &RootCreateRequest {
            organization: "Test Org LLC".into(),
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
    let issuer_dir = tmp.path().join(purpose.as_str());
    engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root_dir.clone(),
            root_signer: SignerSource {
                key_dir: root_dir.join("private/keys"),
            },
            name: "Base".into(),
            purpose,
            ou: None,
            slots: vec![
                (AlgorithmId::P256, "a".into()),
                (AlgorithmId::P256, "b".into()),
            ],
            cross_signing: CrossSigningMode::Both,
            not_before: ts(NB),
            not_after: ts("2031-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            permitted_dns: vec![],
            private_store: None,
        },
        &issuer_dir,
    )
    .unwrap();
    (root_dir, issuer_dir)
}

/// A CSR made the way a broker host would: its own key, its own
/// (deliberately over-reaching) requested names, which issuance must
/// ignore.
fn host_csr(dir: &Path, algorithm: &'static rcgen::SignatureAlgorithm) -> (PathBuf, Vec<u8>) {
    let key = rcgen::KeyPair::generate_for(algorithm).unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["broker.example.com".to_string()]).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let csr = params.serialize_request(&key).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("broker.csr.pem");
    std::fs::write(&path, csr.pem().unwrap()).unwrap();
    (path, key.public_key_raw().to_vec())
}

fn broker_request(
    issuer: &Path,
    profile: &str,
    csr: Option<PathBuf>,
    dns: &[&str],
    ips: &[&str],
) -> LeafIssueRequest {
    LeafIssueRequest {
        issuer_bundle: issuer.to_path_buf(),
        issuer_signer: SignerSource {
            key_dir: issuer.join("private/keys"),
        },
        profile: profile.into(),
        name: "queue-broker".into(),
        common_name: None,
        dns_names: dns.iter().map(|s| s.to_string()).collect(),
        ip_addresses: ips.iter().map(|s| s.parse().unwrap()).collect(),
        csr,
        key_algorithm: None,
        issuer_algorithm: None,
        issuer_partition: None,
        not_before: ts(NB),
        not_after: CalendarDuration::days(90).add_to(ts(NB)).unwrap(),
        private_store: None,
    }
}

fn leaf_der(leaf_dir: &Path) -> Vec<u8> {
    let pem = std::fs::read(leaf_dir.join("leaf.crt.pem")).unwrap();
    fleet_cert::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap())
        .unwrap()
        .remove(0)
        .1
}

#[test]
fn broker_leaf_from_a_host_csr() {
    let tmp = TempDir::new("issue");
    let (root_dir, issuer_dir) = hierarchy(&tmp, IssuerPurpose::General);
    for (profile, eku_oid) in [
        ("broker-server", "1.3.6.1.5.5.7.3.1"),
        ("broker-client", "1.3.6.1.5.5.7.3.2"),
    ] {
        let (csr, host_public) = host_csr(
            &tmp.path().join(profile).join("host"),
            &rcgen::PKCS_ECDSA_P256_SHA256,
        );
        let leaf_dir = tmp.path().join(profile).join("leaf");
        let manifest = engine::issue_leaf(
            &broker_request(&issuer_dir, profile, Some(csr), &[BROKER_DNS], &[BROKER_IP]),
            &leaf_dir,
        )
        .unwrap();

        // The key never reached the issuer: no key file anywhere in
        // the leaf bundle, and the manifest records host-local custody.
        assert_eq!(manifest.key_location, "host-local");
        assert_eq!(manifest.certificates[0].key_path, None);
        assert!(!leaf_dir.join("private").exists());

        let der = leaf_der(&leaf_dir);
        let (_, cert) = x509_parser::certificate::X509Certificate::from_der(&der).unwrap();
        let tbs = &cert.tbs_certificate;
        // It certifies the host's key.
        assert_eq!(
            tbs.subject_pki.subject_public_key.data.as_ref(),
            &host_public[..]
        );
        // CN is the first internal DNS name; the CSR's own names and
        // its CA request were ignored.
        assert_eq!(
            tbs.subject
                .iter_common_name()
                .next()
                .unwrap()
                .as_str()
                .unwrap(),
            BROKER_DNS
        );
        let san = tbs.subject_alternative_name().unwrap().unwrap();
        let ip: IpAddr = BROKER_IP.parse().unwrap();
        let mut dns_seen = Vec::new();
        let mut ip_seen = Vec::new();
        for name in &san.value.general_names {
            match name {
                GeneralName::DNSName(d) => dns_seen.push(d.to_string()),
                GeneralName::IPAddress(raw) => ip_seen.push(raw.to_vec()),
                other => panic!("unexpected SAN {other:?}"),
            }
        }
        assert_eq!(dns_seen, vec![BROKER_DNS.to_string()]);
        let IpAddr::V4(v4) = ip else { unreachable!() };
        assert_eq!(ip_seen, vec![v4.octets().to_vec()]);
        let bc = tbs.basic_constraints().unwrap().unwrap();
        assert!(!bc.value.ca);
        let eku = tbs.extended_key_usage().unwrap().unwrap();
        assert_eq!(eku.value.server_auth, eku_oid.ends_with(".1"));
        assert_eq!(eku.value.client_auth, eku_oid.ends_with(".2"));
        // CRL-only: no AIA (and so no OCSP URL), no CDP.
        for ext in tbs.extensions() {
            let oid = ext.oid.to_id_string();
            assert_ne!(oid, "1.3.6.1.5.5.7.1.1", "{profile} leaf carries AIA");
            assert_ne!(oid, "2.5.29.31", "{profile} leaf carries a CDP");
            assert!(
                !matches!(
                    ext.parsed_extension(),
                    ParsedExtension::AuthorityInfoAccess(_)
                ),
                "{profile} leaf carries AIA"
            );
        }

        // It chains to the roots through the Base issuer set.
        let leaf_b = load_bundle(&leaf_dir).unwrap();
        let issuer_b = load_bundle(&issuer_dir).unwrap();
        let root_b = load_bundle(&root_dir).unwrap();
        let pool = engine::collect_pool(&[&issuer_b, &root_b]).unwrap();
        let leaf = fleet_cert::chain::parse_cert(&der).unwrap();
        assert_eq!(leaf.subject_algorithm, Some(AlgorithmId::P256));
        let report = fleet_cert::chain::validate_chain(
            &leaf,
            &pool,
            TrustSelection::All,
            ts("2026-02-01T00:00:00Z"),
        );
        assert!(
            report.valid,
            "{profile}: {:?}",
            report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
        );
        assert_eq!(leaf_b.manifest.purpose.as_deref(), Some(profile));

        if openssl_present() {
            let ok = std::process::Command::new("openssl")
                .args([
                    "verify",
                    "-purpose",
                    if profile == "broker-server" {
                        "sslserver"
                    } else {
                        "sslclient"
                    },
                    "-attime",
                    "1769904000", // 2026-02-01
                    "-CAfile",
                    root_dir
                        .join("public/bundles/roots-all.pem")
                        .to_str()
                        .unwrap(),
                    "-untrusted",
                    issuer_dir
                        .join("public/certs/p256/a/signed-by-p256.crt.pem")
                        .to_str()
                        .unwrap(),
                    leaf_dir.join("leaf.crt.pem").to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                ok.status.success(),
                "{profile}: openssl verify failed: {}",
                String::from_utf8_lossy(&ok.stderr)
            );
        }
    }
}

#[test]
fn broker_leaves_are_signed_by_either_partition() {
    let tmp = TempDir::new("partition");
    let (_, issuer_dir) = hierarchy(&tmp, IssuerPurpose::General);
    for partition in ["a", "b"] {
        let (csr, _) = host_csr(
            &tmp.path().join(partition).join("host"),
            &rcgen::PKCS_ECDSA_P256_SHA256,
        );
        let mut req = broker_request(
            &issuer_dir,
            "broker-server",
            Some(csr),
            &[BROKER_DNS],
            &[BROKER_IP],
        );
        req.issuer_partition = Some(partition.into());
        let leaf_dir = tmp.path().join(partition).join("leaf");
        let m = engine::issue_leaf(&req, &leaf_dir).unwrap();
        assert_eq!(m.certificates[0].partition, partition);
    }
}

#[test]
fn broker_issuance_refusals() {
    let tmp = TempDir::new("refuse");
    let (_roots, base_dir) = hierarchy(&tmp, IssuerPurpose::General);
    let (csr, _) = host_csr(&tmp.path().join("host"), &rcgen::PKCS_ECDSA_P256_SHA256);
    let refused = |req: LeafIssueRequest, tag: &str| -> String {
        let out = tmp.path().join(format!("leaf-{tag}"));
        let err = engine::issue_leaf(&req, &out).expect_err(tag).to_string();
        assert!(
            !out.exists(),
            "{tag}: a refused request wrote {}",
            out.display()
        );
        err
    };

    // No CSR: the issuer never generates a broker key.
    let err = refused(
        broker_request(
            &base_dir,
            "broker-server",
            None,
            &[BROKER_DNS],
            &[BROKER_IP],
        ),
        "no-csr",
    );
    assert!(err.contains("CSR"), "{err}");

    // SAN policy, checked before anything is loaded or signed.
    for (tag, dns, ips) in [
        ("no-dns", &[][..], &[BROKER_IP][..]),
        ("no-ip", &[BROKER_DNS][..], &[][..]),
        ("wildcard", &["*.fleet.internal"][..], &[BROKER_IP][..]),
        ("public-name", &["broker.example.com"][..], &[BROKER_IP][..]),
        ("public-ip", &[BROKER_DNS][..], &["203.0.113.7"][..]),
        ("lan-ip", &[BROKER_DNS][..], &["192.168.1.20"][..]),
    ] {
        let err = refused(
            broker_request(&base_dir, "broker-client", Some(csr.clone()), dns, ips),
            tag,
        );
        assert!(err.contains("broker-client"), "{tag}: {err}");
    }
    let mut public_cn = broker_request(
        &base_dir,
        "broker-server",
        Some(csr.clone()),
        &[BROKER_DNS],
        &[BROKER_IP],
    );
    public_cn.common_name = Some("broker.example.com".into());
    refused(public_cn, "public-cn");

    // ML-DSA (or any non-P-256) broker keys are deferred.
    let (ed_csr, _) = host_csr(&tmp.path().join("host-ed"), &rcgen::PKCS_ED25519);
    let err = refused(
        broker_request(
            &base_dir,
            "broker-server",
            Some(ed_csr),
            &[BROKER_DNS],
            &[BROKER_IP],
        ),
        "ed25519",
    );
    assert!(err.contains("P-256") || err.contains("p256"), "{err}");

    // A CSR whose signature does not verify.
    let text = std::fs::read_to_string(&csr).unwrap();
    let blocks = fleet_cert::bundle::pem_decode_all(&text).unwrap();
    let mut der = blocks[0].1.clone();
    let last = der.len() - 1;
    der[last] ^= 0x01;
    let bad = tmp.path().join("bad.csr.der");
    std::fs::write(&bad, &der).unwrap();
    refused(
        broker_request(
            &base_dir,
            "broker-server",
            Some(bad),
            &[BROKER_DNS],
            &[BROKER_IP],
        ),
        "bad-signature",
    );

    // A CSR leaf has no key for a private store to hold.
    let mut stored = broker_request(
        &base_dir,
        "broker-server",
        Some(csr.clone()),
        &[BROKER_DNS],
        &[BROKER_IP],
    );
    stored.private_store = Some(tmp.path().join("store"));
    refused(stored, "private-store");

    // Only the Base issuer set signs broker leaves.
    let web = TempDir::new("refuse-web");
    let (_, web_dir) = hierarchy(&web, IssuerPurpose::WebServer);
    let err = refused(
        broker_request(
            &web_dir,
            "broker-server",
            Some(csr),
            &[BROKER_DNS],
            &[BROKER_IP],
        ),
        "web-issuer",
    );
    assert!(err.contains("Base"), "{err}");
}

#[test]
fn ordinary_profiles_take_ip_sans_and_csrs() {
    let tmp = TempDir::new("ordinary");
    let (_, issuer_dir) = hierarchy(&tmp, IssuerPurpose::General);
    // An IP-only server leaf keeps its IP SAN and gets no DNS SAN
    // defaulted from the CN.
    let mut req = broker_request(&issuer_dir, "server", None, &[], &["10.1.2.3"]);
    req.common_name = Some("db".into());
    let leaf_dir = tmp.path().join("ip-only");
    let m = engine::issue_leaf(&req, &leaf_dir).unwrap();
    assert_eq!(m.key_location, "bundle-private");
    let der = leaf_der(&leaf_dir);
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(&der).unwrap();
    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .unwrap()
        .unwrap();
    assert_eq!(san.value.general_names.len(), 1);
    assert!(matches!(
        san.value.general_names[0],
        GeneralName::IPAddress(raw) if raw == [10, 1, 2, 3]
    ));

    // A `server` leaf from a CSR certifies the CSR's key.
    let (csr, host_public) = host_csr(&tmp.path().join("host"), &rcgen::PKCS_ECDSA_P256_SHA256);
    let req = broker_request(&issuer_dir, "server", Some(csr), &["www.example.test"], &[]);
    let leaf_dir = tmp.path().join("csr");
    let m = engine::issue_leaf(&req, &leaf_dir).unwrap();
    assert_eq!(m.key_location, "host-local");
    let der = leaf_der(&leaf_dir);
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(&der).unwrap();
    assert_eq!(
        cert.tbs_certificate
            .subject_pki
            .subject_public_key
            .data
            .as_ref(),
        &host_public[..]
    );
}

fn openssl_present() -> bool {
    std::process::Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
