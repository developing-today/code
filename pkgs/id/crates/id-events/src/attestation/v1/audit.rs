//! The frozen v1 audit-report object schema (#2039 phase 4).
//!
//! An audit report is evidence, separate from a witness decision: it binds
//! the report author's role, the match and agreement, the exact transcript
//! and evidence hashes, the pinned program, rules and runtime, the audit-tool
//! version, the scope that was checked, the verdict, and any L3 metadata
//! references and flags (design `doc/2026-11-01_design_id2039.md` §5).
//!
//! This module freezes the shape and the honest-scope rules that every
//! report must satisfy, and signs reports ([`sign_audit_report`], #2039
//! phase 6) only when the signer's key fits the role the report claims. The
//! reducer ([`super::reducer`]) only checks a report's evidence closure and
//! never lets a report change the attestation state; a registry decides
//! whether to accept one as an `audit-resolved` basis
//! ([`super::registry::handoff`]).

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use super::receipt::WitnessTerms;
use super::{V1SignError, VERSION, sign_body};
use crate::attestation::{GenesisAgreement, PlayerId, player_id};
use id_store::chain::Event;

/// The event kind that carries an audit-report body.
pub const AUDIT_REPORT_KIND: &str = "attestation.audit_report";

/// The role the report's author held when it audited the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditorRole {
    /// The witness named in the agreement, auditing as well as deciding.
    Witness,
    /// An independent auditor with no seat and no witness slot.
    Independent,
    /// A registry auditing evidence it gathered.
    Registry,
}

impl AuditorRole {
    /// Whether `author` may sign a report in this role for `agreement`. A
    /// seated player never audits its own match. The witness role belongs to
    /// the genesis-named witness or pinned fallback; an independent auditor
    /// holds no witness slot and is not the receipt issuer; a registry is any
    /// unseated key auditing evidence it gathered.
    #[must_use]
    pub fn fits(self, agreement: &GenesisAgreement, author: &PlayerId) -> bool {
        if agreement.players.contains(author) {
            return false;
        }
        let witness = WitnessTerms::may_decide(agreement, author);
        match self {
            Self::Witness => witness,
            Self::Independent => {
                !witness && WitnessTerms::of(agreement).receipt_issuer.as_ref() != Some(author)
            }
            Self::Registry => true,
        }
    }
}

/// An inclusive interval of transcript checkpoints, `[first, last]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointInterval {
    /// The first checkpoint index in the interval.
    pub first: u64,
    /// The last checkpoint index in the interval (inclusive).
    pub last: u64,
}

/// What the replay covered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditScope {
    /// Every checkpoint from the pre-play header to the footer was replayed.
    FullHistory,
    /// Only the listed intervals were replayed. The list survives handoff.
    Sampled {
        /// The exact intervals that were checked, in ascending order.
        intervals: Vec<CheckpointInterval>,
    },
}

/// The L0–L2 verdict vocabulary (design §5). Display strings follow the
/// design exactly, e.g. `REPLAY DIVERGES — checkpoint interval [a,b]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditVerdict {
    /// L0/L1 failure: unsupported format, bad signature, broken hash chain
    /// or mismatched pre-play header, footer or binding.
    InvalidRecord {
        /// What failed.
        reason: String,
    },
    /// A clean full-history replay reproduced every checkpoint and the final
    /// state. Only valid with [`AuditScope::FullHistory`].
    ResultConfirmed,
    /// Replay diverged; the first mismatching checkpoint interval. A replay
    /// without per-tick commitments cannot name an exact tick.
    ReplayDiverges {
        /// The first interval whose checkpoint hash did not reproduce.
        interval: CheckpointInterval,
    },
    /// Sampled replay matched on exactly these intervals. Never a confirmed
    /// result. Only valid with [`AuditScope::Sampled`] naming the same list.
    PartiallyVerified {
        /// The intervals that matched.
        intervals: Vec<CheckpointInterval>,
    },
    /// Missing inputs or runtime limitations prevented a verdict.
    Inconclusive {
        /// Why.
        reason: String,
    },
}

impl std::fmt::Display for AuditVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRecord { reason } => write!(f, "INVALID RECORD — {reason}"),
            Self::ResultConfirmed => write!(f, "RESULT CONFIRMED — full-history"),
            Self::ReplayDiverges { interval } => write!(
                f,
                "REPLAY DIVERGES — checkpoint interval [{},{}]",
                interval.first, interval.last
            ),
            Self::PartiallyVerified { intervals } => {
                write!(f, "PARTIALLY VERIFIED — intervals [")?;
                for (index, interval) in intervals.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "[{},{}]", interval.first, interval.last)?;
                }
                write!(f, "]")
            }
            Self::Inconclusive { reason } => write!(f, "INCONCLUSIVE — {reason}"),
        }
    }
}

/// The optional L3 plausibility result. Probabilistic screening, never
/// proof of fair play or guilt; the L2 basis is carried beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Plausibility {
    /// Screening raised these flags.
    Flagged {
        /// The flags raised.
        flags: Vec<String>,
    },
    /// Screening at this depth raised nothing.
    NoFlags {
        /// The screening depth.
        depth: u32,
    },
    /// No observed metadata, RTT-only timing, or tick-quantized input left
    /// the signal undecidable.
    Inconclusive {
        /// Why, e.g. `no observed metadata`.
        reason: String,
    },
}

/// A v1 audit report body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    /// Schema version; [`VERSION`].
    pub version: u8,
    /// The role the author held.
    pub role: AuditorRole,
    /// The match (game) id the agreement names.
    pub match_id: String,
    /// The agreement hash the audited match answers.
    pub agreement: Hash,
    /// The exact transcript hash that was audited.
    pub transcript: Hash,
    /// Every other evidence object the audit relied on, by hash: claims,
    /// signed inputs, witness decisions, frontiers.
    pub evidence: Vec<Hash>,
    /// The pinned program/logic hash the replay ran.
    pub logic_hash: Hash,
    /// The pinned rules identity.
    pub rules: Hash,
    /// The pinned runtime/build the replay ran on. v1 validity is scoped to
    /// this runtime (Open decision 3, option b).
    pub runtime: String,
    /// The audit tool and its version.
    pub tool_version: String,
    /// What the replay covered.
    pub scope: AuditScope,
    /// The L0–L2 verdict.
    pub verdict: AuditVerdict,
    /// The optional L3 result; absent when L3 was not attempted.
    pub plausibility: Option<Plausibility>,
    /// References to (encrypted) observed-metadata objects; never raw IPs.
    pub metadata: Vec<Hash>,
}

impl AuditReport {
    /// The canonical body bytes this report is signed over.
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes a report from a signed event's body bytes, refusing repeated
    /// object keys (#8794).
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        id_core::json_unique::from_slice_without_duplicate_keys(bytes)
    }

    /// Checks the honest-scope rules (invariant 6): a sampled audit never
    /// confirms a result, a partial verdict names exactly the sampled
    /// intervals, a full-history audit is never partial, and intervals are
    /// well formed and ascending.
    pub fn validate(&self) -> Result<(), AuditReportError> {
        if self.version != VERSION {
            return Err(AuditReportError::WrongVersion);
        }
        if let AuditScope::Sampled { intervals } = &self.scope {
            check_intervals(intervals)?;
        }
        match (&self.scope, &self.verdict) {
            (AuditScope::Sampled { .. }, AuditVerdict::ResultConfirmed) => {
                Err(AuditReportError::SampledConfirmation)
            }
            (AuditScope::FullHistory, AuditVerdict::PartiallyVerified { .. }) => {
                Err(AuditReportError::FullHistoryPartial)
            }
            (
                AuditScope::Sampled { intervals },
                AuditVerdict::PartiallyVerified { intervals: checked },
            ) if intervals != checked => Err(AuditReportError::IntervalsDiffer),
            (_, AuditVerdict::ReplayDiverges { interval }) => {
                check_intervals(std::slice::from_ref(interval))
            }
            _ => Ok(()),
        }
    }

    /// Every hash this report depends on: the transcript, evidence and
    /// metadata references.
    #[must_use]
    pub fn dependencies(&self) -> Vec<Hash> {
        std::iter::once(self.transcript)
            .chain(self.evidence.iter().copied())
            .chain(self.metadata.iter().copied())
            .collect()
    }
}

/// Signs an audit report. The report must pass the honest-scope rules, name
/// this agreement, its match id and its pinned logic, and claim a role the
/// signer's key fits ([`AuditorRole::fits`]).
///
/// # Errors
///
/// [`V1SignError::InvalidAudit`] for a report that breaks the scope rules,
/// [`V1SignError::WrongAgreement`] or [`V1SignError::WrongLogic`] for a
/// report about another match, and [`V1SignError::WrongRole`] when the
/// signer cannot hold the claimed role.
pub fn sign_audit_report(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    report: &AuditReport,
    after: Vec<Hash>,
) -> Result<Event, V1SignError> {
    report.validate().map_err(V1SignError::InvalidAudit)?;
    if report.agreement != agreement.agreement_hash()? || report.match_id != agreement.game_id {
        return Err(V1SignError::WrongAgreement);
    }
    if report.logic_hash != agreement.logic_hash {
        return Err(V1SignError::WrongLogic);
    }
    if !report.role.fits(agreement, &player_id(key)) {
        return Err(V1SignError::WrongRole);
    }
    sign_body(
        key,
        seq_num,
        backlink,
        report.body()?,
        AUDIT_REPORT_KIND,
        after,
    )
    .map_err(V1SignError::from)
}

fn check_intervals(intervals: &[CheckpointInterval]) -> Result<(), AuditReportError> {
    if intervals.is_empty() {
        return Err(AuditReportError::BadInterval);
    }
    let mut previous_last: Option<u64> = None;
    for interval in intervals {
        if interval.first > interval.last
            || previous_last.is_some_and(|last| interval.first <= last)
        {
            return Err(AuditReportError::BadInterval);
        }
        previous_last = Some(interval.last);
    }
    Ok(())
}

/// Why an audit report breaks the frozen v1 rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditReportError {
    /// The report is not schema version [`VERSION`].
    WrongVersion,
    /// A sampled audit claims `RESULT CONFIRMED`.
    SampledConfirmation,
    /// A full-history audit reports a partial verification.
    FullHistoryPartial,
    /// A partial verdict's intervals differ from the sampled scope's list.
    IntervalsDiffer,
    /// An interval list is empty, reversed, overlapping or out of order.
    BadInterval,
}

impl std::fmt::Display for AuditReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongVersion => write!(f, "audit report is not schema version {VERSION}"),
            Self::SampledConfirmation => {
                write!(f, "a sampled audit can never confirm a result")
            }
            Self::FullHistoryPartial => {
                write!(f, "a full-history audit cannot report partial verification")
            }
            Self::IntervalsDiffer => write!(
                f,
                "partial verdict intervals differ from the sampled scope's interval list"
            ),
            Self::BadInterval => write!(
                f,
                "checkpoint intervals must be non-empty, ordered, and non-overlapping"
            ),
        }
    }
}

impl std::error::Error for AuditReportError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        AuditReport, AuditReportError, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval,
        Plausibility,
    };
    use crate::attestation::v1::VERSION;
    use p2panda_core::Hash;

    const fn interval(first: u64, last: u64) -> CheckpointInterval {
        CheckpointInterval { first, last }
    }

    fn report(scope: AuditScope, verdict: AuditVerdict) -> AuditReport {
        AuditReport {
            version: VERSION,
            role: AuditorRole::Independent,
            match_id: "tic-tac-toe".to_owned(),
            agreement: Hash::digest(b"agreement"),
            transcript: Hash::digest(b"transcript"),
            evidence: vec![Hash::digest(b"claim a"), Hash::digest(b"claim b")],
            logic_hash: Hash::digest(b"logic"),
            rules: Hash::digest(b"rules"),
            runtime: "x86_64-linux/id-0.1.0".to_owned(),
            tool_version: "match-audit 1".to_owned(),
            scope,
            verdict,
            plausibility: Some(Plausibility::Inconclusive {
                reason: "no observed metadata".to_owned(),
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn honest_scope_rules_table() {
        let sampled = || AuditScope::Sampled {
            intervals: vec![interval(0, 9), interval(20, 29)],
        };
        let rows: Vec<(&str, AuditReport, Result<(), AuditReportError>)> = vec![
            (
                "full-history confirmation",
                report(AuditScope::FullHistory, AuditVerdict::ResultConfirmed),
                Ok(()),
            ),
            (
                "sampled confirmation is refused",
                report(sampled(), AuditVerdict::ResultConfirmed),
                Err(AuditReportError::SampledConfirmation),
            ),
            (
                "sampled partial with the same list",
                report(
                    sampled(),
                    AuditVerdict::PartiallyVerified {
                        intervals: vec![interval(0, 9), interval(20, 29)],
                    },
                ),
                Ok(()),
            ),
            (
                "sampled partial with a different list",
                report(
                    sampled(),
                    AuditVerdict::PartiallyVerified {
                        intervals: vec![interval(0, 9)],
                    },
                ),
                Err(AuditReportError::IntervalsDiffer),
            ),
            (
                "full-history partial is refused",
                report(
                    AuditScope::FullHistory,
                    AuditVerdict::PartiallyVerified {
                        intervals: vec![interval(0, 9)],
                    },
                ),
                Err(AuditReportError::FullHistoryPartial),
            ),
            (
                "overlapping sampled intervals",
                report(
                    AuditScope::Sampled {
                        intervals: vec![interval(0, 9), interval(5, 12)],
                    },
                    AuditVerdict::Inconclusive {
                        reason: "missing inputs".to_owned(),
                    },
                ),
                Err(AuditReportError::BadInterval),
            ),
            (
                "empty sampled scope",
                report(
                    AuditScope::Sampled {
                        intervals: Vec::new(),
                    },
                    AuditVerdict::Inconclusive {
                        reason: "missing inputs".to_owned(),
                    },
                ),
                Err(AuditReportError::BadInterval),
            ),
            (
                "reversed divergence interval",
                report(
                    AuditScope::FullHistory,
                    AuditVerdict::ReplayDiverges {
                        interval: interval(9, 3),
                    },
                ),
                Err(AuditReportError::BadInterval),
            ),
            (
                "divergence names its first interval",
                report(
                    AuditScope::FullHistory,
                    AuditVerdict::ReplayDiverges {
                        interval: interval(30, 39),
                    },
                ),
                Ok(()),
            ),
            (
                "invalid record",
                report(
                    AuditScope::FullHistory,
                    AuditVerdict::InvalidRecord {
                        reason: "header signature".to_owned(),
                    },
                ),
                Ok(()),
            ),
            (
                "wrong version",
                AuditReport {
                    version: VERSION + 1,
                    ..report(AuditScope::FullHistory, AuditVerdict::ResultConfirmed)
                },
                Err(AuditReportError::WrongVersion),
            ),
        ];
        for (name, report, expected) in rows {
            assert_eq!(report.validate(), expected, "{name}");
        }
    }

    #[test]
    fn verdicts_render_the_design_vocabulary() {
        assert_eq!(
            AuditVerdict::ResultConfirmed.to_string(),
            "RESULT CONFIRMED — full-history"
        );
        assert_eq!(
            AuditVerdict::ReplayDiverges {
                interval: interval(10, 19)
            }
            .to_string(),
            "REPLAY DIVERGES — checkpoint interval [10,19]"
        );
        assert_eq!(
            AuditVerdict::PartiallyVerified {
                intervals: vec![interval(0, 9), interval(20, 29)]
            }
            .to_string(),
            "PARTIALLY VERIFIED — intervals [[0,9], [20,29]]"
        );
        assert_eq!(
            AuditVerdict::Inconclusive {
                reason: "missing inputs".to_owned()
            }
            .to_string(),
            "INCONCLUSIVE — missing inputs"
        );
    }

    #[test]
    fn body_round_trips_and_refuses_repeated_keys() {
        let report = report(
            AuditScope::Sampled {
                intervals: vec![interval(0, 9)],
            },
            AuditVerdict::PartiallyVerified {
                intervals: vec![interval(0, 9)],
            },
        );
        let body = report.body().unwrap();
        assert_eq!(AuditReport::from_body(&body).unwrap(), report);
        let mut repeated = br#"{"runtime":"other","#.to_vec();
        repeated.extend_from_slice(&body[1..]);
        let error = AuditReport::from_body(&repeated).unwrap_err();
        assert!(error.to_string().contains(r#""runtime""#), "{error}");
        assert_eq!(report.dependencies().len(), 3);
    }
}
