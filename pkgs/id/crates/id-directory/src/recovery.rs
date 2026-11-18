//! Guardian recovery: the annex records and Rule R (#2382).
//!
//! Implements the record formats and the successor-key rule of
//! `doc/2026-11-16T11-00-00Z_design_guardian_recovery`. A user names
//! guardians and a threshold in the keyset (`GuardianSetUpdated`). To
//! recover, a brand-new successor key publishes a [`AnnexEntry::RecoveryRequest`]
//! pinned to a keyset head; guardians sign [`AnnexEntry::RecoveryApproval`]s;
//! after [`RECOVERY_DELAY_SECONDS`] with no [`AnnexEntry::RecoveryCancellation`],
//! the successor signs a `RecoveryRotation` chain record that installs
//! it and revokes every prior key.
//!
//! Annex records sit beside the keyset chain, not in it: a request is
//! signed by a key that has no authority yet, so it could never be a
//! valid chain continuation. They confer nothing by existing; they are
//! evidence that [`check_rotation`] weighs. Approvals are a threshold of
//! ordinary Ed25519 signatures (Decision 5): no key is ever split or
//! reconstructed, and no record carries key material.
//!
//! [`RecoveryEvidence`] folds a passing rotation into the chain through
//! `crate::keyset::fold_with_recovery`, under checkpoint finality: a
//! rotation never displaces a final head, and yields to any ordinary
//! continuation of the head it pins. A guardian removal waits out the
//! delay before it counts (Decision 6), timed by the first-seen times
//! that `crate::recovery_relay` attests.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::keyset::{
    AdminKeyState, DeviceState, KeysetEntry, KeysetRecord, KeysetState, RecoveryRule,
    validate_hash_or_empty, verify_signature, verifying_key,
};
use id_core::hex::hex_encode;

/// The delay between a request's first sighting and the earliest
/// rotation it can authorise: 72 hours (Decision 4).
pub const RECOVERY_DELAY_SECONDS: u64 = 259_200;

/// The one cancellation basis v1 verifiers accept (Decision 4).
pub const CANCELLED_BY_SURVIVING_KEY: &str = "cancelled-by-surviving-key";

/// Reserved for a future threshold-objection rule; v1 verifiers reject
/// it (Decision 4).
pub const CANCELLED_BY_GUARDIAN_PROCESS: &str = "cancelled-by-guardian-process";

/// One recovery annex record's content. Unknown fields are refused, so
/// no body can smuggle share-shaped material past the schema
/// (acceptance test 13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnexEntry {
    RecoveryRequest {
        identity_id: String,
        keyset_head_hash: String,
        successor_admin_key: String,
        successor_device_key: String,
        created_at: u64,
        reason: String,
    },
    RecoveryApproval {
        identity_id: String,
        request_id: String,
        guardian: String,
        approved_at: u64,
    },
    RecoveryCancellation {
        identity_id: String,
        request_id: String,
        basis: String,
    },
}

/// A signed annex record. `prev_hash` carries the keyset head hash the
/// record was written against, pinning every recovery act to one
/// folded state (Decision 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnexRecord {
    pub author: String,
    pub prev_hash: String,
    pub hash: String,
    pub entry: AnnexEntry,
    pub signature: String,
}

#[derive(Serialize)]
struct AnnexBody<'a> {
    author: &'a str,
    prev_hash: &'a str,
    entry: &'a AnnexEntry,
}

fn annex_body(author: &str, prev_hash: &str, entry: &AnnexEntry) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AnnexBody {
        author,
        prev_hash,
        entry,
    })?)
}

impl AnnexRecord {
    /// Signs `entry` with `key` against the keyset head `prev_hash`.
    /// Refuses what [`AnnexRecord::verify`] would reject.
    ///
    /// # Errors
    ///
    /// Fails if the entry is malformed or the signer is not the one the
    /// entry requires.
    pub fn sign(key: &SigningKey, prev_hash: impl Into<String>, entry: AnnexEntry) -> Result<Self> {
        let author = hex_encode(&key.verifying_key().to_bytes());
        let prev_hash = prev_hash.into();
        validate_annex(&author, &prev_hash, &entry)?;
        let body = annex_body(&author, &prev_hash, &entry)?;
        Ok(Self {
            hash: hex_encode(&Sha256::digest(&body)),
            signature: hex_encode(&key.sign(&body).to_bytes()),
            author,
            prev_hash,
            entry,
        })
    }

    /// Verifies the canonical body, hash and author signature.
    ///
    /// # Errors
    ///
    /// Fails on a malformed entry, a hash mismatch or a bad signature.
    pub fn verify(&self) -> Result<()> {
        validate_annex(&self.author, &self.prev_hash, &self.entry)?;
        let author = verifying_key(&self.author).context("malformed annex author")?;
        let body = annex_body(&self.author, &self.prev_hash, &self.entry)?;
        ensure!(
            hex_encode(&Sha256::digest(&body)) == self.hash,
            "annex record hash does not match its content"
        );
        verify_signature(&author, &body, &self.signature).context("annex signature")
    }

    /// The identity the record is about.
    #[must_use]
    pub fn identity_id(&self) -> &str {
        match &self.entry {
            AnnexEntry::RecoveryRequest { identity_id, .. }
            | AnnexEntry::RecoveryApproval { identity_id, .. }
            | AnnexEntry::RecoveryCancellation { identity_id, .. } => identity_id,
        }
    }

    /// A request's content address: its record hash. Single-use, and
    /// it binds the successor keys and the pinned head.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        matches!(self.entry, AnnexEntry::RecoveryRequest { .. }).then_some(self.hash.as_str())
    }
}

fn validate_annex(author: &str, prev_hash: &str, entry: &AnnexEntry) -> Result<()> {
    validate_hash_or_empty(prev_hash, false, "annex prev_hash")?;
    match entry {
        AnnexEntry::RecoveryRequest {
            identity_id,
            keyset_head_hash,
            successor_admin_key,
            successor_device_key,
            ..
        } => {
            verifying_key(identity_id).context("identity id")?;
            verifying_key(successor_admin_key).context("successor admin key")?;
            verifying_key(successor_device_key).context("successor device key")?;
            ensure!(
                keyset_head_hash == prev_hash,
                "a recovery request's prev_hash must be the head it pins"
            );
            ensure!(
                author == successor_admin_key,
                "a recovery request must be signed by its successor admin key"
            );
        }
        AnnexEntry::RecoveryApproval {
            identity_id,
            request_id,
            guardian,
            ..
        } => {
            verifying_key(identity_id).context("identity id")?;
            validate_hash_or_empty(request_id, false, "request_id")?;
            verifying_key(guardian).context("guardian identity")?;
        }
        AnnexEntry::RecoveryCancellation {
            identity_id,
            request_id,
            basis,
        } => {
            verifying_key(identity_id).context("identity id")?;
            validate_hash_or_empty(request_id, false, "request_id")?;
            ensure!(
                basis == CANCELLED_BY_SURVIVING_KEY || basis == CANCELLED_BY_GUARDIAN_PROCESS,
                "unknown cancellation basis"
            );
        }
    }
    Ok(())
}

/// An annex record with the time the directory first saw it (unix
/// seconds). The directory attests the time; signatures carry the
/// authority, so a relay can delay a record but never forge one.
#[derive(Clone, Debug)]
pub struct Seen<'a> {
    pub record: &'a AnnexRecord,
    pub first_seen: u64,
}

/// Everything Rule R weighs for one rotation (Decision 3).
#[derive(Clone, Debug)]
pub struct RotationCheck<'a> {
    /// The identity's fold at the request's pinned head.
    pub head: &'a KeysetState,
    /// The request, as first seen by the directory.
    pub request: Seen<'a>,
    /// Approval records available for this rotation.
    pub approvals: &'a [AnnexRecord],
    /// Each guardian identity's current fold, by identity id.
    pub guardian_folds: &'a BTreeMap<String, KeysetState>,
    /// Cancellations seen for this identity.
    pub cancellations: &'a [Seen<'a>],
    /// Request ids already consumed by a rotation or a cancellation.
    pub consumed: &'a BTreeSet<String>,
    /// The `RecoveryRotation` chain record, and when it was first seen.
    pub rotation: &'a KeysetRecord,
    pub rotation_first_seen: u64,
}

/// What a rotation that passes Rule R installs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Successor {
    pub admin_key: String,
    pub device_key: String,
}

/// Rule R (Decision 3): is `check.rotation` a valid recovery of the
/// identity folded at `check.head`?
///
/// # Errors
///
/// Fails, naming the first unmet condition, when the rotation must not
/// fold.
pub fn check_rotation(check: &RotationCheck<'_>) -> Result<Successor> {
    let rotation = check.rotation;
    rotation.verify().context("rotation record")?;
    let KeysetEntry::RecoveryRotation {
        request_id,
        approval_hashes,
        successor_admin_key,
        successor_device_key,
    } = &rotation.entry
    else {
        bail!("not a recovery rotation");
    };

    // 1. The request exists, verifies, is unconsumed, and matches.
    let request = check.request.record;
    request.verify().context("recovery request")?;
    let AnnexEntry::RecoveryRequest {
        identity_id,
        keyset_head_hash,
        successor_admin_key: requested_admin,
        successor_device_key: requested_device,
        created_at,
        ..
    } = &request.entry
    else {
        bail!("the rotation's request is not a recovery request");
    };
    ensure!(
        request.hash == *request_id,
        "rotation names another request"
    );
    ensure!(
        !check.consumed.contains(request_id),
        "recovery request already consumed"
    );
    ensure!(
        requested_admin == successor_admin_key && requested_device == successor_device_key,
        "rotation installs keys the request did not name"
    );
    ensure!(
        *identity_id == check.head.identity_id,
        "request is for another identity"
    );
    ensure!(
        *keyset_head_hash == check.head.head_hash,
        "head fold is not the one the request pins"
    );
    ensure!(
        rotation.prev_hash == *keyset_head_hash,
        "a rotation continues the head its request pins"
    );

    // 2. At least `threshold` distinct guardians of the pinned set
    // approved, each with an active admin key of their own identity.
    let set = check
        .head
        .guardian_set
        .as_ref()
        .context("the pinned head names no guardians")?;
    let mut approving = BTreeSet::new();
    for hash in approval_hashes {
        let approval = check
            .approvals
            .iter()
            .find(|approval| approval.hash == *hash)
            .with_context(|| format!("approval {hash} is not available"))?;
        let guardian = approving_guardian(approval, check.head, check.guardian_folds, request_id)?;
        approving.insert(guardian);
    }
    let threshold = usize::try_from(set.threshold).unwrap_or(usize::MAX);
    ensure!(
        approving.len() >= threshold,
        "{} distinct guardian approvals, threshold {}",
        approving.len(),
        set.threshold
    );

    // 3. The delay elapsed, measured so a relay can only lengthen it.
    let request_seen = check.request.first_seen;
    ensure!(
        request_seen >= *created_at,
        "request first seen before it was created: a backdated attestation"
    );
    let earliest = request_seen
        .checked_add(RECOVERY_DELAY_SECONDS)
        .context("recovery delay overflow")?;
    ensure!(
        check.rotation_first_seen >= earliest,
        "recovery delay has not elapsed"
    );
    if let Some(cancel) = check.cancellations.iter().find(|cancel| {
        cancel.first_seen < check.rotation_first_seen
            && cancels(cancel.record, check.head, request_id)
    }) {
        bail!("recovery request was cancelled by {}", cancel.record.author);
    }

    // 4. Signed by the successor admin key: KeysetRecord::verify holds
    // the author to `successor_admin_key`.
    Ok(Successor {
        admin_key: successor_admin_key.clone(),
        device_key: successor_device_key.clone(),
    })
}

/// The guardian identity an approval counts for, or why it counts for
/// none.
pub(crate) fn approving_guardian(
    approval: &AnnexRecord,
    head: &KeysetState,
    guardian_folds: &BTreeMap<String, KeysetState>,
    request_id: &str,
) -> Result<String> {
    approval.verify().context("approval")?;
    let AnnexEntry::RecoveryApproval {
        identity_id,
        request_id: approved,
        guardian,
        ..
    } = &approval.entry
    else {
        bail!("approval {} is not an approval", approval.hash);
    };
    ensure!(
        *identity_id == head.identity_id && approved == request_id,
        "approval {} is for another request",
        approval.hash
    );
    ensure!(
        approval.prev_hash == head.head_hash,
        "approval {} pins another head",
        approval.hash
    );
    let set = head
        .guardian_set
        .as_ref()
        .context("the pinned head names no guardians")?;
    ensure!(
        set.guardians.contains(guardian),
        "approval {} is from {guardian}, not a guardian at the pinned head",
        approval.hash
    );
    let fold = guardian_folds
        .get(guardian)
        .with_context(|| format!("no fold for guardian {guardian}"))?;
    ensure!(
        fold.identity_id == *guardian && fold.is_admin(&approval.author),
        "approval {} is not signed by an active admin key of guardian {guardian}",
        approval.hash
    );
    Ok(guardian.clone())
}

/// Does `record` validly cancel the request? One signature from a key
/// that, at the pinned head, is an active admin or an unrevoked device.
pub(crate) fn cancels(record: &AnnexRecord, head: &KeysetState, request_id: &str) -> bool {
    let AnnexEntry::RecoveryCancellation {
        identity_id,
        request_id: cancelled,
        basis,
    } = &record.entry
    else {
        return false;
    };
    record.verify().is_ok()
        && *identity_id == head.identity_id
        && cancelled == request_id
        && basis == CANCELLED_BY_SURVIVING_KEY
        && (head.is_admin(&record.author) || head.is_device(&record.author))
}

/// The state a passing rotation folds to (Decision 3).
///
/// The successor admin key is active; every prior admin key and device
/// key is revoked; the successor device is enrolled; the guardian set
/// and the identity id carry over.
///
/// # Errors
///
/// Fails on a record count overflow.
pub fn apply_rotation(
    head: &KeysetState,
    rotation: &KeysetRecord,
    successor: &Successor,
) -> Result<KeysetState> {
    let mut state = rotated_state(head, rotation, successor);
    state.head_hash.clone_from(&rotation.hash);
    state.records_folded = state
        .records_folded
        .checked_add(1)
        .context("keyset record count overflow")?;
    Ok(state)
}

/// [`apply_rotation`] before the head advances.
fn rotated_state(
    head: &KeysetState,
    rotation: &KeysetRecord,
    successor: &Successor,
) -> KeysetState {
    let mut state = head.clone();
    for admin in state.admin_keys.values_mut() {
        *admin = AdminKeyState::Revoked;
    }
    for device in state.devices.values_mut() {
        device.revoked = true;
    }
    state
        .admin_keys
        .insert(successor.admin_key.clone(), AdminKeyState::Active);
    state.devices.insert(
        successor.device_key.clone(),
        DeviceState {
            label: "recovered device".to_owned(),
            enrolled_at: rotation.lamport,
            revoked: false,
        },
    );
    state
}

/// The recovery evidence a replica holds for one identity.
///
/// That is every annex record it has with its directory first-seen
/// time, each guardian identity's current fold, and when the directory
/// first saw each keyset record, by record hash. As a [`RecoveryRule`]
/// it folds exactly the rotations Rule R accepts, and matures delayed
/// guardian-set changes (Decision 6) by those times.
#[derive(Clone, Copy, Debug)]
pub struct RecoveryEvidence<'a> {
    pub annex: &'a [Seen<'a>],
    pub guardian_folds: &'a BTreeMap<String, KeysetState>,
    pub keyset_first_seen: &'a BTreeMap<String, u64>,
}

impl RecoveryEvidence<'_> {
    /// Rule R for `rotation` continuing the fold `head`.
    ///
    /// The guardian set counted is the one in force when the request
    /// was first seen: a pending removal that had not matured by then
    /// still counts the removed guardians, and not the added ones
    /// (Decision 6.2). Single use needs no consumed set here: a rotation must continue
    /// the head its request pins, so two rotations of one request are
    /// siblings and at most one folds.
    ///
    /// # Errors
    ///
    /// Fails when the evidence lacks the request or the rotation's
    /// first-seen time, or when [`check_rotation`] does.
    pub fn check(&self, head: &KeysetState, rotation: &KeysetRecord) -> Result<Successor> {
        let KeysetEntry::RecoveryRotation { request_id, .. } = &rotation.entry else {
            bail!("not a recovery rotation");
        };
        let request = self
            .annex
            .iter()
            .find(|seen| {
                seen.record.hash == *request_id
                    && matches!(seen.record.entry, AnnexEntry::RecoveryRequest { .. })
            })
            .with_context(|| format!("recovery request {request_id} is not available"))?;
        let rotation_first_seen = self
            .first_seen(&rotation.hash)
            .context("the rotation has no first-seen time")?;
        let head = self.at(head, request.first_seen);
        let approvals: Vec<AnnexRecord> = self
            .annex
            .iter()
            .filter(|seen| matches!(seen.record.entry, AnnexEntry::RecoveryApproval { .. }))
            .map(|seen| seen.record.clone())
            .collect();
        let cancellations: Vec<Seen<'_>> = self
            .annex
            .iter()
            .filter(|seen| matches!(seen.record.entry, AnnexEntry::RecoveryCancellation { .. }))
            .cloned()
            .collect();
        check_rotation(&RotationCheck {
            head: &head,
            request: request.clone(),
            approvals: &approvals,
            guardian_folds: self.guardian_folds,
            cancellations: &cancellations,
            consumed: &BTreeSet::new(),
            rotation,
            rotation_first_seen,
        })
    }
}

impl RecoveryEvidence<'_> {
    /// `head` with its pending guardian change matured as of `at`.
    pub(crate) fn at(&self, head: &KeysetState, at: u64) -> KeysetState {
        let pending_seen = head
            .pending_guardian_set
            .as_ref()
            .and_then(|pending| self.first_seen(&pending.record_hash));
        head.matured(pending_seen, Some(at))
    }
}

impl RecoveryRule for RecoveryEvidence<'_> {
    fn rotate(&self, head: &KeysetState, record: &KeysetRecord) -> Option<KeysetState> {
        let successor = self.check(head, record).ok()?;
        let at_rotation = self.at(head, self.first_seen(&record.hash)?);
        Some(rotated_state(&at_rotation, record, &successor))
    }

    fn first_seen(&self, record_hash: &str) -> Option<u64> {
        self.keyset_first_seen.get(record_hash).copied()
    }
}

/// Who must hear of every request, approval, cancellation and rotation
/// (Decision 4): every unrevoked device and every guardian in the fold
/// at the pinned head.
#[must_use]
pub fn notify_targets(head: &KeysetState) -> BTreeSet<String> {
    let devices = head
        .devices
        .keys()
        .filter(|key| head.is_device(key))
        .cloned();
    let guardians = head
        .guardian_set
        .iter()
        .flat_map(|set| set.guardians.iter().cloned());
    devices.chain(guardians).collect()
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
