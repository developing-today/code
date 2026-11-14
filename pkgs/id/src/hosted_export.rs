//! Hosted export and import (spec §6; tracker #2852 slice 4, #3810).
//!
//! Implements the export format (§6.1) and the self-hosted node's
//! import verification (§6.2 steps 1–3) for hosted ID stores, over the
//! record types slices 1–2 landed in [`crate::hosted_binding`]. The
//! client-side-encryption choice (§7) is threaded through the export
//! from the binding record: [`ExportManifest::client_side_encryption`]
//! mirrors [`BindingRecord::client_side_encryption`], and a verified
//! import reports it so the importing node knows the payloads it is
//! about to ingest are ciphertext it cannot read without the user's
//! keys.
//!
//! ## What is here
//!
//! - [`ExportObject`] / [`ExportBlob`]: self-authenticating content.
//!   An object's hash is SHA-256 of its bytes and its signature is the
//!   producing key's signature over that hash; a blob's hash is
//!   SHA-256 of its bytes. Objects carry the blob hashes they cite.
//! - [`ExportManifest`]: `{export_id, binding_id, iroh_id, created_at,
//!   object_count, blob_count, sha256 of the concatenated object
//!   hashes}` (§6.1 item 4), signed by the user's Iroh root key when
//!   available; when it is not (e.g. export during a lost-key rebind
//!   wait) the service signs instead and the manifest is marked
//!   `unkeyed_export`.
//! - [`build_export`]: assemble an export in canonical (hash) order
//!   from a binding record, its rebind records, objects, and blobs.
//! - [`verify_import`]: §6.2 verification, before anything is
//!   ingested. Every refusal is a typed [`ExportError`] naming the
//!   rule that failed; a partial or tampered export imports nothing.
//!
//! ## What is not here
//!
//! The enumeration of a live per-user store (the slice-3 DO scaffold)
//! and R2 blob fetching: `build_export` takes the object/blob sets as
//! inputs, so the DO/R2 wiring is a follow-on that feeds it. Ingest
//! itself (§6.2 step 4) follows the platform's normal rules and is
//! likewise downstream of a verified import.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hosted_binding::{BindingRecord, HostedBindingService, RebindRecord, hex_encode};

/// Domain separator for the manifest signature, so a manifest
/// signature can never be replayed as another record type's.
pub const EXPORT_MANIFEST_DOMAIN: &str = "hosted-export-manifest/v1";

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn hex_decode(text: &str, len: usize) -> Option<Vec<u8>> {
    if text.len() != len * 2 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(len);
    for pair in bytes.chunks_exact(2) {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        out.push(u8::try_from(hi * 16 + lo).ok()?);
    }
    Some(out)
}

fn verify_hex_signature(public_key_hex: &str, message: &[u8], signature_hex: &str) -> bool {
    let Some(key_bytes) = hex_decode(public_key_hex, 32) else {
        return false;
    };
    let Ok(key_array) = <[u8; 32]>::try_from(key_bytes) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&key_array) else {
        return false;
    };
    let Some(sig_bytes) = hex_decode(signature_hex, 64) else {
        return false;
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&sig_array) else {
        return false;
    };
    verifying_key.verify_strict(message, &signature).is_ok()
}

/// One signed graph object in an export (§6.1 item 1): self-
/// authenticating by construction — `hash` is SHA-256 of `bytes` and
/// `signature` is `signer`'s signature over the hash bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportObject {
    /// SHA-256 of `bytes`, hex.
    pub hash: String,
    /// The producing key (an Iroh root key in the binding chain), hex.
    pub signer: String,
    /// Signature over the 32 hash bytes, hex.
    pub signature: String,
    /// Hashes of the blobs this object cites (§6.2 step 3), hex.
    pub blob_refs: Vec<String>,
    /// The object bytes.
    pub bytes: Vec<u8>,
}

impl ExportObject {
    /// Build and sign an object from its bytes.
    #[must_use]
    pub fn new(signing_key: &SigningKey, bytes: Vec<u8>, blob_refs: Vec<String>) -> Self {
        let hash = sha256_hex(&bytes);
        let hash_bytes = hex_decode(&hash, 32).unwrap_or_default();
        let signature = signing_key.sign(&hash_bytes).to_bytes();
        Self {
            hash,
            signer: hex_encode(&signing_key.verifying_key().to_bytes()),
            signature: hex_encode(&signature),
            blob_refs,
            bytes,
        }
    }
}

/// One blob in an export (§6.1 item 2), carried by hash: `hash` is
/// SHA-256 of `bytes`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportBlob {
    /// SHA-256 of `bytes`, hex.
    pub hash: String,
    /// The blob bytes.
    pub bytes: Vec<u8>,
}

impl ExportBlob {
    /// Build a blob from its bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        let hash = sha256_hex(&bytes);
        Self { hash, bytes }
    }
}

/// The export manifest (§6.1 item 4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifest {
    /// Opaque export ID, minted by the producer.
    pub export_id: String,
    /// The binding this export belongs to.
    pub binding_id: String,
    /// The binding's current Iroh root public key, hex.
    pub iroh_id: String,
    /// Export time (RFC 3339).
    pub created_at: String,
    /// Number of objects in the export.
    pub object_count: u64,
    /// Number of blobs in the export.
    pub blob_count: u64,
    /// SHA-256 of the concatenated object hash bytes, objects in
    /// canonical (hash) order, hex.
    pub objects_digest: String,
    /// True when the service signed because the user's key was
    /// unavailable (§6.1); §6.2 step 1 treats these differently.
    pub unkeyed_export: bool,
    /// The §7 client-side-encryption choice in force for the binding:
    /// when true, object payloads and blob contents are ciphertext.
    pub client_side_encryption: bool,
    /// Signature over [`ExportManifest::canonical_bytes`], hex: the
    /// user's root key, or the service key when `unkeyed_export`.
    pub signature: String,
}

impl ExportManifest {
    /// The canonical bytes the manifest signature covers: every field
    /// except the signature itself, domain-separated and
    /// length-prefixed per field.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let fields = [
            self.export_id.as_str(),
            self.binding_id.as_str(),
            self.iroh_id.as_str(),
            self.created_at.as_str(),
            &self.object_count.to_string(),
            &self.blob_count.to_string(),
            self.objects_digest.as_str(),
            if self.unkeyed_export {
                "unkeyed"
            } else {
                "keyed"
            },
            if self.client_side_encryption {
                "cse"
            } else {
                "plain"
            },
        ];
        let mut out = Vec::new();
        out.extend_from_slice(EXPORT_MANIFEST_DOMAIN.as_bytes());
        for field in fields {
            out.extend_from_slice(&(field.len() as u64).to_be_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out
    }
}

/// A complete export (§6.1 items 1–4): manifest, objects, blobs, the
/// binding record (with full `history` and `challenge_transcript`),
/// any rebind records backing the history, and the producing
/// service's public key so import verification needs no hosted-service
/// access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedExport {
    /// The signed manifest.
    pub manifest: ExportManifest,
    /// Objects in canonical (hash) order.
    pub objects: Vec<ExportObject>,
    /// Blobs, in hash order.
    pub blobs: Vec<ExportBlob>,
    /// The binding record (§1.2).
    pub binding: BindingRecord,
    /// Rebind records (§4.4) backing `binding.history` steps.
    pub rebind_records: Vec<RebindRecord>,
    /// Producing service's public key, hex: verifies the binding
    /// attestation, rebind records, and an `unkeyed_export` manifest.
    pub service_public_key: String,
}

/// A named import refusal (§6.2): verification failed at the named
/// rule and nothing was ingested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The manifest names a different binding or key than the
    /// binding record it travels with.
    ManifestBindingMismatch,
    /// The manifest signature does not verify (against the user's
    /// root key, or the service key for an `unkeyed_export`).
    ManifestSignatureInvalid,
    /// An `unkeyed_export` presented without the operator's explicit
    /// untrusted-source confirmation (§6.2 step 1).
    UnkeyedNotConfirmed,
    /// The binding record's key signature does not verify against
    /// its own challenge transcript (§6.2 step 2).
    BindingKeySignatureInvalid,
    /// The binding record's service attestation does not verify
    /// against the export's pinned service key (§6.2 step 2).
    BindingAttestationInvalid,
    /// A `delayed-rebind` history step has no backing rebind record
    /// in the export, or a rebind record fails verification (§6.2
    /// step 2). Names the superseded key.
    HistoryUnbacked {
        /// The history entry's Iroh root public key, hex.
        iroh_id: String,
    },
    /// A rebind record belongs to a different binding.
    RebindRecordMismatch,
    /// Manifest counts do not match the received sets.
    CountMismatch,
    /// The manifest's concatenated-hash digest does not match the
    /// received objects (§6.2 step 3).
    ObjectsDigestMismatch,
    /// An object's hash is not SHA-256 of its bytes.
    ObjectHashMismatch,
    /// An object's signature does not verify, or its signer is not a
    /// key in the binding chain.
    ObjectSignatureInvalid,
    /// The same object hash appears twice.
    DuplicateObject,
    /// A blob's hash is not SHA-256 of its bytes (§6.2 step 3).
    BlobHashMismatch,
    /// A blob no verified object references (§6.2 step 3).
    UnreferencedBlob,
    /// An object cites a blob the export does not carry (§6.2 step 3).
    MissingBlob,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ManifestBindingMismatch => {
                write!(
                    f,
                    "manifest does not name the binding record it travels with"
                )
            }
            Self::ManifestSignatureInvalid => {
                write!(f, "manifest signature does not verify")
            }
            Self::UnkeyedNotConfirmed => {
                write!(
                    f,
                    "unkeyed_export refused: operator has not confirmed an \
                     untrusted-source import"
                )
            }
            Self::BindingKeySignatureInvalid => {
                write!(f, "binding record key signature does not verify")
            }
            Self::BindingAttestationInvalid => {
                write!(f, "binding record service attestation does not verify")
            }
            Self::HistoryUnbacked { iroh_id } => {
                write!(f, "history step for {iroh_id} has no backing rebind record")
            }
            Self::RebindRecordMismatch => {
                write!(f, "rebind record belongs to a different binding")
            }
            Self::CountMismatch => {
                write!(f, "manifest counts do not match the received sets")
            }
            Self::ObjectsDigestMismatch => {
                write!(
                    f,
                    "manifest objects digest does not match the received objects"
                )
            }
            Self::ObjectHashMismatch => {
                write!(f, "object hash is not the SHA-256 of its bytes")
            }
            Self::ObjectSignatureInvalid => {
                write!(
                    f,
                    "object signature does not verify under a binding-chain key"
                )
            }
            Self::DuplicateObject => write!(f, "duplicate object hash in export"),
            Self::BlobHashMismatch => {
                write!(f, "blob hash is not the SHA-256 of its bytes")
            }
            Self::UnreferencedBlob => {
                write!(f, "blob is not referenced by any verified object")
            }
            Self::MissingBlob => write!(f, "object cites a blob the export does not carry"),
        }
    }
}

impl std::error::Error for ExportError {}

/// What a successful [`verify_import`] established (§6.2 steps 1–3).
/// Ingest (step 4) proceeds under the platform's normal rules from
/// here; nothing in this type is a privileged write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedImport {
    /// The binding the export belongs to.
    pub binding_id: String,
    /// The binding's `sub` assertion, as attested by the service.
    pub sub: String,
    /// The binding's current Iroh root public key, hex.
    pub iroh_id: String,
    /// Verified object count.
    pub object_count: u64,
    /// Verified blob count.
    pub blob_count: u64,
    /// True when the export was a service-signed `unkeyed_export`
    /// accepted under operator confirmation.
    pub unkeyed_export: bool,
    /// The §7 choice in force: payloads are client-side ciphertext.
    pub client_side_encryption: bool,
}

/// SHA-256 of the concatenated object hash bytes, objects in
/// canonical (hash) order (§6.1 item 4).
fn objects_digest(objects: &[ExportObject]) -> String {
    let mut sorted: Vec<&ExportObject> = objects.iter().collect();
    sorted.sort_by(|a, b| a.hash.cmp(&b.hash));
    let mut hasher = Sha256::new();
    for object in sorted {
        if let Some(bytes) = hex_decode(&object.hash, 32) {
            hasher.update(bytes);
        }
    }
    hex_encode(&hasher.finalize())
}

/// Assemble an export (§6.1) from a hosted store's contents.
///
/// Objects are stored in canonical (hash) order and blobs in hash
/// order. When `user_key` is present it signs the manifest; when it
/// is `None` (the user's key is unavailable, e.g. during a lost-key
/// rebind wait) the service signs and the manifest is marked
/// `unkeyed_export`. The §7 flag is taken from the binding record —
/// it can only change by re-encrypting via export/import, so the
/// export always states the choice the data was actually stored
/// under.
#[must_use]
pub fn build_export(
    service: &HostedBindingService,
    binding: &BindingRecord,
    rebind_records: Vec<RebindRecord>,
    mut objects: Vec<ExportObject>,
    mut blobs: Vec<ExportBlob>,
    user_key: Option<&SigningKey>,
    export_id: impl Into<String>,
    created_at: impl Into<String>,
) -> HostedExport {
    objects.sort_by(|a, b| a.hash.cmp(&b.hash));
    blobs.sort_by(|a, b| a.hash.cmp(&b.hash));
    let mut manifest = ExportManifest {
        export_id: export_id.into(),
        binding_id: binding.binding_id.clone(),
        iroh_id: binding.iroh_id.clone(),
        created_at: created_at.into(),
        object_count: objects.len() as u64,
        blob_count: blobs.len() as u64,
        objects_digest: objects_digest(&objects),
        unkeyed_export: user_key.is_none(),
        client_side_encryption: binding.client_side_encryption,
        signature: String::new(),
    };
    let signature = match user_key {
        Some(key) => key.sign(&manifest.canonical_bytes()).to_bytes(),
        None => service.sign_service_bytes(&manifest.canonical_bytes()),
    };
    manifest.signature = hex_encode(&signature);
    HostedExport {
        manifest,
        objects,
        blobs,
        binding: binding.clone(),
        rebind_records,
        service_public_key: hex_encode(&service.service_public_key()),
    }
}

/// Verify an export before ingesting anything (§6.2 steps 1–3).
///
/// `confirm_untrusted` is the node's operator explicitly confirming
/// an untrusted-source import; it is the ONLY way an `unkeyed_export`
/// is accepted, and it never substitutes for any signature, chain,
/// or integrity check.
///
/// # Errors
///
/// Returns the [`ExportError`] naming the first rule that failed;
/// checks run in the spec's order (manifest, binding chain,
/// integrity) and a failure anywhere imports nothing.
pub fn verify_import(
    export: &HostedExport,
    confirm_untrusted: bool,
) -> Result<VerifiedImport, ExportError> {
    let manifest = &export.manifest;
    let binding = &export.binding;
    let Some(service_key_bytes) = hex_decode(&export.service_public_key, 32) else {
        return Err(ExportError::BindingAttestationInvalid);
    };
    let Ok(service_key) = <[u8; 32]>::try_from(service_key_bytes) else {
        return Err(ExportError::BindingAttestationInvalid);
    };

    // Step 1: the manifest names this binding, and its signature
    // verifies under the right key for its trust class.
    if manifest.binding_id != binding.binding_id || manifest.iroh_id != binding.iroh_id {
        return Err(ExportError::ManifestBindingMismatch);
    }
    if manifest.unkeyed_export {
        if !verify_hex_signature(
            &export.service_public_key,
            &manifest.canonical_bytes(),
            &manifest.signature,
        ) {
            return Err(ExportError::ManifestSignatureInvalid);
        }
        if !confirm_untrusted {
            return Err(ExportError::UnkeyedNotConfirmed);
        }
    } else if !verify_hex_signature(
        &binding.iroh_id,
        &manifest.canonical_bytes(),
        &manifest.signature,
    ) {
        return Err(ExportError::ManifestSignatureInvalid);
    }

    // Step 2: the binding chain. The transcript is reconstructed
    // from the exported record itself, never fetched.
    if !HostedBindingService::verify_key_signature(binding) {
        return Err(ExportError::BindingKeySignatureInvalid);
    }
    if !HostedBindingService::verify_service_attestation(&service_key, binding) {
        return Err(ExportError::BindingAttestationInvalid);
    }
    for record in &export.rebind_records {
        if record.binding_id != binding.binding_id {
            return Err(ExportError::RebindRecordMismatch);
        }
        if !HostedBindingService::verify_rebind_record(&service_key, record) {
            return Err(ExportError::HistoryUnbacked {
                iroh_id: record.old_iroh_id.clone(),
            });
        }
    }
    for entry in &binding.history {
        if entry.via == "delayed-rebind" {
            let backed = entry.rebind_record_id.as_ref().is_some_and(|id| {
                export
                    .rebind_records
                    .iter()
                    .any(|r| &r.rebind_id == id && r.old_iroh_id == entry.iroh_id)
            });
            if !backed {
                return Err(ExportError::HistoryUnbacked {
                    iroh_id: entry.iroh_id.clone(),
                });
            }
        }
        // `old-key-signature` steps are backed by the rebind record's
        // countersignature chain when a record is included (verified
        // above); the fast path itself is the old key's signature
        // over the transfer, checked when the record was published.
    }

    // Step 3: integrity — counts and digest over the received set,
    // every object's hash and signature, and every blob's hash
    // against the references of verified objects, all before ingest.
    if manifest.object_count != export.objects.len() as u64
        || manifest.blob_count != export.blobs.len() as u64
    {
        return Err(ExportError::CountMismatch);
    }
    if objects_digest(&export.objects) != manifest.objects_digest {
        return Err(ExportError::ObjectsDigestMismatch);
    }
    let mut chain_keys: Vec<&str> = vec![binding.iroh_id.as_str()];
    chain_keys.extend(binding.history.iter().map(|e| e.iroh_id.as_str()));
    let mut seen_objects = std::collections::HashSet::new();
    let mut referenced_blobs = std::collections::HashSet::new();
    for object in &export.objects {
        if !seen_objects.insert(object.hash.clone()) {
            return Err(ExportError::DuplicateObject);
        }
        if sha256_hex(&object.bytes) != object.hash {
            return Err(ExportError::ObjectHashMismatch);
        }
        let Some(hash_bytes) = hex_decode(&object.hash, 32) else {
            return Err(ExportError::ObjectHashMismatch);
        };
        if !chain_keys.contains(&object.signer.as_str())
            || !verify_hex_signature(&object.signer, &hash_bytes, &object.signature)
        {
            return Err(ExportError::ObjectSignatureInvalid);
        }
        for blob_ref in &object.blob_refs {
            referenced_blobs.insert(blob_ref.clone());
        }
    }
    let mut carried_blobs = std::collections::HashSet::new();
    for blob in &export.blobs {
        if sha256_hex(&blob.bytes) != blob.hash {
            return Err(ExportError::BlobHashMismatch);
        }
        carried_blobs.insert(blob.hash.clone());
    }
    for blob in &export.blobs {
        if !referenced_blobs.contains(&blob.hash) {
            return Err(ExportError::UnreferencedBlob);
        }
    }
    for referenced in &referenced_blobs {
        if !carried_blobs.contains(referenced) {
            return Err(ExportError::MissingBlob);
        }
    }

    Ok(VerifiedImport {
        binding_id: binding.binding_id.clone(),
        sub: binding.sub.clone(),
        iroh_id: binding.iroh_id.clone(),
        object_count: manifest.object_count,
        blob_count: manifest.blob_count,
        unkeyed_export: manifest.unkeyed_export,
        client_side_encryption: manifest.client_side_encryption,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUB: &str = "google-subject-1";
    const CREATED: &str = "2026-11-13T00:00:00Z";

    fn root_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn service() -> HostedBindingService {
        HostedBindingService::with_seed("hosted.example", [9u8; 32])
    }

    fn bind(service: &mut HostedBindingService, seed: u8, cse: Option<bool>) -> BindingRecord {
        service.signup_account(SUB, None, "free").unwrap();
        service
            .complete_browser_ceremony(SUB, &root_key(seed), cse)
            .unwrap()
    }

    fn sample_export(cse: Option<bool>) -> (HostedBindingService, HostedExport) {
        let mut svc = service();
        let binding = bind(&mut svc, 21, cse);
        let blob = ExportBlob::new(b"blob payload".to_vec());
        let object = ExportObject::new(
            &root_key(21),
            b"object payload".to_vec(),
            vec![blob.hash.clone()],
        );
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            vec![blob],
            Some(&root_key(21)),
            "export-1",
            CREATED,
        );
        (svc, export)
    }

    #[test]
    fn export_import_round_trip() {
        let (_svc, export) = sample_export(None);
        let verified = verify_import(&export, false).unwrap();
        assert_eq!(verified.binding_id, export.binding.binding_id);
        assert_eq!(verified.sub, SUB);
        assert_eq!(verified.object_count, 1);
        assert_eq!(verified.blob_count, 1);
        assert!(!verified.unkeyed_export);
        // CSE defaults ON at binding (#2046 ruling) and the export
        // states the choice the data was stored under (§7).
        assert!(verified.client_side_encryption);
    }

    #[test]
    fn cse_choice_is_threaded_from_the_binding() {
        let (_svc, export) = sample_export(Some(false));
        assert!(!export.manifest.client_side_encryption);
        let verified = verify_import(&export, false).unwrap();
        assert!(!verified.client_side_encryption);
    }

    #[test]
    fn tampered_manifest_is_refused() {
        let (_svc, mut export) = sample_export(None);
        export.manifest.created_at = "2026-01-01T00:00:00Z".to_string();
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ManifestSignatureInvalid)
        );
    }

    #[test]
    fn tampered_object_is_refused() {
        let (_svc, mut export) = sample_export(None);
        export.objects[0].bytes = b"forged payload".to_vec();
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ObjectHashMismatch)
        );
    }

    #[test]
    fn substituted_blob_is_refused() {
        // Object signatures and the objects digest do not cover blob
        // bytes; the blob hash check (§6.2 step 3) is what catches a
        // swapped blob.
        let (_svc, mut export) = sample_export(None);
        export.blobs[0].bytes = b"substituted".to_vec();
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::BlobHashMismatch)
        );
    }

    #[test]
    fn unreferenced_blob_is_refused() {
        let (_svc, mut export) = sample_export(None);
        let extra = ExportBlob::new(b"smuggled".to_vec());
        export.blobs.push(extra);
        export.manifest.blob_count = 2;
        // Re-sign so only the blob rule can fail.
        export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::UnreferencedBlob)
        );
    }

    #[test]
    fn unkeyed_export_needs_operator_confirmation() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let object = ExportObject::new(&root_key(21), b"object".to_vec(), Vec::new());
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            Vec::new(),
            None,
            "export-unkeyed",
            CREATED,
        );
        assert!(export.manifest.unkeyed_export);
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::UnkeyedNotConfirmed)
        );
        let verified = verify_import(&export, true).unwrap();
        assert!(verified.unkeyed_export);
    }

    #[test]
    fn unkeyed_export_with_forged_service_signature_is_refused() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let mut export = build_export(
            &svc,
            &binding,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            "export-unkeyed",
            CREATED,
        );
        export.manifest.signature = hex_encode(
            &root_key(99)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&export, true),
            Err(ExportError::ManifestSignatureInvalid)
        );
    }

    #[test]
    fn object_from_a_key_outside_the_chain_is_refused() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let foreign = ExportObject::new(&root_key(77), b"foreign".to_vec(), Vec::new());
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![foreign],
            Vec::new(),
            Some(&root_key(21)),
            "export-foreign",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ObjectSignatureInvalid)
        );
    }
}
