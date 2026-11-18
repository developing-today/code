//! B02 key/peer revocation and quarantine — contract module (slice B02, #10204).
//!
//! Emergency containment for key compromise and peer disable (design:
//! part-b versioned design, wire/data model, statements S1.22, S1.84,
//! S1.86). A locally signed emergency revocation/disable record
//! immediately blocks new acceptance and relay authority for that key
//! or peer; previously accepted affected data is flagged or quarantined
//! by policy — never silently rewritten.
//!
//! The contract, in one paragraph: revocation records name a subject
//! (one verifying key or one peer), an effective interval, and a
//! reason. While a record's interval covers the local instant, no new
//! data signed by that key or supplied by that peer may be accepted or
//! relayed. Applying a record sweeps already-accepted records for the
//! subject: records admitted at or after the effective start are
//! `quarantined` (withheld from release pending explicit
//! revalidation); records admitted before it are `flagged` (still
//! releasable, but marked historically `indeterminate`). A later
//! revocation whose effective start covers a flagged record's admission
//! escalates it to `quarantined`, so the widest suspicion wins; nothing
//! ever de-escalates. Quarantine is sticky: only explicit revalidation — an independent good-through
//! witness or a fresh attestation signature under a currently
//! unrevoked key — clears it. Signed bytes and admission-time evidence
//! are preserved untouched by every transition. Issuer wall-clock
//! claims (`issuer_claimed_at`) are stored for audit but never
//! consulted: backdating cannot be solved by timestamps, so historical
//! validity without independent key-status/epoch evidence is
//! `indeterminate`. Key lineage itself is untouched — rotation (slice
//! B01) never invalidates prior valid envelopes, and revocation here
//! marks records, not keys, as suspect.
//!
//! Times are logical seconds supplied by the caller (the module never
//! reads a clock), which keeps the state machine deterministic and
//! independently testable. Attestation verification is Ed25519
//! (ed25519-dalek, strict). This module is deliberately
//! self-contained: the federation roots record slice paths without
//! declaring them (slice 9110-02), so the slice's integration test
//! includes this file directly and no shared root names it yet;
//! #9110-09 wires versioned public exports after module outputs land.

use std::error::Error;
use std::fmt;

use ed25519_dalek::{Signature, VerifyingKey};

/// Logical seconds, supplied by the caller.
pub type LogicalTime = u64;

/// Subject of a revocation record: one verifying key or one peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationSubject {
    /// An Ed25519 verifying key: raw 32 bytes, also the key's identity.
    Key([u8; 32]),
    /// A peer by opaque local identifier (the same identifier space as
    /// the B31 peer-class policy's `peer_id`).
    Peer(String),
}

/// Why a revocation/disable record was issued. Carried for audit and
/// policy; every reason blocks identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevocationReason {
    /// The key is suspected compromised; its signatures are suspect.
    KeyCompromise,
    /// The peer is suspected compromised or hostile.
    PeerCompromise,
    /// Operator maintenance suspension; compromise is not asserted.
    AdministrativeDisable,
}

impl fmt::Display for RevocationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyCompromise => write!(f, "key compromise"),
            Self::PeerCompromise => write!(f, "peer compromise"),
            Self::AdministrativeDisable => write!(f, "administrative disable"),
        }
    }
}

/// Emergency revocation/disable record: who, for how long, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationRecord {
    /// Unique id for this revocation record (locally assigned).
    pub record_id: String,
    /// The key or peer this record disables.
    pub subject: RevocationSubject,
    /// First instant the subject loses acceptance/relay authority
    /// (inclusive). May predate `recorded_at`: a revocation learned
    /// late still scopes the affected interval from when the operator
    /// asserts it took effect.
    pub effective_from: LogicalTime,
    /// First instant the subject regains authority (exclusive), if the
    /// disable is bounded (a suspension). `None` is indefinite: the
    /// normal case for compromise.
    pub effective_until: Option<LogicalTime>,
    /// Why the subject was disabled.
    pub reason: RevocationReason,
    /// Local instant this record was recorded. Audit only: decisions
    /// use the effective interval, never this timestamp.
    pub recorded_at: LogicalTime,
}

impl RevocationRecord {
    /// Whether the record's effective interval covers `at`.
    #[must_use]
    pub const fn covers(&self, at: LogicalTime) -> bool {
        if at < self.effective_from {
            return false;
        }
        match self.effective_until {
            Some(until) => at < until,
            None => true,
        }
    }

    /// Whether the effective interval is well shaped: a bounded
    /// interval must end strictly after it starts.
    #[must_use]
    pub const fn interval_valid(&self) -> bool {
        match self.effective_until {
            Some(until) => self.effective_from < until,
            None => true,
        }
    }
}

/// Why new acceptance or relay was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyReason {
    /// The signing key is revoked at the decision instant.
    RevokedKey,
    /// The supplying peer is disabled at the decision instant.
    DisabledPeer,
}

/// Verdict for one (key, peer, instant) acceptance or relay question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decision {
    /// The operation may proceed.
    Allow,
    /// The operation must not proceed.
    Deny(DenyReason),
}

/// Admission-time evidence, recorded when a record is admitted and
/// preserved untouched by every later transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionEvidence {
    /// How many revocation records the registry held when this record
    /// was admitted (admission is denied while the key or peer is
    /// blocked, so these count unrelated subjects' records).
    pub revocations_seen: usize,
    /// The peer policy version the gate evaluated at admission.
    pub policy_version: u64,
}

/// Why a previously accepted record was flagged or quarantined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AffectCause {
    /// Admitted at or after the revocation's effective start: the
    /// subject may already have been compromised when this record
    /// arrived (for example, the revocation reached the local node late).
    AdmittedUnderRevocation,
    /// Admitted before the revocation's effective start: locally the
    /// subject was good at admission, but compromise may predate the
    /// revocation and issuer timestamps cannot prove otherwise.
    AdmittedBeforeRevocation,
}

/// Lifecycle status of one previously accepted record. Transitions go
/// one way: `Accepted -> Flagged -> Quarantined -> Revalidated`, where
/// `Accepted` may go straight to `Quarantined` and either suspect state
/// straight to `Revalidated`.
/// Bytes and admission evidence never change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordStatus {
    /// No revocation touches this record's subject.
    Accepted,
    /// Touches the subject but admitted before the effective start:
    /// still releasable, marked historically `indeterminate`.
    Flagged {
        /// Local instant the flag was applied.
        flagged_at: LogicalTime,
        /// Why the record was flagged.
        cause: AffectCause,
    },
    /// Admitted at or after the effective start: withheld from normal
    /// query/sync release pending explicit revalidation.
    Quarantined {
        /// Local instant the quarantine was applied.
        quarantined_at: LogicalTime,
        /// Why the record was quarantined.
        cause: AffectCause,
    },
    /// Explicitly cleared by independent evidence or re-attestation.
    Revalidated {
        /// Local instant the revalidation was recorded.
        revalidated_at: LogicalTime,
        /// Which evidence cleared the record.
        method: RevalidationMethod,
    },
}

impl RecordStatus {
    /// Whether the record may be released to normal query/sync:
    /// everything but `Quarantined`.
    #[must_use]
    pub const fn is_releasable(&self) -> bool {
        !matches!(self, Self::Quarantined { .. })
    }

    /// What can be said about when (and whether) the record was validly
    /// signed. Flagged and quarantined records are `indeterminate`:
    /// absent independently witnessed key-status/epoch evidence there
    /// is no way to tell, and issuer wall-clock claims never resolve it.
    #[must_use]
    pub const fn historical_validity(&self) -> HistoricalValidity {
        match self {
            Self::Accepted | Self::Revalidated { .. } => HistoricalValidity::Valid,
            Self::Flagged { .. } | Self::Quarantined { .. } => HistoricalValidity::Indeterminate,
        }
    }
}

/// What can be said about a record's historical validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoricalValidity {
    /// Admitted under a good subject and never touched by revocation,
    /// or explicitly cleared by revalidation.
    Valid,
    /// Touched by revocation with no independent key-status/epoch
    /// evidence either way.
    Indeterminate,
}

/// Evidence that clears quarantine/flagging. Both forms are explicit:
/// silence, expiry, and issuer timestamps never revalidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevalidationEvidence {
    /// Independently witnessed key-status/epoch evidence: the subject
    /// key was good at least through `through`. Clears the record when
    /// the witness covers its admission.
    WitnessedGoodThrough {
        /// Instant through which the key is witnessed good (inclusive).
        through: LogicalTime,
    },
    /// A fresh Ed25519 signature over the stored `signed_bytes` under
    /// `key`, which must itself be unrevoked now. The original bytes
    /// are preserved; the attestation only re-anchors them.
    Reattested {
        /// The currently-good attesting key (raw 32 bytes).
        key: [u8; 32],
        /// The attestation signature over the stored bytes.
        signature: [u8; 64],
    },
}

/// Which evidence cleared a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevalidationMethod {
    /// Cleared by an independent good-through witness.
    IndependentWitness,
    /// Cleared by re-attestation under a currently unrevoked key.
    Reattestation,
}

/// Why a revocation, admission, or revalidation operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationError {
    /// A revocation with this id is already recorded.
    DuplicateRevocationId(String),
    /// A record with this id is already stored.
    DuplicateRecordId(String),
    /// The effective interval ends at or before it starts.
    InvertedInterval {
        /// The rejected interval's start.
        from: LogicalTime,
        /// The rejected interval's end.
        until: LogicalTime,
    },
    /// Admission attempted while the signing key is revoked.
    RevokedKey,
    /// Admission attempted while the supplying peer is disabled.
    DisabledPeer,
    /// No stored record has this id.
    UnknownRecord(String),
    /// The record is `Accepted` or already `Revalidated`: there is
    /// nothing to clear.
    NotEligibleForRevalidation(String),
    /// The witness does not cover the record's admission.
    InsufficientWitness {
        /// The witness's good-through instant.
        through: LogicalTime,
        /// The record's admission instant.
        admitted_at: LogicalTime,
    },
    /// The attesting key is itself revoked now.
    AttestingKeyRevoked,
    /// The attestation signature does not verify.
    BadAttestationSignature,
}

impl fmt::Display for RevocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRevocationId(id) => {
                write!(f, "revocation record id already recorded: {id}")
            }
            Self::DuplicateRecordId(id) => write!(f, "record id already stored: {id}"),
            Self::InvertedInterval { from, until } => {
                write!(
                    f,
                    "effective interval ends before it starts: {from}..{until}"
                )
            }
            Self::RevokedKey => write!(f, "signing key is revoked"),
            Self::DisabledPeer => write!(f, "supplying peer is disabled"),
            Self::UnknownRecord(id) => write!(f, "unknown record: {id}"),
            Self::NotEligibleForRevalidation(id) => {
                write!(f, "record is not quarantined or flagged: {id}")
            }
            Self::InsufficientWitness {
                through,
                admitted_at,
            } => write!(
                f,
                "witness through {through} does not cover admission at {admitted_at}"
            ),
            Self::AttestingKeyRevoked => write!(f, "attesting key is revoked"),
            Self::BadAttestationSignature => write!(f, "attestation signature does not verify"),
        }
    }
}

impl Error for RevocationError {}

/// Whether a revocation subject matches an admitted record's key/peer.
fn subject_matches(subject: &RevocationSubject, key: &[u8; 32], peer_id: &str) -> bool {
    match subject {
        RevocationSubject::Key(k) => k == key,
        RevocationSubject::Peer(p) => p == peer_id,
    }
}

/// One previously accepted record under quarantine management. The
/// signed bytes and admission-time evidence are immutable: status
/// transitions only relabel the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedRecord {
    /// Unique id for this record (locally assigned).
    pub record_id: String,
    /// The key that signed the record (raw 32 bytes).
    pub key: [u8; 32],
    /// The peer that supplied the record.
    pub peer_id: String,
    /// Local instant the record was admitted.
    pub admitted_at: LogicalTime,
    /// Key-status/policy evidence observed at admission.
    pub admission_evidence: AdmissionEvidence,
    /// The signer's own wall-clock claim, if any. Stored for audit;
    /// never consulted by any decision in this module.
    pub issuer_claimed_at: Option<LogicalTime>,
    /// The exact signed bytes as received. Never rewritten.
    pub signed_bytes: Vec<u8>,
    /// The record's current quarantine lifecycle status.
    pub status: RecordStatus,
}

/// Emergency revocation registry: revocation records plus the
/// previously accepted records under their quarantine management.
#[derive(Debug, Clone, Default)]
pub struct RevocationRegistry {
    revocations: Vec<RevocationRecord>,
    records: Vec<AcceptedRecord>,
}

impl RevocationRegistry {
    /// An empty registry: no revocations, no records.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All recorded revocations, oldest first.
    #[must_use]
    pub fn revocations(&self) -> &[RevocationRecord] {
        &self.revocations
    }

    /// All stored accepted records, admission order.
    #[must_use]
    pub fn records(&self) -> &[AcceptedRecord] {
        &self.records
    }

    /// Find one stored record by id.
    #[must_use]
    pub fn find_record(&self, record_id: &str) -> Option<&AcceptedRecord> {
        self.records.iter().find(|r| r.record_id == record_id)
    }

    /// Whether the key is blocked at local instant `at`: some recorded
    /// revocation for it covers `at`.
    #[must_use]
    pub fn is_key_blocked_at(&self, key: &[u8; 32], at: LogicalTime) -> bool {
        self.revocations
            .iter()
            .any(|r| matches!(&r.subject, RevocationSubject::Key(k) if k == key) && r.covers(at))
    }

    /// Whether the peer is blocked at local instant `at`: some recorded
    /// revocation for it covers `at`.
    #[must_use]
    pub fn is_peer_blocked_at(&self, peer_id: &str, at: LogicalTime) -> bool {
        self.revocations.iter().any(|r| {
            matches!(&r.subject, RevocationSubject::Peer(p) if p == peer_id) && r.covers(at)
        })
    }

    /// Whether new data signed by `key` and supplied by `peer_id` may
    /// be accepted at local instant `at`: denied when either the key
    /// or the peer is blocked then (the key reason reports first when
    /// both are blocked).
    #[must_use]
    pub fn admission_decision(&self, key: &[u8; 32], peer_id: &str, at: LogicalTime) -> Decision {
        if self.is_key_blocked_at(key, at) {
            Decision::Deny(DenyReason::RevokedKey)
        } else if self.is_peer_blocked_at(peer_id, at) {
            Decision::Deny(DenyReason::DisabledPeer)
        } else {
            Decision::Allow
        }
    }

    /// Whether data attributed to `key` via `peer_id` may be relayed at
    /// local instant `at`. Relay authority dies with acceptance: a
    /// blocked key or peer relays nothing, under the identical rule.
    #[must_use]
    pub fn relay_decision(&self, key: &[u8; 32], peer_id: &str, at: LogicalTime) -> Decision {
        self.admission_decision(key, peer_id, at)
    }

    /// Admit a new record: deny (storing nothing) while the key or
    /// peer is blocked at `admitted_at`, else store it as `Accepted`
    /// with admission-time evidence.
    pub fn admit(
        &mut self,
        record_id: String,
        key: [u8; 32],
        peer_id: String,
        admitted_at: LogicalTime,
        policy_version: u64,
        issuer_claimed_at: Option<LogicalTime>,
        signed_bytes: Vec<u8>,
    ) -> Result<(), RevocationError> {
        if self.records.iter().any(|r| r.record_id == record_id) {
            return Err(RevocationError::DuplicateRecordId(record_id));
        }
        match self.admission_decision(&key, &peer_id, admitted_at) {
            Decision::Allow => {}
            Decision::Deny(DenyReason::RevokedKey) => return Err(RevocationError::RevokedKey),
            Decision::Deny(DenyReason::DisabledPeer) => return Err(RevocationError::DisabledPeer),
        }
        self.records.push(AcceptedRecord {
            record_id,
            key,
            peer_id,
            admitted_at,
            admission_evidence: AdmissionEvidence {
                revocations_seen: self.revocations.len(),
                policy_version,
            },
            issuer_claimed_at,
            signed_bytes,
            status: RecordStatus::Accepted,
        });
        Ok(())
    }

    /// Record an emergency revocation/disable and sweep previously
    /// accepted records for its subject: records admitted at or after
    /// the effective start are `quarantined`; records admitted before
    /// it are `flagged`. A flagged record of the subject admitted at or
    /// after the start is escalated to `quarantined`. Records of other
    /// subjects, and records already quarantined or revalidated, are
    /// untouched — quarantine is sticky and only explicit revalidation
    /// exits it.
    /// Returns the affected record ids in admission order.
    pub fn apply_revocation(
        &mut self,
        record: RevocationRecord,
    ) -> Result<Vec<String>, RevocationError> {
        if !record.interval_valid() {
            return Err(RevocationError::InvertedInterval {
                from: record.effective_from,
                until: record.effective_until.unwrap_or(record.effective_from),
            });
        }
        if self
            .revocations
            .iter()
            .any(|r| r.record_id == record.record_id)
        {
            return Err(RevocationError::DuplicateRevocationId(record.record_id));
        }
        let recorded_at = record.recorded_at;
        let from = record.effective_from;
        let until = record.effective_until;
        let subject = record.subject.clone();
        self.revocations.push(record);
        let mut affected = Vec::new();
        for rec in &mut self.records {
            if !subject_matches(&subject, &rec.key, &rec.peer_id) {
                continue;
            }
            // A bounded interval ends at its exclusive `until`: a record
            // admitted then or later was admitted after the revocation
            // stopped applying, so the sweep leaves it alone.
            if until.is_some_and(|until| rec.admitted_at >= until) {
                continue;
            }
            let escalate =
                matches!(rec.status, RecordStatus::Flagged { .. }) && rec.admitted_at >= from;
            if !matches!(rec.status, RecordStatus::Accepted) && !escalate {
                continue;
            }
            rec.status = if rec.admitted_at >= from {
                RecordStatus::Quarantined {
                    quarantined_at: recorded_at,
                    cause: AffectCause::AdmittedUnderRevocation,
                }
            } else {
                RecordStatus::Flagged {
                    flagged_at: recorded_at,
                    cause: AffectCause::AdmittedBeforeRevocation,
                }
            };
            affected.push(rec.record_id.clone());
        }
        Ok(affected)
    }

    /// Explicitly clear a flagged or quarantined record. An
    /// `Accepted` or already `Revalidated` record has nothing to
    /// clear; a witness must cover the record's admission; an
    /// attestation must verify over the stored bytes under a key that
    /// is unrevoked now. Bytes and admission evidence are untouched.
    pub fn revalidate(
        &mut self,
        record_id: &str,
        evidence: &RevalidationEvidence,
        now: LogicalTime,
    ) -> Result<RevalidationMethod, RevocationError> {
        let index = self
            .records
            .iter()
            .position(|r| r.record_id == record_id)
            .ok_or_else(|| RevocationError::UnknownRecord(record_id.to_string()))?;
        let (admitted_at, bytes) = {
            let rec = &self.records[index];
            if !matches!(
                rec.status,
                RecordStatus::Flagged { .. } | RecordStatus::Quarantined { .. }
            ) {
                return Err(RevocationError::NotEligibleForRevalidation(
                    record_id.to_string(),
                ));
            }
            (rec.admitted_at, rec.signed_bytes.clone())
        };
        let method = match evidence {
            RevalidationEvidence::WitnessedGoodThrough { through } => {
                if *through < admitted_at {
                    return Err(RevocationError::InsufficientWitness {
                        through: *through,
                        admitted_at,
                    });
                }
                RevalidationMethod::IndependentWitness
            }
            RevalidationEvidence::Reattested { key, signature } => {
                let vk = VerifyingKey::from_bytes(key)
                    .map_err(|_| RevocationError::BadAttestationSignature)?;
                let sig = Signature::from_bytes(signature);
                vk.verify_strict(&bytes, &sig)
                    .map_err(|_| RevocationError::BadAttestationSignature)?;
                if self.is_key_blocked_at(key, now) {
                    return Err(RevocationError::AttestingKeyRevoked);
                }
                RevalidationMethod::Reattestation
            }
        };
        self.records[index].status = RecordStatus::Revalidated {
            revalidated_at: now,
            method,
        };
        Ok(method)
    }
}
