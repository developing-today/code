//! From a v0 match-audit verdict to a v1 attestation audit report (#2039
//! phase 6).
//!
//! [`verify_tic_tac_toe_v0`](super::verify_tic_tac_toe_v0) checks a match log
//! at L0–L2. [`attestation_report`] carries that verdict into the attestation
//! evidence graph as a v1 [`AuditReport`] an auditor can sign with
//! [`sign_audit_report`](id_events::attestation::v1::audit::sign_audit_report):
//!
//! - The transcript is the hash of the exact log bytes that were audited, and
//!   the program and rules hashes are the ones the log's signed header pins.
//! - A log whose pinned program is not the agreement's logic is an invalid
//!   record for that match, whatever its replay says.
//! - The scope stays honest: only a sampled replay becomes a sampled scope,
//!   and its exact intervals carry over.
//! - v0 names intervals by tick; v1 names them by checkpoint index. Index 0
//!   is the header's initial state, indices `1..=n` are the log's checkpoint
//!   entries in order, and `n + 1` is the footer's final state.
//! - The v0 actual-play screen becomes the L3 plausibility result; a missing
//!   or unavailable screen stays inconclusive.
//!
//! The report changes nothing by itself: the reducer only checks its evidence
//! closure, and a registry decides whether to rely on it.

use id_events::attestation::GenesisAgreement;
use id_events::attestation::v1::VERSION;
use id_events::attestation::v1::audit::{
    AuditReport, AuditReportError, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval,
    Plausibility,
};
use p2panda_core::Hash;

use super::{
    ActualValidityV0, AuditReportV0, CheckpointIntervalV0, EntryBodyV0, InconclusiveReasonV0,
    MatchLogV0, ScreeningDepthV0, TechnicalVerdictV0,
};

/// Who signs the report, about which match, and with what.
#[derive(Clone, Debug)]
pub struct AttestationContext<'a> {
    /// The agreement the audited match answers.
    pub agreement: &'a GenesisAgreement,
    /// The role the auditor holds.
    pub role: AuditorRole,
    /// Every other evidence object the audit relied on, such as the result
    /// claims, by hash.
    pub evidence: Vec<Hash>,
    /// The pinned runtime the replay ran on.
    pub runtime: String,
    /// The audit tool and its version.
    pub tool_version: String,
}

/// Why a v0 verdict cannot become a v1 report.
#[derive(Debug, thiserror::Error)]
pub enum AttestationReportError {
    /// The log bytes do not decode, so nothing binds them to the match.
    #[error("the audited log does not decode as a v0 match log")]
    Undecodable,
    /// The agreement cannot be hashed.
    #[error("the agreement cannot be hashed: {0}")]
    Agreement(String),
    /// A v0 interval names a tick no checkpoint of the log has.
    #[error("no checkpoint of the log is at tick {0}")]
    UnknownCheckpoint(u64),
    /// The mapped report breaks the v1 honest-scope rules.
    #[error("the mapped report breaks the v1 scope rules: {0}")]
    Scope(AuditReportError),
}

/// The ticks of the log's checkpoint indices: the header state, each
/// checkpoint entry, then the footer's final state.
fn checkpoint_ticks(log: &MatchLogV0) -> Vec<u64> {
    std::iter::once(log.header.start_tick)
        .chain(log.entries.iter().filter_map(|entry| match &entry.body {
            EntryBodyV0::Checkpoint(checkpoint) => Some(checkpoint.tick),
            EntryBodyV0::InputBlock(_) | EntryBodyV0::Event(_) => None,
        }))
        .chain(std::iter::once(log.footer.final_tick))
        .collect()
}

/// Maps a tick interval to checkpoint indices. The start is the latest
/// header or checkpoint state at its tick (replay names the last state that
/// still matched); the end is the next state at its tick, or the start
/// itself when the interval names one state.
fn interval(
    ticks: &[u64],
    v0: CheckpointIntervalV0,
) -> Result<CheckpointInterval, AttestationReportError> {
    let states = &ticks[..ticks.len() - 1];
    let first = states
        .iter()
        .rposition(|tick| *tick == v0.start_tick)
        .ok_or(AttestationReportError::UnknownCheckpoint(v0.start_tick))?;
    let last = ticks
        .iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, tick)| **tick == v0.end_tick)
        .map(|(index, _)| index)
        .or_else(|| (v0.start_tick == v0.end_tick).then_some(first))
        .ok_or(AttestationReportError::UnknownCheckpoint(v0.end_tick))?;
    let index = |index: usize| u64::try_from(index).unwrap_or(u64::MAX);
    Ok(CheckpointInterval {
        first: index(first),
        last: index(last),
    })
}

fn inconclusive_reason(reason: &InconclusiveReasonV0) -> &'static str {
    match reason {
        InconclusiveReasonV0::MissingProgramBuild => "missing program build",
        InconclusiveReasonV0::UnsupportedRules => "unsupported rules",
        InconclusiveReasonV0::MissingReplayCapability => "missing replay capability",
        InconclusiveReasonV0::IncompleteHistory => "incomplete history",
    }
}

fn plausibility(actual: &ActualValidityV0) -> Plausibility {
    match actual {
        ActualValidityV0::InconclusiveNoObservedMetadata => Plausibility::Inconclusive {
            reason: "no observed metadata".to_owned(),
        },
        ActualValidityV0::InconclusiveScreeningUnavailable => Plausibility::Inconclusive {
            reason: "screening unavailable".to_owned(),
        },
        ActualValidityV0::NoFlags { depth } => Plausibility::NoFlags {
            depth: match depth {
                ScreeningDepthV0::IntegrityOnly => 0,
                ScreeningDepthV0::Standard => 1,
                ScreeningDepthV0::Full => 2,
            },
        },
        ActualValidityV0::Flagged { signals } => Plausibility::Flagged {
            flags: signals.iter().map(|signal| format!("{signal:?}")).collect(),
        },
    }
}

/// Builds the v1 report for a v0 verdict on `log_bytes`.
///
/// # Errors
///
/// [`AttestationReportError::Undecodable`] when the bytes are not a v0 log,
/// [`AttestationReportError::UnknownCheckpoint`] when a verdict names a tick
/// the log has no state at, and [`AttestationReportError::Scope`] when the
/// mapped report breaks the v1 honest-scope rules.
pub fn attestation_report(
    log_bytes: &[u8],
    verdict: &AuditReportV0,
    context: AttestationContext<'_>,
) -> Result<AuditReport, AttestationReportError> {
    let log = MatchLogV0::decode(log_bytes).map_err(|_| AttestationReportError::Undecodable)?;
    let agreement = context
        .agreement
        .agreement_hash()
        .map_err(|error| AttestationReportError::Agreement(error.to_string()))?;
    let ticks = checkpoint_ticks(&log);

    let (scope, verdict_v1) = if log.header.program_hash == context.agreement.logic_hash {
        match &verdict.technical {
            TechnicalVerdictV0::InvalidRecord { reason } => (
                AuditScope::FullHistory,
                AuditVerdict::InvalidRecord {
                    reason: format!("{reason:?}"),
                },
            ),
            TechnicalVerdictV0::ResultConfirmed { .. } => {
                (AuditScope::FullHistory, AuditVerdict::ResultConfirmed)
            }
            TechnicalVerdictV0::ReplayDiverges {
                first_mismatching_interval,
            } => (
                AuditScope::FullHistory,
                AuditVerdict::ReplayDiverges {
                    interval: interval(&ticks, *first_mismatching_interval)?,
                },
            ),
            TechnicalVerdictV0::PartiallyVerified { intervals, .. } => {
                let intervals = intervals
                    .iter()
                    .map(|checked| interval(&ticks, *checked))
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    AuditScope::Sampled {
                        intervals: intervals.clone(),
                    },
                    AuditVerdict::PartiallyVerified { intervals },
                )
            }
            TechnicalVerdictV0::Inconclusive { reason } => (
                AuditScope::FullHistory,
                AuditVerdict::Inconclusive {
                    reason: inconclusive_reason(reason).to_owned(),
                },
            ),
        }
    } else {
        (
            AuditScope::FullHistory,
            AuditVerdict::InvalidRecord {
                reason: "the log pins a program other than the agreement's logic".to_owned(),
            },
        )
    };

    let report = AuditReport {
        version: VERSION,
        role: context.role,
        match_id: context.agreement.game_id.clone(),
        agreement,
        transcript: Hash::digest(log_bytes),
        evidence: context.evidence,
        logic_hash: context.agreement.logic_hash,
        rules: log.header.rules_hash,
        runtime: context.runtime,
        tool_version: context.tool_version,
        scope,
        verdict: verdict_v1,
        plausibility: Some(plausibility(&verdict.actual)),
        metadata: Vec::new(),
    };
    report.validate().map_err(AttestationReportError::Scope)?;
    Ok(report)
}

#[cfg(all(test, feature = "sandbox"))]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::{AttestationContext, AttestationReportError, attestation_report};
    use crate::match_audit::{
        ActualValidityV0, AuditReportV0, CheatingSignalV0, CheckpointIntervalV0, EntryBodyV0,
        MatchLogV0, RuntimeScopeV0, TIC_TAC_TOE_REFERENCE_WASM, TechnicalVerdictV0,
        export_tic_tac_toe_reference, verify_tic_tac_toe_v0,
    };
    use id_events::attestation::v1::V1SignError;
    use id_events::attestation::v1::audit::{
        AuditScope, AuditVerdict, AuditorRole, CheckpointInterval, Plausibility, sign_audit_report,
    };
    use id_events::attestation::{GenesisAgreement, player_id};
    use p2panda_core::{Hash, SigningKey};

    const RUNTIME: &str = "id-worlds-sandbox/wasm";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn agreement() -> GenesisAgreement {
        GenesisAgreement {
            game_id: "tic-tac-toe".to_owned(),
            logic_hash: Hash::digest(TIC_TAC_TOE_REFERENCE_WASM),
            players: [player_id(&key(1)), player_id(&key(2))],
            witness: None,
            objection_window_secs: 3600,
            witness_terms: None,
        }
    }

    fn context(agreement: &GenesisAgreement) -> AttestationContext<'_> {
        AttestationContext {
            agreement,
            role: AuditorRole::Independent,
            evidence: vec![Hash::digest(b"claim a"), Hash::digest(b"claim b")],
            runtime: RUNTIME.to_owned(),
            tool_version: "match-audit v0".to_owned(),
        }
    }

    fn reference() -> (MatchLogV0, Vec<u8>) {
        let log =
            export_tic_tac_toe_reference([&key(1), &key(2)], "match-1", "world-1", 7).unwrap();
        let bytes = log.encode().unwrap();
        (log, bytes)
    }

    fn with(technical: TechnicalVerdictV0) -> AuditReportV0 {
        AuditReportV0 {
            technical,
            actual: ActualValidityV0::InconclusiveNoObservedMetadata,
        }
    }

    fn ticks(start_tick: u64, end_tick: u64) -> CheckpointIntervalV0 {
        CheckpointIntervalV0 {
            start_tick,
            end_tick,
        }
    }

    #[test]
    fn a_confirmed_replay_becomes_a_signable_full_history_report() {
        let agreement = agreement();
        let (log, bytes) = reference();
        let verdict = verify_tic_tac_toe_v0(&bytes);
        assert!(matches!(
            verdict.technical,
            TechnicalVerdictV0::ResultConfirmed {
                runtime: RuntimeScopeV0::PinnedRuntime
            }
        ));
        let report = attestation_report(&bytes, &verdict, context(&agreement)).unwrap();
        assert_eq!(report.scope, AuditScope::FullHistory);
        assert_eq!(report.verdict, AuditVerdict::ResultConfirmed);
        assert_eq!(report.transcript, Hash::digest(&bytes));
        assert_eq!(report.rules, log.header.rules_hash);
        assert_eq!(report.logic_hash, agreement.logic_hash);
        assert_eq!(report.agreement, agreement.agreement_hash().unwrap());
        assert_eq!(report.runtime, RUNTIME);
        assert_eq!(
            report.plausibility,
            Some(Plausibility::Inconclusive {
                reason: "no observed metadata".to_owned()
            })
        );

        let event = sign_audit_report(&key(7), 0, None, &agreement, &report, Vec::new()).unwrap();
        assert_eq!(
            event.header.payload_hash,
            Some(Hash::digest(report.body().unwrap()))
        );
        // A seated player never audits its own match.
        assert!(matches!(
            sign_audit_report(&key(1), 0, None, &agreement, &report, Vec::new()),
            Err(V1SignError::WrongRole)
        ));
    }

    #[test]
    fn a_tampered_log_becomes_an_invalid_record() {
        let agreement = agreement();
        let (mut log, _) = reference();
        log.footer.final_state_hash = Hash::digest(b"forged");
        let bytes = log.encode().unwrap();
        let verdict = verify_tic_tac_toe_v0(&bytes);
        let report = attestation_report(&bytes, &verdict, context(&agreement)).unwrap();
        assert_eq!(report.scope, AuditScope::FullHistory);
        assert!(matches!(report.verdict, AuditVerdict::InvalidRecord { .. }));
        assert!(sign_audit_report(&key(7), 0, None, &agreement, &report, Vec::new()).is_ok());
    }

    #[test]
    fn a_log_for_another_program_is_an_invalid_record_for_this_match() {
        let mut agreement = agreement();
        agreement.logic_hash = Hash::digest(b"another program");
        let (_, bytes) = reference();
        let verdict = verify_tic_tac_toe_v0(&bytes);
        let report = attestation_report(&bytes, &verdict, context(&agreement)).unwrap();
        assert!(matches!(report.verdict, AuditVerdict::InvalidRecord { .. }));
        assert_eq!(report.logic_hash, agreement.logic_hash);
    }

    #[test]
    fn tick_intervals_become_checkpoint_indices() {
        let agreement = agreement();
        let (log, bytes) = reference();
        let checkpoints: Vec<u64> = log
            .entries
            .iter()
            .filter_map(|entry| match &entry.body {
                EntryBodyV0::Checkpoint(checkpoint) => Some(checkpoint.tick),
                _ => None,
            })
            .collect();
        assert!(checkpoints.len() >= 2, "{checkpoints:?}");
        let n = u64::try_from(checkpoints.len()).unwrap();
        let start = log.header.start_tick;
        let last = *checkpoints.last().unwrap();
        let diverged = |v0: CheckpointIntervalV0| {
            let report = attestation_report(
                &bytes,
                &with(TechnicalVerdictV0::ReplayDiverges {
                    first_mismatching_interval: v0,
                }),
                context(&agreement),
            )
            .unwrap();
            assert_eq!(report.scope, AuditScope::FullHistory);
            match report.verdict {
                AuditVerdict::ReplayDiverges { interval } => (interval.first, interval.last),
                other => panic!("{other:?}"),
            }
        };
        // The initial state itself did not reproduce.
        assert_eq!(diverged(ticks(start, start)), (0, 0));
        // The first checkpoint did not reproduce from the initial state.
        assert_eq!(diverged(ticks(start, checkpoints[0])), (0, 1));
        // A later checkpoint did not reproduce from the one before it.
        assert_eq!(diverged(ticks(checkpoints[0], checkpoints[1])), (1, 2));
        // The footer's final state did not reproduce from the last checkpoint.
        assert_eq!(diverged(ticks(last, log.footer.final_tick)), (n, n + 1));

        let unknown = attestation_report(
            &bytes,
            &with(TechnicalVerdictV0::ReplayDiverges {
                first_mismatching_interval: ticks(start, 999),
            }),
            context(&agreement),
        );
        assert!(matches!(
            unknown,
            Err(AttestationReportError::UnknownCheckpoint(999))
        ));
    }

    #[test]
    fn a_sampled_replay_stays_sampled_with_its_exact_intervals() {
        let agreement = agreement();
        let (log, bytes) = reference();
        let checkpoints: Vec<u64> = log
            .entries
            .iter()
            .filter_map(|entry| match &entry.body {
                EntryBodyV0::Checkpoint(checkpoint) => Some(checkpoint.tick),
                _ => None,
            })
            .collect();
        assert!(checkpoints.len() >= 4, "{checkpoints:?}");
        let report = attestation_report(
            &bytes,
            &with(TechnicalVerdictV0::PartiallyVerified {
                intervals: vec![
                    ticks(log.header.start_tick, checkpoints[0]),
                    ticks(checkpoints[2], checkpoints[3]),
                ],
                runtime: RuntimeScopeV0::PinnedRuntime,
            }),
            context(&agreement),
        )
        .unwrap();
        let intervals = vec![
            CheckpointInterval { first: 0, last: 1 },
            CheckpointInterval { first: 3, last: 4 },
        ];
        assert_eq!(
            report.scope,
            AuditScope::Sampled {
                intervals: intervals.clone()
            }
        );
        assert_eq!(
            report.verdict,
            AuditVerdict::PartiallyVerified { intervals }
        );

        // Touching intervals break the v1 rules and are refused, not merged.
        let touching = attestation_report(
            &bytes,
            &with(TechnicalVerdictV0::PartiallyVerified {
                intervals: vec![
                    ticks(log.header.start_tick, checkpoints[0]),
                    ticks(checkpoints[0], checkpoints[1]),
                ],
                runtime: RuntimeScopeV0::PinnedRuntime,
            }),
            context(&agreement),
        );
        assert!(matches!(touching, Err(AttestationReportError::Scope(_))));
    }

    #[test]
    fn screening_results_carry_over_as_plausibility() {
        let agreement = agreement();
        let (_, bytes) = reference();
        let report = attestation_report(
            &bytes,
            &AuditReportV0 {
                technical: TechnicalVerdictV0::ResultConfirmed {
                    runtime: RuntimeScopeV0::PinnedRuntime,
                },
                actual: ActualValidityV0::Flagged {
                    signals: vec![CheatingSignalV0::EntropyCollapse],
                },
            },
            context(&agreement),
        )
        .unwrap();
        assert_eq!(
            report.plausibility,
            Some(Plausibility::Flagged {
                flags: vec!["EntropyCollapse".to_owned()]
            })
        );
        // A flag never changes the L2 verdict.
        assert_eq!(report.verdict, AuditVerdict::ResultConfirmed);
    }

    #[test]
    fn undecodable_bytes_are_refused() {
        let agreement = agreement();
        let verdict = verify_tic_tac_toe_v0(b"not a log");
        assert!(matches!(
            attestation_report(b"not a log", &verdict, context(&agreement)),
            Err(AttestationReportError::Undecodable)
        ));
    }
}
