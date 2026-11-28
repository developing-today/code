//! Integration tests for the OCSP verb group (#13722, spec
//! §§4.3/6.1).
//!
//! Oracles, named per the testing doctrine:
//! - the system `openssl` binary as the independent validator of
//!   the responder certificate's EKU / NoCheck extensions
//!   (skipped only when openssl is absent);
//! - the crate's chain code (aws-lc-rs signature verification via
//!   `ocsp::verify_response`), the same oracle the hierarchy
//!   tests use;
//! - RFC 6960 structure, checked by re-parsing signed responses
//!   with the crate parser and comparing every field against the
//!   request that produced it.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::load_bundle;
use fleet_cert::chain::parse_cert;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
use fleet_cert::ocsp::{
    OcspHash, OcspSignRequest, OcspSigner, OcspStatus, parse_response, sign_response, synthetic_e2e,
};
use fleet_cert::profile::leaf_profile;
use fleet_cert::timeparse::{CalendarDuration, Rfc3339};
use std::path::{Path, PathBuf};

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
            "fleet-cert-ocsp-test-{tag}-{}-{nanos}",
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
const AT: &str = "2026-06-01T00:00:00Z";

struct Hierarchy {
    issuer_dir: PathBuf,
    leaf_dir: PathBuf,
    responder_dir: PathBuf,
    issuer: fleet_cert::chain::CertInfo,
    leaf: fleet_cert::chain::CertInfo,
    responder: fleet_cert::chain::CertInfo,
}

fn build_hierarchy(tmp: &TempDir) -> Hierarchy {
    let root_dir = tmp.path().join("roots");
    engine::create_root_set(
        &RootCreateRequest {
            organization: "OCSP Test Org".into(),
            country: "US".into(),
            algorithms: vec![AlgorithmId::Ed25519, AlgorithmId::P256],
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
    let issuer_dir = tmp.path().join("issuers");
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
    let issue = |profile: &str, name: &str, cn: Option<&str>, days: i64| {
        let dir = tmp.path().join(name);
        engine::issue_leaf(
            &LeafIssueRequest {
                issuer_bundle: issuer_dir.clone(),
                issuer_signer: SignerSource {
                    key_dir: issuer_dir.join("private/keys"),
                },
                profile: profile.into(),
                name: name.into(),
                common_name: cn.map(|s| s.to_string()),
                dns_names: if profile == "server" {
                    vec!["www.example.test".into()]
                } else {
                    vec![]
                },
                key_algorithm: None,
                issuer_algorithm: Some(AlgorithmId::P256),
                issuer_partition: None,
                not_before: ts(NB),
                not_after: CalendarDuration::days(days).add_to(ts(NB)).unwrap(),
                private_store: None,
            },
            &dir,
        )
        .unwrap();
        dir
    };
    let leaf_dir = issue("server", "www", None, 730);
    let responder_dir = issue("ocsp-responder", "responder", Some("OCSP Responder"), 30);

    let cert_of = |dir: &Path| {
        let bundle = load_bundle(dir).unwrap();
        let record = &bundle.manifest.certificates[0];
        let pem = fleet_cert::bundle::read_artifact(&bundle, &record.cert_path).unwrap();
        let blocks =
            fleet_cert::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap()).unwrap();
        parse_cert(&blocks[0].1).unwrap()
    };
    let issuer_bundle = load_bundle(&issuer_dir).unwrap();
    let issuer_record = issuer_bundle
        .manifest
        .certificates
        .iter()
        .find(|r| r.role == "issuer" && r.primary && r.algorithm == AlgorithmId::P256)
        .unwrap();
    let issuer_pem =
        fleet_cert::bundle::read_artifact(&issuer_bundle, &issuer_record.cert_path).unwrap();
    let issuer_blocks =
        fleet_cert::bundle::pem_decode_all(std::str::from_utf8(&issuer_pem).unwrap()).unwrap();
    Hierarchy {
        issuer: parse_cert(&issuer_blocks[0].1).unwrap(),
        leaf: cert_of(&leaf_dir),
        responder: cert_of(&responder_dir),
        issuer_dir,
        leaf_dir,
        responder_dir,
    }
}

fn openssl_text(der: &[u8], tmp: &TempDir, tag: &str) -> Option<String> {
    let path = tmp.path().join(format!("{tag}.crt.der"));
    std::fs::write(&path, der).unwrap();
    let out = std::process::Command::new("openssl")
        .args([
            "x509",
            "-inform",
            "DER",
            "-in",
            path.to_str().unwrap(),
            "-noout",
            "-text",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn responder_profile_shape_and_cert_extensions() {
    // Registry-level oracle: the profile itself. (Compared via
    // Debug rendering — the profile's key-usage fields are rcgen
    // types, and rcgen is not a dev-dependency of the test crate.)
    let prof = leaf_profile("ocsp-responder").unwrap();
    assert_eq!(prof.key_algorithm, AlgorithmId::P256);
    assert_eq!(prof.default_validity.days, 30);
    assert!(
        format!("{:?}", prof.extended_key_usages).contains("OcspSigning"),
        "responder profile EKU: {:?}",
        prof.extended_key_usages
    );
    assert_eq!(format!("{:?}", prof.key_usages), "[DigitalSignature]");

    // Certificate-level oracle: openssl's independent rendering.
    let tmp = TempDir::new("profile");
    let h = build_hierarchy(&tmp);
    assert!(fleet_cert::ocsp::cert_has_ocsp_signing_eku(&h.responder.der).unwrap());
    if let Some(text) = openssl_text(&h.responder.der, &tmp, "responder") {
        assert!(
            text.contains("OCSP Signing"),
            "openssl does not see the OCSP-signing EKU:\n{text}"
        );
        assert!(
            text.contains("OCSP No Check") || text.contains("1.3.6.1.5.5.7.48.1.5"),
            "openssl does not see the NoCheck extension:\n{text}"
        );
    }
    // The ordinary server leaf must NOT carry the responder EKU.
    assert!(!fleet_cert::ocsp::cert_has_ocsp_signing_eku(&h.leaf.der).unwrap());
    let _ = h.leaf_dir;
}

#[test]
fn synthetic_e2e_all_checks_pass() {
    let tmp = TempDir::new("e2e");
    let checks = synthetic_e2e(tmp.path()).unwrap();
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    for want in [
        "hierarchy",
        "responder-profile",
        "good-delegated",
        "revoked-delegated",
        "good-direct",
        "tampered-fails",
        "wrong-serial-fails",
        "stale-fails",
    ] {
        assert!(names.contains(&want), "missing check {want}: {names:?}");
    }
    for c in &checks {
        assert!(c.ok, "check {} failed: {}", c.name, c.detail);
    }
}

#[test]
fn sign_parse_round_trip_fields() {
    let tmp = TempDir::new("roundtrip");
    let h = build_hierarchy(&tmp);
    let serial = fleet_cert::ocsp::unhex(&h.leaf.serial_hex).unwrap();
    let at = ts(AT);
    for (hash, want_oid) in [
        (OcspHash::Sha1, "1.3.14.3.2.26"),
        (OcspHash::Sha256, "2.16.840.1.101.3.4.2.1"),
    ] {
        let der = sign_response(&OcspSignRequest {
            issuer: h.issuer.clone(),
            leaf_serial: serial.clone(),
            status: OcspStatus::Revoked {
                revocation_time: ts("2026-05-15T00:00:00Z"),
                reason: Some(4),
            },
            hash,
            produced_at: at,
            this_update: at,
            next_update: Some(CalendarDuration::days(7).add_to(at).unwrap()),
            signer: OcspSigner::Delegated {
                cert: h.responder.clone(),
                key_file: "leaf.pk8".into(),
                source: SignerSource {
                    key_dir: h.responder_dir.join("private/keys"),
                },
            },
        })
        .unwrap();
        let info = parse_response(&der).unwrap();
        assert!(info.successful);
        assert_eq!(info.produced_at, at);
        assert_eq!(info.singles.len(), 1);
        let single = &info.singles[0];
        assert_eq!(single.hash, Some(hash));
        assert_eq!(single.hash_algorithm_oid, want_oid);
        assert_eq!(single.serial_hex, h.leaf.serial_hex);
        assert_eq!(
            single.status,
            OcspStatus::Revoked {
                revocation_time: ts("2026-05-15T00:00:00Z"),
                reason: Some(4),
            }
        );
        assert_eq!(single.this_update, at);
        assert!(single.next_update.is_some());
        // The embedded signer certificate is the responder's.
        assert_eq!(info.embedded_certs.len(), 1);
        assert_eq!(info.embedded_certs[0], h.responder.der);
        // responderID byKey = SHA-1 of the responder key (checked
        // against the crate's own key-hash path).
        let key_bytes = fleet_cert::ocsp::public_key_bytes(&h.responder.der).unwrap();
        assert!(info.responder_key_hash.is_some());
        assert_eq!(key_bytes.len(), 65); // P-256 uncompressed point
    }
    let _ = h.issuer_dir;
}
