//! `fleet cert issuer renew` (#13732, spec §5.2): same-key reissue
//! of an issuer set.
//!
//! Oracles:
//! - each certificate's own SubjectPublicKeyInfo, read back from the
//!   issued DER, for "the key did not change";
//! - the crate's chain validator (aws-lc-rs signature checks) for
//!   "the renewed issuer still chains to its root and can issue";
//! - the manifest the renewal wrote, for the revision and
//!   predecessor trace and the unchanged generation.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::{Bundle, load_bundle, pem_decode_all, read_artifact};
use fleet_cert::chain::TrustSelection;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, IssuerRenewRequest, LeafIssueRequest, RootCreateRequest,
    SignerSource,
};
use fleet_cert::timeparse::Rfc3339;
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
            "fleet-cert-renew-{tag}-{}-{nanos}",
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

const ALGOS: [AlgorithmId; 3] = [
    AlgorithmId::Ed25519,
    AlgorithmId::P256,
    AlgorithmId::MlDsa44,
];

fn make_roots(dir: &Path) {
    engine::create_root_set(
        &RootCreateRequest {
            organization: "Test Org LLC".into(),
            country: "US".into(),
            algorithms: ALGOS.to_vec(),
            generation: 1,
            partition: "A".into(),
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2046-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            private_store: None,
        },
        dir,
    )
    .unwrap();
}

/// A web-server issuer set (so the certificates carry an OU and a
/// permitted DNS name constraint) with A and B slots per algorithm.
fn make_issuers(root: &Path, dir: &Path) {
    engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root.to_path_buf(),
            root_signer: SignerSource {
                key_dir: root.join("private/keys"),
            },
            name: "Web".into(),
            purpose: IssuerPurpose::WebServer,
            ou: None,
            slots: ALGOS
                .iter()
                .flat_map(|a| [(*a, "a".to_string()), (*a, "b".to_string())])
                .collect(),
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2031-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            permitted_dns: vec!["example.test".into()],
            private_store: None,
        },
        dir,
    )
    .unwrap();
}

fn renew_request(issuer: &Path, root: &Path, allow_key_reuse: bool) -> IssuerRenewRequest {
    IssuerRenewRequest {
        issuer_bundle: issuer.to_path_buf(),
        root_bundle: root.to_path_buf(),
        root_signer: SignerSource {
            key_dir: root.join("private/keys"),
        },
        not_before: ts("2027-01-01T00:00:00Z"),
        not_after: ts("2032-01-01T00:00:00Z"),
        allow_key_reuse,
    }
}

fn cert_der(bundle: &Bundle, rel: &str) -> Vec<u8> {
    let pem = read_artifact(bundle, rel).unwrap();
    pem_decode_all(std::str::from_utf8(&pem).unwrap())
        .unwrap()
        .into_iter()
        .find(|(label, _)| label == "CERTIFICATE")
        .unwrap()
        .1
}

fn permitted_dns(der: &[u8]) -> Vec<String> {
    use x509_parser::extensions::{GeneralName, ParsedExtension};
    let (_, cert) = x509_parser::prelude::parse_x509_certificate(der).unwrap();
    let mut out = Vec::new();
    for ext in cert.tbs_certificate.extensions() {
        if let ParsedExtension::NameConstraints(nc) = ext.parsed_extension() {
            for subtree in nc.permitted_subtrees.iter().flatten() {
                if let GeneralName::DNSName(name) = &subtree.base {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

#[test]
fn renewal_reissues_every_issuer_certificate_on_the_same_key() {
    let tmp = TempDir::new("same-key");
    let root_dir = tmp.path().join("roots");
    let issuer_dir = tmp.path().join("web");
    let renewed_dir = tmp.path().join("web-r2");
    make_roots(&root_dir);
    make_issuers(&root_dir, &issuer_dir);

    let manifest =
        engine::renew_issuer_set(&renew_request(&issuer_dir, &root_dir, true), &renewed_dir)
            .unwrap();
    let old = load_bundle(&issuer_dir).unwrap();
    let new = load_bundle(&renewed_dir).unwrap();

    // The trace: revision r2 of the same generation, naming the
    // bundle it renews.
    assert_eq!(manifest.revision, 2);
    assert_eq!(
        manifest.predecessor.as_deref(),
        Some(old.manifest.bundle_id.as_str())
    );
    assert_eq!(manifest.generation, old.manifest.generation);
    assert_ne!(manifest.bundle_id, old.manifest.bundle_id);
    assert_eq!(manifest.purpose, old.manifest.purpose);
    assert_eq!(manifest.key_location, "bundle-private");
    // A bundle that renews nothing keeps the original JSON shape.
    let old_json = std::fs::read_to_string(issuer_dir.join("manifest.json")).unwrap();
    assert!(!old_json.contains("\"revision\""));
    assert!(!old_json.contains("\"predecessor\""));

    assert_eq!(
        new.manifest.certificates.len(),
        old.manifest.certificates.len()
    );
    for before in &old.manifest.certificates {
        let after = new
            .manifest
            .certificates
            .iter()
            .find(|r| r.cert_path == before.cert_path)
            .unwrap_or_else(|| panic!("{} missing from the renewal", before.cert_path));
        let old_der = cert_der(&old, &before.cert_path);
        let new_der = cert_der(&new, &after.cert_path);
        assert_eq!(
            fleet_cert::dns::spki_der(&new_der).unwrap(),
            fleet_cert::dns::spki_der(&old_der).unwrap(),
            "{}: renewal must keep the key",
            before.cert_path
        );
        assert_eq!(after.subject_dn, before.subject_dn);
        assert_eq!(after.issuer_dn, before.issuer_dn);
        assert_eq!(after.ski_hex, before.ski_hex);
        assert_eq!(after.signer_algorithm, before.signer_algorithm);
        assert_eq!(after.primary, before.primary);
        assert_ne!(
            after.serial_hex, before.serial_hex,
            "a renewal gets a new serial"
        );
        assert_ne!(after.fingerprint_sha256, before.fingerprint_sha256);
        assert_eq!(after.not_before, ts("2027-01-01T00:00:00Z"));
        assert_eq!(after.not_after, ts("2032-01-01T00:00:00Z"));
        assert!(after.subject_dn.contains("OU=Web Services"));
        assert_eq!(permitted_dns(&new_der), vec!["example.test".to_string()]);
        // The key file moved over unchanged.
        let key = before.key_path.as_deref().unwrap();
        assert_eq!(after.key_path.as_deref(), Some(key));
        assert_eq!(
            read_artifact(&new, key).unwrap(),
            read_artifact(&old, key).unwrap()
        );
    }

    // Every renewed issuer still chains to its root: issue one leaf
    // per algorithm from the renewed set with the copied keys and
    // validate it inside the new window.
    let root = load_bundle(&root_dir).unwrap();
    for algo in ALGOS {
        let leaf_dir = tmp.path().join(format!("leaf-{}", algo.name()));
        engine::issue_leaf(
            &LeafIssueRequest {
                issuer_bundle: renewed_dir.clone(),
                issuer_signer: SignerSource {
                    key_dir: renewed_dir.join("private/keys"),
                },
                profile: "server".into(),
                name: format!("www-{}", algo.name()),
                common_name: None,
                dns_names: vec!["www.example.test".into()],
                ip_addresses: vec![],
                csr: None,
                key_algorithm: Some(algo),
                issuer_algorithm: Some(algo),
                issuer_partition: None,
                not_before: ts("2027-02-01T00:00:00Z"),
                not_after: ts("2027-05-01T00:00:00Z"),
                private_store: None,
            },
            &leaf_dir,
        )
        .unwrap();
        let leaf = load_bundle(&leaf_dir).unwrap();
        let report = engine::validate_leaf(
            &leaf,
            &[&new, &root],
            TrustSelection::All,
            ts("2027-03-01T00:00:00Z"),
        )
        .unwrap();
        assert!(
            report.valid,
            "{} leaf from the renewed set must validate: {:?}",
            algo.name(),
            report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
        );
    }

    // Renewing the renewal moves the revision on and points at r2.
    let third_dir = tmp.path().join("web-r3");
    let third = engine::renew_issuer_set(&renew_request(&renewed_dir, &root_dir, true), &third_dir)
        .unwrap();
    assert_eq!(third.revision, 3);
    assert_eq!(
        third.predecessor.as_deref(),
        Some(manifest.bundle_id.as_str())
    );
    assert_eq!(third.generation, old.manifest.generation);
}

#[test]
fn renewal_needs_the_key_reuse_opt_in() {
    let tmp = TempDir::new("opt-in");
    let root_dir = tmp.path().join("roots");
    let issuer_dir = tmp.path().join("web");
    let out = tmp.path().join("web-r2");
    make_roots(&root_dir);
    make_issuers(&root_dir, &issuer_dir);
    let err = engine::renew_issuer_set(&renew_request(&issuer_dir, &root_dir, false), &out)
        .unwrap_err()
        .to_string();
    assert!(err.contains("--allow-key-reuse"), "{err}");
    assert!(!out.join("manifest.json").exists());
}

#[test]
fn renewal_refuses_a_root_set_the_issuers_do_not_chain_to() {
    let tmp = TempDir::new("foreign-root");
    let root_dir = tmp.path().join("roots");
    let other_root = tmp.path().join("other-roots");
    let issuer_dir = tmp.path().join("web");
    let out = tmp.path().join("web-r2");
    make_roots(&root_dir);
    make_roots(&other_root);
    make_issuers(&root_dir, &issuer_dir);
    let err = engine::renew_issuer_set(&renew_request(&issuer_dir, &other_root, true), &out)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not the root set"), "{err}");
    assert!(!out.join("manifest.json").exists());
}

#[test]
fn the_cli_renews_and_inspect_shows_the_revision() {
    let tmp = TempDir::new("cli");
    let root_dir = tmp.path().join("roots");
    let issuer_dir = tmp.path().join("web");
    let out = tmp.path().join("web-r2");
    make_roots(&root_dir);
    make_issuers(&root_dir, &issuer_dir);
    let args = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = [
            "issuer",
            "renew",
            "--issuer",
            issuer_dir.to_str().unwrap(),
            "--root",
            root_dir.to_str().unwrap(),
            "--root-key-dir",
            root_dir.join("private/keys").to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--not-before",
            "2027-01-01T00:00:00Z",
            "--valid-for",
            "2y",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    // Without the opt-in the CLI refuses and writes nothing.
    assert_ne!(fleet_cert::cli::run(&args(&[])), 0);
    assert!(!out.join("manifest.json").exists());
    assert_eq!(fleet_cert::cli::run(&args(&["--allow-key-reuse"])), 0);
    let renewed = load_bundle(&out).unwrap();
    assert_eq!(renewed.manifest.revision, 2);
    let json = std::fs::read_to_string(out.join("manifest.json")).unwrap();
    assert!(json.contains("\"revision\": 2"), "{json}");
    assert!(json.contains("\"predecessor\""), "{json}");
}
