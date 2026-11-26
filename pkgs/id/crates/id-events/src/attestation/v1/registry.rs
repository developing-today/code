//! Registry handoff (#2039 phase 6).
//!
//! A registry is a counting view over synced evidence, not an authority over
//! the match (design `doc/2026-11-01_design_id2039.md` §6, Open decision 2).
//! [`handoff`] takes the reducer's answer and the audit reports it admitted
//! and returns what a registry with a given [`AuditPolicy`] would count:
//!
//! - Every live audit report keeps its exact standing. A sampled replay
//!   stays [`AuditStanding::Partial`] with its exact checked intervals; it
//!   never becomes a confirmation through handoff.
//! - The attestation state is never changed. A disputed match stays
//!   `Disputed`, with its objections and witness decisions still in the
//!   graph. A registry may count it under a separately labelled
//!   [`CountingBasis::AuditResolved`] basis only when its policy opts in, an
//!   auditor it accepts confirmed a full-history replay on a runtime it
//!   accepts, no accepted report contradicts that, and the confirmation
//!   names claims that agree on one outcome.
//! - A report confirms the claims it names in its evidence. A confirmation
//!   that names claims with different outcomes or final states resolves
//!   nothing.

use std::collections::HashMap;

use p2panda_core::Hash;

use super::V1MatchState;
use super::audit::{AuditReport, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval};
use super::reducer::{Basis, Evidence, Reduction};
use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim};

/// A registry's audit policy. The default accepts nobody and never counts a
/// match as audit-resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditPolicy {
    /// Whether this registry counts qualifying confirmations as an
    /// `audit-resolved` basis for a disputed match. Off by default.
    pub accept_audit_resolved: bool,
    /// The auditor keys whose reports this registry relies on.
    pub accepted_auditors: Vec<PlayerId>,
    /// The runtimes whose replays this registry relies on (v1 validity is
    /// scoped to the runtime the replay ran on).
    pub accepted_runtimes: Vec<String>,
}

/// What a live audit report says, as a registry reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditStanding {
    /// A clean full-history replay.
    Confirmed,
    /// Replay diverged at this checkpoint interval.
    Diverges(CheckpointInterval),
    /// The record failed L0/L1 checks.
    Invalid,
    /// Sampled replay matched only on these exact intervals.
    Partial(Vec<CheckpointInterval>),
    /// No verdict.
    Inconclusive,
}

/// One live audit report and whether the registry relies on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportStanding {
    /// The report's id.
    pub report: Hash,
    /// Who signed it.
    pub auditor: PlayerId,
    /// The role it was signed in.
    pub role: AuditorRole,
    /// What it says.
    pub standing: AuditStanding,
    /// Whether the registry's policy accepts this auditor and runtime.
    pub relied_on: bool,
}

/// The basis a registry counts a match under. Distinct labels, never merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountingBasis {
    /// Nothing to count: proposed, void, or disputed with no accepted audit
    /// resolution.
    None,
    /// Both players concurred.
    Concession,
    /// Witnessed on complete evidence.
    WitnessComplete,
    /// Witnessed on partial evidence; procedural, not replay proof.
    WitnessPartial,
    /// Disputed, but this registry accepted a full-history confirmation.
    AuditResolved {
        /// The confirming report.
        report: Hash,
        /// The outcome of the claims the report confirms.
        outcome: String,
    },
}

/// A registry's reading of one match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handoff {
    /// The reducer's state, unchanged.
    pub state: V1MatchState,
    /// The basis this registry counts the match under.
    pub basis: CountingBasis,
    /// Every live audit report's standing, in input order.
    pub reports: Vec<ReportStanding>,
}

fn standing(report: &AuditReport) -> AuditStanding {
    match (&report.scope, &report.verdict) {
        (AuditScope::FullHistory, AuditVerdict::ResultConfirmed) => AuditStanding::Confirmed,
        // validate() keeps a sampled scope from confirming; read any other
        // combination conservatively.
        (_, AuditVerdict::ResultConfirmed | AuditVerdict::Inconclusive { .. }) => {
            AuditStanding::Inconclusive
        }
        (_, AuditVerdict::ReplayDiverges { interval }) => AuditStanding::Diverges(*interval),
        (_, AuditVerdict::InvalidRecord { .. }) => AuditStanding::Invalid,
        (_, AuditVerdict::PartiallyVerified { intervals }) => {
            AuditStanding::Partial(intervals.clone())
        }
    }
}

/// The single outcome the claims a report names agree on, if any.
fn confirmed_outcome(report: &AuditReport, claims: &HashMap<Hash, &ResultClaim>) -> Option<String> {
    let mut named = report.evidence.iter().filter_map(|id| claims.get(id));
    let first = named.next()?;
    named
        .all(|claim| {
            claim.outcome == first.outcome && claim.final_state_hash == first.final_state_hash
        })
        .then(|| first.outcome.clone())
}

/// Reads `reduction` and the live audit reports in `evidence` under
/// `policy`. See the module documentation for the rules.
#[must_use]
pub fn handoff(
    policy: &AuditPolicy,
    agreement: &GenesisAgreement,
    evidence: &Evidence<'_>,
    reduction: &Reduction,
) -> Handoff {
    let reports: Vec<(ReportStanding, &AuditReport)> = evidence
        .audits
        .iter()
        .filter(|object| reduction.audits.contains(&object.id))
        .map(|object| {
            let relied_on = policy.accepted_auditors.contains(&object.author)
                && policy.accepted_runtimes.contains(&object.body.runtime);
            (
                ReportStanding {
                    report: object.id,
                    auditor: object.author,
                    role: object.body.role,
                    standing: standing(&object.body),
                    relied_on,
                },
                &object.body,
            )
        })
        .collect();

    let basis = match (reduction.state, reduction.basis) {
        (V1MatchState::Disputed, _) if policy.accept_audit_resolved => {
            audit_resolution(agreement, evidence, reduction, &reports)
        }
        (V1MatchState::Agreed, Basis::Concession) => CountingBasis::Concession,
        (V1MatchState::Witnessed | V1MatchState::Final, Basis::WitnessComplete) => {
            CountingBasis::WitnessComplete
        }
        (V1MatchState::Witnessed | V1MatchState::Final, Basis::WitnessPartial) => {
            CountingBasis::WitnessPartial
        }
        _ => CountingBasis::None,
    };
    Handoff {
        state: reduction.state,
        basis,
        reports: reports.into_iter().map(|(standing, _)| standing).collect(),
    }
}

fn audit_resolution(
    agreement: &GenesisAgreement,
    evidence: &Evidence<'_>,
    reduction: &Reduction,
    reports: &[(ReportStanding, &AuditReport)],
) -> CountingBasis {
    let relied: Vec<&(ReportStanding, &AuditReport)> = reports
        .iter()
        .filter(|(standing, _)| standing.relied_on)
        .collect();
    let contradicted = relied.iter().any(|(standing, _)| {
        matches!(
            standing.standing,
            AuditStanding::Diverges(_) | AuditStanding::Invalid
        )
    });
    if contradicted {
        return CountingBasis::None;
    }
    let Ok(agreement_hash) = agreement.agreement_hash() else {
        return CountingBasis::None;
    };
    let claims: HashMap<Hash, &ResultClaim> = evidence
        .claims
        .iter()
        .filter(|object| {
            object.body.agreement == agreement_hash
                && agreement.players.contains(&object.author)
                && !reduction.ignored.contains(&object.id)
        })
        .map(|object| (object.id, &object.body))
        .collect();
    let mut resolved: Option<(Hash, String)> = None;
    for (standing, report) in relied {
        if standing.standing != AuditStanding::Confirmed {
            continue;
        }
        let Some(outcome) = confirmed_outcome(report, &claims) else {
            return CountingBasis::None;
        };
        match &resolved {
            None => resolved = Some((standing.report, outcome)),
            Some((_, seen)) if *seen != outcome => return CountingBasis::None,
            Some(_) => {}
        }
    }
    resolved.map_or(CountingBasis::None, |(report, outcome)| {
        CountingBasis::AuditResolved { report, outcome }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{AuditPolicy, AuditStanding, CountingBasis, handoff};
    use crate::attestation::v1::V1MatchState;
    use crate::attestation::v1::audit::{
        AuditReport, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval, sign_audit_report,
    };
    use crate::attestation::v1::reducer::{EventPosition, Evidence, ForkProof, Signed, reduce};
    use crate::attestation::v1::{V1SignError, VERSION};
    use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim, player_id};
    use p2panda_core::{Hash, SigningKey};

    fn h(name: &str) -> Hash {
        Hash::digest(name.as_bytes())
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn id(seed: u8) -> PlayerId {
        player_id(&key(seed))
    }

    const RUNTIME: &str = "x86_64-linux";

    fn agreement() -> GenesisAgreement {
        GenesisAgreement {
            game_id: "tic-tac-toe".to_owned(),
            logic_hash: h("logic"),
            players: [id(1), id(2)],
            witness: Some(id(3)),
            objection_window_secs: 3600,
            witness_terms: None,
        }
    }

    fn hex(player: PlayerId) -> String {
        player.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn signed<T>(name: &str, author: PlayerId, seq: u64, body: T) -> Signed<T> {
        Signed {
            id: h(name),
            author,
            seq,
            after: Vec::new(),
            body,
        }
    }

    struct World {
        agreement: GenesisAgreement,
        genesis: Vec<Signed<GenesisAgreement>>,
        claims: Vec<Signed<ResultClaim>>,
        audits: Vec<Signed<AuditReport>>,
        positions: Vec<EventPosition>,
        forks: Vec<ForkProof>,
    }

    impl World {
        /// A disputed match: A claims a win for A, B claims a win for B.
        fn disputed() -> Self {
            let agreement = agreement();
            let claim = |player: u8, winner: u8| ResultClaim {
                agreement: agreement.agreement_hash().unwrap(),
                outcome: hex(id(winner)),
                final_state_hash: h(&format!("final {winner}")),
                frontiers: vec![h("transcript")],
            };
            Self {
                genesis: vec![
                    signed("genesis a", id(1), 0, agreement.clone()),
                    signed("genesis b", id(2), 0, agreement.clone()),
                ],
                claims: vec![
                    signed("claim a", id(1), 2, claim(1, 1)),
                    signed("claim b", id(2), 2, claim(2, 2)),
                ],
                audits: Vec::new(),
                positions: vec![EventPosition {
                    id: h("transcript"),
                    author: id(1),
                    seq: 1,
                }],
                forks: Vec::new(),
                agreement,
            }
        }

        fn audit(&mut self, name: &str, auditor: u8, scope: AuditScope, verdict: AuditVerdict) {
            let body = AuditReport {
                version: VERSION,
                role: AuditorRole::Independent,
                match_id: self.agreement.game_id.clone(),
                agreement: self.agreement.agreement_hash().unwrap(),
                transcript: h("transcript"),
                evidence: vec![h("claim a")],
                logic_hash: self.agreement.logic_hash,
                rules: h("rules"),
                runtime: RUNTIME.to_owned(),
                tool_version: "match-audit 1".to_owned(),
                scope,
                verdict,
                plausibility: None,
                metadata: Vec::new(),
            };
            self.audits.push(signed(name, id(auditor), 0, body));
        }

        fn confirm(&mut self, name: &str, auditor: u8) {
            self.audit(
                name,
                auditor,
                AuditScope::FullHistory,
                AuditVerdict::ResultConfirmed,
            );
        }

        fn handoff(&self, policy: &AuditPolicy) -> super::Handoff {
            let evidence = Evidence {
                genesis: &self.genesis,
                claims: &self.claims,
                audits: &self.audits,
                positions: &self.positions,
                forks: &self.forks,
                ..Evidence::default()
            };
            let reduction = reduce(&self.agreement, &evidence);
            handoff(policy, &self.agreement, &evidence, &reduction)
        }
    }

    fn policy() -> AuditPolicy {
        AuditPolicy {
            accept_audit_resolved: true,
            accepted_auditors: vec![id(7)],
            accepted_runtimes: vec![RUNTIME.to_owned()],
        }
    }

    fn resolved(report: &str) -> CountingBasis {
        CountingBasis::AuditResolved {
            report: h(report),
            outcome: hex(id(1)),
        }
    }

    type Row = (
        &'static str,
        fn(&mut World, &mut AuditPolicy),
        CountingBasis,
    );

    #[test]
    fn handoff_table() {
        let rows: &[Row] = &[
            (
                "a disputed match with no audit counts for nothing",
                |_, _| {},
                CountingBasis::None,
            ),
            (
                "an accepted full-history confirmation resolves the dispute",
                |w, _| w.confirm("audit", 7),
                resolved("audit"),
            ),
            (
                "a registry that has not opted in never resolves",
                |w, p| {
                    w.confirm("audit", 7);
                    p.accept_audit_resolved = false;
                },
                CountingBasis::None,
            ),
            (
                "the default policy never resolves",
                |w, p| {
                    w.confirm("audit", 7);
                    *p = AuditPolicy::default();
                },
                CountingBasis::None,
            ),
            (
                "an auditor the registry does not accept resolves nothing",
                |w, _| w.confirm("audit", 8),
                CountingBasis::None,
            ),
            (
                "a runtime the registry does not accept resolves nothing",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audits[0].body.runtime = "aarch64-darwin".to_owned();
                },
                CountingBasis::None,
            ),
            (
                "sampled replay never resolves",
                |w, _| {
                    let intervals = vec![CheckpointInterval { first: 0, last: 9 }];
                    w.audit(
                        "audit",
                        7,
                        AuditScope::Sampled {
                            intervals: intervals.clone(),
                        },
                        AuditVerdict::PartiallyVerified { intervals },
                    );
                },
                CountingBasis::None,
            ),
            (
                "an accepted divergence blocks the resolution",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audit(
                        "divergence",
                        7,
                        AuditScope::FullHistory,
                        AuditVerdict::ReplayDiverges {
                            interval: CheckpointInterval { first: 4, last: 5 },
                        },
                    );
                },
                CountingBasis::None,
            ),
            (
                "an unaccepted divergence does not block it",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audit(
                        "divergence",
                        8,
                        AuditScope::FullHistory,
                        AuditVerdict::ReplayDiverges {
                            interval: CheckpointInterval { first: 4, last: 5 },
                        },
                    );
                },
                resolved("audit"),
            ),
            (
                "a confirmation naming both disagreeing claims resolves nothing",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audits[0].body.evidence.push(h("claim b"));
                },
                CountingBasis::None,
            ),
            (
                "a confirmation naming no claim resolves nothing",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audits[0].body.evidence.clear();
                },
                CountingBasis::None,
            ),
            (
                "two accepted confirmations of different outcomes resolve nothing",
                |w, _| {
                    w.confirm("audit", 7);
                    w.confirm("other audit", 7);
                    w.audits[1].body.evidence = vec![h("claim b")];
                },
                CountingBasis::None,
            ),
            (
                "a confirmation on forked history is void and resolves nothing",
                |w, _| {
                    w.confirm("audit", 7);
                    w.forks.push(ForkProof {
                        author: id(7),
                        seq: 0,
                    });
                },
                CountingBasis::None,
            ),
            (
                "a seated player's own audit is never admitted",
                |w, p| {
                    w.confirm("audit", 1);
                    p.accepted_auditors.push(id(1));
                },
                CountingBasis::None,
            ),
            (
                "an outsider cannot audit in the witness role",
                |w, _| {
                    w.confirm("audit", 7);
                    w.audits[0].body.role = AuditorRole::Witness;
                },
                CountingBasis::None,
            ),
            (
                "the named witness may audit in the witness role",
                |w, p| {
                    w.confirm("audit", 3);
                    w.audits[0].body.role = AuditorRole::Witness;
                    p.accepted_auditors.push(id(3));
                },
                resolved("audit"),
            ),
            (
                "a concession counts as a concession whatever the audits say",
                |w, _| {
                    w.claims[1].body = w.claims[0].body.clone();
                    w.audit(
                        "divergence",
                        7,
                        AuditScope::FullHistory,
                        AuditVerdict::ReplayDiverges {
                            interval: CheckpointInterval { first: 4, last: 5 },
                        },
                    );
                },
                CountingBasis::Concession,
            ),
        ];
        for &(name, edit, ref basis) in rows {
            let mut world = World::disputed();
            let mut policy = policy();
            edit(&mut world, &mut policy);
            let handoff = world.handoff(&policy);
            assert_eq!(handoff.basis, *basis, "{name}");
            if handoff.basis != CountingBasis::Concession {
                assert_eq!(handoff.state, V1MatchState::Disputed, "{name}");
            }
        }
    }

    #[test]
    fn sampled_reports_stay_partial_with_their_exact_intervals() {
        let mut world = World::disputed();
        let intervals = vec![
            CheckpointInterval { first: 0, last: 3 },
            CheckpointInterval { first: 7, last: 9 },
        ];
        world.audit(
            "sampled",
            7,
            AuditScope::Sampled {
                intervals: intervals.clone(),
            },
            AuditVerdict::PartiallyVerified {
                intervals: intervals.clone(),
            },
        );
        world.confirm("full", 8);
        let handoff = world.handoff(&policy());
        assert_eq!(handoff.state, V1MatchState::Disputed);
        assert_eq!(handoff.reports.len(), 2);
        assert_eq!(
            handoff.reports[0].standing,
            AuditStanding::Partial(intervals)
        );
        assert!(handoff.reports[0].relied_on);
        assert_eq!(handoff.reports[1].standing, AuditStanding::Confirmed);
        assert!(!handoff.reports[1].relied_on);
    }

    #[test]
    fn handoff_keeps_the_objection_and_the_state() {
        let mut world = World::disputed();
        world.claims[1].body = world.claims[0].body.clone();
        world.confirm("audit", 7);
        let objection = crate::attestation::v1::Objection {
            version: VERSION,
            target: h("claim a"),
            grounds: "replay differs".to_owned(),
            author_frontier: Vec::new(),
        };
        let objections = vec![signed("objection", id(2), 3, objection)];
        let evidence = Evidence {
            genesis: &world.genesis,
            claims: &world.claims,
            objections: &objections,
            audits: &world.audits,
            positions: &world.positions,
            ..Evidence::default()
        };
        let reduction = reduce(&world.agreement, &evidence);
        assert_eq!(reduction.state, V1MatchState::Disputed);
        let handoff = handoff(&policy(), &world.agreement, &evidence, &reduction);
        assert_eq!(handoff.state, V1MatchState::Disputed);
        assert_eq!(handoff.basis, resolved("audit"));
        // Reducing again shows the objection is still in force.
        assert_eq!(
            reduce(&world.agreement, &evidence).state,
            V1MatchState::Disputed
        );
    }

    fn report(role: AuditorRole, agreement: &GenesisAgreement) -> AuditReport {
        AuditReport {
            version: VERSION,
            role,
            match_id: agreement.game_id.clone(),
            agreement: agreement.agreement_hash().unwrap(),
            transcript: h("transcript"),
            evidence: vec![h("claim a")],
            logic_hash: agreement.logic_hash,
            rules: h("rules"),
            runtime: RUNTIME.to_owned(),
            tool_version: "match-audit 1".to_owned(),
            scope: AuditScope::FullHistory,
            verdict: AuditVerdict::ResultConfirmed,
            plausibility: None,
            metadata: Vec::new(),
        }
    }

    #[test]
    fn audit_reports_sign_only_in_a_role_the_key_fits() {
        let agreement = agreement();
        let sign = |seed: u8, report: &AuditReport| {
            sign_audit_report(&key(seed), 0, None, &agreement, report, Vec::new())
        };
        let independent = report(AuditorRole::Independent, &agreement);
        let event = sign(7, &independent).unwrap();
        assert_eq!(
            event.header.extensions.kind,
            crate::attestation::v1::audit::AUDIT_REPORT_KIND
        );
        assert_eq!(
            event.header.payload_hash,
            Some(Hash::digest(independent.body().unwrap()))
        );
        assert!(matches!(sign(1, &independent), Err(V1SignError::WrongRole)));
        assert!(matches!(sign(3, &independent), Err(V1SignError::WrongRole)));
        let witness = report(AuditorRole::Witness, &agreement);
        assert!(sign(3, &witness).is_ok());
        assert!(matches!(sign(7, &witness), Err(V1SignError::WrongRole)));
        assert!(sign(7, &report(AuditorRole::Registry, &agreement)).is_ok());
        assert!(matches!(
            sign(2, &report(AuditorRole::Registry, &agreement)),
            Err(V1SignError::WrongRole)
        ));

        let mut sampled = independent.clone();
        sampled.scope = AuditScope::Sampled {
            intervals: vec![CheckpointInterval { first: 0, last: 1 }],
        };
        assert!(matches!(
            sign(7, &sampled),
            Err(V1SignError::InvalidAudit(_))
        ));
        let mut elsewhere = independent.clone();
        elsewhere.match_id = "chess".to_owned();
        assert!(matches!(
            sign(7, &elsewhere),
            Err(V1SignError::WrongAgreement)
        ));
        let mut other_logic = independent;
        other_logic.logic_hash = h("other logic");
        assert!(matches!(
            sign(7, &other_logic),
            Err(V1SignError::WrongLogic)
        ));
    }
}
