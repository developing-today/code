//! `fleet cert inventory list|show|search|audit` (§6.1): read-only
//! cross-bundle inventory.
//!
//! The verbs scan bundle directories, reconcile certificates, keys
//! and expiry against the manifests, and report drift, upcoming
//! expiries and orphaned material. **Nothing here writes**: bundle
//! loading is the read path ([`crate::bundle::load_bundle`]), and a
//! bundle that fails verification is still inventoried from its
//! (parseable) manifest so damage is reported, not hidden.
//!
//! Expiry horizons (§6.1 audit): CA certificates (roles `root`,
//! `root-cross`, `issuer`) warn at the CA horizon (default 5y —
//! renewal planning for CA material starts early); everything else
//! warns at the leaf horizon (default 90d). Both are overridable per
//! run; horizon arithmetic is Gregorian via
//! [`CalendarDuration::add_to`] (spec §5.4), never a second count.

use crate::bundle::{list_bundles, load_bundle, pem_decode_all, sha256_hex};
use crate::chain::parse_cert;
use crate::model::BundleManifest;
use crate::timeparse::{CalendarDuration, Rfc3339};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Default upcoming-expiry horizon for CA certificates (spec §6.1:
/// issuer 5y).
pub const DEFAULT_CA_WITHIN: CalendarDuration = CalendarDuration::years(5);
/// Default upcoming-expiry horizon for leaf certificates (spec §6.1:
/// leaf 90d).
pub const DEFAULT_LEAF_WITHIN: CalendarDuration = CalendarDuration::days(90);

/// Roles that count as CA material for horizon purposes.
fn is_ca_role(role: &str) -> bool {
    matches!(role, "root" | "root-cross" | "issuer")
}

/// A read-only inventory request (§6.1).
#[derive(Debug, Clone)]
pub struct InventoryRequest {
    /// Directory to scan (the directory itself plus its immediate
    /// subdirectories, per [`list_bundles`]).
    pub dir: PathBuf,
    /// The instant expiry is evaluated at.
    pub at: Rfc3339,
    /// Upcoming-expiry horizon for CA certificates.
    pub ca_within: CalendarDuration,
    /// Upcoming-expiry horizon for leaf (non-CA) certificates.
    pub leaf_within: CalendarDuration,
}

impl InventoryRequest {
    /// Scan `dir` as of now with the spec §6.1 default horizons
    /// (CA 5y, leaf 90d).
    pub fn scan(dir: &Path) -> Self {
        InventoryRequest {
            dir: dir.to_path_buf(),
            at: Rfc3339::now(),
            ca_within: DEFAULT_CA_WITHIN,
            leaf_within: DEFAULT_LEAF_WITHIN,
        }
    }
}

/// One reconciliation finding for a bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftFinding {
    /// `bundle-unverifiable` | `checksum-mismatch` | `size-mismatch` |
    /// `missing-file` | `orphaned-file` | `cert-unparseable` |
    /// `fingerprint-mismatch` | `cert-path-unindexed` |
    /// `unreferenced-cert-file` | `unreferenced-key-file`.
    pub kind: String,
    /// Bundle-relative path (`.` for bundle-level findings).
    pub path: String,
    pub detail: String,
}

/// Expiry status of one certificate at the evaluation instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpiryStatus {
    NotYetValid,
    Expired,
    Expiring,
    Ok,
}

impl ExpiryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExpiryStatus::NotYetValid => "not-yet-valid",
            ExpiryStatus::Expired => "expired",
            ExpiryStatus::Expiring => "expiring",
            ExpiryStatus::Ok => "ok",
        }
    }
}

/// One certificate's inventory status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertStatus {
    pub role: String,
    pub subject_dn: String,
    pub issuer_dn: String,
    pub algorithm: String,
    pub partition: String,
    pub fingerprint_sha256: String,
    pub serial_hex: String,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    /// Whole days from the evaluation instant to `not_after`
    /// (negative when expired).
    pub days_remaining: i64,
    pub status: ExpiryStatus,
    /// Bundle-relative PEM path from the manifest record.
    pub cert_path: String,
}

/// One bundle's inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleInventory {
    pub dir: PathBuf,
    pub bundle_id: String,
    pub name: String,
    pub bundle_type: String,
    /// Whether [`load_bundle`] verified the bundle (schema, presence,
    /// every indexed SHA-256). A bundle that fails verification is
    /// still inventoried from its parseable manifest; its damage is
    /// itemized in `drift`.
    pub verified: bool,
    pub certs: Vec<CertStatus>,
    pub drift: Vec<DriftFinding>,
}

/// Whole-scan totals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InventoryTotals {
    pub bundles: usize,
    pub bundles_verified: usize,
    pub certificates: usize,
    pub expired: usize,
    pub expiring: usize,
    pub drift_findings: usize,
}

/// The full audit result (§6.1 `inventory audit`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryReport {
    pub dir: PathBuf,
    pub at: Rfc3339,
    pub ca_within: CalendarDuration,
    pub leaf_within: CalendarDuration,
    pub bundles: Vec<BundleInventory>,
    pub totals: InventoryTotals,
}

/// One search hit (§6.1 `inventory search`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub bundle_dir: PathBuf,
    pub bundle_name: String,
    pub bundle_type: String,
    pub cert: CertStatus,
}

/// Search filters (§6.1 `inventory search`). `None` (or `false`)
/// means no constraint on that field; `expired` selects only expired
/// certificates when true.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub subject: Option<String>,
    pub fingerprint: Option<String>,
    pub role: Option<String>,
    pub algorithm: Option<String>,
    pub expired: bool,
}

fn expiry_status(
    not_before: Rfc3339,
    not_after: Rfc3339,
    horizon: Rfc3339,
    at: Rfc3339,
) -> (ExpiryStatus, i64) {
    let days_remaining = (not_after.0 - at.0).whole_days();
    let status = if at < not_before {
        ExpiryStatus::NotYetValid
    } else if at > not_after {
        ExpiryStatus::Expired
    } else if not_after <= horizon {
        ExpiryStatus::Expiring
    } else {
        ExpiryStatus::Ok
    };
    (status, days_remaining)
}

fn cert_status(
    record: &crate::model::CertRecord,
    at: Rfc3339,
    ca_horizon: Rfc3339,
    leaf_horizon: Rfc3339,
) -> CertStatus {
    let horizon = if is_ca_role(&record.role) {
        ca_horizon
    } else {
        leaf_horizon
    };
    let (status, days_remaining) = expiry_status(record.not_before, record.not_after, horizon, at);
    CertStatus {
        role: record.role.clone(),
        subject_dn: record.subject_dn.clone(),
        issuer_dn: record.issuer_dn.clone(),
        algorithm: record.algorithm.name().to_string(),
        partition: record.partition.clone(),
        fingerprint_sha256: record.fingerprint_sha256.clone(),
        serial_hex: record.serial_hex.clone(),
        not_before: record.not_before,
        not_after: record.not_after,
        days_remaining,
        status,
        cert_path: record.cert_path.clone(),
    }
}

/// Recursively list a bundle's on-disk files as paths relative to
/// the bundle directory, sorted. `manifest.json` is excluded: it is
/// the index, not indexed material.
fn disk_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(top) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&top)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if let Ok(rel) = path.strip_prefix(dir) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if rel != "manifest.json" {
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

fn audit_one(
    dir: &Path,
    manifest: &BundleManifest,
    at: Rfc3339,
    ca_horizon: Rfc3339,
    leaf_horizon: Rfc3339,
) -> BundleInventory {
    let mut drift = Vec::new();
    let verified = match load_bundle(dir) {
        Ok(_) => true,
        Err(e) => {
            drift.push(DriftFinding {
                kind: "bundle-unverifiable".to_string(),
                path: ".".to_string(),
                detail: e.to_string(),
            });
            // Itemize the damage per file so the report names every
            // broken artifact instead of one aggregate error.
            let indexed: BTreeMap<&str, _> = manifest
                .files
                .iter()
                .map(|f| (f.path.as_str(), f))
                .collect();
            for file in &manifest.files {
                let path = dir.join(&file.path);
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        if sha256_hex(&bytes) != file.sha256 {
                            drift.push(DriftFinding {
                                kind: "checksum-mismatch".to_string(),
                                path: file.path.clone(),
                                detail: format!(
                                    "on-disk SHA-256 {} != indexed {}",
                                    sha256_hex(&bytes),
                                    file.sha256
                                ),
                            });
                        } else if bytes.len() as u64 != file.bytes {
                            drift.push(DriftFinding {
                                kind: "size-mismatch".to_string(),
                                path: file.path.clone(),
                                detail: format!(
                                    "on-disk {} bytes != indexed {} bytes",
                                    bytes.len(),
                                    file.bytes
                                ),
                            });
                        }
                    }
                    Err(e) => drift.push(DriftFinding {
                        kind: "missing-file".to_string(),
                        path: file.path.clone(),
                        detail: format!("indexed but unreadable: {e}"),
                    }),
                }
            }
            for rel in disk_files(dir) {
                if !indexed.contains_key(rel.as_str()) {
                    drift.push(DriftFinding {
                        kind: "orphaned-file".to_string(),
                        path: rel.clone(),
                        detail: "on disk but absent from the manifest file index (§7.1)"
                            .to_string(),
                    });
                }
            }
            false
        }
    };
    // Orphaned material (§7.1) is reported whether or not the bundle
    // verified: extra on-disk files do not break `load_bundle`'s
    // checksum check, but they are still unindexed material.
    {
        let indexed: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        for rel in disk_files(dir) {
            if !indexed.contains(rel.as_str())
                && !drift
                    .iter()
                    .any(|d| d.kind == "orphaned-file" && d.path == rel)
            {
                drift.push(DriftFinding {
                    kind: "orphaned-file".to_string(),
                    path: rel.clone(),
                    detail: "on disk but absent from the manifest file index (§7.1)".to_string(),
                });
            }
        }
    }

    // Manifest-internal reconciliation: every certificate record must
    // resolve to indexed, parseable bytes whose fingerprint matches
    // the record; every indexed certificate/key file must be claimed
    // by a record. Runs whether or not the bundle verified: a
    // checksum-clean bundle can still reference the wrong bytes.
    let indexed: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let mut claimed_cert_paths: BTreeSet<&str> = BTreeSet::new();
    let mut claimed_key_paths: BTreeSet<&str> = BTreeSet::new();
    let mut certs = Vec::new();
    for record in &manifest.certificates {
        if !indexed.contains(record.cert_path.as_str()) {
            drift.push(DriftFinding {
                kind: "cert-path-unindexed".to_string(),
                path: record.cert_path.clone(),
                detail: format!(
                    "certificate record {} ({}) is not in the file index",
                    record.fingerprint_sha256, record.subject_dn
                ),
            });
        } else {
            claimed_cert_paths.insert(record.cert_path.as_str());
            match std::fs::read(dir.join(&record.cert_path)) {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    match pem_decode_all(&text) {
                        Ok(blocks) => match blocks.into_iter().find(|(l, _)| l == "CERTIFICATE") {
                            Some((_, der)) => {
                                if sha256_hex(&der) != record.fingerprint_sha256 {
                                    drift.push(DriftFinding {
                                        kind: "fingerprint-mismatch".to_string(),
                                        path: record.cert_path.clone(),
                                        detail: format!(
                                            "on-disk SHA-256 {} != record {} ({})",
                                            sha256_hex(&der),
                                            record.fingerprint_sha256,
                                            record.subject_dn
                                        ),
                                    });
                                } else if let Ok(info) = parse_cert(&der) {
                                    if info.subject_dn != record.subject_dn {
                                        drift.push(DriftFinding {
                                            kind: "subject-mismatch".to_string(),
                                            path: record.cert_path.clone(),
                                            detail: format!(
                                                "on-disk subject '{}' != record '{}'",
                                                info.subject_dn, record.subject_dn
                                            ),
                                        });
                                    }
                                }
                            }
                            None => drift.push(DriftFinding {
                                kind: "cert-unparseable".to_string(),
                                path: record.cert_path.clone(),
                                detail: "no CERTIFICATE PEM block".to_string(),
                            }),
                        },
                        Err(e) => drift.push(DriftFinding {
                            kind: "cert-unparseable".to_string(),
                            path: record.cert_path.clone(),
                            detail: e.to_string(),
                        }),
                    }
                }
                Err(e) => drift.push(DriftFinding {
                    kind: "missing-file".to_string(),
                    path: record.cert_path.clone(),
                    detail: format!("certificate record unreadable: {e}"),
                }),
            }
        }
        if let Some(key_path) = &record.key_path {
            claimed_key_paths.insert(key_path.as_str());
        }
        certs.push(cert_status(record, at, ca_horizon, leaf_horizon));
    }
    for file in &manifest.files {
        if file.kind == "certificate-pem" && !claimed_cert_paths.contains(file.path.as_str()) {
            drift.push(DriftFinding {
                kind: "unreferenced-cert-file".to_string(),
                path: file.path.clone(),
                detail: "indexed certificate file claimed by no certificate record".to_string(),
            });
        }
        if file.kind == "private-key-pkcs8" && !claimed_key_paths.contains(file.path.as_str()) {
            drift.push(DriftFinding {
                kind: "unreferenced-key-file".to_string(),
                path: file.path.clone(),
                detail: "indexed private key claimed by no certificate record".to_string(),
            });
        }
    }

    BundleInventory {
        dir: dir.to_path_buf(),
        bundle_id: manifest.bundle_id.clone(),
        name: manifest.name.clone(),
        bundle_type: manifest.bundle_type.as_str().to_string(),
        verified,
        certs,
        drift,
    }
}

/// Run a full read-only audit over every bundle under `req.dir`.
pub fn audit(req: &InventoryRequest) -> crate::Result<InventoryReport> {
    let ca_horizon = req.ca_within.add_to(req.at)?;
    let leaf_horizon = req.leaf_within.add_to(req.at)?;
    let mut bundles = Vec::new();
    for (dir, manifest) in list_bundles(&req.dir, None) {
        bundles.push(audit_one(&dir, &manifest, req.at, ca_horizon, leaf_horizon));
    }
    bundles.sort_by(|a, b| a.dir.cmp(&b.dir));
    let mut totals = InventoryTotals::default();
    totals.bundles = bundles.len();
    for b in &bundles {
        if b.verified {
            totals.bundles_verified += 1;
        }
        totals.certificates += b.certs.len();
        totals.drift_findings += b.drift.len();
        for c in &b.certs {
            match c.status {
                ExpiryStatus::Expired => totals.expired += 1,
                ExpiryStatus::Expiring => totals.expiring += 1,
                _ => {}
            }
        }
    }
    Ok(InventoryReport {
        dir: req.dir.clone(),
        at: req.at,
        ca_within: req.ca_within,
        leaf_within: req.leaf_within,
        bundles,
        totals,
    })
}

fn filter_hit(filter: &SearchFilter, cert: &CertStatus) -> bool {
    if let Some(sub) = &filter.subject {
        if !cert
            .subject_dn
            .to_ascii_lowercase()
            .contains(&sub.to_ascii_lowercase())
        {
            return false;
        }
    }
    if let Some(fp) = &filter.fingerprint {
        if !cert
            .fingerprint_sha256
            .to_ascii_lowercase()
            .starts_with(&fp.to_ascii_lowercase())
        {
            return false;
        }
    }
    if let Some(role) = &filter.role {
        if cert.role != *role {
            return false;
        }
    }
    if let Some(algo) = &filter.algorithm {
        if cert.algorithm != *algo {
            return false;
        }
    }
    if filter.expired && cert.status != ExpiryStatus::Expired {
        return false;
    }
    true
}

/// Search every certificate record under `dir` (read-only; no bundle
/// verification — search consults manifest records and the
/// evaluation instant only).
pub fn search(
    dir: &Path,
    at: Rfc3339,
    ca_within: CalendarDuration,
    leaf_within: CalendarDuration,
    filter: &SearchFilter,
) -> crate::Result<Vec<SearchHit>> {
    let ca_horizon = ca_within.add_to(at)?;
    let leaf_horizon = leaf_within.add_to(at)?;
    let mut hits = Vec::new();
    for (bundle_dir, manifest) in list_bundles(dir, None) {
        for record in &manifest.certificates {
            let cert = cert_status(record, at, ca_horizon, leaf_horizon);
            if filter_hit(filter, &cert) {
                hits.push(SearchHit {
                    bundle_dir: bundle_dir.clone(),
                    bundle_name: manifest.name.clone(),
                    bundle_type: manifest.bundle_type.as_str().to_string(),
                    cert,
                });
            }
        }
    }
    hits.sort_by(|a, b| {
        (&a.bundle_dir, &a.cert.fingerprint_sha256)
            .cmp(&(&b.bundle_dir, &b.cert.fingerprint_sha256))
    });
    Ok(hits)
}

// ---------------------------------------------------------------- rendering

fn status_cell(c: &CertStatus) -> String {
    c.status.as_str().to_string()
}

fn cert_line(prefix: &str, c: &CertStatus) -> String {
    format!(
        "{prefix}{:10} {:8} {:40} expires {} ({:+}d, {})\n",
        c.role,
        c.algorithm,
        c.subject_dn,
        c.not_after,
        c.days_remaining,
        status_cell(c),
    )
}

/// Render `inventory list`: one line per bundle found under `dir`.
pub fn render_list(report: &InventoryReport) -> (String, serde_json::Value) {
    let mut text = format!(
        "Inventory: {} bundle(s) under {} (as of {}):\n",
        report.bundles.len(),
        report.dir.display(),
        report.at
    );
    if report.bundles.is_empty() {
        text.push_str("  (none)\n");
    }
    for b in &report.bundles {
        text.push_str(&format!(
            "  {} — {} ({}, {} certificates, {})\n",
            b.dir.display(),
            b.name,
            b.bundle_type,
            b.certs.len(),
            if b.verified { "verified" } else { "UNVERIFIED" }
        ));
    }
    let value = serde_json::json!({
        "dir": report.dir.display().to_string(),
        "at": report.at.to_string_canonical(),
        "bundles": report.bundles.iter().map(|b| serde_json::json!({
            "dir": b.dir.display().to_string(),
            "bundle_id": b.bundle_id,
            "name": b.name,
            "bundle_type": b.bundle_type,
            "verified": b.verified,
            "certificates": b.certs.len(),
            "drift_findings": b.drift.len(),
        })).collect::<Vec<_>>(),
    });
    (text, value)
}

/// Render `inventory show` for one audited bundle.
pub fn render_show(b: &BundleInventory, at: Rfc3339) -> (String, serde_json::Value) {
    let mut text = format!(
        "Bundle {} ({}) at {}\n  bundle_id: {}\n  verified: {}\n  certificates: {}\n",
        b.name,
        b.bundle_type,
        b.dir.display(),
        b.bundle_id,
        if b.verified { "yes" } else { "NO" },
        b.certs.len(),
    );
    for c in &b.certs {
        text.push_str(&cert_line("  ", c));
    }
    if b.drift.is_empty() {
        text.push_str("  drift: none\n");
    } else {
        text.push_str(&format!("  drift ({}):\n", b.drift.len()));
        for d in &b.drift {
            text.push_str(&format!("    [{}] {} — {}\n", d.kind, d.path, d.detail));
        }
    }
    let value = serde_json::to_value(b).unwrap_or_else(|_| serde_json::json!({}));
    let value = match value {
        serde_json::Value::Object(mut m) => {
            m.insert(
                "at".to_string(),
                serde_json::json!(at.to_string_canonical()),
            );
            serde_json::Value::Object(m)
        }
        v => v,
    };
    (text, value)
}

/// Render `inventory search` hits.
pub fn render_search(hits: &[SearchHit], at: Rfc3339) -> (String, serde_json::Value) {
    let mut text = format!(
        "Inventory search: {} match(es) (as of {}):\n",
        hits.len(),
        at
    );
    if hits.is_empty() {
        text.push_str("  (none)\n");
    }
    for h in hits {
        text.push_str(&format!(
            "  {} [{}] ",
            h.bundle_dir.display(),
            h.bundle_type,
        ));
        text.push_str(&cert_line("", &h.cert));
    }
    let value = serde_json::json!({
        "at": at.to_string_canonical(),
        "matches": hits.len(),
        "hits": hits,
    });
    (text, value)
}

/// Render `inventory audit`: per-bundle findings plus totals.
pub fn render_audit(report: &InventoryReport) -> (String, serde_json::Value) {
    let t = &report.totals;
    let mut text = format!(
        "Inventory audit under {} (as of {}):\n  bundles: {} ({} verified)\n  certificates: {} ({} expired, {} expiring)\n  drift findings: {}\n",
        report.dir.display(),
        report.at,
        t.bundles,
        t.bundles_verified,
        t.certificates,
        t.expired,
        t.expiring,
        t.drift_findings,
    );
    for b in &report.bundles {
        let interesting = b.drift.len()
            + b.certs
                .iter()
                .filter(|c| c.status != ExpiryStatus::Ok)
                .count();
        if interesting == 0 {
            text.push_str(&format!(
                "  {} — {} ({}): clean\n",
                b.dir.display(),
                b.name,
                b.bundle_type,
            ));
            continue;
        }
        text.push_str(&format!(
            "  {} — {} ({}):\n",
            b.dir.display(),
            b.name,
            b.bundle_type,
        ));
        for c in &b.certs {
            if c.status != ExpiryStatus::Ok {
                text.push_str(&cert_line("    ", c));
            }
        }
        for d in &b.drift {
            text.push_str(&format!("    [{}] {} — {}\n", d.kind, d.path, d.detail));
        }
    }
    let value = serde_json::to_value(report).unwrap_or_else(|_| serde_json::json!({}));
    (text, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Rfc3339 {
        Rfc3339::parse(s).unwrap()
    }

    #[test]
    fn horizon_boundary_is_expiring() {
        // Oracle: expiry_status — a certificate whose not_after is
        // exactly the horizon instant counts as expiring (<=).
        let at = ts("2026-06-01T00:00:00Z");
        let horizon = CalendarDuration::days(90).add_to(at).unwrap();
        let (status, days) = expiry_status(ts("2026-01-01T00:00:00Z"), horizon, horizon, at);
        assert_eq!(status, ExpiryStatus::Expiring);
        assert_eq!(days, 90);
    }

    #[test]
    fn expired_beats_horizon_and_future_is_not_yet_valid() {
        // Oracle: RFC 5280 §6.1 reading the chain validator uses
        // (not_before <= at <= not_after valid).
        let at = ts("2026-06-01T00:00:00Z");
        let far = CalendarDuration::years(5).add_to(at).unwrap();
        let (expired, days) = expiry_status(
            ts("2026-01-01T00:00:00Z"),
            ts("2026-05-01T00:00:00Z"),
            far,
            at,
        );
        assert_eq!(expired, ExpiryStatus::Expired);
        assert!(days < 0);
        let (future, _) = expiry_status(
            ts("2026-07-01T00:00:00Z"),
            ts("2027-07-01T00:00:00Z"),
            far,
            at,
        );
        assert_eq!(future, ExpiryStatus::NotYetValid);
    }

    #[test]
    fn ca_and_leaf_roles_take_different_horizons() {
        // Oracle: is_ca_role — same remaining lifetime (2y) is
        // expiring for a CA cert (5y horizon) but ok for a leaf.
        let at = ts("2026-06-01T00:00:00Z");
        let ca_horizon = CalendarDuration::years(5).add_to(at).unwrap();
        let leaf_horizon = CalendarDuration::days(90).add_to(at).unwrap();
        let nb = ts("2026-01-01T00:00:00Z");
        let na = ts("2028-06-01T00:00:00Z");
        let (ca_status, _) = expiry_status(nb, na, ca_horizon, at);
        let (leaf_status, _) = expiry_status(nb, na, leaf_horizon, at);
        assert_eq!(ca_status, ExpiryStatus::Expiring);
        assert_eq!(leaf_status, ExpiryStatus::Ok);
    }
}
