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
fn apply_stages_exact_public_bytes_idempotently_and_verifies() {
    // Oracle: only manifest-public bytes land at URL-derived paths; repeated apply is stable.
    let tmp = TempDir::new("apply");
    let bundle = root_bundle(&tmp);
    let staging = tmp.path().join("stage");
    let report = fleet_cert::publish::apply_staging(&bundle, &staging).unwrap();
    let first_bytes: Vec<_> = report
        .written
        .iter()
        .map(|rel| (rel.clone(), std::fs::read(staging.join(rel)).unwrap()))
        .collect();
    let manifest = fleet_cert::bundle::load_bundle(&bundle).unwrap();
    let expected: Vec<_> = manifest
        .manifest
        .files
        .iter()
        .filter(|f| f.visibility == "public")
        .map(|f| {
            f.path
                .strip_prefix("public/")
                .unwrap_or(&f.path)
                .to_string()
        })
        .collect();
    let mut expected = expected;
    expected.sort();
    assert_eq!(report.written, expected);
    for file in manifest
        .manifest
        .files
        .iter()
        .filter(|f| f.visibility == "public")
    {
        let rel = file.path.strip_prefix("public/").unwrap_or(&file.path);
        assert_eq!(
            std::fs::read(staging.join(rel)).unwrap(),
            std::fs::read(bundle.join(&file.path)).unwrap()
        );
    }
    assert!(
        manifest
            .manifest
            .files
            .iter()
            .filter(|f| f.visibility != "public")
            .all(|f| !staging
                .join(f.path.strip_prefix("public/").unwrap_or(&f.path))
                .exists())
    );
    let again = fleet_cert::publish::apply_staging(&bundle, &staging).unwrap();
    let second_bytes: Vec<_> = again
        .written
        .iter()
        .map(|rel| (rel.clone(), std::fs::read(staging.join(rel)).unwrap()))
        .collect();
    assert_eq!(first_bytes, second_bytes);
    let verified = fleet_cert::publish::verify_bundle(&PublishVerifyRequest {
        bundle,
        published_dir: staging,
        host: None,
    })
    .unwrap();
    assert!(
        verified.valid,
        "staged mirror must verify: {:?}",
        verified.checks
    );
}

#[test]
fn apply_refuses_manifest_path_traversal_before_writing() {
    // Oracle: even a checksum-valid bundle path cannot escape the staging root.
    let tmp = TempDir::new("traversal");
    let bundle = root_bundle(&tmp);
    let manifest_path = bundle.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let entry = manifest["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|f| f["visibility"] == "public")
        .unwrap();
    let bytes = std::fs::read(bundle.join(entry["path"].as_str().unwrap())).unwrap();
    let escaped_rel = "public/../escaped-public.pem";
    std::fs::write(bundle.join("escaped-public.pem"), &bytes).unwrap();
    entry["path"] = escaped_rel.into();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let staging = tmp.path().join("traversal-stage");
    std::fs::create_dir_all(&staging).unwrap();
    let marker = staging.join("keep.txt");
    std::fs::write(&marker, b"unchanged").unwrap();
    assert!(fleet_cert::publish::apply_staging(&bundle, &staging).is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), b"unchanged");
    assert!(!tmp.path().join("escaped-public.pem").exists());
}

#[cfg(unix)]
#[test]
fn apply_refuses_symlink_escape_before_writing_any_artifacts() {
    // Oracle: refusal leaves staging unchanged and writes no artifact outside it.
    let tmp = TempDir::new("symlink");
    let bundle = root_bundle(&tmp);
    let staging = tmp.path().join("stage");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let manifest = fleet_cert::bundle::load_bundle(&bundle).unwrap();
    let first = manifest
        .manifest
        .files
        .iter()
        .find(|f| f.visibility == "public")
        .unwrap();
    let rel = Path::new(first.path.strip_prefix("public/").unwrap_or(&first.path));
    if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(staging.join(parent)).unwrap();
    }
    let parent = staging.join(rel.parent().unwrap_or(Path::new(".")));
    let file = rel.file_name().unwrap();
    let target = parent.join(file);
    std::os::unix::fs::symlink(outside.join("escaped"), &target).unwrap();
    let before: Vec<_> = std::fs::read_dir(&staging)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(fleet_cert::publish::apply_staging(&bundle, &staging).is_err());
    assert_eq!(
        std::fs::read_dir(&staging)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>(),
        before
    );
    assert!(!outside.join("escaped").exists());
}

#[test]
fn cli_refuses_live_endpoint_target_before_filesystem_effects() {
    // Oracle: live target options are rejected as usage before touching the requested path.
    let tmp = TempDir::new("live-refused");
    let bundle = root_bundle(&tmp);
    let target = tmp.path().join("must-not-be-created");
    let args = vec![
        "publish".to_string(),
        "apply".to_string(),
        "--bundle".to_string(),
        bundle.display().to_string(),
        "--staging-dir".to_string(),
        target.display().to_string(),
        "https://production.example.test".to_string(),
    ];
    assert_eq!(fleet_cert::run(&args), 2);
    assert!(!target.exists());
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
