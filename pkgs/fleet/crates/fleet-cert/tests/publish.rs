//! Publish plan + verify (slice 1 of #13723).
//!
//! Oracles:
//! - §4.4 field shape (schema/host/records/artifacts/prerequisites);
//! - §7.1 manifest checksum index (verify must catch tamper/missing);
//! - §3.5 distribution set (plan covers exactly the bundle's public files).

use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{PublishPlanRequest, PublishVerifyRequest, RootCreateRequest};
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
            "fleet-cert-publish-{tag}-{}-{nanos}",
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

fn root_bundle(tmp: &TempDir) -> PathBuf {
    let dir = tmp.path().join("roots");
    let req = RootCreateRequest {
        organization: "Test Org LLC".into(),
        country: "US".into(),
        algorithms: vec![fleet_cert::algorithm::AlgorithmId::Ed25519],
        generation: 1,
        partition: "A".into(),
        cross_signing: CrossSigningMode::Both,
        not_before: ts("2026-01-01T00:00:00Z"),
        not_after: ts("2046-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        private_store: None,
    };
    engine::create_root_set(&req, &dir).unwrap();
    dir
}

fn copy_public_to_mirror(bundle: &Path, mirror: &Path) {
    // Mirror layout: <mirror>/<url-path> where url-path strips `public/`.
    let b = fleet_cert::bundle::load_bundle(bundle).unwrap();
    for f in &b.manifest.files {
        if f.visibility != "public" {
            continue;
        }
        let rel = f.path.strip_prefix("public/").unwrap_or(&f.path);
        let src = bundle.join(&f.path);
        let dst = mirror.join(rel);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(&src, &dst).unwrap();
    }
}

#[test]
fn plan_covers_public_files_with_stable_urls() {
    // Oracle: §4.4 shape + §3.5 set (exactly the public files).
    let tmp = TempDir::new("plan");
    let bundle = root_bundle(&tmp);
    let plan = fleet_cert::publish::plan_bundle(&PublishPlanRequest {
        bundle: bundle.clone(),
        host: "pki.example.test".into(),
        dns_target: None,
    })
    .unwrap();
    assert_eq!(plan.schema, fleet_cert::publish::PUBLISH_SCHEMA);
    assert_eq!(plan.base_url, "http://pki.example.test");
    let manifest = fleet_cert::bundle::load_bundle(&bundle).unwrap();
    let public = manifest
        .manifest
        .files
        .iter()
        .filter(|f| f.visibility == "public")
        .count();
    assert_eq!(plan.artifacts.len(), public);
    for a in &plan.artifacts {
        assert!(a.url.starts_with("http://pki.example.test/"));
        assert!(!a.url.ends_with('/'));
        assert_eq!(a.media_type, "application/pkix-cert");
    }
    // No DNS target -> no executable records, prerequisite recorded.
    assert!(plan.dns_records.is_empty());
    assert!(plan.prerequisites.iter().any(|p| p.contains("placeholder")));
}

#[test]
fn verify_accepts_mirror_and_reports_mismatch() {
    // Oracle: §7.1 checksum index — tamper/missing must fail.
    let tmp = TempDir::new("verify");
    let bundle = root_bundle(&tmp);
    let mirror = tmp.path().join("mirror");
    copy_public_to_mirror(&bundle, &mirror);
    let ok = fleet_cert::publish::verify_bundle(&PublishVerifyRequest {
        bundle: bundle.clone(),
        published_dir: mirror.clone(),
        host: None,
    })
    .unwrap();
    assert!(ok.valid, "clean mirror must verify: {:?}", ok.checks);

    // Tamper with one mirrored file.
    let first = fleet_cert::bundle::load_bundle(&bundle)
        .unwrap()
        .manifest
        .files
        .iter()
        .find(|f| f.visibility == "public")
        .unwrap()
        .path
        .clone();
    let rel = first.strip_prefix("public/").unwrap_or(&first);
    let p = mirror.join(rel);
    let mut bytes = std::fs::read(&p).unwrap();
    bytes[10] ^= 0xff;
    std::fs::write(&p, bytes).unwrap();
    let bad = fleet_cert::publish::verify_bundle(&PublishVerifyRequest {
        bundle: bundle.clone(),
        published_dir: mirror.clone(),
        host: None,
    })
    .unwrap();
    assert!(!bad.valid);
    assert!(bad.checks.iter().any(|c| !c.ok && c.name.contains(&first)));

    // Missing file.
    std::fs::remove_file(&p).unwrap();
    let missing = fleet_cert::publish::verify_bundle(&PublishVerifyRequest {
        bundle,
        published_dir: mirror,
        host: None,
    })
    .unwrap();
    assert!(!missing.valid);
}
