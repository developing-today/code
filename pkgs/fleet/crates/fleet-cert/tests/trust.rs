//! Trust bundles and the broker-client trust store (BT-2,
//! #14400): `fleet cert trust export` assembling the §3.5 bundle
//! from synthetic root sets, and the §27.3 item 9 two-phase
//! rollover — stage → acknowledge → activate → retire —
//! demonstrated end to end in a synthetic estate.
//!
//! Oracles: the design of record's rules (doc/2026-11-24T13-59-26Z_
//! design_broker-tls §6), spec §3.5 (bundle file membership),
//! spec §27.3 item 9 (no activation without acknowledgement;
//! last-known-good retention), and the crate's own strict bundle
//! verification (a tampered file fails its manifest checksum).

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::{load_bundle, pem_decode_all};
use fleet_cert::chain::TrustSelection;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{BundleType, RootCreateRequest};
use fleet_cert::timeparse::Rfc3339;
use fleet_cert::trust::{
    acknowledge, activate, active_anchors, export_trust_bundle, read_descriptor, retire,
    stage_bundle, store_status, TrustExportRequest,
};
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
            "fleet-cert-trust-{tag}-{}-{nanos}",
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

fn root_request(algorithms: Vec<AlgorithmId>, generation: u32) -> RootCreateRequest {
    RootCreateRequest {
        organization: "Test Org LLC".into(),
        country: "US".into(),
        algorithms,
        generation,
        partition: "A".into(),
        cross_signing: CrossSigningMode::Both,
        not_before: ts(NB),
        not_after: ts("2046-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        private_store: None,
    }
}

fn root_set(tmp: &TempDir, tag: &str, algorithms: Vec<AlgorithmId>, generation: u32) -> PathBuf {
    let dir = tmp.path().join(tag);
    engine::create_root_set(&root_request(algorithms, generation), &dir).unwrap();
    dir
}

fn export(tmp: &TempDir, tag: &str, roots: &[PathBuf], version: u32) -> PathBuf {
    let out = tmp.path().join(tag);
    let manifest = export_trust_bundle(
        &TrustExportRequest {
            root_bundles: roots.to_vec(),
            version,
            name: "Fleet Trust Bundle".into(),
            at: ts(AT),
        },
        &out,
    )
    .unwrap();
    assert_eq!(manifest.bundle_type, BundleType::Trust);
    assert_eq!(manifest.bundle_version, version);
    out
}

fn pem_count(bundle_dir: &Path, rel: &str) -> Option<usize> {
    let text = std::fs::read_to_string(bundle_dir.join(rel)).ok()?;
    Some(pem_decode_all(&text).unwrap().len())
}

#[test]
fn export_install_rollover_end_to_end() {
    let tmp = TempDir::new("e2e");
    let store = tmp.path().join("store");

    // Estate v1: two classical roots.
    let roots1 = root_set(
        &tmp,
        "roots1",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        1,
    );
    let bundle1 = export(&tmp, "tb1", &[roots1], 1);

    // §3.5 membership: classical + all carry both roots, no PQ
    // file exists, cross-certs ride along as untrusted material.
    assert_eq!(
        pem_count(&bundle1, "public/bundles/roots-classical.pem"),
        Some(2)
    );
    assert_eq!(pem_count(&bundle1, "public/bundles/roots-all.pem"), Some(2));
    assert_eq!(pem_count(&bundle1, "public/bundles/roots-pq.pem"), None);
    assert!(pem_count(&bundle1, "public/bundles/cross-certs.pem").unwrap() >= 1);
    let loaded = load_bundle(&bundle1).unwrap();
    let descriptor = read_descriptor(&loaded).unwrap();
    assert_eq!(descriptor.version, 1);
    assert_eq!(descriptor.root_fingerprints.len(), 2);
    assert_eq!(descriptor.sources.len(), 1);

    // Install v1: stage, then activation must refuse before the
    // acknowledgement exists (§27.3 item 9).
    let vref1 = stage_bundle(&store, &bundle1).unwrap();
    assert_eq!(vref1.version, 1);
    let err = activate(&store, None).unwrap_err();
    assert!(
        err.to_string().contains("without acknowledgements"),
        "unexpected error: {err}"
    );
    let ack1 = acknowledge(&store, None, ts(AT)).unwrap();
    assert_eq!(ack1.version, 1);
    assert_eq!(ack1.content_sha256, vref1.content_sha256);
    activate(&store, None).unwrap();

    let (active_ref, anchors) = active_anchors(&store, TrustSelection::Classical).unwrap();
    assert_eq!(active_ref.version, 1);
    assert_eq!(pem_decode_all(std::str::from_utf8(&anchors).unwrap()).unwrap().len(), 2);
    // No PQ anchors in v1: that selection is not served.
    assert!(active_anchors(&store, TrustSelection::Pq).is_err());

    // Estate v2: fresh keys, plus an ML-DSA root.
    let roots2 = root_set(
        &tmp,
        "roots2",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256, AlgorithmId::MlDsa44],
        2,
    );
    let bundle2 = export(&tmp, "tb2", &[roots2], 2);
    assert_eq!(pem_count(&bundle2, "public/bundles/roots-all.pem"), Some(3));
    assert_eq!(pem_count(&bundle2, "public/bundles/roots-pq.pem"), Some(1));

    stage_bundle(&store, &bundle2).unwrap();
    acknowledge(&store, Some(2), ts(AT)).unwrap();
    activate(&store, None).unwrap();

    let status = store_status(&store).unwrap();
    assert_eq!(status.active.as_ref().map(|v| v.version), Some(2));
    assert_eq!(status.previous.as_ref().map(|v| v.version), Some(1));
    assert_eq!(status.staged, None);
    assert_eq!(status.acks.len(), 2);
    assert_eq!(status.installed_versions, vec![1, 2]);
    let (_, pq_anchors) = active_anchors(&store, TrustSelection::Pq).unwrap();
    assert_eq!(
        pem_decode_all(std::str::from_utf8(&pq_anchors).unwrap()).unwrap().len(),
        1
    );

    // Retire guards: the active version is in use and the
    // previous one is the last-known-good rollback path.
    assert!(retire(&store, 2).is_err());
    assert!(retire(&store, 1).is_err());
}

#[test]
fn corrupt_distribution_never_touches_active() {
    let tmp = TempDir::new("corrupt");
    let store = tmp.path().join("store");

    let roots1 = root_set(
        &tmp,
        "roots1",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        1,
    );
    let bundle1 = export(&tmp, "tb1", &[roots1.clone()], 1);
    stage_bundle(&store, &bundle1).unwrap();
    acknowledge(&store, None, ts(AT)).unwrap();
    activate(&store, None).unwrap();

    // A tampered distribution fails its manifest checksum at
    // stage time; the active version is untouched.
    let roots2 = root_set(
        &tmp,
        "roots2",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        2,
    );
    let bundle2 = export(&tmp, "tb2", &[roots2], 2);
    let classical = bundle2.join("public/bundles/roots-classical.pem");
    let mut bytes = std::fs::read(&classical).unwrap();
    bytes.push(b'X');
    std::fs::write(&classical, bytes).unwrap();
    assert!(stage_bundle(&store, &bundle2).is_err());
    let status = store_status(&store).unwrap();
    assert_eq!(status.active.as_ref().map(|v| v.version), Some(1));
    assert_eq!(status.staged, None);

    // Tampering with the *installed* staged copy is caught when
    // the acknowledgement re-verifies what it would bind.
    let bundle2b = export(&tmp, "tb2b", &[roots1], 2);
    stage_bundle(&store, &bundle2b).unwrap();
    let installed = store
        .join("versions/2/public/bundles/roots-all.pem");
    let mut bytes = std::fs::read(&installed).unwrap();
    bytes.push(b'Y');
    std::fs::write(&installed, bytes).unwrap();
    assert!(acknowledge(&store, None, ts(AT)).is_err());
    let status = store_status(&store).unwrap();
    assert_eq!(status.active.as_ref().map(|v| v.version), Some(1));
}

#[test]
fn version_reuse_with_different_content_is_refused() {
    let tmp = TempDir::new("reuse");
    let store = tmp.path().join("store");

    let roots_a = root_set(&tmp, "rootsA", vec![AlgorithmId::Ed25519], 1);
    let bundle_a = export(&tmp, "tbA", &[roots_a], 1);
    stage_bundle(&store, &bundle_a).unwrap();

    // A different estate exported under the same version number:
    // the version stamp no longer identifies one bundle.
    let roots_b = root_set(&tmp, "rootsB", vec![AlgorithmId::Ed25519], 1);
    let bundle_b = export(&tmp, "tbB", &[roots_b], 1);
    let err = stage_bundle(&store, &bundle_b).unwrap_err();
    assert!(
        err.to_string().contains("different content"),
        "unexpected error: {err}"
    );
}

#[test]
fn export_requires_a_classical_anchor() {
    let tmp = TempDir::new("pqonly");
    let roots = root_set(&tmp, "rootsPQ", vec![AlgorithmId::MlDsa44], 1);
    let out = tmp.path().join("tbPQ");
    let err = export_trust_bundle(
        &TrustExportRequest {
            root_bundles: vec![roots],
            version: 1,
            name: "Fleet Trust Bundle".into(),
            at: ts(AT),
        },
        &out,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("classical"),
        "unexpected error: {err}"
    );
}

#[test]
fn retire_drops_a_staged_version() {
    let tmp = TempDir::new("retire");
    let store = tmp.path().join("store");
    let roots = root_set(
        &tmp,
        "roots1",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        1,
    );
    let bundle = export(&tmp, "tb1", &[roots], 1);
    stage_bundle(&store, &bundle).unwrap();
    retire(&store, 1).unwrap();
    let status = store_status(&store).unwrap();
    assert_eq!(status.staged, None);
    assert!(status.installed_versions.is_empty());
}
