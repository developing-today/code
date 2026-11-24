//! Fenced jurisdiction migration control plane: the pure phase state
//! machine (design §7 runbook; issue #3404 slice 5).
//!
//! A migration is an authenticated, auditable product state machine, not a
//! script that edits the directory row. The runbook sequence is
//! `Stable → Pending → Frozen → Copying → Verifying → Switched →
//! Cleaning → Retired`, with abort back to `Stable` allowed only before
//! the switch. After the switch the source stays permanently fenced:
//! recovery is forward, or a fresh reverse migration under a new
//! migration ID — never an unfreeze of the old source.
//!
//! ## What is here
//!
//! - [`MigrationPhase`]: the phase DAG. Every edge is an explicit arm of
//!   [`MigrationRecord::transition`]; anything else is
//!   [`MigrationError::ForbiddenTransition`].
//! - [`MigrationRecord`]: the in-memory migration row — source and pending
//!   target jurisdictions, durable migration ID, routing epoch, fencing
//!   token and freeze watermark. Transitions take `&self` and return a new
//!   record, so a refused transition cannot half-mutate state.
//! - The fencing discipline: every transition except `Begin` (which mints
//!   the token) and every abort (which rotates it) must present the
//!   record's current fencing token. Directory state alone never fences a
//!   stale writer; the token is the second factor.
//! - CAS epoch rules: every accepted transition requires
//!   `expected_epoch == record.epoch` and lands on `epoch + 1`. The switch
//!   refuses without a recorded freeze acknowledgement, and abort after
//!   the switch is refused as irreversible.
//! - [`mint_migration_id`] / [`mint_fencing_token`]: 32-lowercase-hex ID
//!   minting in the [`BindingRecord`](crate::hosted_binding::BindingRecord)
//!   `binding_id` shape, over caller-supplied randomness so tests are
//!   deterministic; [`random_migration_id`] / [`random_fencing_token`]
//!   feed them from the OS RNG for production callers.
//!
//! ## What is not here
//!
//! Durable storage of the row (the directory CAS in #3399), the
//! identity-touching entry-point guards that honor the fence (#3400),
//! the live source freeze/drain handshake against the hosted object, and
//! the export/copy/verify orchestration (#3402). Those feed events into
//! this machine; this module decides which events are legal.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hosted_binding::hex_encode;

/// Length of a minted migration ID or fencing token: 32 lowercase hex
/// chars (128 bits), the same shape as a binding `binding_id`.
pub const MIGRATION_ID_HEX_LEN: usize = 32;

/// Longest accepted jurisdiction label; labels are `[a-z0-9-]` and the
/// design's labels (`us`, `eu`, `fedramp`) are far shorter.
pub const MAX_JURISDICTION_LEN: usize = 63;

/// A phase of one fenced migration. The DAG is:
/// `Stable → Pending → Frozen → Copying → Verifying → Switched →
/// Cleaning → Retired`, plus pre-switch abort edges back to `Stable`.
/// There is deliberately no edge out of `Switched`/`Cleaning` toward
/// `Stable`: post-switch recovery is forward or a fresh reverse
/// migration under a new ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationPhase {
    /// No migration in flight; the source serves normally.
    Stable,
    /// Directory CAS accepted: source is in maintenance, new mutations
    /// are rejected, in-flight work is draining. Not yet a fence: the
    /// coordinator must still wait for the freeze acknowledgement.
    Pending,
    /// The source confirmed a durable freeze watermark. Export may start.
    Frozen,
    /// Snapshot export, blob copy and target import under this
    /// migration ID are in flight. The target is import-only.
    Copying,
    /// Read-back verification of the target against the manifest.
    Verifying,
    /// The directory CAS to the target is the linearization point. The
    /// source is permanently fenced for this migration ID; the target
    /// is activating with the new epoch.
    Switched,
    /// Source retention, then fenced idempotent source cleanup.
    Cleaning,
    /// The migration record is retired after required retention.
    /// Terminal: every further event is refused.
    Retired,
}

/// An event fed into [`MigrationRecord::transition`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationEvent {
    /// Open a migration: CAS the directory from stable
    /// `(source, epoch)` to pending `(source, target, M, epoch+1)`.
    Begin {
        target: String,
        migration_id: String,
        fencing_token: String,
    },
    /// The source confirmed a durable freeze watermark: no new
    /// mutations will commit and in-flight ones drained. Export must
    /// wait for this; the pending directory state alone is not a fence.
    AcknowledgeFreeze { watermark: String },
    /// Start snapshot export and target copy/import under `M`.
    BeginCopy,
    /// Start read-back verification of the target.
    BeginVerify,
    /// Atomically CAS the directory to `(target, M, switched)` and
    /// activate the target. Refused without a freeze acknowledgement.
    CasSwitch,
    /// Start post-switch source retention and fenced cleanup.
    BeginCleanup,
    /// Retire the record after required retention. Terminal.
    Retire,
    /// Pre-switch abort: discard/quarantine the target, CAS back to
    /// the source with a new epoch and explicitly unfreeze it. The
    /// token rotates so writers of the aborted attempt stay fenced.
    /// Refused after the switch.
    Abort {
        reason: String,
        new_fencing_token: String,
    },
}

impl MigrationEvent {
    /// Stable name for error reporting and audit logs.
    pub fn name(&self) -> &'static str {
        match self {
            MigrationEvent::Begin { .. } => "begin",
            MigrationEvent::AcknowledgeFreeze { .. } => "acknowledge_freeze",
            MigrationEvent::BeginCopy => "begin_copy",
            MigrationEvent::BeginVerify => "begin_verify",
            MigrationEvent::CasSwitch => "cas_switch",
            MigrationEvent::BeginCleanup => "begin_cleanup",
            MigrationEvent::Retire => "retire",
            MigrationEvent::Abort { .. } => "abort",
        }
    }
}

/// The CAS context a writer presents with an event: the epoch it read
/// and the fencing token it holds. Both must match the record or the
/// writer is stale and the transition is refused without side effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionContext<'a> {
    /// The epoch the writer read before proposing this event.
    pub expected_epoch: u64,
    /// The fencing token the writer holds for this migration attempt.
    pub fencing_token: &'a str,
}

/// The in-memory migration row. The durable twin lives in the hosted
/// directory (#3399); this type owns the transition rules both twins
/// must obey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// Jurisdiction serving the account today; never changes in place.
    pub source: String,
    /// Pending target jurisdiction; `None` unless a migration is open.
    pub target: Option<String>,
    /// Durable migration ID; `None` unless a migration is open.
    pub migration_id: Option<String>,
    /// Current phase of the runbook.
    pub phase: MigrationPhase,
    /// Routing epoch. Strictly +1 per accepted transition; the
    /// directory CAS compares it, and stale routes carry older ones.
    pub epoch: u64,
    /// Current fencing token. Empty while [`MigrationPhase::Stable`];
    /// every non-stable transition must present it.
    pub fencing_token: String,
    /// Durable freeze watermark confirmed by the source; `Some` from
    /// [`MigrationPhase::Frozen`] until a pre-switch abort clears it.
    pub freeze_watermark: Option<String>,
}

/// Every refusal is a typed error naming the rule that failed; a
/// refused event mutates nothing.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MigrationError {
    /// A jurisdiction label must be 1–63 chars of `[a-z0-9-]`.
    /// Absent, invalid or unprovisioned values fail closed.
    #[error("invalid jurisdiction label {0:?}: must be 1-63 chars of [a-z0-9-]")]
    InvalidJurisdiction(String),
    /// A migration must move between two different jurisdictions.
    #[error("source and target jurisdictions must differ")]
    SourceEqualsTarget,
    /// Migration IDs are 32 lowercase hex chars, minted once per attempt.
    #[error("malformed migration id: must be 32 lowercase hex chars")]
    MalformedMigrationId,
    /// Fencing tokens are 32 lowercase hex chars.
    #[error("malformed fencing token: must be 32 lowercase hex chars")]
    MalformedFencingToken,
    /// An abort must rotate its token so the aborted coordinator cannot
    /// reuse its authority on a later attempt.
    #[error("abort fencing token must rotate from the current migration token")]
    FencingTokenNotRotated,
    /// The freeze acknowledgement must carry the durable watermark the
    /// source confirmed; an empty watermark fences nothing.
    #[error("empty freeze watermark: the source must confirm a durable freeze before export")]
    EmptyFreezeWatermark,
    /// The writer does not hold the record's current fencing token: it
    /// is stale (an aborted attempt, a superseded coordinator) and is
    /// fenced without side effects.
    #[error(
        "stale fencing token for phase {phase:?}: the writer does not hold this migration attempt"
    )]
    StaleFencingToken { phase: MigrationPhase },
    /// The writer CAS'd on an epoch that is no longer current. On an
    /// ambiguous CAS or timeout the coordinator must reread the
    /// authoritative record and resume from the recorded phase — never
    /// guess which side is active.
    #[error("stale epoch: writer saw {expected}, record is at {actual}")]
    EpochMismatch { expected: u64, actual: u64 },
    /// The edge is not in the phase DAG (for example, switching from
    /// `Copying`, or verifying from `Pending`).
    #[error("forbidden transition: event {event} is not legal from phase {phase:?}")]
    ForbiddenTransition {
        phase: MigrationPhase,
        event: &'static str,
    },
    /// The directory pending state alone is not a fence: the switch is
    /// refused until the source's freeze acknowledgement is recorded.
    #[error("switch refused: no freeze acknowledgement recorded for this migration")]
    SwitchRequiresFreezeAck,
    /// After the switch the source stays permanently fenced for this
    /// migration ID. Recover forward, or open a fresh reverse
    /// migration; never unfreeze the old source.
    #[error("switch is irreversible: post-switch recovery is forward or a fresh reverse migration")]
    SwitchIsIrreversible,
    /// The retired record is terminal; every event is refused.
    #[error("migration record is retired: terminal, no further events accepted")]
    RetiredIsTerminal,
    /// Aborts must say why, for the audit trail.
    #[error("abort requires a non-empty reason")]
    EmptyAbortReason,
}

fn valid_jurisdiction(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_JURISDICTION_LEN
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_hex_id(text: &str) -> bool {
    text.len() == MIGRATION_ID_HEX_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn check_jurisdiction(label: &str) -> Result<(), MigrationError> {
    if valid_jurisdiction(label) {
        Ok(())
    } else {
        Err(MigrationError::InvalidJurisdiction(label.to_string()))
    }
}

/// Mint a migration ID over caller-supplied randomness (16 bytes →
/// 32 lowercase hex chars), the `binding_id` shape.
pub fn mint_migration_id(randomness: &[u8; 16]) -> String {
    hex_encode(randomness)
}

/// Mint a fencing token over caller-supplied randomness (16 bytes →
/// 32 lowercase hex chars).
pub fn mint_fencing_token(randomness: &[u8; 16]) -> String {
    hex_encode(randomness)
}

/// Mint a migration ID from the OS RNG (production callers).
pub fn random_migration_id() -> String {
    mint_migration_id(&rand::random::<[u8; 16]>())
}

/// Mint a fencing token from the OS RNG (production callers).
pub fn random_fencing_token() -> String {
    mint_fencing_token(&rand::random::<[u8; 16]>())
}

impl MigrationRecord {
    /// A stable record for a serving source jurisdiction: epoch 0, no
    /// token, no pending target. Fail-closed on the label.
    pub fn stable(source: &str) -> Result<Self, MigrationError> {
        check_jurisdiction(source)?;
        Ok(Self {
            source: source.to_string(),
            target: None,
            migration_id: None,
            phase: MigrationPhase::Stable,
            epoch: 0,
            fencing_token: String::new(),
            freeze_watermark: None,
        })
    }

    /// Apply one event under a CAS context, returning the next record.
    /// The machine checks, in order: retirement, fencing token, epoch,
    /// then the phase DAG. A refusal mutates nothing.
    pub fn transition(
        &self,
        event: &MigrationEvent,
        ctx: &TransitionContext<'_>,
    ) -> Result<Self, MigrationError> {
        if self.phase == MigrationPhase::Retired {
            return Err(MigrationError::RetiredIsTerminal);
        }
        // `Begin` mints the attempt's token, so there is no prior token
        // to present; every other event must present the current one.
        let establishes_token = matches!(event, MigrationEvent::Begin { .. })
            && self.phase == MigrationPhase::Stable
            && self.fencing_token.is_empty();
        if !establishes_token && ctx.fencing_token != self.fencing_token {
            return Err(MigrationError::StaleFencingToken { phase: self.phase });
        }
        if ctx.expected_epoch != self.epoch {
            return Err(MigrationError::EpochMismatch {
                expected: ctx.expected_epoch,
                actual: self.epoch,
            });
        }
        let mut next = self.clone();
        match (&self.phase, event) {
            (
                MigrationPhase::Stable,
                MigrationEvent::Begin {
                    target,
                    migration_id,
                    fencing_token,
                },
            ) => {
                check_jurisdiction(target)?;
                if target == &self.source {
                    return Err(MigrationError::SourceEqualsTarget);
                }
                if !valid_hex_id(migration_id) {
                    return Err(MigrationError::MalformedMigrationId);
                }
                if !valid_hex_id(fencing_token) {
                    return Err(MigrationError::MalformedFencingToken);
                }
                next.target = Some(target.clone());
                next.migration_id = Some(migration_id.clone());
                next.fencing_token = fencing_token.clone();
                next.phase = MigrationPhase::Pending;
            }
            (MigrationPhase::Pending, MigrationEvent::AcknowledgeFreeze { watermark }) => {
                if watermark.is_empty() {
                    return Err(MigrationError::EmptyFreezeWatermark);
                }
                next.freeze_watermark = Some(watermark.clone());
                next.phase = MigrationPhase::Frozen;
            }
            (MigrationPhase::Frozen, MigrationEvent::BeginCopy) => {
                next.phase = MigrationPhase::Copying;
            }
            (MigrationPhase::Copying, MigrationEvent::BeginVerify) => {
                next.phase = MigrationPhase::Verifying;
            }
            (MigrationPhase::Verifying, MigrationEvent::CasSwitch) => {
                if next.freeze_watermark.is_none() {
                    return Err(MigrationError::SwitchRequiresFreezeAck);
                }
                next.phase = MigrationPhase::Switched;
            }
            (MigrationPhase::Switched, MigrationEvent::BeginCleanup) => {
                next.phase = MigrationPhase::Cleaning;
            }
            (MigrationPhase::Cleaning, MigrationEvent::Retire) => {
                next.phase = MigrationPhase::Retired;
            }
            (
                MigrationPhase::Pending
                | MigrationPhase::Frozen
                | MigrationPhase::Copying
                | MigrationPhase::Verifying,
                MigrationEvent::Abort {
                    reason,
                    new_fencing_token,
                },
            ) => {
                if reason.is_empty() {
                    return Err(MigrationError::EmptyAbortReason);
                }
                if !valid_hex_id(new_fencing_token) {
                    return Err(MigrationError::MalformedFencingToken);
                }
                if new_fencing_token == &self.fencing_token {
                    return Err(MigrationError::FencingTokenNotRotated);
                }
                // CAS back to the source with a new epoch and explicitly
                // unfreeze it; the rotated token keeps writers of the
                // aborted attempt fenced.
                next.target = None;
                next.migration_id = None;
                next.fencing_token = new_fencing_token.clone();
                next.freeze_watermark = None;
                next.phase = MigrationPhase::Stable;
            }
            (MigrationPhase::Switched | MigrationPhase::Cleaning, MigrationEvent::Abort { .. }) => {
                return Err(MigrationError::SwitchIsIrreversible);
            }
            (phase, event) => {
                return Err(MigrationError::ForbiddenTransition {
                    phase: *phase,
                    event: event.name(),
                });
            }
        }
        next.epoch = self.epoch + 1;
        Ok(next)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const SOURCE: &str = "us";
    const TARGET: &str = "eu";
    const MIGRATION_ID: &str = "0123456789abcdef0123456789abcdef";
    const TOKEN: &str = "fedcba9876543210fedcba9876543210";
    const NEXT_TOKEN: &str = "00000000000000000000000000000000";
    const STALE_TOKEN: &str = "ffffffffffffffffffffffffffffffff";
    const WATERMARK: &str = "freeze-watermark-epoch-7";

    fn stable() -> MigrationRecord {
        MigrationRecord::stable(SOURCE).unwrap()
    }

    fn ctx_for(record: &MigrationRecord) -> TransitionContext<'_> {
        TransitionContext {
            expected_epoch: record.epoch,
            fencing_token: &record.fencing_token,
        }
    }

    fn begin_event() -> MigrationEvent {
        MigrationEvent::Begin {
            target: TARGET.to_string(),
            migration_id: MIGRATION_ID.to_string(),
            fencing_token: TOKEN.to_string(),
        }
    }

    /// Drive a record through the golden runbook to `Switched`.
    fn to_switched() -> MigrationRecord {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        let frozen = pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx_for(&pending),
            )
            .unwrap();
        let copying = frozen
            .transition(&MigrationEvent::BeginCopy, &ctx_for(&frozen))
            .unwrap();
        let verifying = copying
            .transition(&MigrationEvent::BeginVerify, &ctx_for(&copying))
            .unwrap();
        verifying
            .transition(&MigrationEvent::CasSwitch, &ctx_for(&verifying))
            .unwrap()
    }

    #[test]
    fn happy_path_runs_the_golden_runbook_to_retired() {
        let switched = to_switched();
        assert_eq!(switched.phase, MigrationPhase::Switched);
        assert_eq!(switched.epoch, 5);
        assert_eq!(switched.target.as_deref(), Some(TARGET));
        assert_eq!(switched.migration_id.as_deref(), Some(MIGRATION_ID));
        assert_eq!(switched.fencing_token, TOKEN);
        assert_eq!(switched.freeze_watermark.as_deref(), Some(WATERMARK));

        let cleaning = switched
            .transition(&MigrationEvent::BeginCleanup, &ctx_for(&switched))
            .unwrap();
        assert_eq!(cleaning.phase, MigrationPhase::Cleaning);
        assert_eq!(cleaning.epoch, 6);

        let retired = cleaning
            .transition(&MigrationEvent::Retire, &ctx_for(&cleaning))
            .unwrap();
        assert_eq!(retired.phase, MigrationPhase::Retired);
        assert_eq!(retired.epoch, 7);
    }

    #[test]
    fn stale_fencing_token_is_fenced_without_side_effects() {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        let stale = TransitionContext {
            expected_epoch: pending.epoch,
            fencing_token: STALE_TOKEN,
        };
        assert_eq!(
            pending.transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &stale,
            ),
            Err(MigrationError::StaleFencingToken {
                phase: MigrationPhase::Pending,
            })
        );
        // The refusal mutated nothing: the live token still works.
        let frozen = pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx_for(&pending),
            )
            .unwrap();
        assert_eq!(frozen.phase, MigrationPhase::Frozen);
    }

    #[test]
    fn stale_epoch_is_refused_as_ambiguous_cas() {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        let frozen = pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx_for(&pending),
            )
            .unwrap();
        // A writer that read the Pending epoch races the freeze: its
        // event must fail against the Frozen record, not guess.
        let raced = TransitionContext {
            expected_epoch: pending.epoch,
            fencing_token: &pending.fencing_token,
        };
        assert_eq!(
            frozen.transition(&MigrationEvent::BeginCopy, &raced),
            Err(MigrationError::EpochMismatch {
                expected: pending.epoch,
                actual: frozen.epoch,
            })
        );
    }

    #[test]
    fn begin_validates_its_inputs_fail_closed() {
        let record = stable();
        let ctx = ctx_for(&record);
        // Same source and target: no migration to run.
        assert_eq!(
            record.transition(
                &MigrationEvent::Begin {
                    target: SOURCE.to_string(),
                    migration_id: MIGRATION_ID.to_string(),
                    fencing_token: TOKEN.to_string(),
                },
                &ctx,
            ),
            Err(MigrationError::SourceEqualsTarget)
        );
        // Invalid jurisdiction labels fail closed.
        for bad in ["", "US", "us_east", "eu!"] {
            assert_eq!(
                record.transition(
                    &MigrationEvent::Begin {
                        target: bad.to_string(),
                        migration_id: MIGRATION_ID.to_string(),
                        fencing_token: TOKEN.to_string(),
                    },
                    &ctx,
                ),
                Err(MigrationError::InvalidJurisdiction(bad.to_string())),
                "label {bad:?} must fail closed"
            );
        }
        // Malformed IDs and tokens are refused, not truncated.
        assert_eq!(
            record.transition(
                &MigrationEvent::Begin {
                    target: TARGET.to_string(),
                    migration_id: "short".to_string(),
                    fencing_token: TOKEN.to_string(),
                },
                &ctx,
            ),
            Err(MigrationError::MalformedMigrationId)
        );
        assert_eq!(
            record.transition(
                &MigrationEvent::Begin {
                    target: TARGET.to_string(),
                    migration_id: MIGRATION_ID.to_string(),
                    fencing_token: "UPPERCASE-NOT-HEX-AT-ALL-00000000".to_string(),
                },
                &ctx,
            ),
            Err(MigrationError::MalformedFencingToken)
        );
        // A stable record has no source label at all: also closed.
        assert_eq!(
            MigrationRecord::stable(""),
            Err(MigrationError::InvalidJurisdiction(String::new()))
        );
    }

    #[test]
    fn switch_without_freeze_ack_is_refused() {
        // No public event sequence reaches Verifying without a
        // watermark, so the stranded-record case is built through the
        // serde twin the durable row shares: the guard must still hold.
        let stranded: MigrationRecord = serde_json::from_value(serde_json::json!({
            "source": SOURCE,
            "target": TARGET,
            "migration_id": MIGRATION_ID,
            "phase": "Verifying",
            "epoch": 4,
            "fencing_token": TOKEN,
            "freeze_watermark": null,
        }))
        .unwrap();
        assert_eq!(
            stranded.transition(&MigrationEvent::CasSwitch, &ctx_for(&stranded)),
            Err(MigrationError::SwitchRequiresFreezeAck)
        );
    }

    #[test]
    fn empty_freeze_watermark_fences_nothing() {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        assert_eq!(
            pending.transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: String::new(),
                },
                &ctx_for(&pending),
            ),
            Err(MigrationError::EmptyFreezeWatermark)
        );
    }

    #[test]
    fn abort_before_switch_returns_to_stable_with_a_new_epoch() {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        let frozen = pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx_for(&pending),
            )
            .unwrap();
        let aborted = frozen
            .transition(
                &MigrationEvent::Abort {
                    reason: "target bucket unprovisioned".to_string(),
                    new_fencing_token: NEXT_TOKEN.to_string(),
                },
                &ctx_for(&frozen),
            )
            .unwrap();
        // CAS back to the source: new epoch, unfrozen, old attempt gone.
        assert_eq!(aborted.phase, MigrationPhase::Stable);
        assert_eq!(aborted.epoch, frozen.epoch + 1);
        assert_eq!(aborted.target, None);
        assert_eq!(aborted.migration_id, None);
        assert_eq!(aborted.freeze_watermark, None);
        // The rotated token fences the aborted attempt's writers, and a
        // fresh migration can open under a new ID afterwards.
        assert_eq!(aborted.fencing_token, NEXT_TOKEN);
        let stale = TransitionContext {
            expected_epoch: aborted.epoch,
            fencing_token: TOKEN,
        };
        assert_eq!(
            aborted.transition(&begin_event(), &stale),
            Err(MigrationError::StaleFencingToken {
                phase: MigrationPhase::Stable,
            })
        );
        let reopened = aborted
            .transition(&begin_event(), &ctx_for(&aborted))
            .unwrap();
        assert_eq!(reopened.phase, MigrationPhase::Pending);
    }

    #[test]
    fn abort_after_switch_is_irreversible() {
        let switched = to_switched();
        let abort = MigrationEvent::Abort {
            reason: "operator changed their mind".to_string(),
            new_fencing_token: NEXT_TOKEN.to_string(),
        };
        assert_eq!(
            switched.transition(&abort, &ctx_for(&switched)),
            Err(MigrationError::SwitchIsIrreversible)
        );
        let cleaning = switched
            .transition(&MigrationEvent::BeginCleanup, &ctx_for(&switched))
            .unwrap();
        assert_eq!(
            cleaning.transition(&abort, &ctx_for(&cleaning)),
            Err(MigrationError::SwitchIsIrreversible)
        );
        // And an empty reason never aborts, even before the switch.
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        assert_eq!(
            pending.transition(
                &MigrationEvent::Abort {
                    reason: String::new(),
                    new_fencing_token: NEXT_TOKEN.to_string(),
                },
                &ctx_for(&pending),
            ),
            Err(MigrationError::EmptyAbortReason)
        );
        assert_eq!(
            pending.transition(
                &MigrationEvent::Abort {
                    reason: "must rotate token".to_string(),
                    new_fencing_token: TOKEN.to_string(),
                },
                &ctx_for(&pending),
            ),
            Err(MigrationError::FencingTokenNotRotated)
        );
    }

    #[test]
    fn retired_is_terminal_and_wrong_phase_edges_are_forbidden() {
        let switched = to_switched();
        let cleaning = switched
            .transition(&MigrationEvent::BeginCleanup, &ctx_for(&switched))
            .unwrap();
        let retired = cleaning
            .transition(&MigrationEvent::Retire, &ctx_for(&cleaning))
            .unwrap();
        assert_eq!(
            retired.transition(&MigrationEvent::BeginCopy, &ctx_for(&retired)),
            Err(MigrationError::RetiredIsTerminal)
        );
        // Skipping phases is forbidden: no verify from Pending, no
        // switch from Copying, no retire from Stable.
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        assert_eq!(
            pending.transition(&MigrationEvent::BeginVerify, &ctx_for(&pending)),
            Err(MigrationError::ForbiddenTransition {
                phase: MigrationPhase::Pending,
                event: "begin_verify",
            })
        );
        let frozen = pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx_for(&pending),
            )
            .unwrap();
        let copying = frozen
            .transition(&MigrationEvent::BeginCopy, &ctx_for(&frozen))
            .unwrap();
        assert_eq!(
            copying.transition(&MigrationEvent::CasSwitch, &ctx_for(&copying)),
            Err(MigrationError::ForbiddenTransition {
                phase: MigrationPhase::Copying,
                event: "cas_switch",
            })
        );
        assert_eq!(
            stable().transition(&MigrationEvent::Retire, &ctx_for(&stable())),
            Err(MigrationError::ForbiddenTransition {
                phase: MigrationPhase::Stable,
                event: "retire",
            })
        );
    }

    #[test]
    fn minted_ids_have_the_binding_id_shape() {
        for bytes in [[0u8; 16], [1u8; 16], [171u8; 16], [255u8; 16]] {
            let id = mint_migration_id(&bytes);
            let token = mint_fencing_token(&bytes);
            assert_eq!(id.len(), MIGRATION_ID_HEX_LEN);
            assert_eq!(token.len(), MIGRATION_ID_HEX_LEN);
            assert!(valid_hex_id(&id));
            assert!(valid_hex_id(&token));
        }
        assert_eq!(
            mint_migration_id(&[0u8; 16]),
            "00000000000000000000000000000000"
        );
        // Production minting is unique per call with overwhelming
        // probability; two draws must differ.
        assert_ne!(random_migration_id(), random_fencing_token());
    }

    #[test]
    fn serde_twin_matches_the_durable_row_shape() {
        let pending = stable()
            .transition(&begin_event(), &ctx_for(&stable()))
            .unwrap();
        let value = serde_json::to_value(&pending).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "source": SOURCE,
                "target": TARGET,
                "migration_id": MIGRATION_ID,
                "phase": "Pending",
                "epoch": 1,
                "fencing_token": TOKEN,
                "freeze_watermark": null,
            })
        );
        let round_tripped: MigrationRecord = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, pending);
    }

    // Oracle for the property test below: the settled design's §7
    // transition table. An accepted event moves exactly one DAG edge
    // and exactly epoch+1; a refused event changes nothing and names a
    // stale-writer or illegal-edge rule; post-switch phases never
    // return to Stable under the same migration ID.
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn random_sequences_obey_the_dag_and_epoch_rules(
            seed in 0u64..256,
            events in proptest::collection::vec(0u8..8, 0..12),
        ) {
            // Deterministic walk: each byte picks an event family and
            // whether the writer presents the live token and epoch.
            let mut record = MigrationRecord::stable(SOURCE).unwrap();
            let mut rng_state = seed.wrapping_add(0x9e37_79b9);
            let mut next_byte = || {
                rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (rng_state >> 33) as u8
            };
            for pick in events {
                let token_choice = next_byte() % 3;
                let token = match token_choice {
                    0 => record.fencing_token.clone(),
                    1 => STALE_TOKEN.to_string(),
                    _ => String::new(),
                };
                let epoch = if next_byte() % 4 == 0 {
                    record.epoch.wrapping_add(1)
                } else {
                    record.epoch
                };
                let event = match pick {
                    0 => MigrationEvent::Begin {
                        target: TARGET.to_string(),
                        migration_id: MIGRATION_ID.to_string(),
                        fencing_token: mint_fencing_token(&[next_byte(); 16]),
                    },
                    1 => MigrationEvent::AcknowledgeFreeze {
                        watermark: if next_byte() % 5 == 0 {
                            String::new()
                        } else {
                            format!("watermark-{}", next_byte())
                        },
                    },
                    2 => MigrationEvent::BeginCopy,
                    3 => MigrationEvent::BeginVerify,
                    4 => MigrationEvent::CasSwitch,
                    5 => MigrationEvent::BeginCleanup,
                    6 => MigrationEvent::Retire,
                    _ => MigrationEvent::Abort {
                        reason: if next_byte() % 5 == 0 {
                            String::new()
                        } else {
                            "property abort".to_string()
                        },
                        new_fencing_token: mint_fencing_token(&[next_byte(); 16]),
                    },
                };
                let before = record.clone();
                let ctx = TransitionContext {
                    expected_epoch: epoch,
                    fencing_token: &token,
                };
                match record.transition(&event, &ctx) {
                    Ok(after) => {
                        // Exactly one epoch step, never a jump or stall.
                        prop_assert_eq!(after.epoch, before.epoch + 1);
                        // Post-switch phases never return to Stable
                        // under the same attempt: recovery is forward.
                        if matches!(
                            before.phase,
                            MigrationPhase::Switched | MigrationPhase::Cleaning
                        ) {
                            prop_assert_ne!(after.phase, MigrationPhase::Stable);
                        }
                        // Abort clears the attempt; every other edge
                        // keeps the migration ID it started with.
                        if matches!(event, MigrationEvent::Abort { .. }) {
                            prop_assert_eq!(after.phase, MigrationPhase::Stable);
                            prop_assert_eq!(after.migration_id.as_deref(), None);
                        } else if matches!(event, MigrationEvent::Begin { .. }) {
                            prop_assert!(after.migration_id.is_some());
                        } else {
                            prop_assert_eq!(after.migration_id.as_deref(), before.migration_id.as_deref());
                        }
                        record = after;
                    }
                    Err(err) => {
                        // A refusal changes nothing and always names a
                        // stale-writer or illegal-edge rule.
                        prop_assert!(matches!(
                            err,
                            MigrationError::StaleFencingToken { .. }
                                | MigrationError::EpochMismatch { .. }
                                | MigrationError::ForbiddenTransition { .. }
                                | MigrationError::SwitchIsIrreversible
                                | MigrationError::RetiredIsTerminal
                                | MigrationError::SwitchRequiresFreezeAck
                                | MigrationError::InvalidJurisdiction(_)
                                | MigrationError::SourceEqualsTarget
                                | MigrationError::MalformedMigrationId
                                | MigrationError::MalformedFencingToken
                                | MigrationError::FencingTokenNotRotated
                                | MigrationError::EmptyFreezeWatermark
                                | MigrationError::EmptyAbortReason
                        ), "transition refusal must name a machine rule");
                    }
                }
            }
        }
    }
}
