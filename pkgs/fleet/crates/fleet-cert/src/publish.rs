//! Publication planning and verification (spec §4 + §3.5,
//! program phase §13 F; tree §6.1).
//!
//! Governing-section note (slice-1 resolution for #13723 vs #9157):
//! `cli.rs` historically labelled `publish` as "§13 phase F"
//! machinery while the issue scopes to "§3.5 distribution points".
//! Both are true and neither contradicts the other:
//! - §4 (esp. §4.4) defines the publication manifest contract
//!   (`publish-plan.yaml`, schema `fleet.pki.publish/v1`) and the
//!   AIA/CDP/OCSP distribution rules;
//! - §3.5 defines what gets distributed (root trust bundles such as
//!   `public/bundles/roots-all.pem` plus per-algorithm roots and
//!   cross-certificates);
//! - §13 phase F is the program phase that schedules this work,
//!   not a competing spec section.
//! `plan` renders without touching live points, `verify` checks a
//! mirror read-only, and `apply_staging` writes public bytes only to
//! an explicit local staging directory. Live `status` remains deferred
//! per the #9155 gate map; this module has no network publication path.

use crate::bundle::load_bundle;
use crate::chain::Check;
use crate::error::Result;
use crate::model::{PublishPlanRequest, PublishVerifyRequest};
use crate::timeparse::Rfc3339;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Schema id for the §4.4 publication plan.
pub const PUBLISH_SCHEMA: &str = "fleet.pki.publish/v1";

/// Default distribution host proposal (§4.1: `pki.developing.today`
/// is a proposed convention, not an assertion it exists).
pub const DEFAULT_PUBLISH_HOST: &str = "pki.developing.today";

/// One publishable artifact: a public bundle file plus where it must
/// be served from and with what MIME type (§4.4 `artifacts`,
/// §24.7 MIME rules).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishArtifact {
    /// Bundle-relative source path (e.g. `public/bundles/roots-all.pem`).
    pub source: String,
    /// Target HTTP URL (immutable object URL, never a bare host).
    pub url: String,
    /// `application/pkix-cert` | `application/pkix-crl` |
    /// `application/pkcs7-mime`.
    pub media_type: String,
    pub visibility: String,
    pub sha256: String,
    pub bytes: u64,
}

/// One DNS change proposal (§4.4 `records`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsRecord {
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    pub ttl: u32,
    pub value: String,
    pub action: String,
}

/// The §4.4 publication plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPlan {
    pub schema: String,
    pub bundle: String,
    pub bundle_id: String,
    pub bundle_type: String,
    pub host: String,
    pub base_url: String,
    pub artifacts: Vec<PublishArtifact>,
    pub dns_records: Vec<DnsRecord>,
    pub prerequisites: Vec<String>,
    pub generated_at: Rfc3339,
}

/// Verify report: one check per planned artifact plus a manifest
/// checksum gate. Oracle: the bundle manifest's own SHA-256 index
/// (§7.1) — a mirror file must byte-match the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishVerifyReport {
    pub bundle: String,
    pub published_dir: String,
    pub valid: bool,
    pub checks: Vec<Check>,
}

/// Result of applying public artifacts to a local staging mirror.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishApplyReport {
    pub bundle: String,
    pub staging_dir: String,
    pub written: Vec<String>,
}

/// MIME type for a published object path (§24.7).
pub fn media_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".p7c") || lower.ends_with(".p7b") {
        "application/pkcs7-mime"
    } else if lower.ends_with(".crl") || lower.ends_with(".crl.pem") || lower.ends_with(".crl.der")
    {
        "application/pkix-crl"
    } else {
        "application/pkix-cert"
    }
}

/// URL path for a bundle-relative source: strip the leading
/// `public/` so `public/bundles/roots-all.pem` serves at
/// `http://<host>/bundles/roots-all.pem` (§4.4 example shape).
pub fn url_path_for(source: &str) -> &str {
    source.strip_prefix("public/").unwrap_or(source)
}

fn normalize_host(req_host: &str) -> String {
    let h = req_host.trim().trim_end_matches('/');
    if h.contains("://") {
        // Caller passed a full base URL as host; keep it verbatim.
        h.to_string()
    } else {
        h.to_string()
    }
}

/// Render the publication plan read-only: loads the bundle (which
/// verifies its §7.1 checksums) and enumerates public files. Never
/// touches the network or any live endpoint.
pub fn plan_bundle(req: &PublishPlanRequest) -> Result<PublishPlan> {
    let bundle = load_bundle(&req.bundle)?;
    let host = normalize_host(&req.host);
    let base_url = if host.contains("://") {
        host.clone()
    } else {
        format!("http://{host}")
    };
    let mut artifacts = Vec::new();
    for f in &bundle.manifest.files {
        if f.visibility != "public" {
            continue;
        }
        let url_path = url_path_for(&f.path);
        artifacts.push(PublishArtifact {
            source: f.path.clone(),
            url: format!("{}/{}", base_url.trim_end_matches('/'), url_path),
            media_type: media_type_for(&f.path).to_string(),
            visibility: f.visibility.clone(),
            sha256: f.sha256.clone(),
            bytes: f.bytes,
        });
    }
    artifacts.sort_by(|a, b| a.source.cmp(&b.source));
    let (dns_records, mut prerequisites) = match &req.dns_target {
        Some(ip) => (
            vec![DnsRecord {
                record_type: "A".to_string(),
                name: format!("{host}.").trim_start_matches("http://").trim_start_matches("https://").to_string(),
                ttl: 3600,
                value: ip.clone(),
                action: "conditional-create".to_string(),
            }],
            vec![
                "Serve each artifact at its exact URL with the listed content-type; no redirects for AIA/CDP paths.".to_string(),
            ],
        ),
        None => (
            Vec::new(),
            vec![
                "Replace placeholder DNS target; never publish a literal placeholder (§4.4).".to_string(),
                "Provide --dns-target <ip> to render an executable A record.".to_string(),
            ],
        ),
    };
    prerequisites.push(
        "Publishing is not equivalent to DNS changes; validate reachability and content-type after upload.".to_string(),
    );
    Ok(PublishPlan {
        schema: PUBLISH_SCHEMA.to_string(),
        bundle: req.bundle.display().to_string(),
        bundle_id: bundle.manifest.bundle_id.clone(),
        bundle_type: bundle.manifest.bundle_type.as_str().to_string(),
        host: host.clone(),
        base_url,
        artifacts,
        dns_records,
        prerequisites,
        generated_at: Rfc3339::now(),
    })
}

/// Apply only public bundle artifacts to an explicit local staging mirror.
/// This function has no network or live-endpoint path. Existing symlinks,
/// path traversal, and non-directory parents are refused before writing.
pub fn apply_staging(bundle_dir: &Path, staging_dir: &Path) -> Result<PublishApplyReport> {
    let bundle = load_bundle(bundle_dir)?;
    ensure_staging_root(staging_dir)?;
    let canonical_root = std::fs::canonicalize(staging_dir)?;
    let mut public_files = Vec::new();
    for file in &bundle.manifest.files {
        if file.visibility != "public" {
            continue;
        }
        let rel = url_path_for(&file.path);
        let rel_path = Path::new(rel);
        if rel_path.as_os_str().is_empty()
            || rel_path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(crate::error::CertError::policy(format!(
                "unsafe publication path '{}'",
                file.path
            )));
        }
        let source = bundle_dir.join(&file.path);
        let bytes = std::fs::read(&source)?;
        if bytes.len() as u64 != file.bytes || crate::bundle::sha256_hex(&bytes) != file.sha256 {
            return Err(crate::error::CertError::validation(format!(
                "bundle artifact checksum/size mismatch: {}",
                file.path
            )));
        }
        public_files.push((rel_path.to_path_buf(), bytes));
    }

    // Validate the complete destination tree before any artifact write.
    for (rel, _) in &public_files {
        ensure_safe_parents(&canonical_root, rel)?;
        let target = canonical_root.join(rel);
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                return Err(crate::error::CertError::policy(format!(
                    "unsafe staging destination: {}",
                    target.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let mut written = Vec::new();
    for (rel, bytes) in public_files {
        let target = canonical_root.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, bytes)?;
        written.push(rel.display().to_string());
    }
    written.sort();
    Ok(PublishApplyReport {
        bundle: bundle_dir.display().to_string(),
        staging_dir: canonical_root.display().to_string(),
        written,
    })
}

fn ensure_staging_root(root: &Path) -> Result<()> {
    if root.exists() {
        let meta = std::fs::symlink_metadata(root)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(crate::error::CertError::policy(format!(
                "staging directory must be a real directory: {}",
                root.display()
            )));
        }
    } else {
        std::fs::create_dir_all(root)?;
    }
    Ok(())
}

fn ensure_safe_parents(root: &Path, rel: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in rel.parent().into_iter().flat_map(Path::components) {
        match component {
            std::path::Component::Normal(name) => current.push(name),
            _ => {
                return Err(crate::error::CertError::policy(
                    "unsafe staging path component",
                ));
            }
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(crate::error::CertError::policy(format!(
                    "unsafe staging parent: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Verify a published mirror read-only: every public bundle file
/// must exist under `published_dir` at its URL path with identical
/// SHA-256. Never writes, never fetches over the network.
pub fn verify_bundle(req: &PublishVerifyRequest) -> Result<PublishVerifyReport> {
    let bundle = load_bundle(&req.bundle)?;
    let plan_req = PublishPlanRequest {
        bundle: req.bundle.clone(),
        host: req
            .host
            .clone()
            .unwrap_or_else(|| DEFAULT_PUBLISH_HOST.to_string()),
        dns_target: None,
    };
    let plan = plan_bundle(&plan_req)?;
    let mut checks = vec![Check {
        name: "manifest-checksums".into(),
        ok: true,
        detail: format!(
            "{} files verified in source bundle",
            bundle.manifest.files.len()
        ),
    }];
    for art in &plan.artifacts {
        let rel = url_path_for(&art.source);
        let path = req.published_dir.join(rel);
        let name = format!("published:{}", art.source);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let digest = crate::bundle::sha256_hex(&bytes);
                if digest == art.sha256 && bytes.len() as u64 == art.bytes {
                    checks.push(Check {
                        name,
                        ok: true,
                        detail: format!("{} matches ({} bytes)", art.url, art.bytes),
                    });
                } else {
                    checks.push(Check {
                        name,
                        ok: false,
                        detail: format!(
                            "checksum/size mismatch for {}: expected sha256 {} / {} bytes, got {} / {} bytes",
                            art.source,
                            art.sha256,
                            art.bytes,
                            digest,
                            bytes.len()
                        ),
                    });
                }
            }
            Err(e) => checks.push(Check {
                name,
                ok: false,
                detail: format!(
                    "missing published file {} ({}): {e}",
                    path.display(),
                    art.url
                ),
            }),
        }
    }
    let valid = checks.iter().all(|c| c.ok);
    Ok(PublishVerifyReport {
        bundle: req.bundle.display().to_string(),
        published_dir: req.published_dir.display().to_string(),
        valid,
        checks,
    })
}

/// Minimal `publish-plan.yaml` renderer (§4.4 shape). Hand-rolled so
/// this slice adds no YAML dependency; output is informational, the
/// JSON form is canonical.
pub fn render_plan_yaml(plan: &PublishPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema: {}\n", plan.schema));
    out.push_str(&format!("host: {}\n", plan.host));
    out.push_str("records:\n");
    if plan.dns_records.is_empty() {
        out.push_str("  []\n");
    } else {
        for r in &plan.dns_records {
            out.push_str(&format!(
                "  - type: {}\n    name: {}\n    ttl: {}\n    value: \"{}\"\n    action: {}\n",
                r.record_type, r.name, r.ttl, r.value, r.action
            ));
        }
    }
    out.push_str("artifacts:\n");
    for a in &plan.artifacts {
        out.push_str(&format!(
            "  - source: {}\n    url: {}\n    media_type: {}\n    visibility: {}\n    sha256: {}\n    bytes: {}\n",
            a.source, a.url, a.media_type, a.visibility, a.sha256, a.bytes
        ));
    }
    out.push_str("prerequisites:\n");
    for p in &plan.prerequisites {
        out.push_str(&format!("  - \"{}\"\n", p.replace('"', "'")));
    }
    out
}

/// Human-readable plan text.
pub fn render_plan_text(plan: &PublishPlan) -> String {
    let mut out = format!(
        "Publish plan: {} ({}, {})\n  host: {} (base {})\n  artifacts: {}\n",
        plan.bundle,
        plan.bundle_id,
        plan.bundle_type,
        plan.host,
        plan.base_url,
        plan.artifacts.len()
    );
    for a in &plan.artifacts {
        out.push_str(&format!(
            "  {} -> {}\n    {} {} bytes sha256:{}..\n",
            a.source,
            a.url,
            a.media_type,
            a.bytes,
            &a.sha256[..12.min(a.sha256.len())]
        ));
    }
    if plan.dns_records.is_empty() {
        out.push_str("  dns: (no changes — supply --dns-target for an executable record)\n");
    } else {
        for r in &plan.dns_records {
            out.push_str(&format!(
                "  dns: {} {} {} ttl={} ({})\n",
                r.record_type, r.name, r.value, r.ttl, r.action
            ));
        }
    }
    for p in &plan.prerequisites {
        out.push_str(&format!("  prerequisite: {p}\n"));
    }
    out.push_str("  note: read-only plan; nothing was uploaded (apply is a later gated slice).\n");
    out
}

pub fn published_path(published_dir: &Path, source: &str) -> std::path::PathBuf {
    published_dir.join(url_path_for(source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_mime_rules() {
        // Oracle: §4.4 example shape + §24.7 MIME table.
        assert_eq!(
            url_path_for("public/bundles/roots-all.pem"),
            "bundles/roots-all.pem"
        );
        assert_eq!(url_path_for("leaf.crt.pem"), "leaf.crt.pem");
        assert_eq!(media_type_for("ca.crt.pem"), "application/pkix-cert");
        assert_eq!(media_type_for("issuers.p7c"), "application/pkcs7-mime");
        assert_eq!(media_type_for("root.crl.pem"), "application/pkix-crl");
    }

    #[test]
    fn yaml_renderer_keeps_schema_shape() {
        // Oracle: §4.4 field names (schema/host/records/artifacts/prerequisites).
        let plan = PublishPlan {
            schema: PUBLISH_SCHEMA.into(),
            bundle: "b".into(),
            bundle_id: "id".into(),
            bundle_type: "root-set".into(),
            host: "pki.example.test".into(),
            base_url: "http://pki.example.test".into(),
            artifacts: vec![PublishArtifact {
                source: "public/bundles/roots-all.pem".into(),
                url: "http://pki.example.test/bundles/roots-all.pem".into(),
                media_type: "application/pkix-cert".into(),
                visibility: "public".into(),
                sha256: "abc".into(),
                bytes: 3,
            }],
            dns_records: vec![],
            prerequisites: vec!["x".into()],
            generated_at: Rfc3339::now(),
        };
        let y = render_plan_yaml(&plan);
        assert!(y.contains("schema: fleet.pki.publish/v1"));
        assert!(y.contains("source: public/bundles/roots-all.pem"));
        assert!(y.contains("url: http://pki.example.test/bundles/roots-all.pem"));
    }
}
