//! Integration tests for the fleet-cert Phase 1 slice (#9157).
//!
//! Every test names its oracle — the independent source of truth its
//! assertions are checked against:
//! - the spec's own inventory arithmetic (§§3.2, 24.2),
//! - the crate's chain validator (aws-lc-rs signature checks),
//! - the system `openssl` binary as a fully independent validator
//!   (skipped only when openssl is absent from the test host; the
//!   ML-DSA leg additionally skips, loudly, when the host openssl
//!   predates ML-DSA support),
//! - the manifest checksum index (§7.1).

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::load_bundle;
use fleet_cert::chain::TrustSelection;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
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
            "fleet-cert-test-{tag}-{}-{nanos}",
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

fn root_request(algorithms: Vec<AlgorithmId>) -> RootCreateRequest {
    RootCreateRequest {
        organization: "Test Org LLC".into(),
        country: "US".into(),
        algorithms,
        generation: 1,
        partition: "A".into(),
        cross_signing: CrossSigningMode::Both,
        not_before: ts(NB),
        not_after: ts("2046-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        private_store: None,
    }
}

fn issuer_request(root: &Path, slots: Vec<(AlgorithmId, String)>) -> IssuerCreateRequest {
    IssuerCreateRequest {
        root_bundle: root.to_path_buf(),
        root_signer: SignerSource {
            key_dir: root.join("private/keys"),
        },
        name: "Base".into(),
        purpose: IssuerPurpose::General,
        ou: None,
        slots,
        cross_signing: CrossSigningMode::Both,
        not_before: ts(NB),
        not_after: ts("2031-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        permitted_dns: vec![],
        private_store: None,
    }
}

fn leaf_request(
    issuer: &Path,
    profile: &str,
    name: &str,
    key_algorithm: Option<AlgorithmId>,
    issuer_algorithm: Option<AlgorithmId>,
    valid_for: CalendarDuration,
) -> LeafIssueRequest {
    LeafIssueRequest {
        issuer_bundle: issuer.to_path_buf(),
        issuer_signer: SignerSource {
            key_dir: issuer.join("private/keys"),
        },
        profile: profile.into(),
        name: name.into(),
        common_name: None,
        dns_names: vec!["www.example.test".into()],
        ip_addresses: vec![],
        csr: None,
        key_algorithm,
        issuer_algorithm,
        issuer_partition: None,
        not_before: ts(NB),
        not_after: valid_for.add_to(ts(NB)).unwrap(),
        private_store: None,
    }
}

/// Build a root set + Base issuer set (A,B) in `tmp`; returns
/// (root_dir, issuer_dir). Oracle for counts: §§3.2/24.2 inventory.
fn build_hierarchy(tmp: &TempDir, algorithms: Vec<AlgorithmId>) -> (PathBuf, PathBuf) {
    let root_dir = tmp.path().join("roots");
    let manifest = engine::create_root_set(&root_request(algorithms.clone()), &root_dir).unwrap();
    let expected_roots = fleet_cert::graph::GraphPolicy::spec_default()
        .expected_root_certificates(&algorithms, CrossSigningMode::Both);
    assert_eq!(
        manifest.certificates.len(),
        expected_roots,
        "root inventory must match the §3.2 policy arithmetic"
    );
    let slots: Vec<(AlgorithmId, String)> = algorithms
        .iter()
        .flat_map(|a| [(a.clone(), "a".to_string()), (a.clone(), "b".to_string())])
        .collect();
    let issuer_dir = tmp.path().join("base");
    let im = engine::create_issuer_set(&issuer_request(&root_dir, slots), &issuer_dir).unwrap();
    // §24.2: per partition, classical keys carry 3 variants and the
    // ML-DSA key 4, scaled by which roots exist.
    let graph = fleet_cert::graph::GraphPolicy::spec_default();
    let expected_variants: usize = algorithms
        .iter()
        .map(|a| graph.expected_issuer_variants(*a, &algorithms, CrossSigningMode::Both) * 2)
        .sum();
    assert_eq!(
        im.certificates.len(),
        expected_variants,
        "issuer inventory must match the §24.2 policy arithmetic"
    );
    (root_dir, issuer_dir)
}

#[test]
fn full_synthetic_hierarchy_validates() {
    // Oracle: the spec inventory arithmetic (§§3.2/24.2) + the
    // crate validator's per-link signature checks.
    let tmp = TempDir::new("full");
    let (root_dir, issuer_dir) = build_hierarchy(
        &tmp,
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa4096,
            AlgorithmId::MlDsa44,
        ],
    );
    // Root bundle self-validation: every signature + inventory.
    let checks = engine::validate_root_bundle(&root_dir).unwrap();
    assert!(
        checks.iter().all(|c| c.ok),
        "root validation failed: {:?}",
        checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
    );
    // §3.2 exact count for the default set under `both`.
    let root_bundle = load_bundle(&root_dir).unwrap();
    assert_eq!(root_bundle.manifest.certificates.len(), 13);
    let issuer_bundle = load_bundle(&issuer_dir).unwrap();
    assert_eq!(issuer_bundle.manifest.certificates.len(), 26);
    // Distinct issuer keys: 8 key files for 8 slots.
    let mut key_paths: Vec<&str> = issuer_bundle
        .manifest
        .certificates
        .iter()
        .filter_map(|r| r.key_path.as_deref())
        .collect();
    key_paths.sort();
    key_paths.dedup();
    assert_eq!(key_paths.len(), 8, "A/B slots must be independent keys");

    // One leaf per idp-ca profile; every chain validates.
    let root_b = load_bundle(&root_dir).unwrap();
    let issuer_b = load_bundle(&issuer_dir).unwrap();
    for (profile_name, _) in [
        ("server", ()),
        ("client", ()),
        ("saml-signing", ()),
        ("oidc-signing", ()),
    ] {
        let leaf_dir = tmp.path().join(format!("leaf-{profile_name}"));
        let req = leaf_request(
            &issuer_dir,
            profile_name,
            &format!("leaf-{profile_name}"),
            None,
            None,
            CalendarDuration::days(365),
        );
        engine::issue_leaf(&req, &leaf_dir).unwrap();
        let leaf_b = load_bundle(&leaf_dir).unwrap();
        let report =
            engine::validate_leaf(&leaf_b, &[&issuer_b, &root_b], TrustSelection::All, ts(AT))
                .unwrap();
        assert!(
            report.valid,
            "{profile_name} chain invalid: {:?}",
            report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
        );
        assert_eq!(report.path.len(), 3, "leaf → issuer → root");
    }
}

#[test]
fn pq_policy_negative_and_positive() {
    // Oracle: §3.2's verifier-policy rule — under a PQ-only trust
    // selection, a classical path must be rejected even though
    // classical cross-certificates over the ML-DSA key exist, and
    // the all-ML-DSA path must be accepted.
    let tmp = TempDir::new("pq");
    let (root_dir, issuer_dir) = build_hierarchy(
        &tmp,
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::MlDsa44,
        ],
    );
    let root_b = load_bundle(&root_dir).unwrap();
    let issuer_b = load_bundle(&issuer_dir).unwrap();

    // Classical leaf (profile default P-256 key + issuer).
    let leaf_dir = tmp.path().join("leaf-classical");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "classical",
            None,
            None,
            CalendarDuration::days(90),
        ),
        &leaf_dir,
    )
    .unwrap();
    let leaf_b = load_bundle(&leaf_dir).unwrap();
    let report = engine::validate_leaf(
        &leaf_b,
        &[&issuer_b, &root_b],
        TrustSelection::Pq,
        ts("2026-02-01T00:00:00Z"),
    )
    .unwrap();
    assert!(
        !report.valid,
        "classical leaf must be rejected under PQ-only trust"
    );
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "path-builds" && !c.ok),
        "the rejection must be the missing PQ path, not a side effect: {:?}",
        report.checks
    );

    // ML-DSA leaf under the ML-DSA issuer partition.
    let pq_dir = tmp.path().join("leaf-pq");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "pq",
            Some(AlgorithmId::MlDsa44),
            Some(AlgorithmId::MlDsa44),
            CalendarDuration::days(90),
        ),
        &pq_dir,
    )
    .unwrap();
    let pq_b = load_bundle(&pq_dir).unwrap();
    let report = engine::validate_leaf(
        &pq_b,
        &[&issuer_b, &root_b],
        TrustSelection::Pq,
        ts("2026-02-01T00:00:00Z"),
    )
    .unwrap();
    assert!(
        report.valid,
        "all-ML-DSA path must validate under PQ trust: {:?}",
        report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
    );
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "pq-only-path" && c.ok)
    );
    // And the same leaf is fine under the all-roots selection.
    let report = engine::validate_leaf(
        &pq_b,
        &[&issuer_b, &root_b],
        TrustSelection::All,
        ts("2026-02-01T00:00:00Z"),
    )
    .unwrap();
    assert!(report.valid);
}

#[test]
fn cross_signed_path_survives_primary_root_removal() {
    // Oracle: path building must find the cross-signed variant —
    // with the P-256 root removed from the pool, a P-256 issuer's
    // leaf still chains to a classical cross-signing root.
    let tmp = TempDir::new("cross");
    let (root_dir, issuer_dir) = build_hierarchy(
        &tmp,
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::MlDsa44,
        ],
    );
    let leaf_dir = tmp.path().join("leaf");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "x",
            None,
            None,
            CalendarDuration::days(90),
        ),
        &leaf_dir,
    )
    .unwrap();
    let leaf_b = load_bundle(&leaf_dir).unwrap();
    let issuer_b = load_bundle(&issuer_dir).unwrap();
    let root_b = load_bundle(&root_dir).unwrap();
    let mut pool = engine::collect_pool(&[&issuer_b, &root_b]).unwrap();
    // Remove the P-256 self-signed root from the pool.
    pool.retain(|c| {
        !(c.subject_algorithm == Some(AlgorithmId::P256)
            && c.subject_dn.contains("Root")
            && c.issuer_dn == c.subject_dn)
    });
    let leaf_record = &leaf_b.manifest.certificates[0];
    let pem = std::fs::read(leaf_dir.join(&leaf_record.cert_path)).unwrap();
    let blocks = fleet_cert::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap()).unwrap();
    let leaf = fleet_cert::chain::parse_cert(&blocks[0].1).unwrap();
    let report = fleet_cert::chain::validate_chain(
        &leaf,
        &pool,
        TrustSelection::All,
        ts("2026-02-01T00:00:00Z"),
    );
    assert!(
        report.valid,
        "cross-signed path must validate without the primary root: {:?}",
        report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
    );
    let anchor = report.path.last().unwrap();
    assert!(
        anchor.contains("Ed25519") || anchor.contains("ML-DSA"),
        "anchor should be a cross-signing root, got {anchor}"
    );
}

/// Whether the host `openssl` can verify ML-DSA signatures.
/// ML-DSA support landed in OpenSSL 3.5; older builds (e.g. the
/// fleet-command executor image) list no ML-DSA signature
/// algorithms. Probe the algorithm list rather than parsing a
/// version string, so provider configuration is taken into account.
fn openssl_supports_ml_dsa() -> bool {
    std::process::Command::new("openssl")
        .args(["list", "-signature-algorithms"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("ML-DSA"))
        .unwrap_or(false)
}

#[test]
fn openssl_independent_validation() {
    // Oracle: the system openssl binary — a fully independent
    // implementation (§11.2). Skipped only when openssl is absent;
    // the ML-DSA leg below additionally skips when the host openssl
    // predates ML-DSA support (#15136).
    if std::process::Command::new("openssl")
        .arg("version")
        .output()
        .is_err()
    {
        eprintln!("openssl not available; skipping independent validation");
        return;
    }
    let tmp = TempDir::new("openssl");
    let (root_dir, issuer_dir) = build_hierarchy(
        &tmp,
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::MlDsa44,
        ],
    );
    let verify = |cafile: &Path, untrusted: &Path, leaf: &Path| -> bool {
        std::process::Command::new("openssl")
            .args([
                "verify",
                "-CAfile",
                cafile.to_str().unwrap(),
                "-untrusted",
                untrusted.to_str().unwrap(),
                leaf.to_str().unwrap(),
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    // Classical leaf against roots-all via its primary issuer.
    let leaf_dir = tmp.path().join("leaf");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "ossl",
            None,
            None,
            CalendarDuration::years(3),
        ),
        &leaf_dir,
    )
    .unwrap();
    assert!(verify(
        &root_dir.join("public/bundles/roots-all.pem"),
        &issuer_dir.join("public/certs/p256/a/signed-by-p256.crt.pem"),
        &leaf_dir.join("leaf.crt.pem"),
    ));
    // ML-DSA leaf against roots-pq via the ML-DSA primary issuer.
    // This leg needs an openssl that can verify ML-DSA (3.5+);
    // where the host openssl predates it, skip the leg loudly
    // instead of failing on an environment limit (#15136).
    if openssl_supports_ml_dsa() {
        let pq_dir = tmp.path().join("leaf-pq");
        engine::issue_leaf(
            &leaf_request(
                &issuer_dir,
                "server",
                "ossl-pq",
                Some(AlgorithmId::MlDsa44),
                Some(AlgorithmId::MlDsa44),
                CalendarDuration::years(3),
            ),
            &pq_dir,
        )
        .unwrap();
        assert!(verify(
            &root_dir.join("public/bundles/roots-pq.pem"),
            &issuer_dir.join("public/certs/ml-dsa-44/a/signed-by-ml-dsa-44.crt.pem"),
            &pq_dir.join("leaf.crt.pem"),
        ));
    } else {
        eprintln!(
            "openssl lacks ML-DSA support (pre-3.5); skipping the ML-DSA leg of independent validation"
        );
    }
    // Cross path: the P-256 issuer variant signed by the Ed25519
    // root, anchored on the Ed25519 root alone.
    let ed_root_pem = root_dir.join("public/roots/ed25519/root.crt.pem");
    assert!(verify(
        &ed_root_pem,
        &issuer_dir.join("public/certs/p256/a/signed-by-ed25519.crt.pem"),
        &leaf_dir.join("leaf.crt.pem"),
    ));
}

#[test]
fn signer_source_and_legacy_gates() {
    // Oracle: §24.3 (explicit signer source) and §24.1(1) (legacy
    // gate) as policy errors, not silent fallbacks.
    let tmp = TempDir::new("gates");
    // Empty signer source directory: the root key load must fail
    // with a policy error naming the signer source.
    let empty = tmp.path().join("empty-keys");
    std::fs::create_dir_all(&empty).unwrap();
    let err = engine::load_signer_key(
        &SignerSource { key_dir: empty },
        "ed25519.pk8",
        AlgorithmId::Ed25519,
    )
    .err()
    .expect("missing key must error");
    assert!(
        format!("{err}").contains("signer source"),
        "error must name the signer-source rule: {err}"
    );
    // RSA-2048 root without the opt-in is refused; with it, created.
    let mut req = root_request(vec![AlgorithmId::Rsa2048]);
    let out = tmp.path().join("legacy-denied");
    let err = engine::create_root_set(&req, &out)
        .err()
        .expect("must refuse");
    assert!(format!("{err}").contains("--allow-legacy-root-strength"));
    req.allow_legacy_root_strength = true;
    let out2 = tmp.path().join("legacy-allowed");
    let m = engine::create_root_set(&req, &out2).unwrap();
    assert_eq!(m.certificates.len(), 1);
}

#[test]
fn leaf_validity_clamps_to_issuer_with_warning() {
    // Oracle: §5.4 — a leaf must not outlive its issuer; the clamp
    // is applied and recorded as a manifest warning.
    let tmp = TempDir::new("clamp");
    let (root_dir, issuer_dir) = build_hierarchy(&tmp, vec![AlgorithmId::Ed25519]);
    let _ = root_dir;
    let leaf_dir = tmp.path().join("leaf");
    let mut req = leaf_request(
        &issuer_dir,
        "server",
        "longlived",
        None,
        None,
        CalendarDuration::years(20),
    );
    req.not_after = ts("2046-01-01T00:00:00Z");
    let m = engine::issue_leaf(&req, &leaf_dir).unwrap();
    assert_eq!(
        m.certificates[0].not_after,
        ts("2031-01-01T00:00:00Z"),
        "leaf not-after must clamp to the issuer's"
    );
    assert!(
        m.warnings.iter().any(|w| w.contains("clamped")),
        "the clamp must be recorded as a warning: {:?}",
        m.warnings
    );
}

#[test]
fn tampered_bundle_is_rejected() {
    // Oracle: the §7.1 manifest checksum index — flipping bytes in
    // an indexed artifact must make the bundle unloadable.
    let tmp = TempDir::new("tamper");
    let root_dir = tmp.path().join("roots");
    engine::create_root_set(&root_request(vec![AlgorithmId::Ed25519]), &root_dir).unwrap();
    assert!(load_bundle(&root_dir).is_ok());
    let cert_path = root_dir.join("public/roots/ed25519/root.crt.pem");
    let mut bytes = std::fs::read(&cert_path).unwrap();
    bytes[40] ^= 0xff;
    std::fs::write(&cert_path, bytes).unwrap();
    let err = load_bundle(&root_dir).err().expect("tamper must be caught");
    assert!(format!("{err}").contains("checksum"), "got: {err}");
}

#[test]
fn revoked_leaf_is_reported_by_validation() {
    // Oracle: §6.1/§8 revocation semantics — a leaf that validates
    // before revocation must be reported invalid, with a `revoked`
    // check, once its revocation is in effect; the revocation record
    // must be checksummed in the manifest index.
    let tmp = TempDir::new("revoke-leaf");
    let (root_dir, issuer_dir) = build_hierarchy(&tmp, vec![AlgorithmId::Ed25519]);
    let leaf_dir = tmp.path().join("leaf");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "revoked-leaf",
            None,
            None,
            CalendarDuration::days(365),
        ),
        &leaf_dir,
    )
    .unwrap();

    let before = engine::validate_leaf(
        &load_bundle(&leaf_dir).unwrap(),
        &[
            &load_bundle(&issuer_dir).unwrap(),
            &load_bundle(&root_dir).unwrap(),
        ],
        TrustSelection::All,
        ts(AT),
    )
    .unwrap();
    assert!(before.valid, "leaf must validate before revocation");

    let records = fleet_cert::revoke::revoke_bundle(&fleet_cert::revoke::RevokeRequest {
        bundle: leaf_dir.clone(),
        target: fleet_cert::model::RevokeTarget::Leaf,
        reason: "key-compromise".into(),
        revoked_at: ts(AT),
    })
    .unwrap();
    assert_eq!(records.len(), 1, "one leaf certificate revoked");

    let leaf_b = load_bundle(&leaf_dir).unwrap();
    assert_eq!(leaf_b.manifest.revocations.len(), 1);
    assert!(
        leaf_b
            .manifest
            .files
            .iter()
            .any(|f| f.path == records[0].path),
        "revocation record must be in the file index"
    );

    let after = engine::validate_leaf(
        &leaf_b,
        &[
            &load_bundle(&issuer_dir).unwrap(),
            &load_bundle(&root_dir).unwrap(),
        ],
        TrustSelection::All,
        ts(AT),
    )
    .unwrap();
    assert!(!after.valid, "revoked leaf must not validate");
    assert!(
        after.checks.iter().any(|c| c.name == "revoked" && !c.ok),
        "validation must report the revocation: {:?}",
        after.checks
    );

    // Idempotent: revoking again adds no record.
    let again = fleet_cert::revoke::revoke_bundle(&fleet_cert::revoke::RevokeRequest {
        bundle: leaf_dir.clone(),
        target: fleet_cert::model::RevokeTarget::Leaf,
        reason: "key-compromise".into(),
        revoked_at: ts(AT),
    })
    .unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(
        load_bundle(&leaf_dir).unwrap().manifest.revocations.len(),
        1
    );
}

#[test]
fn revoked_issuer_invalidates_its_leaves_and_wrong_target_is_refused() {
    // Oracle: the same validator as above — a revoked issuer CA on
    // the path fails every leaf under it; a leaf target against an
    // issuer bundle is a policy refusal, not a silent no-op.
    let tmp = TempDir::new("revoke-issuer");
    let (root_dir, issuer_dir) = build_hierarchy(&tmp, vec![AlgorithmId::Ed25519]);
    let leaf_dir = tmp.path().join("leaf");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "client",
            "under-issuer",
            None,
            None,
            CalendarDuration::days(365),
        ),
        &leaf_dir,
    )
    .unwrap();

    let wrong = fleet_cert::revoke::revoke_bundle(&fleet_cert::revoke::RevokeRequest {
        bundle: issuer_dir.clone(),
        target: fleet_cert::model::RevokeTarget::Leaf,
        reason: "unspecified".into(),
        revoked_at: ts(AT),
    });
    assert!(
        wrong.is_err(),
        "leaf revoke against an issuer set must be refused"
    );

    fleet_cert::revoke::revoke_bundle(&fleet_cert::revoke::RevokeRequest {
        bundle: issuer_dir.clone(),
        target: fleet_cert::model::RevokeTarget::Issuer,
        reason: "ca-compromise".into(),
        revoked_at: ts(AT),
    })
    .unwrap();

    let report = engine::validate_leaf(
        &load_bundle(&leaf_dir).unwrap(),
        &[
            &load_bundle(&issuer_dir).unwrap(),
            &load_bundle(&root_dir).unwrap(),
        ],
        TrustSelection::All,
        ts(AT),
    )
    .unwrap();
    assert!(
        !report.valid,
        "leaf under a revoked issuer must not validate"
    );
    assert!(report.checks.iter().any(|c| c.name == "revoked" && !c.ok));
}

#[test]
fn signed_crl_contains_revoked_leaf_and_verifies() {
    // Oracles: x509-parser checks CRL structure/serials, aws-lc-rs verifies
    // the signature against the issuer certificate, and the chain validator
    // confirms the same revocation record invalidates the leaf path.
    let tmp = TempDir::new("crl");
    let (root_dir, issuer_dir) = build_hierarchy(&tmp, vec![AlgorithmId::Ed25519]);
    let leaf_dir = tmp.path().join("leaf");
    engine::issue_leaf(
        &leaf_request(
            &issuer_dir,
            "server",
            "crl-leaf",
            None,
            None,
            CalendarDuration::days(365),
        ),
        &leaf_dir,
    )
    .unwrap();
    fleet_cert::revoke::revoke_bundle(&fleet_cert::revoke::RevokeRequest {
        bundle: leaf_dir.clone(),
        target: fleet_cert::model::RevokeTarget::Leaf,
        reason: "key-compromise".into(),
        revoked_at: ts(AT),
    })
    .unwrap();

    let issuer = load_bundle(&issuer_dir).unwrap();
    let issuer_cert = issuer
        .manifest
        .certificates
        .iter()
        .find(|r| r.role == "issuer" && r.primary)
        .unwrap();
    let request = fleet_cert::crl::GenerateRequest {
        issuer_bundle: issuer_dir.clone(),
        signer: SignerSource {
            key_dir: issuer_dir.join("private/keys"),
        },
        revocation_bundles: vec![leaf_dir.clone()],
        this_update: ts(AT),
        next_update: ts("2026-06-08T00:00:00Z"),
        crl_number: 1,
        previous_crl: None,
    };
    let der = fleet_cert::crl::generate(&request, Some(&issuer_cert.fingerprint_sha256)).unwrap();
    let inspected = fleet_cert::crl::inspect(&der).unwrap();
    let leaf = load_bundle(&leaf_dir).unwrap();
    assert_eq!(inspected.revoked.len(), 1);
    assert_eq!(
        inspected.revoked[0].serial_hex,
        leaf.manifest.certificates[0].serial_hex
    );
    let verified = fleet_cert::crl::verify(&der, &issuer_dir, &[leaf_dir.clone()], ts(AT)).unwrap();
    assert!(
        verified.valid,
        "CRL verification failed: {:?}",
        verified.checks
    );

    let mut tampered = der.clone();
    *tampered.last_mut().unwrap() ^= 1;
    let tampered_report =
        fleet_cert::crl::verify(&tampered, &issuer_dir, &[leaf_dir.clone()], ts(AT)).unwrap();
    assert!(!tampered_report.valid);
    assert!(!tampered_report.signature_valid);

    let duplicate_evidence = fleet_cert::crl::generate(
        &fleet_cert::crl::GenerateRequest {
            revocation_bundles: vec![leaf_dir.clone(), leaf_dir.clone()],
            ..request
        },
        Some(&issuer_cert.fingerprint_sha256),
    );
    assert!(duplicate_evidence.is_err());

    let report = engine::validate_leaf(
        &leaf,
        &[&issuer, &load_bundle(&root_dir).unwrap()],
        TrustSelection::All,
        ts(AT),
    )
    .unwrap();
    assert!(!report.valid);
    assert!(report.checks.iter().any(|c| c.name == "revoked" && !c.ok));
}
