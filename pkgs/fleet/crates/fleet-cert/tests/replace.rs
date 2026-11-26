//! `cert root replace` / `cert issuer replace` (spec §5.2): the
//! orchestration moves whole hierarchies, verifies every chain, and
//! leaves nothing behind when a step fails.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::{Bundle, load_bundle};
use fleet_cert::chain::TrustSelection;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
use fleet_cert::replace::{ReplaceRequest, replace};
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
            "fleet-cert-replace-{tag}-{}-{nanos}",
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

fn make_roots(dir: &Path, org: &str) {
    engine::create_root_set(
        &RootCreateRequest {
            organization: org.into(),
            country: "US".into(),
            algorithms: vec![
                AlgorithmId::Ed25519,
                AlgorithmId::P256,
                AlgorithmId::MlDsa44,
            ],
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

fn make_issuers(
    root: &Path,
    dir: &Path,
    name: &str,
    purpose: IssuerPurpose,
    slots: Vec<(AlgorithmId, String)>,
) {
    engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root.to_path_buf(),
            root_signer: SignerSource {
                key_dir: root.join("private/keys"),
            },
            name: name.into(),
            purpose,
            ou: None,
            slots,
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2031-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            permitted_dns: vec![],
            private_store: None,
        },
        dir,
    )
    .unwrap();
}

fn web_slots() -> Vec<(AlgorithmId, String)> {
    vec![
        (AlgorithmId::P256, "a".into()),
        (AlgorithmId::P256, "b".into()),
        (AlgorithmId::MlDsa44, "a".into()),
    ]
}

fn base_slots() -> Vec<(AlgorithmId, String)> {
    vec![
        (AlgorithmId::Ed25519, "a".into()),
        (AlgorithmId::P256, "a".into()),
    ]
}

fn request(root: &Path, issuers: &[&Path], rotate_roots: bool) -> ReplaceRequest {
    ReplaceRequest {
        root_bundle: root.to_path_buf(),
        root_signer: SignerSource {
            key_dir: root.join("private/keys"),
        },
        issuer_bundles: issuers.iter().map(|p| p.to_path_buf()).collect(),
        rotate_roots,
        not_before: ts("2027-01-01T00:00:00Z"),
        root_not_after: ts("2047-01-01T00:00:00Z"),
        issuer_not_after: ts("2032-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        private_store: None,
    }
}

fn leaf(issuer: &Path, out: &Path, algo: AlgorithmId) -> Bundle {
    engine::issue_leaf(
        &LeafIssueRequest {
            issuer_bundle: issuer.to_path_buf(),
            issuer_signer: SignerSource {
                key_dir: issuer.join("private/keys"),
            },
            profile: "server".into(),
            name: "svc".into(),
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
        out,
    )
    .unwrap();
    load_bundle(out).unwrap()
}

fn valid(leaf: &Bundle, pool: &[&Bundle], trust: TrustSelection) -> bool {
    engine::validate_leaf(leaf, pool, trust, ts("2027-03-01T00:00:00Z"))
        .unwrap()
        .valid
}

#[test]
fn root_replace_moves_every_issuer_set_onto_new_roots_with_an_overlap() {
    let tmp = TempDir::new("root");
    let roots = tmp.path().join("roots");
    let web = tmp.path().join("web");
    let base = tmp.path().join("base");
    make_roots(&roots, "Test Org LLC");
    make_issuers(&roots, &web, "Web", IssuerPurpose::WebServer, web_slots());
    make_issuers(&roots, &base, "Base", IssuerPurpose::General, base_slots());
    let out = tmp.path().join("next");
    let report = replace(&request(&roots, &[&web, &base], true), &out).unwrap();

    let old_roots = load_bundle(&roots).unwrap();
    let root = report
        .root
        .as_ref()
        .expect("root replace rotates the roots");
    assert_eq!(root.manifest.generation, 2);
    assert_eq!(root.predecessor, old_roots.manifest.bundle_id);
    let new_roots = load_bundle(Path::new(&root.dir)).unwrap();
    assert_eq!(report.issuers.len(), 2);
    for issuer in &report.issuers {
        assert_eq!(issuer.manifest.generation, 2);
        assert_eq!(
            issuer.manifest.parent.as_ref().unwrap().bundle_id,
            root.manifest.bundle_id
        );
    }
    let overlaps = report
        .checks
        .iter()
        .filter(|c| c.detail.contains("to the old roots"))
        .count();
    assert_eq!(overlaps, web_slots().len() + base_slots().len());
    assert!(report.checks.iter().all(|c| c.ok));
    assert_eq!(report.retire_old_after, ts("2031-01-01T00:00:00Z"));
    assert_eq!(report.next_steps.len(), 4);

    // New leaves validate on the old roots and on the new ones.
    let new_web = load_bundle(Path::new(&report.issuers[0].dir)).unwrap();
    for algo in [AlgorithmId::P256, AlgorithmId::MlDsa44] {
        let l = leaf(
            Path::new(&report.issuers[0].dir),
            &tmp.path().join(format!("leaf-{}", algo.name())),
            algo,
        );
        assert!(valid(&l, &[&new_web, &old_roots], TrustSelection::All));
        assert!(valid(&l, &[&new_web, &new_roots], TrustSelection::All));
    }

    let err = replace(&request(&roots, &[&web], true), &out)
        .unwrap_err()
        .to_string();
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn a_failed_replace_leaves_nothing_behind() {
    let tmp = TempDir::new("failed");
    let roots = tmp.path().join("roots");
    let other_roots = tmp.path().join("other-roots");
    let web = tmp.path().join("web");
    let foreign = tmp.path().join("foreign");
    make_roots(&roots, "Test Org LLC");
    make_roots(&other_roots, "Other Org LLC");
    make_issuers(&roots, &web, "Web", IssuerPurpose::WebServer, web_slots());
    make_issuers(
        &other_roots,
        &foreign,
        "Foreign",
        IssuerPurpose::General,
        base_slots(),
    );
    let store = tmp.path().join("store");
    std::fs::create_dir_all(&store).unwrap();
    let out = tmp.path().join("next");
    let mut req = request(&roots, &[&web, &foreign], true);
    req.private_store = Some(store.clone());
    let err = replace(&req, &out).unwrap_err().to_string();
    assert!(err.contains("not the root set"), "{err}");
    assert!(!out.exists());
    assert!(!store.join("roots").exists());
    assert!(!store.join("issuers").join("web").exists());
    // The predecessors are untouched and still work.
    let l = leaf(&web, &tmp.path().join("leaf"), AlgorithmId::P256);
    assert!(valid(
        &l,
        &[&load_bundle(&web).unwrap(), &load_bundle(&roots).unwrap()],
        TrustSelection::All
    ));

    let err = replace(&request(&roots, &[], true), &out)
        .unwrap_err()
        .to_string();
    assert!(err.contains("at least one issuer set"), "{err}");
}

#[test]
fn issuer_replace_rotates_under_the_same_roots() {
    let tmp = TempDir::new("issuer");
    let roots = tmp.path().join("roots");
    let web = tmp.path().join("web");
    make_roots(&roots, "Test Org LLC");
    make_issuers(&roots, &web, "Web", IssuerPurpose::WebServer, web_slots());
    let out = tmp.path().join("next");
    let report = replace(&request(&roots, &[&web], false), &out).unwrap();
    let old_roots = load_bundle(&roots).unwrap();
    assert!(report.root.is_none());
    assert_eq!(report.issuers.len(), 1);
    let issuer = &report.issuers[0];
    assert_eq!(issuer.manifest.generation, 2);
    assert_eq!(
        issuer.manifest.parent.as_ref().unwrap().bundle_id,
        old_roots.manifest.bundle_id
    );
    assert!(
        issuer
            .manifest
            .certificates
            .iter()
            .all(|r| r.role == "issuer")
    );
    assert!(report.checks.iter().all(|c| c.ok));
    assert_eq!(report.next_steps.len(), 2);
    let l = leaf(
        Path::new(&issuer.dir),
        &tmp.path().join("leaf"),
        AlgorithmId::P256,
    );
    assert!(valid(
        &l,
        &[&load_bundle(Path::new(&issuer.dir)).unwrap(), &old_roots],
        TrustSelection::All
    ));
}
