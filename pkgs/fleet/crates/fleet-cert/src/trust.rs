//! Trust-bundle export and broker-client consumption (BT-2,
//! #14400; design of record doc/2026-11-24T13-59-26Z_design_
//! broker-tls §6, spec §§3.5, 27.3 item 9).
//!
//! Two halves, one versioned format:
//!
//! - **Export** repackages the self-signed roots of one or more
//!   verified root-set bundles into a standalone §7 bundle of type
//!   [`BundleType::Trust`]: the §3.5 files (`roots-classical.pem`,
//!   `roots-all.pem`, `roots-pq.pem` when a PQ root exists,
//!   `cross-certs.pem` when cross-certificates exist) plus a
//!   provenance descriptor. The manifest's `bundle_version` is the
//!   trust-bundle version — the stamp consumers acknowledge. A
//!   trust bundle carries no key material, ever.
//! - **The client trust store** is the minimal broker-scoped form
//!   of the §27.3 item 9 controller pattern: two-phase rollover as
//!   explicit, separately callable steps —
//!   1. [`stage_bundle`]: verify a distributed bundle and install
//!      it beside the active one. A bundle that fails verification
//!      never stages, so a failed distribution cannot touch the
//!      active anchors.
//!   2. [`acknowledge`]: record the client's acknowledgement,
//!      binding the bundle version to its content hash after a
//!      fresh verification of the installed copy. This record is
//!      what a client presents (BT-3+ handshake work) and what a
//!      broker health surface aggregates to see stale clients.
//!   3. [`activate`]: flip the active pointer. Refused without a
//!      matching acknowledgement — rollover never activates
//!      without acknowledgements. The displaced version is kept
//!      as last-known-good.
//!   4. [`retire`]: drop an installed version that is neither
//!      active nor last-known-good.
//!
//! The general §27.3 trust-bundle controller (#9164 line) absorbs
//! this path rather than replacing it: same §7 manifest, same
//! version stamp, same ack binding.

use crate::bundle::{
    Artifact, load_bundle, pem_decode_all, read_artifact, sha256_hex, write_bundle,
};
use crate::chain::TrustSelection;
use crate::error::{CertError, Result};
use crate::model::{BundleManifest, BundleType, CertRecord, MANIFEST_SCHEMA};
use crate::naming::slugify;
use crate::timeparse::Rfc3339;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Schema of the provenance descriptor inside a trust bundle.
pub const TRUST_DESCRIPTOR_SCHEMA: &str = "fleet-cert-trust-bundle/v1";
/// Schema of the client trust store's state file.
pub const TRUST_STORE_SCHEMA: &str = "fleet-cert-trust-store/v1";

const CLASSICAL_PEM: &str = "public/bundles/roots-classical.pem";
const ALL_PEM: &str = "public/bundles/roots-all.pem";
const PQ_PEM: &str = "public/bundles/roots-pq.pem";
const CROSS_PEM: &str = "public/bundles/cross-certs.pem";
const DESCRIPTOR_PATH: &str = "public/trust-bundle.json";

// ---------------------------------------------------------------- export

/// `cert trust export`: assemble a versioned trust bundle from
/// root-set bundles.
#[derive(Debug, Clone)]
pub struct TrustExportRequest {
    /// Root-set bundle directories (each strictly verified on
    /// load). Several sets — e.g. both partitions — contribute
    /// their roots to one trust bundle.
    pub root_bundles: Vec<PathBuf>,
    /// The trust-bundle version. Monotonic across exports; it is
    /// the stamp consumers acknowledge and brokers compare.
    pub version: u32,
    /// Human name for the bundle manifest.
    pub name: String,
    pub at: Rfc3339,
}

/// One source root set of a trust bundle (provenance).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustSource {
    pub bundle_id: String,
    pub generation: u32,
    /// Partitions the source's roots belong to (sorted, deduped).
    pub partitions: Vec<String>,
    /// SHA-256 fingerprints of the roots taken from this source.
    pub root_fingerprints: Vec<String>,
}

/// The provenance descriptor written into every trust bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustDescriptor {
    pub schema: String,
    pub version: u32,
    pub exported_at: Rfc3339,
    pub sources: Vec<TrustSource>,
    /// All anchor fingerprints in the bundle (sorted).
    pub root_fingerprints: Vec<String>,
    /// Cross-certificate fingerprints (untrusted path-building
    /// material, never anchors; sorted).
    pub cross_fingerprints: Vec<String>,
}

/// Content identity of a bundle: SHA-256 over the manifest's
/// sorted `path:sha256` file index. Two bundles with the same
/// version but different content hashes are different bundles —
/// version reuse with different content is a distribution error.
pub fn bundle_content_sha256(manifest: &BundleManifest) -> String {
    let mut files: Vec<&crate::model::FileRecord> = manifest.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut acc = String::new();
    for f in files {
        acc.push_str(&f.path);
        acc.push(':');
        acc.push_str(&f.sha256);
        acc.push('\n');
    }
    sha256_hex(acc.as_bytes())
}

/// Assemble and write the trust bundle; returns its manifest.
/// The output directory must not already hold a bundle
/// (`write_bundle` refuses to overwrite).
pub fn export_trust_bundle(req: &TrustExportRequest, out_dir: &Path) -> Result<BundleManifest> {
    if req.version == 0 {
        return Err(CertError::usage(
            "trust bundle versions start at 1 (version 0 is never a valid stamp)",
        ));
    }
    if req.root_bundles.is_empty() {
        return Err(CertError::usage(
            "trust export needs at least one root-set bundle (--root <dir>)",
        ));
    }

    // fingerprint -> (record, PEM text) for anchors and cross certs.
    let mut roots: BTreeMap<String, (CertRecord, Vec<u8>)> = BTreeMap::new();
    let mut cross: BTreeMap<String, (CertRecord, Vec<u8>)> = BTreeMap::new();
    let mut sources: Vec<TrustSource> = Vec::new();
    let mut organization = String::new();
    let mut country = String::new();
    let mut generation = 0u32;

    for dir in &req.root_bundles {
        let bundle = load_bundle(dir)?;
        if bundle.manifest.bundle_type != BundleType::RootSet {
            return Err(CertError::usage(format!(
                "trust export source {} is a {} bundle, not a root set",
                dir.display(),
                bundle.manifest.bundle_type.as_str()
            )));
        }
        if organization.is_empty() {
            organization = bundle.manifest.organization.clone();
            country = bundle.manifest.country.clone();
        }
        generation = generation.max(bundle.manifest.generation);
        let mut source = TrustSource {
            bundle_id: bundle.manifest.bundle_id.clone(),
            generation: bundle.manifest.generation,
            partitions: Vec::new(),
            root_fingerprints: Vec::new(),
        };
        for record in &bundle.manifest.certificates {
            let target = match record.role.as_str() {
                "root" => {
                    if record.signer_algorithm.is_some() {
                        return Err(CertError::manifest(format!(
                            "root record {} in {} is not self-signed; refusing to export it as a trust anchor",
                            record.fingerprint_sha256, bundle.manifest.bundle_id
                        )));
                    }
                    &mut roots
                }
                "root-cross" => &mut cross,
                _ => continue,
            };
            let pem = read_artifact(&bundle, &record.cert_path)?;
            if let Some((_, existing)) = target.get(&record.fingerprint_sha256) {
                if *existing != pem {
                    return Err(CertError::policy(format!(
                        "fingerprint {} appears with different certificate bytes across source bundles; refusing an ambiguous trust bundle",
                        record.fingerprint_sha256
                    )));
                }
                continue;
            }
            if record.role == "root" {
                source
                    .root_fingerprints
                    .push(record.fingerprint_sha256.clone());
                if !source.partitions.contains(&record.partition) {
                    source.partitions.push(record.partition.clone());
                }
            }
            target.insert(record.fingerprint_sha256.clone(), (record.clone(), pem));
        }
        source.partitions.sort();
        source.root_fingerprints.sort();
        sources.push(source);
    }

    if roots.is_empty() {
        return Err(CertError::policy(
            "trust export found no self-signed roots in the source bundles",
        ));
    }
    let classical: Vec<&(CertRecord, Vec<u8>)> = roots
        .values()
        .filter(|(r, _)| r.algorithm.is_classical())
        .collect();
    if classical.is_empty() {
        return Err(CertError::policy(
            "trust export requires at least one classical root: the broker bundle is the \
             classical compatibility bundle (roots-classical / roots-all, spec §3.5); \
             a PQ-only trust store is a different distribution",
        ));
    }

    let join_pems = |set: &[&(CertRecord, Vec<u8>)]| -> Vec<u8> {
        let mut out = Vec::new();
        for (_, pem) in set {
            out.extend_from_slice(pem);
            if !out.ends_with(b"\n") {
                out.push(b'\n');
            }
        }
        out
    };
    let all: Vec<&(CertRecord, Vec<u8>)> = roots.values().collect();
    let pq: Vec<&(CertRecord, Vec<u8>)> = roots
        .values()
        .filter(|(r, _)| r.algorithm.is_post_quantum())
        .collect();

    let mut artifacts = vec![
        Artifact {
            rel_path: ALL_PEM.into(),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: join_pems(&all),
        },
        Artifact {
            rel_path: CLASSICAL_PEM.into(),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: join_pems(&classical),
        },
    ];
    if !pq.is_empty() {
        artifacts.push(Artifact {
            rel_path: PQ_PEM.into(),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: join_pems(&pq),
        });
    }
    if !cross.is_empty() {
        let cross_set: Vec<&(CertRecord, Vec<u8>)> = cross.values().collect();
        artifacts.push(Artifact {
            rel_path: CROSS_PEM.into(),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: join_pems(&cross_set),
        });
    }

    let descriptor = TrustDescriptor {
        schema: TRUST_DESCRIPTOR_SCHEMA.to_string(),
        version: req.version,
        exported_at: req.at,
        sources,
        root_fingerprints: roots.keys().cloned().collect(),
        cross_fingerprints: cross.keys().cloned().collect(),
    };
    artifacts.push(Artifact {
        rel_path: DESCRIPTOR_PATH.into(),
        kind: "trust-descriptor-json".into(),
        visibility: "public".into(),
        bytes: (serde_json::to_string_pretty(&descriptor)? + "\n").into_bytes(),
    });

    let mut certificates: Vec<CertRecord> = roots.values().map(|(r, _)| r.clone()).collect();
    certificates.extend(cross.values().map(|(r, _)| r.clone()));
    let fingerprint_line = descriptor.root_fingerprints.join(",");
    let manifest = BundleManifest {
        schema: MANIFEST_SCHEMA.to_string(),
        bundle_id: format!(
            "trust-v{}-{}",
            req.version,
            &sha256_hex(fingerprint_line.as_bytes())[..12]
        ),
        bundle_type: BundleType::Trust,
        bundle_version: req.version,
        name: req.name.clone(),
        slug: slugify(&req.name)?,
        organization,
        country,
        purpose: Some("trust-distribution".to_string()),
        generation,
        revision: 1,
        predecessor: None,
        created_at: req.at,
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        parent: None,
        // A trust bundle holds no key material in any location.
        key_location: "none".to_string(),
        cross_signing: None,
        certificates,
        files: Vec::new(),
        warnings: Vec::new(),
    };
    write_bundle(out_dir, manifest, &artifacts)
}

/// Read the descriptor of a loaded trust bundle.
pub fn read_descriptor(bundle: &crate::bundle::Bundle) -> Result<TrustDescriptor> {
    let bytes = read_artifact(bundle, DESCRIPTOR_PATH)?;
    Ok(serde_json::from_slice(&bytes)?)
}

// ---------------------------------------------------------------- store

/// A reference to one installed trust-bundle version, bound to
/// the exact content that was verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRef {
    pub version: u32,
    pub bundle_id: String,
    pub content_sha256: String,
}

impl VersionRef {
    fn of(manifest: &BundleManifest) -> Self {
        VersionRef {
            version: manifest.bundle_version,
            bundle_id: manifest.bundle_id.clone(),
            content_sha256: bundle_content_sha256(manifest),
        }
    }
}

/// A client's acknowledgement of one trust-bundle version: the
/// version was distributed, installed, and verified with exactly
/// this content. Activation requires a matching ack (§27.3
/// item 9: rollover never activates without acknowledgements).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckRecord {
    pub version: u32,
    pub bundle_id: String,
    pub content_sha256: String,
    pub acknowledged_at: Rfc3339,
}

/// The trust store's state file (`<store>/store.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreState {
    pub schema: String,
    pub active: Option<VersionRef>,
    /// The displaced active version, retained as last-known-good.
    pub previous: Option<VersionRef>,
    /// A verified, installed version awaiting acknowledgement /
    /// activation.
    pub staged: Option<VersionRef>,
    pub acks: Vec<AckRecord>,
}

impl Default for StoreState {
    fn default() -> Self {
        StoreState {
            schema: TRUST_STORE_SCHEMA.to_string(),
            active: None,
            previous: None,
            staged: None,
            acks: Vec::new(),
        }
    }
}

/// The full picture `trust status` reports — the client half of
/// the §6 ack surface (stale clients are the ones whose active
/// version lags the estate's current version).
#[derive(Debug, Clone, Serialize)]
pub struct TrustStoreStatus {
    pub store: PathBuf,
    pub active: Option<VersionRef>,
    pub previous: Option<VersionRef>,
    pub staged: Option<VersionRef>,
    pub acks: Vec<AckRecord>,
    pub installed_versions: Vec<u32>,
}

fn state_path(store: &Path) -> PathBuf {
    store.join("store.json")
}

fn version_dir(store: &Path, version: u32) -> PathBuf {
    store.join("versions").join(version.to_string())
}

fn load_state(store: &Path) -> Result<StoreState> {
    match std::fs::read_to_string(state_path(store)) {
        Ok(text) => {
            let state: StoreState = serde_json::from_str(&text)?;
            if state.schema != TRUST_STORE_SCHEMA {
                return Err(CertError::manifest(format!(
                    "trust store at {} has unsupported state schema '{}' (this build reads {TRUST_STORE_SCHEMA})",
                    store.display(),
                    state.schema
                )));
            }
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoreState::default()),
        Err(e) => Err(e.into()),
    }
}

fn save_state(store: &Path, state: &StoreState) -> Result<()> {
    std::fs::create_dir_all(store)?;
    let tmp = store.join("store.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(state)? + "\n")?;
    std::fs::rename(&tmp, state_path(store))?;
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .ok_or_else(|| CertError::manifest(format!("bad path {}", path.display())))?;
        let target = dst.join(name);
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Verify the installed copy of `vref` in the store and return
/// its manifest. Fails closed: any drift between the installed
/// files and the recorded content hash is a manifest error.
fn verify_installed(store: &Path, vref: &VersionRef) -> Result<BundleManifest> {
    let bundle = load_bundle(&version_dir(store, vref.version))?;
    if bundle.manifest.bundle_id != vref.bundle_id
        || bundle_content_sha256(&bundle.manifest) != vref.content_sha256
    {
        return Err(CertError::manifest(format!(
            "installed trust bundle version {} in {} does not match its recorded content (bundle drift); refusing to trust it",
            vref.version,
            store.display()
        )));
    }
    Ok(bundle.manifest)
}

/// Phase 1 — distribute & install: verify `bundle_dir` strictly,
/// install it under the store, and mark it staged. The active
/// version is never touched here, so a distribution that fails
/// verification leaves the client's anchors exactly as they were.
pub fn stage_bundle(store: &Path, bundle_dir: &Path) -> Result<VersionRef> {
    let bundle = load_bundle(bundle_dir)?;
    if bundle.manifest.bundle_type != BundleType::Trust {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a trust bundle; only trust bundles install into a trust store",
            bundle_dir.display(),
            bundle.manifest.bundle_type.as_str()
        )));
    }
    let vref = VersionRef::of(&bundle.manifest);
    let mut state = load_state(store)?;

    for held in [&state.active, &state.previous].into_iter().flatten() {
        if held.version == vref.version && held.content_sha256 != vref.content_sha256 {
            return Err(CertError::policy(format!(
                "trust bundle version {} is already {} with different content; version reuse with different content is a distribution error",
                vref.version,
                if state.active.as_ref() == Some(held) {
                    "active"
                } else {
                    "the last-known-good"
                }
            )));
        }
    }
    if let Some(active) = &state.active {
        if vref.version < active.version {
            return Err(CertError::policy(format!(
                "trust bundle version {} is older than the active version {}; rollover moves forward (the retained previous version is the rollback path)",
                vref.version, active.version
            )));
        }
        if vref.version == active.version {
            // Same version, same content (checked above): already
            // installed and active — staging is a no-op.
            return Ok(vref);
        }
    }

    let dir = version_dir(store, vref.version);
    if dir.join("manifest.json").exists() {
        let installed = load_bundle(&dir)?;
        if bundle_content_sha256(&installed.manifest) != vref.content_sha256 {
            return Err(CertError::policy(format!(
                "trust bundle version {} is already installed with different content; version reuse with different content is a distribution error",
                vref.version
            )));
        }
    } else {
        copy_dir(bundle_dir, &dir)?;
        // Verify the copy that actually landed, not the source.
        verify_installed(store, &vref)?;
    }

    state.staged = Some(vref.clone());
    save_state(store, &state)?;
    Ok(vref)
}

/// Phase 2 — acknowledge: verify the installed staged bundle
/// once more and record the acknowledgement binding its version
/// to its content hash. `version` defaults to the staged one.
pub fn acknowledge(store: &Path, version: Option<u32>, at: Rfc3339) -> Result<AckRecord> {
    let mut state = load_state(store)?;
    let staged = state.staged.clone().ok_or_else(|| {
        CertError::usage(
            "nothing is staged in this trust store; stage a bundle before acknowledging it",
        )
    })?;
    if let Some(v) = version {
        if v != staged.version {
            return Err(CertError::usage(format!(
                "acknowledge names version {v} but the staged version is {}",
                staged.version
            )));
        }
    }
    // The ack binds what is installed *now*, verified fresh.
    verify_installed(store, &staged)?;
    if let Some(existing) = state
        .acks
        .iter()
        .find(|a| a.version == staged.version && a.content_sha256 == staged.content_sha256)
    {
        return Ok(existing.clone());
    }
    let ack = AckRecord {
        version: staged.version,
        bundle_id: staged.bundle_id.clone(),
        content_sha256: staged.content_sha256.clone(),
        acknowledged_at: at,
    };
    state.acks.push(ack.clone());
    save_state(store, &state)?;
    Ok(ack)
}

/// Phase 3 — activate: make the staged version the active one.
/// Refused without an acknowledgement matching the staged
/// version *and* content. The displaced active version becomes
/// the retained last-known-good.
pub fn activate(store: &Path, version: Option<u32>) -> Result<VersionRef> {
    let mut state = load_state(store)?;
    let staged = state.staged.clone().ok_or_else(|| {
        CertError::usage(
            "nothing is staged in this trust store; stage a bundle before activating it",
        )
    })?;
    if let Some(v) = version {
        if v != staged.version {
            return Err(CertError::usage(format!(
                "activate names version {v} but the staged version is {}",
                staged.version
            )));
        }
    }
    let acked = state
        .acks
        .iter()
        .any(|a| a.version == staged.version && a.content_sha256 == staged.content_sha256);
    if !acked {
        return Err(CertError::policy(format!(
            "rollover never activates without acknowledgements (spec §27.3 item 9): no acknowledgement recorded for trust bundle version {} with its staged content",
            staged.version
        )));
    }
    verify_installed(store, &staged)?;
    if state.active.as_ref() != Some(&staged) {
        state.previous = state.active.take();
        state.active = Some(staged.clone());
    }
    state.staged = None;
    save_state(store, &state)?;
    Ok(staged)
}

/// Phase 4 — retire an installed version. The active version and
/// the last-known-good version are never retired by this verb:
/// the active one is in use, and the previous one is the
/// rollback path.
pub fn retire(store: &Path, version: u32) -> Result<()> {
    let mut state = load_state(store)?;
    if state.active.as_ref().map(|v| v.version) == Some(version) {
        return Err(CertError::policy(format!(
            "trust bundle version {version} is active; it cannot be retired"
        )));
    }
    if state.previous.as_ref().map(|v| v.version) == Some(version) {
        return Err(CertError::policy(format!(
            "trust bundle version {version} is the last-known-good rollback version; it is retained"
        )));
    }
    let dir = version_dir(store, version);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    if state.staged.as_ref().map(|v| v.version) == Some(version) {
        state.staged = None;
    }
    save_state(store, &state)?;
    Ok(())
}

/// The store's full status (see [`TrustStoreStatus`]).
pub fn store_status(store: &Path) -> Result<TrustStoreStatus> {
    let state = load_state(store)?;
    let mut installed_versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(store.join("versions")) {
        for e in entries.flatten() {
            if let Some(v) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) {
                installed_versions.push(v);
            }
        }
    }
    installed_versions.sort_unstable();
    Ok(TrustStoreStatus {
        store: store.to_path_buf(),
        active: state.active,
        previous: state.previous,
        staged: state.staged,
        acks: state.acks,
        installed_versions,
    })
}

/// The anchor PEM a broker client validates against: the
/// selection's §3.5 file from the *active* trust bundle,
/// verified fresh on every read (fail closed on drift).
pub fn active_anchors(store: &Path, selection: TrustSelection) -> Result<(VersionRef, Vec<u8>)> {
    let state = load_state(store)?;
    let active = state.active.clone().ok_or_else(|| {
        CertError::policy(
            "no active trust bundle in this store; stage, acknowledge and activate one first",
        )
    })?;
    let bundle_dir = version_dir(store, active.version);
    let bundle = load_bundle(&bundle_dir)?;
    if bundle_content_sha256(&bundle.manifest) != active.content_sha256 {
        return Err(CertError::manifest(format!(
            "active trust bundle version {} in {} does not match its recorded content (bundle drift); failing closed",
            active.version,
            store.display()
        )));
    }
    let rel = match selection {
        TrustSelection::All => ALL_PEM,
        TrustSelection::Classical => CLASSICAL_PEM,
        TrustSelection::Pq => PQ_PEM,
    };
    let bytes = read_artifact(&bundle, rel).map_err(|_| {
        CertError::policy(format!(
            "active trust bundle version {} has no {} file; that trust selection is not served by this bundle",
            active.version,
            selection.as_str()
        ))
    })?;
    // The anchors must be parseable certificates, not just bytes
    // that once hashed correctly.
    if pem_decode_all(
        std::str::from_utf8(&bytes)
            .map_err(|_| CertError::manifest("anchor file is not UTF-8 PEM text".to_string()))?,
    )?
    .is_empty()
    {
        return Err(CertError::manifest(format!(
            "active trust bundle version {}: {} contains no certificates",
            active.version, rel
        )));
    }
    Ok((active, bytes))
}
