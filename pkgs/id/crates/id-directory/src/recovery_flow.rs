//! The guardian recovery client flow (#2382): request, approve,
//! cancel, wait, rotate.
//!
//! These build the records of a recovery and read where one stands.
//! They decide nothing a verifier would not: [`progress`] reports
//! `Ready` only when [`crate::recovery::check_rotation`] would accept a
//! rotation first seen now, given the same evidence. Transport is
//! the caller's: records go to and come from a
//! [`crate::recovery_relay::RecoveryRelay`] however the two sides meet.

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use std::collections::BTreeMap;

use crate::keyset::{KeysetEntry, KeysetRecord, KeysetState};
use crate::recovery::{
    AnnexEntry, AnnexRecord, CANCELLED_BY_SURVIVING_KEY, RECOVERY_DELAY_SECONDS, RecoveryEvidence,
    approving_guardian, cancels,
};
use id_core::hex::hex_encode;

/// A recovery request from a brand-new successor key, pinned to `head`,
/// the last keyset state the requester knows.
///
/// # Errors
///
/// Fails if the successor device key is malformed.
pub fn request(
    successor_admin: &SigningKey,
    successor_device_key: &str,
    head: &KeysetState,
    created_at: u64,
    reason: &str,
) -> Result<AnnexRecord> {
    AnnexRecord::sign(
        successor_admin,
        head.head_hash.clone(),
        AnnexEntry::RecoveryRequest {
            identity_id: head.identity_id.clone(),
            keyset_head_hash: head.head_hash.clone(),
            successor_admin_key: hex_encode(&successor_admin.verifying_key().to_bytes()),
            successor_device_key: successor_device_key.to_owned(),
            created_at,
            reason: reason.to_owned(),
        },
    )
}

/// A guardian's approval of `request`, signed with an admin key of the
/// guardian identity `guardian`. It pins the head the request pins.
///
/// # Errors
///
/// Fails if `request` is not a verifying recovery request.
pub fn approve(
    guardian_admin: &SigningKey,
    guardian: &str,
    request: &AnnexRecord,
    approved_at: u64,
) -> Result<AnnexRecord> {
    let (identity_id, request_id) = requested(request)?;
    AnnexRecord::sign(
        guardian_admin,
        request.prev_hash.clone(),
        AnnexEntry::RecoveryApproval {
            identity_id,
            request_id,
            guardian: guardian.to_owned(),
            approved_at,
        },
    )
}

/// A cancellation of `request` by a surviving admin or device key.
///
/// # Errors
///
/// Fails if `request` is not a verifying recovery request.
pub fn cancel(key: &SigningKey, request: &AnnexRecord) -> Result<AnnexRecord> {
    let (identity_id, request_id) = requested(request)?;
    AnnexRecord::sign(
        key,
        request.prev_hash.clone(),
        AnnexEntry::RecoveryCancellation {
            identity_id,
            request_id,
            basis: CANCELLED_BY_SURVIVING_KEY.to_owned(),
        },
    )
}

/// The rotation that completes `request` once [`progress`] reads
/// `Ready`, signed by the successor admin key and continuing the head
/// the request pins. `lamport` must exceed every counter in the log.
///
/// # Errors
///
/// Fails if `request` is not a verifying request from this successor.
pub fn rotate(
    successor_admin: &SigningKey,
    request: &AnnexRecord,
    approval_hashes: Vec<String>,
    lamport: u64,
) -> Result<KeysetRecord> {
    request.verify().context("recovery request")?;
    let AnnexEntry::RecoveryRequest {
        successor_admin_key,
        successor_device_key,
        ..
    } = &request.entry
    else {
        bail!("not a recovery request");
    };
    ensure!(
        *successor_admin_key == hex_encode(&successor_admin.verifying_key().to_bytes()),
        "only the request's successor admin key can complete it"
    );
    KeysetRecord::sign(
        successor_admin,
        lamport,
        request.prev_hash.clone(),
        KeysetEntry::RecoveryRotation {
            request_id: request.hash.clone(),
            approval_hashes,
            successor_admin_key: successor_admin_key.clone(),
            successor_device_key: successor_device_key.clone(),
        },
        None,
    )
}

/// Where a recovery request stands as of some time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// A surviving key cancelled it; it can never complete.
    Cancelled {
        /// The cancelling key.
        by: String,
    },
    /// Not enough approvals yet, or the delay has not run.
    Waiting {
        /// Distinct guardians whose approval counts.
        approvals: usize,
        /// How many must approve.
        threshold: u32,
        /// The earliest time a rotation can be first seen.
        ready_at: u64,
    },
    /// A rotation first seen now passes Rule R with these approvals.
    Ready {
        /// One counting approval per guardian, sorted.
        approval_hashes: Vec<String>,
    },
}

/// Where the request `request_id` stands at `now`, judged against
/// `head`, the fold it pins, and the evidence a relay holds.
///
/// # Errors
///
/// Fails if the evidence lacks the request, the request does not pin
/// `head`, or the head names no guardians.
pub fn progress(
    evidence: &RecoveryEvidence<'_>,
    head: &KeysetState,
    request_id: &str,
    now: u64,
) -> Result<Progress> {
    let seen = evidence
        .annex
        .iter()
        .find(|seen| seen.record.hash == request_id)
        .with_context(|| format!("recovery request {request_id} is not available"))?;
    let AnnexEntry::RecoveryRequest {
        keyset_head_hash,
        created_at,
        ..
    } = &seen.record.entry
    else {
        bail!("{request_id} is not a recovery request");
    };
    ensure!(
        *keyset_head_hash == head.head_hash,
        "the request pins another head"
    );
    let head = evidence.at(head, seen.first_seen);
    if let Some(cancel) = evidence
        .annex
        .iter()
        .find(|cancel| cancel.first_seen <= now && cancels(cancel.record, &head, request_id))
    {
        return Ok(Progress::Cancelled {
            by: cancel.record.author.clone(),
        });
    }
    let threshold = head
        .guardian_set
        .as_ref()
        .context("the pinned head names no guardians")?
        .threshold;
    let mut counting: BTreeMap<String, String> = BTreeMap::new();
    for approval in evidence.annex {
        if !matches!(approval.record.entry, AnnexEntry::RecoveryApproval { .. }) {
            continue;
        }
        if let Ok(guardian) =
            approving_guardian(approval.record, &head, evidence.guardian_folds, request_id)
        {
            let hash = &approval.record.hash;
            let kept = counting.entry(guardian).or_insert_with(|| hash.clone());
            if hash < kept {
                kept.clone_from(hash);
            }
        }
    }
    let ready_at = seen
        .first_seen
        .max(*created_at)
        .checked_add(RECOVERY_DELAY_SECONDS)
        .context("recovery delay overflow")?;
    let approvals = counting.len();
    if approvals >= usize::try_from(threshold).unwrap_or(usize::MAX) && now >= ready_at {
        let mut approval_hashes: Vec<String> = counting.into_values().collect();
        approval_hashes.sort();
        return Ok(Progress::Ready { approval_hashes });
    }
    Ok(Progress::Waiting {
        approvals,
        threshold,
        ready_at,
    })
}

/// The identity and request id of a verifying recovery request.
fn requested(request: &AnnexRecord) -> Result<(String, String)> {
    request.verify().context("recovery request")?;
    let AnnexEntry::RecoveryRequest { identity_id, .. } = &request.entry else {
        bail!("not a recovery request");
    };
    Ok((identity_id.clone(), request.hash.clone()))
}
