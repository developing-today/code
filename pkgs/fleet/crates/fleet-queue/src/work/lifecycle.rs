//! V2 work-item lifecycle state and the compatibility crosswalk from
//! the stored Fleet queue task states (#9501, slice CLI-01 under the
//! #9206 design).
//!
//! This module is deliberately a pure model: it names the V2 workflow
//! states, the candidate transition table from the #9206 design (§3),
//! and the loss-aware projection of each stored queue `tasks.status`
//! value onto the workflow dimension. It performs no I/O, renames no
//! stored value, and adopts nothing at runtime. The queue keeps its
//! current vocabulary; this crosswalk is the contract a future V2
//! adapter must follow.
//!
//! Loss-aware means every projection declares what it does not carry.
//! A source state either projects with its losses named, projects only
//! when named evidence is present (conditional), or is explicitly
//! unmapped with the reason recorded. The source state is preserved
//! verbatim on every mapping, so no projection can silently rename or
//! drop it.

/// One stored Fleet queue task state: a value of `tasks.status` in the
/// queue store (schema default `queued`). These spellings are the
/// compatibility surface — never rename them here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueState {
    /// Waiting to be claimed (the stored default).
    Queued,
    /// A worker holds a live claim/lease on the task.
    Claimed,
    /// Parked until a wake time or an explicit wake.
    Parked,
    /// Blocked, by a hard dependency or manually.
    Blocked,
    /// Ownership was transferred to another worker.
    HandedOff,
    /// The queue closed the task as done.
    Done,
    /// Terminal: cancelled by an authorized actor.
    Cancelled,
    /// Terminal: failed.
    Failed,
}

impl QueueState {
    /// Every stored queue task state, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Claimed,
        Self::Parked,
        Self::Blocked,
        Self::HandedOff,
        Self::Done,
        Self::Cancelled,
        Self::Failed,
    ];

    /// The stored spelling of this state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Parked => "parked",
            Self::Blocked => "blocked",
            Self::HandedOff => "handed_off",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Parse a stored `tasks.status` value. Unknown spellings —
    /// including unratified proposal vocabulary such as `in_progress`
    /// — return `None`; a stored value is never silently renamed.
    pub fn from_stored(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == value)
    }
}

/// A V2 workflow state: the lifecycle phase of a canonical work item
/// (#9206 design §3). Workflow is one of four independent dimensions;
/// readiness, ownership/execution, and remote-resource state are
/// separate dimensions and are not encoded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowState {
    /// Exists and is not yet active.
    Open,
    /// Claimed and being worked.
    Active,
    /// Suspended with ownership released; resumable.
    Parked,
    /// Waiting on a blocker.
    Blocked,
    /// Completion intent recorded; remote effects being verified.
    Completing,
    /// Terminal: all required effects verified complete.
    Completed,
    /// Terminal: cancelled.
    Cancelled,
    /// Terminal: failed.
    Failed,
}

impl WorkflowState {
    /// Every V2 workflow state, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Open,
        Self::Active,
        Self::Parked,
        Self::Blocked,
        Self::Completing,
        Self::Completed,
        Self::Cancelled,
        Self::Failed,
    ];

    /// The canonical V2 spelling of this state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Blocked => "blocked",
            Self::Completing => "completing",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Terminal states admit no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

/// The candidate V2 transition table, from the #9206 design §3
/// transition matrix. Transitions are validated against this table,
/// never by arbitrary assignment. Terminal states have no outgoing
/// edges, and no state transitions to itself.
pub const TRANSITIONS: &[(WorkflowState, WorkflowState)] = &[
    (WorkflowState::Open, WorkflowState::Active),
    (WorkflowState::Open, WorkflowState::Blocked),
    (WorkflowState::Open, WorkflowState::Cancelled),
    (WorkflowState::Open, WorkflowState::Failed),
    (WorkflowState::Active, WorkflowState::Parked),
    (WorkflowState::Active, WorkflowState::Blocked),
    (WorkflowState::Active, WorkflowState::Completing),
    (WorkflowState::Active, WorkflowState::Cancelled),
    (WorkflowState::Active, WorkflowState::Failed),
    (WorkflowState::Parked, WorkflowState::Active),
    (WorkflowState::Parked, WorkflowState::Cancelled),
    (WorkflowState::Parked, WorkflowState::Failed),
    (WorkflowState::Blocked, WorkflowState::Open),
    (WorkflowState::Blocked, WorkflowState::Active),
    (WorkflowState::Blocked, WorkflowState::Cancelled),
    (WorkflowState::Blocked, WorkflowState::Failed),
    (WorkflowState::Completing, WorkflowState::Completed),
    (WorkflowState::Completing, WorkflowState::Cancelled),
    (WorkflowState::Completing, WorkflowState::Failed),
];

/// Whether the V2 transition table admits `from -> to`.
pub fn is_valid_transition(from: WorkflowState, to: WorkflowState) -> bool {
    TRANSITIONS.contains(&(from, to))
}

/// How far a queue-to-workflow projection can be trusted to mean what
/// it says. Every mapping declares its fidelity; nothing is implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingFidelity {
    /// The workflow state has the same phase meaning as the source
    /// state; no dimension is collapsed and nothing material is lost.
    Exact,
    /// The projection holds, but the named losses on the mapping are
    /// information the workflow state does not carry.
    Lossy,
    /// The projection holds only when the evidence named on the
    /// mapping is present; without it the source state is explicitly
    /// unmapped rather than projected on a guess.
    Conditional,
}

/// Evidence a projection may consult. Evidence is supplied by the
/// caller from authoritative records; it is never inferred from the
/// queue state itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionEvidence {
    /// A completion receipt for the task was authoritatively verified.
    /// The #9206 design (§3) allows queue `done` to support workflow
    /// `completed` only with this receipt.
    pub verified_completion_receipt: bool,
}

/// The loss-aware projection of one stored queue state onto the V2
/// workflow dimension. `source` is always the exact input state, so
/// the projection preserves the source state verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateMapping {
    /// The source queue state, preserved verbatim.
    pub source: QueueState,
    /// The projected workflow state, or `None` when the source state
    /// is explicitly unmapped under the supplied evidence.
    pub workflow: Option<WorkflowState>,
    /// The declared fidelity of this projection.
    pub fidelity: MappingFidelity,
    /// Information the workflow state does not carry, named one loss
    /// per entry. Empty only for `Exact` projections.
    pub losses: &'static [&'static str],
    /// The evidence a `Conditional` projection requires, when any.
    pub requires: Option<&'static str>,
    /// Why the source state is unmapped, when `workflow` is `None`.
    pub unmapped_reason: Option<&'static str>,
}

/// Project one stored queue state onto the V2 workflow dimension,
/// loss-aware: each source state maps with its losses (or its required
/// evidence) declared, or is explicitly unmapped with a reason.
pub fn map_queue_state(source: QueueState, evidence: ProjectionEvidence) -> StateMapping {
    let exact = |workflow| StateMapping {
        source,
        workflow: Some(workflow),
        fidelity: MappingFidelity::Exact,
        losses: &[],
        requires: None,
        unmapped_reason: None,
    };
    let lossy = |workflow, losses| StateMapping {
        source,
        workflow: Some(workflow),
        fidelity: MappingFidelity::Lossy,
        losses,
        requires: None,
        unmapped_reason: None,
    };
    match source {
        // #9206 §3: queue `queued` can project workflow `open`. The
        // workflow phase does not carry queue scheduling detail.
        QueueState::Queued => lossy(
            WorkflowState::Open,
            &[
                "attempt count and retry budget are queue scheduling detail the workflow phase does not carry",
                "a pending not_before wake time is not represented in the workflow dimension",
            ],
        ),
        // A claim is principally ownership, not workflow (#9206 §3):
        // projecting to `active` collapses the ownership/execution
        // dimension into the workflow dimension.
        QueueState::Claimed => lossy(
            WorkflowState::Active,
            &[
                "a claim is ownership, not proof a session started; the projection asserts the active phase without the execution dimension",
                "lease holder identity and lease generation/expiry are not carried by the workflow state",
            ],
        ),
        QueueState::Parked => exact(WorkflowState::Parked),
        // #9206 §3: dependency-blocked and manually blocked need a
        // source/reason distinction the workflow state alone lacks.
        QueueState::Blocked => lossy(
            WorkflowState::Blocked,
            &[
                "the blocker source — hard dependency vs manual block — is not distinguished by the workflow state alone",
            ],
        ),
        // A handoff changes ownership, not phase: the work remains
        // active, but the transfer itself is an ownership-dimension
        // fact the workflow state does not record.
        QueueState::HandedOff => lossy(
            WorkflowState::Active,
            &[
                "the handoff event and the previous owner are ownership-dimension facts the workflow state does not record",
                "the receiving claim's lease generation is not carried by the workflow state",
            ],
        ),
        // #9206 §3: queue `done` may support workflow `completed`
        // only with a verified completion receipt. Without one the
        // state is explicitly unmapped — never guessed.
        QueueState::Done => {
            let requires = Some("verified completion receipt");
            if evidence.verified_completion_receipt {
                StateMapping {
                    source,
                    workflow: Some(WorkflowState::Completed),
                    fidelity: MappingFidelity::Conditional,
                    losses: &[
                        "how the remote effects were verified lives on the completion receipt, not in either state",
                    ],
                    requires,
                    unmapped_reason: None,
                }
            } else {
                StateMapping {
                    source,
                    workflow: None,
                    fidelity: MappingFidelity::Conditional,
                    losses: &[],
                    requires,
                    unmapped_reason: Some(
                        "queue `done` without a verified completion receipt must not project to workflow `completed`",
                    ),
                }
            }
        }
        QueueState::Cancelled => exact(WorkflowState::Cancelled),
        QueueState::Failed => exact(WorkflowState::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_EVIDENCE: ProjectionEvidence = ProjectionEvidence {
        verified_completion_receipt: false,
    };
    const WITH_RECEIPT: ProjectionEvidence = ProjectionEvidence {
        verified_completion_receipt: true,
    };

    /// The CLI-01 oracle: every stored queue state is accounted for by
    /// the crosswalk — it maps loss-aware (fidelity and losses
    /// declared) or is explicitly unmapped with a reason — and the
    /// mapping preserves the source state verbatim, so no stored value
    /// is silently renamed or dropped.
    #[test]
    fn lifecycle_queue_state_mapping_preserves_source_state() {
        for source in QueueState::ALL {
            for evidence in [NO_EVIDENCE, WITH_RECEIPT] {
                let mapping = map_queue_state(source, evidence);
                assert_eq!(mapping.source, source, "source state not preserved");
                assert_eq!(
                    QueueState::from_stored(mapping.source.as_str()),
                    Some(source),
                    "source spelling must round-trip to the same state",
                );
                match mapping.workflow {
                    Some(_) => {
                        assert!(
                            mapping.unmapped_reason.is_none(),
                            "a mapped state carries no unmapped reason"
                        );
                        match mapping.fidelity {
                            MappingFidelity::Exact => assert!(
                                mapping.losses.is_empty(),
                                "an exact projection declares no losses"
                            ),
                            MappingFidelity::Lossy => assert!(
                                !mapping.losses.is_empty(),
                                "a lossy projection must name its losses"
                            ),
                            MappingFidelity::Conditional => assert!(
                                mapping.requires.is_some(),
                                "a conditional projection must name its evidence"
                            ),
                        }
                    }
                    None => {
                        assert!(
                            mapping.unmapped_reason.is_some(),
                            "an unmapped state must record why"
                        );
                        assert_eq!(
                            mapping.fidelity,
                            MappingFidelity::Conditional,
                            "only a conditional projection may be unmapped"
                        );
                        assert!(
                            mapping.requires.is_some(),
                            "an unmapped conditional projection must name the missing evidence"
                        );
                    }
                }
            }
        }

        // The one conditional row, pinned explicitly: queue `done`
        // projects to workflow `completed` only with a verified
        // completion receipt, and is explicitly unmapped without one.
        let without = map_queue_state(QueueState::Done, NO_EVIDENCE);
        assert_eq!(without.workflow, None);
        let with = map_queue_state(QueueState::Done, WITH_RECEIPT);
        assert_eq!(with.workflow, Some(WorkflowState::Completed));
    }

    #[test]
    fn lifecycle_crosswalk_never_projects_completing() {
        // `completing` is entered only by the active -> completing
        // transition; no stored queue state projects to it.
        for source in QueueState::ALL {
            for evidence in [NO_EVIDENCE, WITH_RECEIPT] {
                assert_ne!(
                    map_queue_state(source, evidence).workflow,
                    Some(WorkflowState::Completing),
                    "queue state {} must not project to completing",
                    source.as_str(),
                );
            }
        }
    }

    #[test]
    fn lifecycle_stored_spellings_are_exact_and_closed() {
        // The stored vocabulary is exactly these eight spellings.
        let spellings: Vec<&str> = QueueState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            spellings,
            [
                "queued",
                "claimed",
                "parked",
                "blocked",
                "handed_off",
                "done",
                "cancelled",
                "failed"
            ]
        );
        // Proposal vocabulary that was not ratified as a stored value
        // never parses — no silent rename into the stored set.
        for foreign in [
            "in_progress",
            "available",
            "completed",
            "open",
            "active",
            "",
        ] {
            assert_eq!(QueueState::from_stored(foreign), None, "{foreign}");
        }
    }

    #[test]
    fn lifecycle_transition_table_is_closed_and_terminal_safe() {
        // Every matrix edge from the #9206 design §3 is admitted.
        for (from, to) in TRANSITIONS {
            assert!(
                is_valid_transition(*from, *to),
                "{} -> {}",
                from.as_str(),
                to.as_str()
            );
        }
        // Terminal states admit nothing, and no state self-transitions.
        for state in WorkflowState::ALL {
            assert!(
                !is_valid_transition(state, state),
                "self transition {}",
                state.as_str()
            );
            if state.is_terminal() {
                for to in WorkflowState::ALL {
                    assert!(
                        !is_valid_transition(state, to),
                        "terminal {} -> {}",
                        state.as_str(),
                        to.as_str()
                    );
                }
            }
        }
        // Skipping the lifecycle is not a transition.
        assert!(!is_valid_transition(
            WorkflowState::Open,
            WorkflowState::Completed
        ));
        assert!(!is_valid_transition(
            WorkflowState::Open,
            WorkflowState::Completing
        ));
        assert!(!is_valid_transition(
            WorkflowState::Parked,
            WorkflowState::Completing
        ));
        assert!(!is_valid_transition(
            WorkflowState::Blocked,
            WorkflowState::Completed
        ));
        assert!(!is_valid_transition(
            WorkflowState::Active,
            WorkflowState::Completed
        ));
    }
}
