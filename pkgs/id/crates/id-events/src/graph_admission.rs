//! Append-list admission state machine for A-GRAPH-9127-02 (#10237).
//!
//! Signature verification happens before this state machine. The policy is
//! supplied on every operation so approval rechecks current membership,
//! policy and revocation instead of trusting the snapshot seen at submit.

use std::collections::{BTreeMap, BTreeSet};

use id_core::graph::append_list::{AdmissionOutcome, AttributeSubmission, PendingId};
use id_core::graph::attribute::{AdmissionRule, AppendMode};
use thiserror::Error;

/// Current policy inputs used by submission and approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionPolicy {
    /// Whether the attribute accepts submissions.
    pub mode: AppendMode,
    /// Which submitters can be accepted without moderation.
    pub rule: AdmissionRule,
    /// Current group members.
    pub members: BTreeSet<String>,
    /// Current group administrators.
    pub admins: BTreeSet<String>,
    /// Current revoked signers.
    pub revoked: BTreeSet<String>,
    /// Monotonic policy epoch used by callers for CAS fencing.
    pub epoch: u64,
}

/// Admission failures; none mutate the accepted order.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AdmissionError {
    /// The signed evidence was invalid.
    #[error("submission signature is invalid")]
    InvalidSignature,
    /// The attribute is closed.
    #[error("attribute is closed")]
    Closed,
    /// The idempotency key was reused for different bytes.
    #[error("idempotency key conflicts with different bytes")]
    IdempotencyConflict,
    /// The signer is not eligible under the current policy.
    #[error("submitter is not currently eligible")]
    NotEligible,
    /// The signer was revoked after submit.
    #[error("submitter is revoked")]
    Revoked,
    /// The named pending item does not exist.
    #[error("pending submission not found")]
    NotFound,
}

/// Deterministic append admission state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppendAdmission {
    accepted: Vec<String>,
    pending: BTreeMap<PendingId, AttributeSubmission>,
    idempotency: BTreeMap<String, Vec<u8>>,
}

impl AppendAdmission {
    /// Submit verified evidence under the current policy.
    pub fn submit(
        &mut self,
        submission: AttributeSubmission,
        policy: AdmissionPolicy,
    ) -> Result<AdmissionOutcome, AdmissionError> {
        if !submission.signature_valid {
            return Err(AdmissionError::InvalidSignature);
        }
        if let Some(bytes) = self.idempotency.get(&submission.idempotency_key) {
            if bytes != &submission.bytes {
                return Err(AdmissionError::IdempotencyConflict);
            }
            let pending = PendingId::from(submission.id.as_str());
            return if self.pending.contains_key(&pending) {
                Ok(AdmissionOutcome::Pending(pending))
            } else {
                Ok(AdmissionOutcome::Accepted)
            };
        }
        if !policy.mode.accepts_submissions() {
            return Err(AdmissionError::Closed);
        }
        self.idempotency
            .insert(submission.idempotency_key.clone(), submission.bytes.clone());
        let pending = PendingId::from(submission.id.as_str());
        if policy.mode == AppendMode::Open && eligible(&policy, &submission.signer) {
            self.accepted.push(submission.id);
            Ok(AdmissionOutcome::Accepted)
        } else {
            self.pending.insert(pending.clone(), submission);
            Ok(AdmissionOutcome::Pending(pending))
        }
    }

    /// Approve one pending submission after rechecking current policy state.
    pub fn approve(&mut self, id: &str, policy: &AdmissionPolicy) -> Result<(), AdmissionError> {
        let pending = self
            .pending
            .get(&PendingId::from(id))
            .ok_or(AdmissionError::NotFound)?;
        if policy.revoked.contains(&pending.signer) {
            return Err(AdmissionError::Revoked);
        }
        if !eligible(policy, &pending.signer) {
            return Err(AdmissionError::NotEligible);
        }
        let pending = self
            .pending
            .remove(&PendingId::from(id))
            .ok_or(AdmissionError::NotFound)?;
        self.accepted.push(pending.id);
        Ok(())
    }

    /// Accepted IDs in authoritative append order.
    #[must_use]
    pub fn accepted(&self) -> &[String] {
        &self.accepted
    }

    /// Pending submissions keyed by their stable IDs.
    #[must_use]
    pub fn pending(&self) -> &BTreeMap<PendingId, AttributeSubmission> {
        &self.pending
    }
}

fn eligible(policy: &AdmissionPolicy, signer: &str) -> bool {
    if policy.revoked.contains(signer) {
        return false;
    }
    match policy.rule {
        AdmissionRule::OwnerOnly => policy.admins.contains(signer),
        AdmissionRule::Admins => policy.admins.contains(signer),
        AdmissionRule::Members => policy.members.contains(signer),
        AdmissionRule::AdminsAndMembers => {
            policy.admins.contains(signer) || policy.members.contains(signer)
        }
    }
}
