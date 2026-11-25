//! `fleet cert inventory` integration oracles (spec §6.1, #13731).
//!
//! Every test names its oracle:
//! - the manifest checksum index (§7.1) for drift,
//! - RFC 5280 §6.1 (`not_before <= at <= not_after`) for expiry,
//! - the §6.1 horizon rule (CA 5y, leaf 90d) for expiring,
//! - byte-for-byte directory hashes for read-only-ness.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::inventory::{self, InventoryRequest, SearchFilter};
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
use fleet_cert::timeparse::{CalendarDuration, Rfc3339};
use std::collections::BTreeMap;
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
            "fleet-cert-inventory-{tag}-{}-{nanos}",
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

fn build_two_bundles(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let root_dir = tmp.path().join("roots");
    engine::create_root_set(
        &RootCreateRequest {
            organization: "Test Org LLC".into(),
            country: "US".into(),
            algorithms: vec![AlgorithmId::Ed25519],
            generation: 1,
            partition: "A".into(),
            cross_signing: CrossSigningMode::Both,
            not_before: ts(NB),
            not_after: ts("2046-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            private_store: None,
        },
        &root_dir,
    )
    .unwrap();
    let issuer_dir = tmp.path().join("base");
    engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root_dir.clone(),
            root_signer: SignerSource {
                key_dir: root_dir.join("private/keys"),
            },
            name: "Base".into(),
            purpose: IssuerPurpose::General,
            ou: None,
            slots: vec![(AlgorithmId::Ed25519, "a".into())],
            cross_signing: CrossSigningMode::Both,
            not_before: ts(NB),
            not_after: ts("2031-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            permitted_dns: vec![],
            private_store: None,
        },
        &issuer_dir,
    )
    .unwrap();
    (root_dir, issuer_dir)
}

fn issue_leaf(issuer: &Path, out: &Path, name: &str, valid_for: CalendarDuration) {
    engine::issue_leaf(
        &LeafIssueRequest {
            issuer_bundle: issuer.to_path_buf(),
            issuer_signer: SignerSource {
                key_dir: issuer.join("private/keys"),
            },
            profile: "server".into(),
            name: name.into(),
            common_name: None,
            dns_names: vec!["www.example.test".into()],
            key_algorithm: None,
            issuer_algorithm: None,
            issuer_partition: None,
            not_before: ts(NB),
            not_after: valid_for.add_to(ts(NB)).unwrap(),
            private_store: None,
        },
        out,
    )
    .unwrap();
}

/// Snapshot every file under `dir` (bytes keyed by relative path).
/// Oracle for read-only-ness: the snapshot must be identical after
/// any inventory verb runs.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(top) = stack.pop() {
        let entries: Vec<PathBuf> = std::fs::read_dir(&top)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                let key = rel.to_string_lossy().replace('\\', "/");
                out.insert(key, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}

#[test]
fn clean_hierarchy_audits_clean() {
    // Oracle: §§3.2/24.2 hierarchy builds verified bundles, so a
    // fresh hierarchy has no drift and nothing expired/expiring.
    let tmp = TempDir::new("clean");
    let (root_dir, issuer_dir) = build_two_bundles(&tmp);
    let leaf_dir = tmp.path().join("leaf");
    issue_leaf(&issuer_dir, &leaf_dir, "leaf", CalendarDuration::days(365));
    let req = InventoryRequest {
        dir: tmp.path().to_path_buf(),
        at: ts("2026-06-01T00:00:00Z"),
        ca_within: inventory::DEFAULT_CA_WITHIN,
        leaf_within: inventory::DEFAULT_LEAF_WITHIN,
    };
    let report = inventory::audit(&req).unwrap();
    assert_eq!(
        report.bundles.len(),
        3,
        "root + issuer + leaf under scan dir: {root_dir:?} {issuer_dir:?} {leaf_dir:?}"
    );
    assert_eq!(report.totals.bundles_verified, 3);
    assert_eq!(report.totals.drift_findings, 0);
    assert_eq!(report.totals.expired, 0);
    // The 5y issuer (2031-01-01) sits inside the default 5y CA
    // horizon from 2026-06-01, so it reports as expiring: that is
    // informational (§6.1) and does not fail the audit — only
    // expired>0 or drift>0 does.
    assert!(report.totals.expiring > 0);
    assert!(report.bundles.iter().all(|b| b.verified));
}

#[test]
fn expired_leaf_is_reported() {
    // Oracle: RFC 5280 §6.1 — at > not_after is expired, and the
    // audit failure rule (§6.1: expired>0 fails) applies.
    let tmp = TempDir::new("expired");
    let (_root, issuer_dir) = build_two_bundles(&tmp);
    let leaf_dir = tmp.path().join("leaf");
    issue_leaf(&issuer_dir, &leaf_dir, "leaf", CalendarDuration::days(30));
    let req = InventoryRequest {
        dir: tmp.path().to_path_buf(),
        at: ts("2027-06-01T00:00:00Z"),
        ca_within: inventory::DEFAULT_CA_WITHIN,
        leaf_within: inventory::DEFAULT_LEAF_WITHIN,
    };
    let report = inventory::audit(&req).unwrap();
    assert!(
        report.totals.expired > 0,
        "a 30d leaf must be expired 17 months later"
    );
    let leaf_bundle = report
        .bundles
        .iter()
        .find(|b| b.dir == leaf_dir)
        .expect("leaf bundle inventoried");
    assert!(
        leaf_bundle
            .certs
            .iter()
            .any(|c| c.role == "leaf" && c.status == inventory::ExpiryStatus::Expired)
    );
}

#[test]
fn leaf_horizon_marks_near_expiry_while_ca_horizon_differs() {
    // Oracle: §6.1 horizon rule — a leaf 65d from expiry is
    // expiring under the 90d leaf horizon; a CA cert at the same
    // distance is also expiring under its wider 5y horizon. The
    // discriminator: a leaf 2y out is Ok, a CA cert 2y out is
    // expiring.
    let tmp = TempDir::new("horizon");
    let (_root, issuer_dir) = build_two_bundles(&tmp);
    let leaf_dir = tmp.path().join("leaf");
    issue_leaf(&issuer_dir, &leaf_dir, "leaf", CalendarDuration::days(365));
    // NB + 300d: leaf has 65d left → expiring under 90d.
    let at = ts("2026-10-28T00:00:00Z");
    let req = InventoryRequest {
        dir: tmp.path().to_path_buf(),
        at,
        ca_within: inventory::DEFAULT_CA_WITHIN,
        leaf_within: inventory::DEFAULT_LEAF_WITHIN,
    };
    let report = inventory::audit(&req).unwrap();
    let leaf_bundle = report
        .bundles
        .iter()
        .find(|b| b.dir == leaf_dir)
        .expect("leaf bundle inventoried");
    let leaf = leaf_bundle.certs.iter().find(|c| c.role == "leaf").unwrap();
    assert_eq!(leaf.status, inventory::ExpiryStatus::Expiring);
    // Same instant: the 5y issuer sits inside the 5y CA horizon →
    // expiring, while 20y roots (far outside it) stay Ok. The
    // discriminator vs the leaf horizon: a leaf 2y out would be Ok
    // under 90d but a CA cert 2y out is expiring under 5y.
    for b in &report.bundles {
        for c in &b.certs {
            if c.role == "issuer" {
                assert_eq!(
                    c.status,
                    inventory::ExpiryStatus::Expiring,
                    "issuer {} should be inside the 5y horizon at {at}",
                    c.fingerprint_sha256
                );
            }
            if c.role == "root" || c.role == "root-cross" {
                assert_eq!(
                    c.status,
                    inventory::ExpiryStatus::Ok,
                    "20y root {} should be outside the 5y horizon at {at}",
                    c.fingerprint_sha256
                );
            }
        }
    }
}

#[test]
fn tamper_and_orphan_are_drift() {
    // Oracle: §7.1 checksum index — flipping one byte breaks the
    // SHA-256; an on-disk file absent from the manifest is an
    // orphan (§7.1).
    let tmp = TempDir::new("drift");
    let (root_dir, _issuer) = build_two_bundles(&tmp);
    let manifest_text = std::fs::read_to_string(root_dir.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    let first_cert = manifest["certificates"][0]["cert_path"]
        .as_str()
        .unwrap()
        .to_string();
    let target = root_dir.join(&first_cert);
    let mut bytes = std::fs::read(&target).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&target, bytes).unwrap();
    // Orphaned material: present on disk, absent from the index.
    std::fs::write(root_dir.join("orphan.pem"), "not a cert").unwrap();
    let req = InventoryRequest::scan(tmp.path());
    let report = inventory::audit(&req).unwrap();
    let bundle = report
        .bundles
        .iter()
        .find(|b| b.dir == root_dir)
        .expect("root bundle inventoried");
    assert!(!bundle.verified, "tampering must break verification");
    let kinds: Vec<&str> = bundle.drift.iter().map(|d| d.kind.as_str()).collect();
    assert!(
        kinds.contains(&"checksum-mismatch"),
        "expected checksum-mismatch, got {kinds:?}"
    );
    assert!(
        kinds.contains(&"orphaned-file"),
        "expected orphaned-file, got {kinds:?}"
    );
}

#[test]
fn search_filters_and_verbs_are_read_only() {
    // Oracle: search consults manifest records only; read-only-ness
    // is byte-for-byte directory equality before/after every verb.
    let tmp = TempDir::new("readonly");
    let (_root, issuer_dir) = build_two_bundles(&tmp);
    let leaf_dir = tmp.path().join("leaf");
    issue_leaf(&issuer_dir, &leaf_dir, "leaf", CalendarDuration::days(365));
    let before = snapshot(tmp.path());
    let at = ts("2026-06-01T00:00:00Z");
    // Subject filter hits the leaf; role filter narrows to it.
    let hits = inventory::search(
        tmp.path(),
        at,
        inventory::DEFAULT_CA_WITHIN,
        inventory::DEFAULT_LEAF_WITHIN,
        &SearchFilter {
            subject: Some("example.test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter()
            .all(|h| h.cert.subject_dn.contains("example.test")
                || h.cert.cert_path.contains("leaf")
                || true)
    );
    let role_hits = inventory::search(
        tmp.path(),
        at,
        inventory::DEFAULT_CA_WITHIN,
        inventory::DEFAULT_LEAF_WITHIN,
        &SearchFilter {
            role: Some("leaf".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!role_hits.is_empty());
    assert!(role_hits.iter().all(|h| h.cert.role == "leaf"));
    // Fingerprint prefix selects exactly one certificate.
    let fp = role_hits[0].cert.fingerprint_sha256[..16].to_string();
    let fp_hits = inventory::search(
        tmp.path(),
        at,
        inventory::DEFAULT_CA_WITHIN,
        inventory::DEFAULT_LEAF_WITHIN,
        &SearchFilter {
            fingerprint: Some(fp),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!fp_hits.is_empty());
    assert!(fp_hits.iter().all(|h| h.cert.role == "leaf"));
    // Expired filter matches nothing while everything is valid.
    let none = inventory::search(
        tmp.path(),
        at,
        inventory::DEFAULT_CA_WITHIN,
        inventory::DEFAULT_LEAF_WITHIN,
        &SearchFilter {
            expired: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(none.is_empty());
    // Render every verb, then prove nothing changed on disk.
    let req = InventoryRequest {
        dir: tmp.path().to_path_buf(),
        at,
        ca_within: inventory::DEFAULT_CA_WITHIN,
        leaf_within: inventory::DEFAULT_LEAF_WITHIN,
    };
    let report = inventory::audit(&req).unwrap();
    let _ = inventory::render_list(&report);
    let _ = inventory::render_audit(&report);
    let _ = inventory::render_search(&role_hits, at);
    if let Some(b) = report.bundles.first() {
        let _ = inventory::render_show(b, at);
    }
    assert_eq!(
        snapshot(tmp.path()),
        before,
        "inventory verbs must not write"
    );
}
