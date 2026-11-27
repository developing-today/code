//! Publish plan + verify (slice 1 of #13723) + local staging
//! apply (slice 2 of #13723).
//!
//! Oracles:
//! - §4.4 field shape (schema/host/records/artifacts/prerequisites);
//! - §7.1 manifest checksum index (verify must catch tamper/missing);
//! - §3.5 distribution set (plan covers exactly the bundle's public files);
//! - slice 2: apply writes exactly the public bytes to URL-derived
//!   mirror paths, is idempotent, never stages private files, and
//!   refuses traversal/symlink escape plus any live endpoint target
//!   before filesystem or network effects.

use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{PublishPlanRequest, PublishVerifyRequest, RootCreateRequest};
use fleet_cert::publish::{apply_bundle, staging_destination, PublishApplyRequest};
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

#[test]
fn apply_stages_exact_public_bytes_idempotently_and_verifies() {
    // Oracle: every planned public artifact matches byte-for-byte after
    // repeated apply; private manifest entries are absent; verify passes.
    let tmp = TempDir::new("apply");
    let bundle = root_bundle(&tmp);
    let staging = tmp.path().join("mirror");
    let first = apply_bundle(&PublishApplyRequest {
        bundle: bundle.clone(),
        staging_dir: staging.clone(),
        host: None,
    })
    .unwrap();
    assert!(first.valid);
    let second = apply_bundle(&PublishApplyRequest {
        bundle: bundle.clone(),
        staging_dir: staging.clone(),
        host: None,
    })
    .unwrap();
    assert!(second.valid);
    let loaded = fleet_cert::bundle::load_bundle(&bundle).unwrap();
    for f in &loaded.manifest.files {
        let rel = f.path.strip_prefix("public/").unwrap_or(&f.path);
        let target = staging.join(rel);
        if f.visibility == "public" {
            assert_eq!(std::fs::read(target).unwrap(), std::fs::read(bundle.join(&f.path)).unwrap());
        } else {
            assert!(!target.exists(), "private artifact was staged: {}", f.path);
        }
    }
    let verified = fleet_cert::publish::verify_bundle(&PublishVerifyRequest {
        bundle,
        published_dir: staging,
        host: None,
    }).unwrap();
    assert!(verified.valid, "staged mirror must verify: {:?}", verified.checks);
}

#[test]
fn apply_rejects_traversal_and_symlink_escape_before_writing() {
    // Oracle: hostile artifact paths and symlinked parents leave the
    // staging tree untouched and never write outside its root.
    let tmp = TempDir::new("apply-escape");
    let bundle = root_bundle(&tmp);
    let loaded = fleet_cert::bundle::load_bundle(&bundle).unwrap();
    let source = loaded.manifest.files.iter().find(|f| f.visibility == "public").unwrap();
    let escaped = staging_destination(Path::new("/safe/staging"), "public/../outside");
    assert!(escaped.is_err());
    let staging = tmp.path().join("mirror");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&staging).unwrap();
    // Symlink the exact first path component of a real public artifact,
    // so the escape route is guaranteed to sit on a write path.
    let first_rel = source
        .path
        .strip_prefix("public/")
        .unwrap_or(&source.path)
        .to_string();
    let first_component: &str = first_rel.split('/').next().unwrap();
    assert!(!first_component.is_empty() && !first_component.contains(".."));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, staging.join(first_component)).unwrap();
    #[cfg(not(unix))]
    std::fs::create_dir_all(staging.join(first_component)).unwrap();
    let result = apply_bundle(&PublishApplyRequest {
        bundle,
        staging_dir: staging,
        host: None,
    });
    assert!(result.is_err());
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    assert!(!outside.join(Path::new(&source.path).file_name().unwrap()).exists());
}

fn cli_args(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_apply_stages_mirror_and_verify_reports_in_sync() {
    // Oracle: end-to-end CLI path — apply then verify agree.
    let tmp = TempDir::new("apply-cli");
    let bundle = root_bundle(&tmp);
    let staging = tmp.path().join("staging");
    let code = fleet_cert::cli::run(&cli_args(&[
        "publish",
        "apply",
        "--bundle",
        bundle.to_str().unwrap(),
        "--staging-dir",
        staging.to_str().unwrap(),
    ]));
    assert_eq!(code, 0);
    let verify = fleet_cert::cli::run(&cli_args(&[
        "publish",
        "verify",
        "--bundle",
        bundle.to_str().unwrap(),
        "--published-dir",
        staging.to_str().unwrap(),
    ]));
    assert_eq!(verify, 0);
}

#[test]
fn cli_apply_refuses_live_target_before_filesystem_effects() {
    // Oracle: the #9155 production gate — any live endpoint target is
    // refused with no staging directory created and nothing written.
    let tmp = TempDir::new("apply-cli-gate");
    let bundle = root_bundle(&tmp);
    let staging = tmp.path().join("staging");
    for flag in ["--endpoint", "--live-target", "--production"] {
        let code = fleet_cert::cli::run(&cli_args(&[
            "publish",
            "apply",
            "--bundle",
            bundle.to_str().unwrap(),
            "--staging-dir",
            staging.to_str().unwrap(),
            flag,
            "https://pki.live.example/",
        ]));
        assert_ne!(code, 0, "{flag} must be refused");
    }
    assert!(
        !staging.exists(),
        "refused apply must not create the staging directory"
    );
}

#[test]
fn cli_apply_requires_explicit_staging_dir() {
    // Oracle: no implicit/default destination — the operator names it.
    let tmp = TempDir::new("apply-cli-usage");
    let bundle = root_bundle(&tmp);
    let code = fleet_cert::cli::run(&cli_args(&[
        "publish", "apply", "--bundle", bundle.to_str().unwrap(),
    ]));
    assert_ne!(code, 0);
}

#[test]
fn cli_publish_status_stays_deferred() {
    // Oracle: live state stays behind the production gate.
    let code = fleet_cert::cli::run(&cli_args(&["publish", "status"]));
    assert_ne!(code, 0);
}
