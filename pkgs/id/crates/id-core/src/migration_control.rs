//! Fenced jurisdiction migration control plane: the pure decision core (#3404).
//!
//! Moving one account's hosted store from one jurisdiction to another is only
//! safe if every writer can be fenced off the source at a durable point, the
//! target serves nothing until the switch completes, and an interrupted
//! migration resumes from its record instead of from memory. This module is
//! that record and the rules for advancing it. It does no I/O: the directory
//! (#3399) persists [`MigrationRecord`], the routing guards (#3400) ask
//! [`MigrationRecord::admit_write`] before admitting a mutation, and the
//! migration worker (#3402) drives the phases through
//! [`MigrationRecord::apply`]. Later integrations — WebSockets, alarms, RPC
//! and bridges — all consult this same decision, so no path can honour a
//! weaker fence than another.
//!
//! The rules, from the issue:
//!
//! - The fence is the durable record, never directory state alone. A writer
//!   presents the routing epoch it read; once [`Transition::RequestFreeze`]
//!   bumps the epoch, every writer still holding the old epoch is denied with
//!   [`WriteDeny::StaleEpoch`], wherever it connects.
//! - One operator at a time, fenced by a lease. Every transition and every
//!   lease change requires the record version the caller read (compare-and-
//!   swap). Fencing tokens increase monotonically over the record's whole
//!   life and are never reused, so a stale operator — one whose lease expired
//!   and was taken over — can never transition or write again.
//! - An ambiguous CAS outcome is safe to retry: re-applying the same
//!   transition id with the version it was first applied at reports
//!   [`ApplyOutcome::AlreadyApplied`] and changes nothing. A different
//!   transition at a stale version is rejected, never guessed at.
//! - Export waits for the freeze acknowledgement: the phase machine reaches
//!   [`MigrationPhase::Exporting`] only through [`MigrationPhase::Frozen`],
//!   which is entered by recording the source's freeze acknowledgement.
//! - The target is non-serving until the switch: [`WriteSide::Target`] writes
//!   are admitted only in [`MigrationPhase::Switched`].
//! - Post-switch recovery is forward-only. There is no transition back from
//!   [`MigrationPhase::Switched`]; going back means a fresh reverse migration
//!   ([`MigrationRecord::reverse_of`]).
//!
//! Invariants the tests pin as oracles:
//!
//! - At most one side admits writes in any phase, and during the freeze
//!   window neither side does.
//! - The record version increases by exactly one per applied mutation, and a
//!   failed call never mutates the record.
//! - Fencing tokens strictly increase; the token counter survives a
//!   serialize/restore round trip, so a resumed migration cannot reissue a
//!   token a stale writer still holds.
//!
//! Time is an explicit `now_ms` parameter throughout, so tests drive a fake
//! clock (the [`crate::waiting`] convention).

use serde::{Deserialize, Serialize};

use crate::residency::Jurisdiction;

/// Durable identifier of one migration. Chosen by the creator, unique per
/// account, and never reused — a reverse migration gets a fresh id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MigrationId(pub String);

/// Caller-chosen idempotency key for one transition request. Retrying a
/// request reuses its id; using the same id for a different transition is
/// rejected as [`MigrationError::TransitionIdReuse`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TransitionId(pub String);

/// Where a migration stands. The order of declaration is the happy-path
/// order; [`MigrationPhase::Maintenance`], [`MigrationPhase::Switched`] and
/// [`MigrationPhase::Aborted`] sit outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MigrationPhase {
    /// Created, target recorded; the source still serves.
    Pending,
    /// Freeze requested and the routing epoch bumped; waiting for the source
    /// to acknowledge the freeze.
    Freezing,
    /// Freeze acknowledged: the source is frozen and drained. Export may
    /// begin from here, and only from here.
    Frozen,
    /// The frozen store is being copied to the target.
    Exporting,
    /// Export complete; the target is being activated but serves nothing
    /// until the switch transition completes.
    Activating,
    /// Paused by an operator. The phase to resume to is kept in the record.
    Maintenance,
    /// Terminal: the switch completed and the target serves. The source is
    /// fenced permanently for this migration.
    Switched,
    /// Terminal: abandoned before the switch; the source still serves.
    Aborted,
}

impl MigrationPhase {
    /// Terminal phases admit no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(self, MigrationPhase::Switched | MigrationPhase::Aborted)
    }
}

/// One requested step of the phase machine, driven through
/// [`MigrationRecord::apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Transition {
    /// Begin freezing the source. Bumps the routing epoch, fencing every
    /// writer that read the pre-freeze epoch.
    RequestFreeze,
    /// Record the source's freeze acknowledgement. The export gate.
    AcknowledgeFreeze,
    /// Start copying the frozen store to the target.
    BeginExport,
    /// The copy finished; start activating the target (still non-serving).
    CompleteExport,
    /// Activation finished: switch serving to the target. Bumps the routing
    /// epoch again, so target writers use the post-switch epoch.
    CompleteSwitch,
    /// Abandon the migration. Legal only before the switch; afterwards
    /// recovery is a fresh reverse migration.
    Abort,
    /// Pause the migration for operator maintenance.
    EnterMaintenance,
    /// Resume from maintenance to the phase the pause began in.
    ExitMaintenance,
}

/// Which side of a migration a write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WriteSide {
    /// The jurisdiction the account is leaving.
    Source,
    /// The jurisdiction the account is moving to.
    Target,
}

/// Why a write was denied. Every denial is fail-closed: a path that cannot
/// classify its write re-reads the record rather than proceeding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WriteDeny {
    /// The writer presented a routing epoch the record has moved past. This
    /// is the durable fence against stale writers.
    StaleEpoch {
        /// The epoch the writer presented.
        presented: u64,
        /// The epoch the record is at.
        current: u64,
    },
    /// The source is frozen (or freezing) and admits no further writes.
    Frozen,
    /// The migration is paused for maintenance; neither side admits writes.
    Maintenance,
    /// This side is not the serving side (the target before the switch, or
    /// the target of an aborted migration).
    NotServing,
    /// The source after the switch: it will never serve this account again
    /// under this migration.
    NoLongerServing,
}

/// The write-admission decision: the single answer every mutation path —
/// WebSocket, alarm, RPC, bridge, direct store write — must take from the
/// durable record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WriteDecision {
    /// The write may proceed on the requested side.
    Allowed,
    /// The write must be rejected, with the reason.
    Denied(WriteDeny),
}

impl WriteDecision {
    /// Whether the write may proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, WriteDecision::Allowed)
    }
}

/// The operator lease: who may drive the migration, and the fencing token
/// that proves it. A token is issued once, ever, per record — see
/// [`MigrationRecord::acquire_lease`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The operator (seat, session or worker) holding the lease.
    pub holder: String,
    /// Monotonically increasing fencing token. A caller whose token is not
    /// the recorded one is stale and is refused every transition.
    pub fencing_token: u64,
    /// The lease is live strictly before this millisecond timestamp.
    pub expires_at_ms: u64,
}

impl Lease {
    /// Whether the lease is still live at `now_ms`.
    pub fn is_live(&self, now_ms: u64) -> bool {
        now_ms < self.expires_at_ms
    }
}

/// The last transition applied to a record, kept so an ambiguous CAS retry
/// can be told apart from a conflicting transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LastTransition {
    id: TransitionId,
    transition: Transition,
    from_version: u64,
    to_version: u64,
}

/// What an [`MigrationRecord::apply`] call did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The transition was applied and the record advanced.
    Applied,
    /// This exact request had already been applied (an ambiguous CAS retry);
    /// the record was not changed.
    AlreadyApplied,
}

/// Every way a control-plane call can fail. All failures leave the record
/// exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    /// The migration id must not be empty.
    #[error("migration id must not be empty")]
    EmptyMigrationId,
    /// The account id must not be empty.
    #[error("account id must not be empty")]
    EmptyAccount,
    /// A jurisdiction label must not be empty.
    #[error("jurisdiction label must not be empty")]
    EmptyJurisdiction,
    /// A migration needs two different jurisdictions.
    #[error("source and target jurisdictions are the same: {0}")]
    SameJurisdiction(String),
    /// The caller's record version is not the record's version: someone else
    /// advanced the record (or the caller's copy is fabricated). Re-read and
    /// retry deliberately; never guess.
    #[error("stale record version: expected {expected}, record is at {actual}")]
    StaleVersion {
        /// The version the caller read.
        expected: u64,
        /// The version the record is at.
        actual: u64,
    },
    /// A transition id was reused for a different request than the one it
    /// first named.
    #[error("transition id {0} was already used for a different transition")]
    TransitionIdReuse(String),
    /// The presented fencing token is not the current lease's token: the
    /// caller's lease was taken over or released.
    #[error("stale fencing token: the current lease holds a newer token")]
    StaleFencingToken,
    /// No lease is recorded; acquire one before transitioning.
    #[error("no live lease: acquire the migration lease before transitioning")]
    NoLiveLease,
    /// The lease existed but expired before this call.
    #[error("lease expired at {expires_at_ms} ms (now {now_ms} ms)")]
    LeaseExpired {
        /// When the lease expired.
        expires_at_ms: u64,
        /// The call's clock.
        now_ms: u64,
    },
    /// A live lease is held by another operator.
    #[error("lease is held by {holder} until {expires_at_ms} ms")]
    LeaseHeld {
        /// The current holder.
        holder: String,
        /// When the lease expires.
        expires_at_ms: u64,
    },
    /// A lease must be granted for a positive duration.
    #[error("lease duration must be positive")]
    ZeroLeaseTtl,
    /// The transition is not legal in the record's current phase.
    #[error("transition {transition:?} is not legal in phase {phase:?}")]
    IllegalTransition {
        /// The phase the record is in.
        phase: MigrationPhase,
        /// The transition that was attempted.
        transition: Transition,
    },
    /// A reverse migration can only be derived from a switched record.
    #[error("record is not in the switched phase")]
    NotSwitched,
}

/// The durable control-plane record for one account's jurisdiction
/// migration. Persisted by the directory; reconstructed verbatim on resume,
/// including the fencing-token counter, so no token is ever reissued.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    id: MigrationId,
    account: String,
    source: Jurisdiction,
    target: Jurisdiction,
    phase: MigrationPhase,
    /// The phase [`Transition::ExitMaintenance`] returns to.
    phase_before_maintenance: Option<MigrationPhase>,
    /// Bumped by the transitions that change who may write
    /// ([`Transition::RequestFreeze`] and [`Transition::CompleteSwitch`]).
    /// Writers present the epoch they read; any other epoch is fenced.
    routing_epoch: u64,
    /// CAS version: increases by exactly one per applied mutation.
    version: u64,
    lease: Option<Lease>,
    /// Next fencing token to issue. Never decreases, never repeats.
    next_fencing_token: u64,
    freeze_acknowledged: bool,
    last_transition: Option<LastTransition>,
}

impl MigrationRecord {
    /// Create a migration in [`MigrationPhase::Pending`], with the first
    /// lease granted to `holder`. The routing epoch starts at 0 — the epoch
    /// pre-migration writers already hold — and the freeze request bumps it.
    pub fn begin(
        id: MigrationId,
        account: &str,
        source: Jurisdiction,
        target: Jurisdiction,
        holder: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<Self, MigrationError> {
        if id.0.is_empty() {
            return Err(MigrationError::EmptyMigrationId);
        }
        if account.is_empty() {
            return Err(MigrationError::EmptyAccount);
        }
        if source.0.is_empty() || target.0.is_empty() {
            return Err(MigrationError::EmptyJurisdiction);
        }
        if source == target {
            return Err(MigrationError::SameJurisdiction(source.0.clone()));
        }
        if lease_ttl_ms == 0 {
            return Err(MigrationError::ZeroLeaseTtl);
        }
        let lease = Lease {
            holder: holder.to_string(),
            fencing_token: 1,
            expires_at_ms: now_ms.saturating_add(lease_ttl_ms),
        };
        Ok(Self {
            id,
            account: account.to_string(),
            source,
            target,
            phase: MigrationPhase::Pending,
            phase_before_maintenance: None,
            routing_epoch: 0,
            version: 0,
            lease: Some(lease),
            next_fencing_token: 2,
            freeze_acknowledged: false,
            last_transition: None,
        })
    }

    /// Derive the fresh reverse migration that is the only way back after a
    /// switch: a new [`MigrationPhase::Pending`] record from this record's
    /// target to its source, under a new migration id and a new lease.
    pub fn reverse_of(
        &self,
        new_id: MigrationId,
        holder: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<Self, MigrationError> {
        if self.phase != MigrationPhase::Switched {
            return Err(MigrationError::NotSwitched);
        }
        Self::begin(
            new_id,
            &self.account,
            self.target.clone(),
            self.source.clone(),
            holder,
            now_ms,
            lease_ttl_ms,
        )
    }

    /// The durable migration id.
    pub fn id(&self) -> &MigrationId {
        &self.id
    }

    /// The account whose store is migrating.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The jurisdiction being left.
    pub fn source(&self) -> &Jurisdiction {
        &self.source
    }

    /// The jurisdiction being moved to.
    pub fn target(&self) -> &Jurisdiction {
        &self.target
    }

    /// The current phase.
    pub fn phase(&self) -> MigrationPhase {
        self.phase
    }

    /// The CAS version. Every applied mutation increases it by exactly one.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The current routing epoch. Writers present the epoch they read; the
    /// record admits only this one.
    pub fn routing_epoch(&self) -> u64 {
        self.routing_epoch
    }

    /// The current lease, if one is recorded (live or expired).
    pub fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// Whether the source's freeze acknowledgement has been recorded — the
    /// fact the export gate stands on.
    pub fn freeze_acknowledged(&self) -> bool {
        self.freeze_acknowledged
    }

    /// The side currently serving writes, if any. During the freeze window
    /// (freeze requested through activation) neither side serves: that gap
    /// is what makes the switch safe.
    pub fn serving_side(&self) -> Option<WriteSide> {
        match self.phase {
            MigrationPhase::Pending | MigrationPhase::Aborted => Some(WriteSide::Source),
            MigrationPhase::Switched => Some(WriteSide::Target),
            MigrationPhase::Freezing
            | MigrationPhase::Frozen
            | MigrationPhase::Exporting
            | MigrationPhase::Activating
            | MigrationPhase::Maintenance => None,
        }
    }

    /// Decide whether a write on `side` presenting `presented_epoch` may
    /// proceed. The epoch is checked first: a writer holding a pre-freeze or
    /// pre-switch epoch is fenced before any phase logic runs.
    pub fn admit_write(&self, side: WriteSide, presented_epoch: u64) -> WriteDecision {
        if presented_epoch != self.routing_epoch {
            return WriteDecision::Denied(WriteDeny::StaleEpoch {
                presented: presented_epoch,
                current: self.routing_epoch,
            });
        }
        match (self.phase, side) {
            (MigrationPhase::Pending, WriteSide::Source)
            | (MigrationPhase::Aborted, WriteSide::Source)
            | (MigrationPhase::Switched, WriteSide::Target) => WriteDecision::Allowed,
            (MigrationPhase::Pending, WriteSide::Target)
            | (MigrationPhase::Aborted, WriteSide::Target) => {
                WriteDecision::Denied(WriteDeny::NotServing)
            }
            (MigrationPhase::Freezing, _)
            | (MigrationPhase::Frozen, WriteSide::Source)
            | (MigrationPhase::Exporting, WriteSide::Source)
            | (MigrationPhase::Activating, WriteSide::Source) => {
                WriteDecision::Denied(WriteDeny::Frozen)
            }
            (MigrationPhase::Frozen, WriteSide::Target)
            | (MigrationPhase::Exporting, WriteSide::Target)
            | (MigrationPhase::Activating, WriteSide::Target) => {
                WriteDecision::Denied(WriteDeny::NotServing)
            }
            (MigrationPhase::Maintenance, _) => WriteDecision::Denied(WriteDeny::Maintenance),
            (MigrationPhase::Switched, WriteSide::Source) => {
                WriteDecision::Denied(WriteDeny::NoLongerServing)
            }
        }
    }

    /// Acquire or renew the operator lease, CAS-checked against
    /// `expected_version`. A fresh token is issued on every grant — including
    /// a renewal by the same holder — so any copy of the previous token is
    /// fenced the moment the new one exists. Taking over an expired lease is
    /// the recovery path for a dead operator.
    pub fn acquire_lease(
        &mut self,
        expected_version: u64,
        holder: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<Lease, MigrationError> {
        if expected_version != self.version {
            return Err(MigrationError::StaleVersion {
                expected: expected_version,
                actual: self.version,
            });
        }
        if lease_ttl_ms == 0 {
            return Err(MigrationError::ZeroLeaseTtl);
        }
        if let Some(lease) = &self.lease {
            if lease.is_live(now_ms) && lease.holder != holder {
                return Err(MigrationError::LeaseHeld {
                    holder: lease.holder.clone(),
                    expires_at_ms: lease.expires_at_ms,
                });
            }
        }
        let lease = Lease {
            holder: holder.to_string(),
            fencing_token: self.next_fencing_token,
            expires_at_ms: now_ms.saturating_add(lease_ttl_ms),
        };
        self.next_fencing_token = self.next_fencing_token.saturating_add(1);
        self.lease = Some(lease.clone());
        self.version = self.version.saturating_add(1);
        Ok(lease)
    }

    /// Release the lease, CAS-checked, presenting the lease's fencing token.
    /// Releasing an already-expired lease is allowed: it is bookkeeping, not
    /// authority.
    pub fn release_lease(
        &mut self,
        expected_version: u64,
        fencing_token: u64,
    ) -> Result<(), MigrationError> {
        if expected_version != self.version {
            return Err(MigrationError::StaleVersion {
                expected: expected_version,
                actual: self.version,
            });
        }
        match &self.lease {
            None => return Err(MigrationError::NoLiveLease),
            Some(lease) if lease.fencing_token != fencing_token => {
                return Err(MigrationError::StaleFencingToken);
            }
            Some(_) => {}
        }
        self.lease = None;
        self.version = self.version.saturating_add(1);
        Ok(())
    }

    /// Apply one phase transition, CAS-checked against `expected_version`
    /// and fenced by the current lease's token.
    ///
    /// Retry contract for an ambiguous CAS outcome: if this exact
    /// `transition_id` was already applied from `expected_version`, the call
    /// reports [`ApplyOutcome::AlreadyApplied`] and mutates nothing — even
    /// if the lease has changed hands since, because no authority is being
    /// exercised. Any other call at a stale version is
    /// [`MigrationError::StaleVersion`].
    pub fn apply(
        &mut self,
        transition: Transition,
        transition_id: &TransitionId,
        expected_version: u64,
        fencing_token: u64,
        now_ms: u64,
    ) -> Result<ApplyOutcome, MigrationError> {
        if let Some(last) = &self.last_transition {
            if last.id == *transition_id {
                if last.transition == transition && last.from_version == expected_version {
                    return Ok(ApplyOutcome::AlreadyApplied);
                }
                return Err(MigrationError::TransitionIdReuse(transition_id.0.clone()));
            }
        }
        if expected_version != self.version {
            return Err(MigrationError::StaleVersion {
                expected: expected_version,
                actual: self.version,
            });
        }
        let lease = self.lease.as_ref().ok_or(MigrationError::NoLiveLease)?;
        if lease.fencing_token != fencing_token {
            return Err(MigrationError::StaleFencingToken);
        }
        if !lease.is_live(now_ms) {
            return Err(MigrationError::LeaseExpired {
                expires_at_ms: lease.expires_at_ms,
                now_ms,
            });
        }
        let next_phase = self.next_phase(transition)?;
        if transition == Transition::AcknowledgeFreeze {
            self.freeze_acknowledged = true;
        }
        if matches!(
            transition,
            Transition::RequestFreeze | Transition::CompleteSwitch
        ) {
            self.routing_epoch = self.routing_epoch.saturating_add(1);
        }
        if transition == Transition::EnterMaintenance {
            self.phase_before_maintenance = Some(self.phase);
        }
        if transition == Transition::ExitMaintenance {
            self.phase_before_maintenance = None;
        }
        let from_version = self.version;
        self.phase = next_phase;
        self.version = self.version.saturating_add(1);
        self.last_transition = Some(LastTransition {
            id: transition_id.clone(),
            transition,
            from_version,
            to_version: self.version,
        });
        Ok(ApplyOutcome::Applied)
    }

    /// The phase `transition` would move to, or the illegality of asking.
    fn next_phase(&self, transition: Transition) -> Result<MigrationPhase, MigrationError> {
        let illegal = || MigrationError::IllegalTransition {
            phase: self.phase,
            transition,
        };
        match (self.phase, transition) {
            (MigrationPhase::Pending, Transition::RequestFreeze) => Ok(MigrationPhase::Freezing),
            (MigrationPhase::Freezing, Transition::AcknowledgeFreeze) => Ok(MigrationPhase::Frozen),
            (MigrationPhase::Frozen, Transition::BeginExport) => Ok(MigrationPhase::Exporting),
            (MigrationPhase::Exporting, Transition::CompleteExport) => {
                Ok(MigrationPhase::Activating)
            }
            (MigrationPhase::Activating, Transition::CompleteSwitch) => {
                Ok(MigrationPhase::Switched)
            }
            (MigrationPhase::Pending, Transition::Abort)
            | (MigrationPhase::Freezing, Transition::Abort)
            | (MigrationPhase::Frozen, Transition::Abort)
            | (MigrationPhase::Exporting, Transition::Abort)
            | (MigrationPhase::Activating, Transition::Abort) => Ok(MigrationPhase::Aborted),
            (MigrationPhase::Maintenance, Transition::Abort) => Ok(MigrationPhase::Aborted),
            (MigrationPhase::Maintenance, Transition::ExitMaintenance) => {
                self.phase_before_maintenance.ok_or_else(illegal)
            }
            (MigrationPhase::Pending, Transition::EnterMaintenance)
            | (MigrationPhase::Freezing, Transition::EnterMaintenance)
            | (MigrationPhase::Frozen, Transition::EnterMaintenance)
            | (MigrationPhase::Exporting, Transition::EnterMaintenance)
            | (MigrationPhase::Activating, Transition::EnterMaintenance) => {
                Ok(MigrationPhase::Maintenance)
            }
            _ => Err(illegal()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const TTL: u64 = 10_000;

    fn jurisdiction(label: &str) -> Jurisdiction {
        Jurisdiction(label.to_string())
    }

    fn record() -> MigrationRecord {
        MigrationRecord::begin(
            MigrationId("mig-1".to_string()),
            "acct-1",
            jurisdiction("eu"),
            jurisdiction("us"),
            "op-a",
            1_000,
            TTL,
        )
        .unwrap()
    }

    fn token(record: &MigrationRecord) -> u64 {
        record.lease().unwrap().fencing_token
    }

    fn apply(
        record: &mut MigrationRecord,
        transition: Transition,
        id: &str,
        now_ms: u64,
    ) -> ApplyOutcome {
        let version = record.version();
        let fencing = token(record);
        record
            .apply(
                transition,
                &TransitionId(id.to_string()),
                version,
                fencing,
                now_ms,
            )
            .unwrap()
    }

    /// Drive a record to `Switched` along the happy path.
    fn switched() -> MigrationRecord {
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_200);
        apply(&mut rec, Transition::BeginExport, "t-export", 1_300);
        apply(&mut rec, Transition::CompleteExport, "t-copied", 1_400);
        apply(&mut rec, Transition::CompleteSwitch, "t-switch", 1_500);
        rec
    }

    #[test]
    fn happy_path_switches_serving_to_the_target() {
        let mut rec = record();
        assert_eq!(rec.phase(), MigrationPhase::Pending);
        assert_eq!(rec.serving_side(), Some(WriteSide::Source));
        assert_eq!(rec.routing_epoch(), 0);
        assert!(rec.admit_write(WriteSide::Source, 0).is_allowed());

        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        assert_eq!(rec.phase(), MigrationPhase::Freezing);
        assert_eq!(rec.routing_epoch(), 1, "freeze bumps the routing epoch");
        assert_eq!(rec.serving_side(), None);
        assert!(!rec.freeze_acknowledged());

        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_200);
        assert_eq!(rec.phase(), MigrationPhase::Frozen);
        assert!(rec.freeze_acknowledged());

        apply(&mut rec, Transition::BeginExport, "t-export", 1_300);
        assert_eq!(rec.phase(), MigrationPhase::Exporting);

        apply(&mut rec, Transition::CompleteExport, "t-copied", 1_400);
        assert_eq!(rec.phase(), MigrationPhase::Activating);
        assert_eq!(rec.serving_side(), None, "target serves nothing yet");

        apply(&mut rec, Transition::CompleteSwitch, "t-switch", 1_500);
        assert_eq!(rec.phase(), MigrationPhase::Switched);
        assert_eq!(rec.routing_epoch(), 2, "switch bumps the routing epoch");
        assert_eq!(rec.serving_side(), Some(WriteSide::Target));
        assert!(rec.admit_write(WriteSide::Target, 2).is_allowed());
        // Five transitions, one version step each.
        assert_eq!(rec.version(), 5);
    }

    #[test]
    fn ambiguous_cas_retry_is_idempotent() {
        // Oracle: re-applying the exact request that already landed reports
        // AlreadyApplied and leaves the record bit-for-bit unchanged.
        let mut rec = record();
        let version_before = rec.version();
        let fencing = token(&rec);
        let first = rec
            .apply(
                Transition::RequestFreeze,
                &TransitionId("t-freeze".to_string()),
                version_before,
                fencing,
                1_100,
            )
            .unwrap();
        assert_eq!(first, ApplyOutcome::Applied);
        let snapshot = rec.clone();
        // The caller never saw the first result (ambiguous CAS) and retries
        // the identical request.
        let retry = rec
            .apply(
                Transition::RequestFreeze,
                &TransitionId("t-freeze".to_string()),
                version_before,
                fencing,
                1_100,
            )
            .unwrap();
        assert_eq!(retry, ApplyOutcome::AlreadyApplied);
        assert_eq!(rec, snapshot, "a retry must not mutate the record");
    }

    #[test]
    fn conflicting_transition_at_a_stale_version_is_rejected() {
        // Oracle: a different transition carrying the version a landed
        // transition started from is a stale CAS, never a guess.
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        let snapshot = rec.clone();
        let err = rec
            .apply(
                Transition::Abort,
                &TransitionId("t-abort".to_string()),
                0,
                token(&rec),
                1_200,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::StaleVersion {
                expected: 0,
                actual: 1
            }
        );
        assert_eq!(rec, snapshot, "a failed call must not mutate the record");
    }

    #[test]
    fn transition_id_reuse_for_a_different_transition_is_rejected() {
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-1", 1_100);
        let err = rec
            .apply(
                Transition::Abort,
                &TransitionId("t-1".to_string()),
                0,
                token(&rec),
                1_200,
            )
            .unwrap_err();
        assert_eq!(err, MigrationError::TransitionIdReuse("t-1".to_string()));
    }

    #[test]
    fn stale_epoch_writers_are_fenced_by_the_freeze() {
        // Oracle: the durable fence. A writer holding the pre-freeze epoch is
        // denied after RequestFreeze, and even a current-epoch source writer
        // is denied Frozen — the source admits nothing while freezing.
        let mut rec = record();
        assert!(rec.admit_write(WriteSide::Source, 0).is_allowed());
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        assert_eq!(
            rec.admit_write(WriteSide::Source, 0),
            WriteDecision::Denied(WriteDeny::StaleEpoch {
                presented: 0,
                current: 1
            })
        );
        assert_eq!(
            rec.admit_write(WriteSide::Source, 1),
            WriteDecision::Denied(WriteDeny::Frozen)
        );
    }

    #[test]
    fn at_most_one_side_serves_and_neither_does_mid_migration() {
        // Oracle: for every reachable phase, admission across both sides at
        // the current epoch allows at most one side, matching serving_side.
        let mut rec = record();
        let mut phases = vec![rec.clone()];
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        phases.push(rec.clone());
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_200);
        phases.push(rec.clone());
        apply(&mut rec, Transition::BeginExport, "t-export", 1_300);
        phases.push(rec.clone());
        apply(&mut rec, Transition::CompleteExport, "t-copied", 1_400);
        phases.push(rec.clone());
        apply(&mut rec, Transition::CompleteSwitch, "t-switch", 1_500);
        phases.push(rec.clone());
        for state in &phases {
            let epoch = state.routing_epoch();
            let source = state.admit_write(WriteSide::Source, epoch).is_allowed();
            let target = state.admit_write(WriteSide::Target, epoch).is_allowed();
            assert!(
                !(source && target),
                "both sides admitted writes in {:?}",
                state.phase()
            );
            let serving = match state.serving_side() {
                Some(WriteSide::Source) => (true, false),
                Some(WriteSide::Target) => (false, true),
                None => (false, false),
            };
            assert_eq!(
                (source, target),
                serving,
                "admission disagrees with serving_side in {:?}",
                state.phase()
            );
        }
    }

    #[test]
    fn target_never_serves_before_the_switch() {
        // Oracle: in every pre-switch phase, target writes at the current
        // epoch are denied.
        let mut rec = record();
        let mut states = vec![rec.clone()];
        for (transition, id, now) in [
            (Transition::RequestFreeze, "t-freeze", 1_100),
            (Transition::AcknowledgeFreeze, "t-ack", 1_200),
            (Transition::BeginExport, "t-export", 1_300),
            (Transition::CompleteExport, "t-copied", 1_400),
        ] {
            apply(&mut rec, transition, id, now);
            states.push(rec.clone());
        }
        for state in &states {
            assert_ne!(state.phase(), MigrationPhase::Switched);
            assert!(
                !state
                    .admit_write(WriteSide::Target, state.routing_epoch())
                    .is_allowed(),
                "target admitted a write in {:?}",
                state.phase()
            );
        }
    }

    #[test]
    fn export_waits_for_the_freeze_acknowledgement() {
        // Oracle: BeginExport is illegal from Pending and from Freezing; the
        // only road to Exporting runs through the recorded acknowledgement.
        let mut rec = record();
        let err = rec
            .apply(
                Transition::BeginExport,
                &TransitionId("t-export".to_string()),
                rec.version(),
                token(&rec),
                1_100,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::IllegalTransition {
                phase: MigrationPhase::Pending,
                transition: Transition::BeginExport
            }
        );
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        let err = rec
            .apply(
                Transition::BeginExport,
                &TransitionId("t-export".to_string()),
                rec.version(),
                token(&rec),
                1_200,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::IllegalTransition {
                phase: MigrationPhase::Freezing,
                transition: Transition::BeginExport
            }
        );
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_300);
        apply(&mut rec, Transition::BeginExport, "t-export-2", 1_400);
        assert_eq!(rec.phase(), MigrationPhase::Exporting);
    }

    #[test]
    fn stale_fencing_token_cannot_transition_after_takeover() {
        // Oracle: once a lease is taken over, the old holder's token is
        // refused everywhere, even at the correct record version.
        let mut rec = record();
        let stale_token = token(&rec);
        // op-a's lease (granted at 1_000 with TTL) expires; op-b takes over.
        let lease_b = rec
            .acquire_lease(rec.version(), "op-b", 1_000 + TTL, TTL)
            .unwrap();
        assert!(lease_b.fencing_token > stale_token);
        let err = rec
            .apply(
                Transition::RequestFreeze,
                &TransitionId("t-freeze".to_string()),
                rec.version(),
                stale_token,
                1_000 + TTL,
            )
            .unwrap_err();
        assert_eq!(err, MigrationError::StaleFencingToken);
        // The new holder proceeds.
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_000 + TTL);
        assert_eq!(rec.phase(), MigrationPhase::Freezing);
    }

    #[test]
    fn live_lease_blocks_a_second_operator_until_expiry() {
        let mut rec = record();
        let err = rec
            .acquire_lease(rec.version(), "op-b", 2_000, TTL)
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::LeaseHeld {
                holder: "op-a".to_string(),
                expires_at_ms: 1_000 + TTL
            }
        );
        // Renewal by the holder rotates the token, fencing its own stale
        // copies.
        let first_token = token(&rec);
        let renewed = rec
            .acquire_lease(rec.version(), "op-a", 2_000, TTL)
            .unwrap();
        assert!(renewed.fencing_token > first_token);
    }

    #[test]
    fn expired_lease_cannot_transition() {
        let mut rec = record();
        let err = rec
            .apply(
                Transition::RequestFreeze,
                &TransitionId("t-freeze".to_string()),
                rec.version(),
                token(&rec),
                1_000 + TTL,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::LeaseExpired {
                expires_at_ms: 1_000 + TTL,
                now_ms: 1_000 + TTL
            }
        );
    }

    #[test]
    fn release_then_reacquire_issues_a_fresh_token() {
        let mut rec = record();
        let first = token(&rec);
        rec.release_lease(rec.version(), first).unwrap();
        assert!(rec.lease().is_none());
        let err = rec
            .apply(
                Transition::RequestFreeze,
                &TransitionId("t-freeze".to_string()),
                rec.version(),
                first,
                1_100,
            )
            .unwrap_err();
        assert_eq!(err, MigrationError::NoLiveLease);
        let lease = rec
            .acquire_lease(rec.version(), "op-b", 1_200, TTL)
            .unwrap();
        assert!(lease.fencing_token > first, "tokens are never reused");
    }

    #[test]
    fn resume_from_serialized_record_continues_safely() {
        // Oracle: a record restored from its serialized form mid-migration
        // continues the phase machine, and its token counter continues —
        // a resumed migration cannot reissue a token.
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_200);
        let bytes = serde_json::to_string(&rec).unwrap();
        let mut restored: MigrationRecord = serde_json::from_str(&bytes).unwrap();
        assert_eq!(restored, rec);
        apply(&mut restored, Transition::BeginExport, "t-export", 1_300);
        assert_eq!(restored.phase(), MigrationPhase::Exporting);
        // Expire the restored lease and take over: the new token must exceed
        // every token issued before the interruption.
        let before = token(&restored);
        let lease = restored
            .acquire_lease(restored.version(), "op-b", 1_200 + TTL, TTL)
            .unwrap();
        assert!(lease.fencing_token > before);
    }

    #[test]
    fn maintenance_pauses_and_resumes_in_place() {
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        apply(&mut rec, Transition::EnterMaintenance, "t-maint", 1_200);
        assert_eq!(rec.phase(), MigrationPhase::Maintenance);
        assert_eq!(
            rec.admit_write(WriteSide::Source, rec.routing_epoch()),
            WriteDecision::Denied(WriteDeny::Maintenance)
        );
        // Ordinary progress is illegal while paused.
        let err = rec
            .apply(
                Transition::AcknowledgeFreeze,
                &TransitionId("t-ack".to_string()),
                rec.version(),
                token(&rec),
                1_300,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::IllegalTransition {
                phase: MigrationPhase::Maintenance,
                transition: Transition::AcknowledgeFreeze
            }
        );
        apply(&mut rec, Transition::ExitMaintenance, "t-resume", 1_400);
        assert_eq!(rec.phase(), MigrationPhase::Freezing);
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_500);
        assert_eq!(rec.phase(), MigrationPhase::Frozen);
    }

    #[test]
    fn abort_before_switch_restores_the_source() {
        let mut rec = record();
        apply(&mut rec, Transition::RequestFreeze, "t-freeze", 1_100);
        apply(&mut rec, Transition::AcknowledgeFreeze, "t-ack", 1_200);
        apply(&mut rec, Transition::BeginExport, "t-export", 1_300);
        apply(&mut rec, Transition::Abort, "t-abort", 1_400);
        assert_eq!(rec.phase(), MigrationPhase::Aborted);
        assert_eq!(rec.serving_side(), Some(WriteSide::Source));
        assert!(
            rec.admit_write(WriteSide::Source, rec.routing_epoch())
                .is_allowed()
        );
        assert!(
            !rec.admit_write(WriteSide::Target, rec.routing_epoch())
                .is_allowed()
        );
    }

    #[test]
    fn post_switch_recovery_is_a_fresh_reverse_migration() {
        // Oracle: there is no transition back from Switched; the way back is
        // a new record with source and target swapped.
        let mut rec = switched();
        let err = rec
            .apply(
                Transition::Abort,
                &TransitionId("t-abort".to_string()),
                rec.version(),
                token(&rec),
                2_000,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MigrationError::IllegalTransition {
                phase: MigrationPhase::Switched,
                transition: Transition::Abort
            }
        );
        // The source is fenced at every epoch it might present.
        assert_eq!(
            rec.admit_write(WriteSide::Source, 2),
            WriteDecision::Denied(WriteDeny::NoLongerServing)
        );
        let reverse = rec
            .reverse_of(MigrationId("mig-2".to_string()), "op-a", 3_000, TTL)
            .unwrap();
        assert_eq!(reverse.phase(), MigrationPhase::Pending);
        assert_eq!(reverse.source(), &jurisdiction("us"));
        assert_eq!(reverse.target(), &jurisdiction("eu"));
        assert_eq!(reverse.serving_side(), Some(WriteSide::Source));
        // A record that never switched has no reverse.
        let pending = record();
        assert_eq!(
            pending
                .reverse_of(MigrationId("mig-3".to_string()), "op-a", 3_000, TTL)
                .unwrap_err(),
            MigrationError::NotSwitched
        );
    }

    #[test]
    fn begin_validates_its_inputs() {
        assert_eq!(
            MigrationRecord::begin(
                MigrationId(String::new()),
                "acct-1",
                jurisdiction("eu"),
                jurisdiction("us"),
                "op-a",
                0,
                TTL
            )
            .unwrap_err(),
            MigrationError::EmptyMigrationId
        );
        assert_eq!(
            MigrationRecord::begin(
                MigrationId("mig-1".to_string()),
                "acct-1",
                jurisdiction("eu"),
                jurisdiction("eu"),
                "op-a",
                0,
                TTL
            )
            .unwrap_err(),
            MigrationError::SameJurisdiction("eu".to_string())
        );
        assert_eq!(
            MigrationRecord::begin(
                MigrationId("mig-1".to_string()),
                "acct-1",
                jurisdiction("eu"),
                jurisdiction("us"),
                "op-a",
                0,
                0
            )
            .unwrap_err(),
            MigrationError::ZeroLeaseTtl
        );
    }
}
