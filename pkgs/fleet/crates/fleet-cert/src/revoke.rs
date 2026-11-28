//! `fleet cert revoke leaf|issuer|root` (§6.1): record a revocation
//! in the target bundle's manifest `revocations/` tree, and let the
//! chain validator report it (§§8/14).
//!
//! A bundle is append-only in spirit: the revocation is a new
//! artifact under `revocations/`, indexed (SHA-256) like every other
//! file, and repeated in the manifest as a typed
//! [`RevocationRecord`]. The bundle's certificates are untouched —
//! the record is what validation consults.

use crate::bundle::{Artifact, Bundle, append_artifacts, load_bundle};
use crate::error::{CertError, Result};
use crate::model::{
    BundleType, REVOCATION_REASONS, REVOCATION_SCHEMA, RevocationRecord, RevokeTarget,
};
use crate::timeparse::Rfc3339;

/// A revocation request (§6.1).
#[derive(Debug, Clone)]
pub struct RevokeRequest {
    /// The bundle directory holding the certificates to revoke.
    pub bundle: std::path::PathBuf,
    pub target: RevokeTarget,
    /// RFC 5280 reason code name (see [`REVOCATION_REASONS`]).
    pub reason: String,
    /// When the revocation takes effect (validation at or after it
    /// reports the certificate revoked).
    pub revoked_at: Rfc3339,
}

/// Which certificate roles a target covers.
fn roles_for(target: RevokeTarget) -> &'static [&'static str] {
    match target {
        RevokeTarget::Leaf => &["leaf"],
        RevokeTarget::Issuer => &["issuer"],
        RevokeTarget::Root => &["root", "root-cross"],
    }
}

fn target_matches_type(target: RevokeTarget, ty: BundleType) -> bool {
    matches!(
        (target, ty),
        (RevokeTarget::Leaf, BundleType::Leaf)
            | (RevokeTarget::Issuer, BundleType::IssuerSet)
            | (RevokeTarget::Root, BundleType::RootSet)
    )
}

/// Revoke the certificates a target covers in one bundle. Idempotent
/// by fingerprint: already-revoked certificates are skipped and the
/// existing records returned, so re-running never duplicates a
/// record or fails.
pub fn revoke_bundle(req: &RevokeRequest) -> Result<Vec<RevocationRecord>> {
    if !REVOCATION_REASONS.contains(&req.reason.as_str()) {
        return Err(CertError::usage(format!(
            "unknown revocation reason '{}' ({})",
            req.reason,
            REVOCATION_REASONS.join(" | ")
        )));
    }
    let bundle = load_bundle(&req.bundle)?;
    if !target_matches_type(req.target, bundle.manifest.bundle_type) {
        return Err(CertError::policy(format!(
            "cannot revoke a {} in a {} bundle: {} revoke takes a {} bundle",
            req.target.as_str(),
            bundle.manifest.bundle_type.as_str(),
            req.target.as_str(),
            match req.target {
                RevokeTarget::Leaf => "leaf",
                RevokeTarget::Issuer => "issuer-set",
                RevokeTarget::Root => "root-set",
            }
        )));
    }
    let roles = roles_for(req.target);
    let targets: Vec<_> = bundle
        .manifest
        .certificates
        .iter()
        .filter(|c| roles.contains(&c.role.as_str()))
        .cloned()
        .collect();
    if targets.is_empty() {
        return Err(CertError::manifest(format!(
            "{} holds no {} certificate to revoke",
            req.bundle.display(),
            req.target.as_str()
        )));
    }

    let mut manifest = bundle.manifest.clone();
    let mut records: Vec<RevocationRecord> = Vec::new();
    let mut artifacts: Vec<Artifact> = Vec::new();
    for cert in &targets {
        if let Some(existing) = manifest
            .revocations
            .iter()
            .find(|r| r.fingerprint_sha256 == cert.fingerprint_sha256)
        {
            records.push(existing.clone());
            continue;
        }
        let rel_path = format!("revocations/{}.json", cert.fingerprint_sha256);
        let record = RevocationRecord {
            schema: REVOCATION_SCHEMA.to_string(),
            target: req.target.as_str().to_string(),
            reason: req.reason.clone(),
            revoked_at: req.revoked_at,
            cert_role: cert.role.clone(),
            subject_dn: cert.subject_dn.clone(),
            serial_hex: cert.serial_hex.clone(),
            fingerprint_sha256: cert.fingerprint_sha256.clone(),
            algorithm: cert.algorithm,
            partition: cert.partition.clone(),
            path: rel_path.clone(),
        };
        let body = serde_json::to_string_pretty(&record)?;
        artifacts.push(Artifact {
            rel_path,
            kind: "revocation-record".into(),
            visibility: "public".into(),
            bytes: (body + "\n").into_bytes(),
        });
        records.push(record.clone());
        manifest.revocations.push(record);
    }
    manifest
        .revocations
        .sort_by(|a, b| a.fingerprint_sha256.cmp(&b.fingerprint_sha256));
    append_artifacts(&req.bundle, manifest, &artifacts)?;
    Ok(records)
}

/// Every revocation an evidence set carries, with the cert-role it
/// applies to. `pool` bundles are the issuer/root sets a leaf is
/// validated against; the leaf bundle itself is included so a revoked
/// leaf is reported too.
pub fn collect_revocations(bundles: &[&Bundle]) -> Vec<RevocationRecord> {
    let mut out = Vec::new();
    for bundle in bundles {
        for record in &bundle.manifest.revocations {
            out.push(record.clone());
        }
    }
    out
}

/// The `revoked` check for one path. Returns `None` when nothing in
/// the path carries a revocation at `at`; otherwise the failing
/// check naming the first revoked certificate.
pub(crate) fn revoked_check(
    path_fingerprints: &[String],
    revocations: &[RevocationRecord],
    at: Rfc3339,
) -> Option<crate::chain::Check> {
    let revoked = path_fingerprints
        .iter()
        .filter_map(|fp| {
            revocations
                .iter()
                .find(|r| &r.fingerprint_sha256 == fp && r.revoked_at <= at)
        })
        .collect::<Vec<_>>();
    if revoked.is_empty() {
        return None;
    }
    Some(crate::chain::Check {
        name: "revoked".into(),
        ok: false,
        detail: revoked
            .iter()
            .map(|r| {
                format!(
                    "{} revoked as {} ({}) at {}",
                    r.subject_dn, r.cert_role, r.reason, r.revoked_at
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    })
}
