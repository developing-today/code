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
//!   An object's bytes are a stored signed envelope (a p2panda
//!   header, the graph's native object form) carried verbatim; its
//!   hash is SHA-256 of those bytes and its signature is the
//!   envelope's own signature, over the canonical header encoding,
//!   by the envelope's author. A blob's hash is SHA-256 of its
//!   bytes. Objects carry the blob hashes they cite.
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
//! ## Production (slice 4b, #13157)
//!
//! The enumeration of a live per-user store and R2 blob fetching
//! live in the hosted data plane: `hosted-worker/src/export.ts`
//! enumerates the DO's applied events and fetches the blobs they
//! cite, and the DO's `GET /v1/bridge/export` route serves the sets
//! in the wire shape of these types for [`build_export`] to
//! assemble. [`ExportObject::from_envelope`] is the same mapping on
//! this side. The object-model mapping the wiring settles —
//! envelope bytes verbatim, SHA-256 as the export-layer digest,
//! the envelope's own signature as the object's signature, blob
//! citations resolved by content from BLAKE3 to SHA-256 — is
//! recorded in the spec as §6.3. Ingest itself (§6.2 step 4) lives
//! in [`crate::hosted_ingest`], downstream of a verified import.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
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
    for pair in bytes.as_chunks::<2>().0 {
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
/// authenticating by construction. `bytes` is the stored signed
/// envelope verbatim (never re-encoded, never re-signed), `hash`
/// is SHA-256 of `bytes`, and `signature` is the envelope's own
/// signature over the canonical header encoding, by `signer`, the
/// envelope's author (§6.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportObject {
    /// SHA-256 of `bytes`, hex.
    pub hash: String,
    /// The producing key (an Iroh root key in the binding chain), hex.
    pub signer: String,
    /// The envelope's own signature, hex.
    pub signature: String,
    /// Hashes of the blobs this object cites (§6.2 step 3), hex:
    /// SHA-256 names, resolved from the envelope's BLAKE3 content
    /// citations by content (§6.3).
    pub blob_refs: Vec<String>,
    /// The object bytes: the stored envelope, verbatim.
    pub bytes: Vec<u8>,
}

impl ExportObject {
    /// Map a stored envelope to its export object (§6.3). The
    /// envelope is decoded exactly as [`crate::chain::decode`]
    /// does, which verifies its signature; the author and signature
    /// are read from the envelope itself — nothing is re-signed,
    /// and a stored form that does not decode is refused rather
    /// than carried.
    ///
    /// `blob_refs` are the object's citations already resolved to
    /// SHA-256 blob names by content (see
    /// [`cited_content_hashes`]); resolving them needs the store's
    /// blobs, so it is the producer's step, not this mapping's.
    ///
    /// # Errors
    ///
    /// [`ExportError::ObjectSignatureInvalid`] when the bytes are
    /// not a verifiable signed envelope.
    pub fn from_envelope(bytes: Vec<u8>, blob_refs: Vec<String>) -> Result<Self, ExportError> {
        let event =
            crate::chain::decode(&bytes).map_err(|_| ExportError::ObjectSignatureInvalid)?;
        Ok(Self {
            hash: sha256_hex(&bytes),
            signer: hex_encode(event.header.verifying_key.as_bytes()),
            signature: hex_encode(&event.header.signature.to_bytes()),
            blob_refs,
            bytes,
        })
    }
}

/// The BLAKE3 content hashes an envelope cites: the `content` of
/// its document payload (`document.put`), and nothing else — in
/// the current `ChainExt` form one envelope cites at most one
/// content. The producer resolves each citation to the SHA-256
/// name of the blob whose bytes hash (BLAKE3) to it (§6.3).
///
/// # Errors
///
/// [`ExportError::ObjectSignatureInvalid`] when the bytes are not
/// a verifiable signed envelope.
pub fn cited_content_hashes(bytes: &[u8]) -> Result<Vec<String>, ExportError> {
    let event = crate::chain::decode(bytes).map_err(|_| ExportError::ObjectSignatureInvalid)?;
    Ok(match &event.header.extensions.document {
        Some(document) => vec![document.content.to_hex()],
        None => Vec::new(),
    })
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
    /// The manifest's §7 client-side-encryption choice differs from
    /// the binding record's. The flag can only change by
    /// re-encrypting via export/import, so an export whose halves
    /// disagree does not describe one stored data set (§6.2 step 1,
    /// §7).
    CseChoiceMismatch,
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
    /// An object's declared blob references do not match the
    /// content citations of its own verified envelope (§6.2
    /// step 3): a reference naming a carried blob whose bytes do
    /// not hash (BLAKE3) to one of the envelope's citations, or a
    /// citation whose resolved blob the references omit.
    /// `blob_refs` is covered by neither the envelope signature
    /// nor the manifest digest, so verification derives the
    /// citations from the envelope itself (#13289).
    BlobCitationMismatch,
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
            Self::CseChoiceMismatch => {
                write!(
                    f,
                    "manifest's client-side-encryption choice differs from the binding record's"
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
            Self::BlobCitationMismatch => {
                write!(
                    f,
                    "object's blob references do not match its envelope's content citations"
                )
            }
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
    // Step 1, §7: the manifest's CSE choice is part of naming the
    // binding's stored state. It is covered by the manifest
    // signature (canonical_bytes), and the binding record's own
    // flag is attested by the service; disagreement means the two
    // halves do not describe one stored data set — an export
    // stating the wrong choice misdirects every downstream reader
    // of the payloads (§7: the flag changes only by re-encrypting
    // via export/import).
    if manifest.client_side_encryption != binding.client_side_encryption {
        return Err(ExportError::CseChoiceMismatch);
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
    // every object's hash and signature, every blob's hash, and
    // every object's references bound to the content citations of
    // its own verified envelope, all before ingest.
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
    // Blob bytes are verified and indexed by citation identity
    // (BLAKE3 of the bytes, the name an envelope cites) before any
    // object is checked: an object's references are only
    // meaningful resolved against the content its envelope
    // actually cites (#13289).
    let mut blob_names_by_citation = std::collections::HashMap::new();
    let mut carried_blobs = std::collections::HashSet::new();
    for blob in &export.blobs {
        if sha256_hex(&blob.bytes) != blob.hash {
            return Err(ExportError::BlobHashMismatch);
        }
        carried_blobs.insert(blob.hash.clone());
        blob_names_by_citation.insert(
            p2panda_core::Hash::digest(&blob.bytes).to_hex(),
            blob.hash.clone(),
        );
    }
    let mut seen_objects = std::collections::HashSet::new();
    let mut referenced_blobs = std::collections::HashSet::new();
    for object in &export.objects {
        if !seen_objects.insert(object.hash.clone()) {
            return Err(ExportError::DuplicateObject);
        }
        if sha256_hex(&object.bytes) != object.hash {
            return Err(ExportError::ObjectHashMismatch);
        }
        // The object's signature verifies in the object's own
        // scheme (§6.3): decoding the envelope as chain.rs does
        // verifies its signature over the canonical header
        // encoding; the decoded author must be the named signer, a
        // binding-chain key, and the decoded signature must be the
        // carried one.
        let event =
            crate::chain::decode(&object.bytes).map_err(|_| ExportError::ObjectSignatureInvalid)?;
        if hex_encode(event.header.verifying_key.as_bytes()) != object.signer
            || !chain_keys.contains(&object.signer.as_str())
            || hex_encode(&event.header.signature.to_bytes()) != object.signature
        {
            return Err(ExportError::ObjectSignatureInvalid);
        }
        // Bind the object's references to its envelope's own
        // citations, read from the decoded header above (the same
        // source `cited_content_hashes` reads): the references
        // must name exactly the carried blobs whose bytes hash to
        // the cited contents — no decoy, no extra, none omitted.
        let declared_refs: std::collections::HashSet<String> =
            object.blob_refs.iter().cloned().collect();
        // A reference naming a blob the export does not carry at
        // all is the plain missing-blob refusal, checked before
        // citation binding so a dangling reference is not masked
        // as a citation mismatch (#13444).
        if declared_refs.iter().any(|r| !carried_blobs.contains(r)) {
            return Err(ExportError::MissingBlob);
        }
        let cited: Vec<String> = match &event.header.extensions.document {
            Some(document) => vec![document.content.to_hex()],
            None => Vec::new(),
        };
        let mut resolved_refs = std::collections::HashSet::new();
        let mut resolved_count = 0usize;
        for citation in &cited {
            if let Some(name) = blob_names_by_citation.get(citation) {
                resolved_refs.insert(name.clone());
                resolved_count += 1;
            }
        }
        if declared_refs != resolved_refs {
            return Err(ExportError::BlobCitationMismatch);
        }
        if resolved_count != cited.len() {
            return Err(ExportError::MissingBlob);
        }
        for blob_ref in &object.blob_refs {
            referenced_blobs.insert(blob_ref.clone());
        }
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::hosted_binding::{
        Challenge, REBIND_VIA_DELAYED, REBIND_VIA_OLD_KEY, parse_rfc3339,
        rebind_authorization_bytes,
    };

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

    fn panda_key(key: &SigningKey) -> p2panda_core::SigningKey {
        p2panda_core::SigningKey::from_bytes(&key.to_bytes())
    }

    fn ext(kind: &str, document: Option<crate::documents::DocumentRef>) -> crate::chain::ChainExt {
        crate::chain::ChainExt {
            kind: kind.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: None,
        }
    }

    /// A signed envelope authored by `key`, in the form the store
    /// holds it.
    fn envelope(
        key: &SigningKey,
        seq: u32,
        backlink: Option<p2panda_core::Hash>,
        ext: crate::chain::ChainExt,
    ) -> crate::chain::Event {
        crate::chain::sign(&panda_key(key), seq, backlink, None, ext).unwrap()
    }

    fn message_object(key: &SigningKey) -> ExportObject {
        let event = envelope(key, 0, None, ext("message", None));
        ExportObject::from_envelope(event.bytes, Vec::new()).unwrap()
    }

    /// A `document.put` object citing `content`, with the citation
    /// resolved to `blob`'s SHA-256 name as the producer resolves
    /// it (§6.3): the envelope cites the content's BLAKE3, the
    /// export names the blob by SHA-256.
    fn document_object(key: &SigningKey, blob: &ExportBlob, content: &[u8]) -> ExportObject {
        let document = crate::documents::DocumentRef::for_bytes("text/plain", content).unwrap();
        let event = envelope(key, 0, None, ext("document.put", Some(document)));
        assert_eq!(
            cited_content_hashes(&event.bytes).unwrap(),
            vec![p2panda_core::Hash::digest(content).to_hex()]
        );
        ExportObject::from_envelope(event.bytes, vec![blob.hash.clone()]).unwrap()
    }

    fn sample_export(cse: Option<bool>) -> (HostedBindingService, HostedExport) {
        let mut svc = service();
        let binding = bind(&mut svc, 21, cse);
        let content = b"blob payload";
        let blob = ExportBlob::new(content.to_vec());
        let object = document_object(&root_key(21), &blob, content);
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
    fn manifest_cse_choice_mismatching_the_binding_is_refused() {
        // §7: the flag can only change by re-encrypting via
        // export/import, so a manifest stating a different choice
        // than the binding record it travels with does not name
        // that binding (§6.2 step 1). Stating encryption under a
        // binding that stored plaintext would decrypt wrong;
        // stating plaintext under an encrypted binding downgrades
        // opaque payloads — the import refuses, and ingests
        // nothing.
        let (_svc, mut export) = sample_export(Some(false));
        assert!(!export.binding.client_side_encryption);
        export.manifest.client_side_encryption = true;
        // Re-sign so only the choice rule can fail: this is the
        // misassembled pair (or the re-signed forgery), not a
        // tampered manifest.
        export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::CseChoiceMismatch)
        );
        // The other direction — CSE binding claimed as plaintext —
        // is refused the same way.
        let (_svc, mut cse_export) = sample_export(None);
        assert!(cse_export.binding.client_side_encryption);
        cse_export.manifest.client_side_encryption = false;
        cse_export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&cse_export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&cse_export, false),
            Err(ExportError::CseChoiceMismatch)
        );
    }

    #[test]
    fn flipping_the_manifest_cse_flag_without_resigning_is_refused() {
        // The CSE choice is inside the manifest's canonical bytes,
        // so a flag flip without the key fails the signature first
        // — the choice is authenticated, not decorative.
        let (_svc, mut export) = sample_export(None);
        export.manifest.client_side_encryption = false;
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ManifestSignatureInvalid)
        );
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
    fn forged_object_signature_is_refused() {
        // A valid envelope whose carried signature field names a
        // different signature: the field is not decorative (§6.3).
        let (_svc, mut export) = sample_export(None);
        export.objects[0].signature = hex_encode(&[7u8; 64]);
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ObjectSignatureInvalid)
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
        let object = message_object(&root_key(21));
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
        let foreign = message_object(&root_key(77));
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

    #[test]
    fn envelope_mapping_is_verbatim() {
        // §6.3: the export object is the stored envelope — hash is
        // SHA-256 of the envelope bytes, signer its author,
        // signature the envelope's own; nothing re-signed.
        let key = root_key(21);
        let event = envelope(&key, 0, None, ext("message", None));
        let object = ExportObject::from_envelope(event.bytes.clone(), Vec::new()).unwrap();
        assert_eq!(object.bytes, event.bytes);
        assert_eq!(object.hash, sha256_hex(&event.bytes));
        assert_eq!(object.signer, hex_encode(&key.verifying_key().to_bytes()));
        assert_eq!(
            object.signature,
            hex_encode(&event.header.signature.to_bytes())
        );
        assert!(cited_content_hashes(&event.bytes).unwrap().is_empty());
        // Bytes that are not a verifiable envelope are refused.
        assert_eq!(
            ExportObject::from_envelope(b"not an envelope".to_vec(), Vec::new()),
            Err(ExportError::ObjectSignatureInvalid)
        );
    }

    #[test]
    fn produced_sets_fixture_verifies() {
        // The cross-language contract (slice 4b): the hosted-worker
        // suite seeds a per-user DO from the seed-7 key and pins
        // the produced sets to this fixture; here the same sets,
        // assembled with a binding whose root key is that key, must
        // pass verify_import with matching counts and digest.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/hosted-worker/test/fixtures/hosted-export-sets.json"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let wire: serde_json::Value = serde_json::from_str(&text).unwrap();
        let objects: Vec<ExportObject> = serde_json::from_value(wire["objects"].clone()).unwrap();
        let blobs: Vec<ExportBlob> = serde_json::from_value(wire["blobs"].clone()).unwrap();

        let mut svc = service();
        let binding = bind(&mut svc, 7, None);
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            objects,
            blobs,
            Some(&root_key(7)),
            "export-fixture",
            CREATED,
        );
        assert_eq!(
            export.manifest.objects_digest,
            wire["objects_digest"].as_str().unwrap()
        );
        let verified = verify_import(&export, false).unwrap();
        assert_eq!(
            verified.object_count,
            wire["object_count"].as_u64().unwrap()
        );
        assert_eq!(verified.blob_count, wire["blob_count"].as_u64().unwrap());
        assert_eq!(verified.object_count, 4);
        assert_eq!(verified.blob_count, 2);
    }

    #[test]
    fn decoy_blob_is_refused() {
        // #13289: the envelope cites the real content; the export
        // carries unrelated decoy bytes and names them in
        // `blob_refs`. `blob_refs` is covered by neither the
        // envelope signature nor the manifest digest, so trusting
        // it lets a signed object pass verification while its cited
        // content is absent. Verification must resolve the citation
        // against blob bytes itself.
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let decoy = ExportBlob::new(b"decoy bytes".to_vec());
        let object = document_object(&root_key(21), &decoy, b"the real content");
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            vec![decoy],
            Some(&root_key(21)),
            "export-decoy",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::BlobCitationMismatch)
        );
    }

    #[test]
    fn cited_blob_missing_is_refused() {
        // The object cites content the export does not carry at
        // all: no blob bytes hash to the envelope's citation.
        let (_svc, mut export) = sample_export(None);
        export.blobs.clear();
        export.objects[0].blob_refs.clear();
        export.manifest.blob_count = 0;
        // Re-sign so only the blob rule can fail.
        export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(verify_import(&export, false), Err(ExportError::MissingBlob));
    }

    #[test]
    fn extra_blob_reference_is_refused() {
        // One object's `blob_refs` also name a second object's
        // blob: a reference its own envelope never makes. The
        // manifest digest covers object hashes, not references, so
        // only citation binding catches the inconsistency.
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let blob_a = ExportBlob::new(b"content a".to_vec());
        let blob_b = ExportBlob::new(b"content b".to_vec());
        let mut object_a = document_object(&root_key(21), &blob_a, b"content a");
        object_a.blob_refs.push(blob_b.hash.clone());
        let object_b = document_object(&root_key(21), &blob_b, b"content b");
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object_a, object_b],
            vec![blob_a, blob_b],
            Some(&root_key(21)),
            "export-extra-ref",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::BlobCitationMismatch)
        );
    }

    #[test]
    fn rebind_history_export_verifies() {
        // A store with a rebind history: objects authored by the
        // old root key and by the current one both verify, backed
        // by the rebind record (§6.2 step 2, §4.4).
        use crate::hosted_binding::{REBIND_VIA_DELAYED, RebindRecord};
        const T0: u64 = 1_700_000_000;
        let mut svc = service();
        svc.set_now_override(Some(T0));
        let binding = bind(&mut svc, 21, None);
        let new_key = root_key(22);

        let challenge = svc
            .request_challenge(SUB, &new_key.verifying_key().to_bytes())
            .unwrap();
        let proof = new_key.sign(&HostedBindingService::canonical_transcript(&challenge));
        let (request, _) = svc
            .request_delayed_rebind(SUB, &challenge.challenge_id, &proof.to_bytes())
            .unwrap();
        let probe = RebindRecord {
            rebind_id: String::new(),
            binding_id: binding.binding_id.clone(),
            old_iroh_id: binding.iroh_id.clone(),
            new_iroh_id: hex_encode(&new_key.verifying_key().to_bytes()),
            effective_at: String::new(),
            via: REBIND_VIA_DELAYED.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: String::new(),
        };
        let counter = new_key.sign(&probe.countersign_bytes()).to_bytes();
        svc.set_now_override(Some(T0 + crate::hosted_binding::REBIND_WAITING_PERIOD_SECS));
        let (updated, record) = svc.effect_due_rebind(&request.rebind_id, &counter).unwrap();
        assert_eq!(updated.history.len(), 1);

        let objects = vec![message_object(&root_key(21)), message_object(&new_key)];
        let export = build_export(
            &svc,
            &updated,
            vec![record],
            objects,
            Vec::new(),
            Some(&new_key),
            "export-rebind",
            CREATED,
        );
        let verified = verify_import(&export, false).unwrap();
        assert_eq!(verified.object_count, 2);
        assert_eq!(verified.blob_count, 0);
        assert_eq!(verified.iroh_id, updated.iroh_id);
    }
    // -- binding-record history (§6.2 step 2) -------------------------------

    const OTHER_SUB: &str = "other-subject-9";

    /// A fresh §3 challenge for `key` on `sub`, with the proof signature.
    fn challenged(
        service: &mut HostedBindingService,
        sub: &str,
        key: &SigningKey,
    ) -> (Challenge, [u8; 64]) {
        let iroh_id = key.verifying_key().to_bytes();
        let challenge = service.request_challenge(sub, &iroh_id).unwrap();
        let proof = key
            .sign(&HostedBindingService::canonical_transcript(&challenge))
            .to_bytes();
        (challenge, proof)
    }

    /// Drive a delayed rebind to effect: request, wait out the 30-day
    /// period, countersign, effect. Returns the updated binding (whose
    /// history names the old key) and the published rebind record.
    fn rebind_delayed(
        service: &mut HostedBindingService,
        sub: &str,
        new_key: &SigningKey,
    ) -> (BindingRecord, RebindRecord) {
        let (challenge, proof) = challenged(service, sub, new_key);
        let (request, _) = service
            .request_delayed_rebind(sub, &challenge.challenge_id, &proof)
            .unwrap();
        let effective = parse_rfc3339(&request.effective_at).unwrap();
        service.set_now_override(Some(effective + 1));
        let probe = RebindRecord {
            rebind_id: String::new(),
            binding_id: request.binding_id.clone(),
            old_iroh_id: request.old_iroh_id.clone(),
            new_iroh_id: request.new_iroh_id.clone(),
            effective_at: String::new(),
            via: REBIND_VIA_DELAYED.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: String::new(),
        };
        let countersignature = new_key.sign(&probe.countersign_bytes()).to_bytes();
        service
            .effect_due_rebind(&request.rebind_id, &countersignature)
            .unwrap()
    }

    /// Drive the §4.1 fast path: the old key authorizes the transfer from
    /// a fresh challenge and the new key countersigns. Effect is
    /// immediate; the history step needs no backing record in the export.
    fn rebind_fast(
        service: &mut HostedBindingService,
        sub: &str,
        old_key: &SigningKey,
        new_key: &SigningKey,
    ) -> (BindingRecord, RebindRecord) {
        let binding = service.binding_for_sub(sub).unwrap().clone();
        let (challenge, proof) = challenged(service, sub, new_key);
        let auth = old_key
            .sign(&rebind_authorization_bytes(
                &binding.binding_id,
                &challenge.iroh_id,
                &challenge.challenge_id,
            ))
            .to_bytes();
        let probe = RebindRecord {
            rebind_id: String::new(),
            binding_id: binding.binding_id.clone(),
            old_iroh_id: binding.iroh_id.clone(),
            new_iroh_id: hex_encode(&new_key.verifying_key().to_bytes()),
            effective_at: String::new(),
            via: REBIND_VIA_OLD_KEY.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: String::new(),
        };
        let countersignature = new_key.sign(&probe.countersign_bytes()).to_bytes();
        service
            .rebind_with_old_key_authorization(
                sub,
                None,
                &challenge.challenge_id,
                &proof,
                &auth,
                &countersignature,
            )
            .unwrap()
    }

    /// Flip the last hex digit of `hex` to a different valid hex digit,
    /// so the value stays well-formed but no longer matches.
    fn flip_last_hex(hex: &str) -> String {
        let mut out = hex.to_owned();
        let last = out.pop().unwrap();
        out.push(if last == '0' { '1' } else { '0' });
        out
    }

    /// An export object over a freshly signed envelope by `key`
    /// (§6.3: the envelope is the object; the document content
    /// keeps objects from the same key distinct, and `blob_refs`
    /// is the export-layer citation set the verifier checks
    /// against the carried blobs).
    fn signed_object(key: &SigningKey, content: &[u8], blob_refs: Vec<String>) -> ExportObject {
        let document = crate::documents::DocumentRef::for_bytes("text/plain", content).unwrap();
        let event = envelope(key, 0, None, ext("document.put", Some(document)));
        ExportObject::from_envelope(event.bytes, blob_refs).unwrap()
    }

    #[test]
    fn delayed_rebind_history_round_trip() {
        let mut svc = service();
        let old_key = root_key(21);
        let new_key = root_key(22);
        bind(&mut svc, 21, None);
        let (binding, record) = rebind_delayed(&mut svc, SUB, &new_key);
        assert_eq!(binding.history.len(), 1);
        assert_eq!(binding.history[0].via, "delayed-rebind");

        // Objects from both sides of the rotation stay verifiable: the
        // binding chain in step 2 admits every key in the history.
        let blob = ExportBlob::new(b"written before the rebind".to_vec());
        let old_object = document_object(&old_key, &blob, b"written before the rebind");
        let new_object = message_object(&new_key);
        let export = build_export(
            &svc,
            &binding,
            vec![record],
            vec![old_object, new_object],
            vec![blob],
            Some(&new_key),
            "export-rebound",
            CREATED,
        );
        let verified = verify_import(&export, false).unwrap();
        assert_eq!(verified.object_count, 2);
        assert_eq!(verified.blob_count, 1);
        assert_eq!(verified.iroh_id, binding.iroh_id);
    }

    #[test]
    fn fast_path_history_verifies_without_backing_records() {
        let mut svc = service();
        let old_key = root_key(21);
        let new_key = root_key(22);
        bind(&mut svc, 21, None);
        let (binding, _record) = rebind_fast(&mut svc, SUB, &old_key, &new_key);
        assert_eq!(binding.history[0].via, "old-key-signature");
        // The fast path is the old key's signature over the transfer,
        // checked at publication: the export carries no backing record.
        let object = message_object(&old_key);
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            Vec::new(),
            Some(&new_key),
            "export-fast",
            CREATED,
        );
        verify_import(&export, false).unwrap();
    }

    #[test]
    fn delayed_history_without_backing_record_is_refused() {
        let mut svc = service();
        bind(&mut svc, 21, None);
        let (binding, _record) = rebind_delayed(&mut svc, SUB, &root_key(22));
        let object = signed_object(&root_key(22), b"post-rotation", Vec::new());
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            Vec::new(),
            Some(&root_key(22)),
            "export-unbacked",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::HistoryUnbacked {
                iroh_id: binding.history[0].iroh_id.clone(),
            })
        );
    }

    #[test]
    fn rebind_record_for_another_binding_is_refused() {
        let mut svc = service();
        bind(&mut svc, 21, None);
        let (binding, mut record) = rebind_delayed(&mut svc, SUB, &root_key(22));
        // A well-formed, service-signed record that belongs to a
        // different binding: only the binding-ID check can catch it.
        record.binding_id = "binding-from-elsewhere".to_owned();
        let export = build_export(
            &svc,
            &binding,
            vec![record],
            Vec::new(),
            Vec::new(),
            Some(&root_key(22)),
            "export-foreign-record",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::RebindRecordMismatch)
        );
    }

    #[test]
    fn tampered_rebind_record_is_refused() {
        let mut svc = service();
        bind(&mut svc, 21, None);
        let (binding, mut record) = rebind_delayed(&mut svc, SUB, &root_key(22));
        record.service_signature = flip_last_hex(&record.service_signature);
        let export = build_export(
            &svc,
            &binding,
            vec![record],
            Vec::new(),
            Vec::new(),
            Some(&root_key(22)),
            "export-tampered-record",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::HistoryUnbacked {
                iroh_id: binding.history[0].iroh_id.clone(),
            })
        );
    }

    // -- remaining step 1–3 refusals ----------------------------------------

    #[test]
    fn manifest_for_another_binding_is_refused() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        svc.signup_account(OTHER_SUB, None, "free").unwrap();
        let other = svc
            .complete_browser_ceremony(OTHER_SUB, &root_key(23), None)
            .unwrap();
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(&root_key(21)),
            "export-mismatch",
            CREATED,
        );
        let mut swapped = export;
        swapped.binding = other;
        assert_eq!(
            verify_import(&swapped, false),
            Err(ExportError::ManifestBindingMismatch)
        );
    }

    #[test]
    fn tampered_binding_key_signature_is_refused() {
        let (_svc, mut export) = sample_export(None);
        export.binding.key_signature = flip_last_hex(&export.binding.key_signature);
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::BindingKeySignatureInvalid)
        );
    }

    #[test]
    fn tampered_service_attestation_is_refused() {
        let (_svc, mut export) = sample_export(None);
        export.binding.service_attestation = flip_last_hex(&export.binding.service_attestation);
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::BindingAttestationInvalid)
        );
    }

    #[test]
    fn count_mismatch_is_refused() {
        let (_svc, mut export) = sample_export(None);
        // One more object than the manifest counts: the manifest
        // signature still verifies, so only the count rule can fail.
        export
            .objects
            .push(signed_object(&root_key(21), b"smuggled", Vec::new()));
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::CountMismatch)
        );
    }

    #[test]
    fn stale_objects_digest_is_refused() {
        let (_svc, mut export) = sample_export(None);
        // Swap in a different well-formed object from the same key:
        // counts still match, but the manifest digest is stale.
        export.objects[0] = signed_object(&root_key(21), b"replacement", Vec::new());
        export.blobs.clear();
        export.manifest.blob_count = 0;
        // Re-sign over the unchanged (now stale) counts and digest.
        export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::ObjectsDigestMismatch)
        );
    }

    #[test]
    fn duplicate_object_is_refused() {
        let (_svc, mut export) = sample_export(None);
        export.objects.push(export.objects[0].clone());
        export.manifest.object_count = 2;
        export.manifest.objects_digest = objects_digest(&export.objects);
        export.manifest.signature = hex_encode(
            &root_key(21)
                .sign(&export.manifest.canonical_bytes())
                .to_bytes(),
        );
        assert_eq!(
            verify_import(&export, false),
            Err(ExportError::DuplicateObject)
        );
    }

    #[test]
    fn missing_blob_is_refused() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        // The object cites a blob the export does not carry.
        let object = signed_object(&root_key(21), b"cites the void", vec!["ab".repeat(32)]);
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![object],
            Vec::new(),
            Some(&root_key(21)),
            "export-missing-blob",
            CREATED,
        );
        assert_eq!(verify_import(&export, false), Err(ExportError::MissingBlob));
    }

    // -- canonical form and the wire ----------------------------------------

    #[test]
    fn export_is_canonical_and_survives_the_wire() {
        let mut svc = service();
        let binding = bind(&mut svc, 21, None);
        let blob_b = ExportBlob::new(b"bbb".to_vec());
        let blob_a = ExportBlob::new(b"a".to_vec());
        let obj_b = signed_object(&root_key(21), b"bbb", vec![blob_b.hash.clone()]);
        let obj_a = signed_object(&root_key(21), b"a", vec![blob_a.hash.clone()]);
        // Deliberately scrambled input order.
        let export = build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![obj_b, obj_a],
            vec![blob_b, blob_a],
            Some(&root_key(21)),
            "export-canonical",
            CREATED,
        );
        // Stored in canonical (hash) order regardless of input order.
        assert!(export.objects.windows(2).all(|w| w[0].hash <= w[1].hash));
        assert!(export.blobs.windows(2).all(|w| w[0].hash <= w[1].hash));
        // Identical by canonical object/hash set after a JSON round trip.
        let wire = serde_json::to_string(&export).unwrap();
        let received: HostedExport = serde_json::from_str(&wire).unwrap();
        assert_eq!(received, export);
        let verified = verify_import(&received, false).unwrap();
        assert_eq!(verified.object_count, 2);
        assert_eq!(verified.blob_count, 2);
    }
}
