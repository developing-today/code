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

use super::contracts::WorkRef;

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

/// Provenance attached to one observed projection dimension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionProvenance {
    /// The authoritative source that supplied this dimension.
    pub source: String,
    /// When the source observation was made, when available.
    pub observed_at: Option<String>,
    /// Source version or revision, when available.
    pub version: Option<String>,
}

/// An observed value and its provenance, or an explicit unknown with
/// provenance describing what was queried and why no value is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionObservation<T> {
    Known {
        value: T,
        provenance: ProjectionProvenance,
    },
    Unknown {
        provenance: ProjectionProvenance,
    },
}

/// Readiness is independent of workflow lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessProjection {
    pub eligible: bool,
    pub reasons: Vec<String>,
}

/// Claim and execution evidence remain separate from workflow state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipExecutionProjection {
    pub owner: Option<String>,
    pub session: Option<String>,
    pub lease_generation: Option<u64>,
    pub lease_expires_at: Option<String>,
    pub execution: ExecutionState,
}

/// Execution evidence does not follow from a live lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    NotStarted,
    Requested,
    Delivered,
    Acknowledged,
    Started,
    Checkpointed,
    Terminal,
}

/// A provider-native linked resource observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteResourceProjection {
    pub kind: String,
    pub identity: String,
    pub state: String,
}

/// Read-only aggregate projection. Its four independent dimensions
/// carry source provenance and can remain unknown without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInspection {
    pub identity: WorkRef,
    pub workflow: ProjectionObservation<StateMapping>,
    pub readiness: ProjectionObservation<ReadinessProjection>,
    pub ownership_execution: ProjectionObservation<OwnershipExecutionProjection>,
    pub remote_resources: ProjectionObservation<Vec<RemoteResourceProjection>>,
}

/// A lookup failure never substitutes a nearby result for the requested
/// work identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkInspectionLookupError {
    NotFoundById {
        work_id: String,
    },
    NotFoundByRef {
        requested: WorkRef,
    },
    Ambiguous {
        work_id: String,
        authorities: Vec<String>,
    },
}

/// Resolve an unqualified work ID only when it names exactly one
/// authority in the supplied read set. Cross-authority collisions are
/// rejected instead of selecting the first result.
pub fn resolve_work_inspection_by_id<'a>(
    work_id: &str,
    inspections: &'a [WorkInspection],
) -> Result<&'a WorkInspection, WorkInspectionLookupError> {
    let matches: Vec<&WorkInspection> = inspections
        .iter()
        .filter(|inspection| inspection.identity.work_id == work_id)
        .collect();
    match matches.as_slice() {
        [] => Err(WorkInspectionLookupError::NotFoundById {
            work_id: work_id.to_string(),
        }),
        [inspection] => Ok(*inspection),
        _ => {
            let mut authorities: Vec<String> = matches
                .iter()
                .map(|inspection| inspection.identity.authority_id.clone())
                .collect();
            authorities.sort();
            authorities.dedup();
            Err(WorkInspectionLookupError::Ambiguous {
                work_id: work_id.to_string(),
                authorities,
            })
        }
    }
}

/// Resolve a fully qualified identity by exact pair equality. A result
/// with the same numeric ID in another authority is never returned.
pub fn resolve_work_inspection_by_ref<'a>(
    requested: &WorkRef,
    inspections: &'a [WorkInspection],
) -> Result<&'a WorkInspection, WorkInspectionLookupError> {
    let mut matches = inspections
        .iter()
        .filter(|inspection| inspection.identity == *requested);
    let Some(inspection) = matches.next() else {
        return Err(WorkInspectionLookupError::NotFoundByRef {
            requested: requested.clone(),
        });
    };
    if matches.next().is_some() {
        return Err(WorkInspectionLookupError::Ambiguous {
            work_id: requested.work_id.clone(),
            authorities: vec![requested.authority_id.clone()],
        });
    }
    Ok(inspection)
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

// ---------------------------------------------------------------------
// Claim eligibility: the REL-01 compatibility model (#9512, against the
// #9178 claim/manual/take-over/resume semantics, FDR-0001..FDR-0004).
//
// This is a pure reference model of WHICH claim door may take WHICH
// stored row. It names two contracts side by side: `Current`, which
// mirrors the queue's claim code on main (`turso_queue::claim`:
// `default_pick_predicate`, `by_id_claimable`, `transfer_refusal`,
// `task_requirements_unmet`), and `Fdr9178`, the accepted #9178
// target. Where they differ is the compatibility table in
// doc/2026-11-25T01-23-08Z_design_rel01_claim_lease_compat_9512/note.md.
// The model performs no I/O and adds no field, column or verb.
// ---------------------------------------------------------------------

/// Which claim contract to evaluate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimContract {
    /// The queue's claim behaviour on main today.
    Current,
    /// The accepted #9178 contract (FDR-0001..FDR-0004).
    Fdr9178,
}

/// The door a claim comes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimPath {
    /// A lane's automatic pick: priority order over the one queue.
    Automatic,
    /// An explicit claim of one task by id (FDR-0002).
    /// `owner_override` is the recorded manual-claim override, which
    /// exists only under the #9178 contract.
    ById { owner_override: bool },
    /// Moving a live claim to the claimer. Today this is the #8210
    /// transfer, which the holder gives away by naming its own
    /// session; under #9178 it is `--take-over` (FDR-0003), which the
    /// claimer may invoke without the holder's consent.
    TakeOver { holder_consents: bool },
}

/// The claim currently held on a row, as its lease fields show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimHold {
    /// No claim (any status other than `claimed`).
    None,
    /// A claim whose lease has not lapsed.
    Live { by_requester: bool },
    /// A `claimed` row whose lease lapsed but which the sweep
    /// (`expire_leases`) has not yet returned to queued or the sink.
    Expired,
}

/// A task's own declared requirements (payload `requires`). An absent
/// requirement is met by every claimer; that is the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskRequirements<'a> {
    pub platform: Option<&'a str>,
    pub lane: Option<&'a str>,
    pub pool: Option<&'a str>,
    pub model: Option<&'a str>,
}

/// The claimer's identity, used only to satisfy a task's requirements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaimerAttrs<'a> {
    pub platform: Option<&'a str>,
    pub lane: Option<&'a str>,
    pub pool: Option<&'a str>,
    pub model: Option<&'a str>,
}

impl TaskRequirements<'_> {
    /// The first requirement the claimer does not meet. A claimer
    /// that does not declare a required attribute does not meet it.
    pub fn unmet(&self, claimer: &ClaimerAttrs<'_>) -> Option<&'static str> {
        let pairs = [
            ("platform", self.platform, claimer.platform),
            ("lane", self.lane, claimer.lane),
            ("pool", self.pool, claimer.pool),
            ("model", self.model, claimer.model),
        ];
        pairs
            .into_iter()
            .find(|(_, wanted, have)| wanted.is_some() && wanted != have)
            .map(|(name, _, _)| name)
    }
}

/// The task facts a claim filter reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskMeta<'a> {
    pub repo: &'a str,
    pub queue: &'a str,
    pub labels: &'a [&'a str],
}

/// The claimer's filter, reduced to the conjunction of equality terms
/// the reference model needs (the full grammar stays in
/// `turso_queue::claim`). An empty filter matches every task; that is
/// the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaimFilter<'a> {
    pub repo: Option<&'a str>,
    pub queue: Option<&'a str>,
    pub label: Option<&'a str>,
}

impl ClaimFilter<'_> {
    pub fn is_empty(&self) -> bool {
        self.repo.is_none() && self.queue.is_none() && self.label.is_none()
    }

    pub fn matches(&self, meta: &TaskMeta<'_>) -> bool {
        self.repo.map_or(true, |r| r == meta.repo)
            && self.queue.map_or(true, |q| q == meta.queue)
            && self.label.map_or(true, |l| meta.labels.contains(&l))
    }
}

/// One stored row as the claim doors see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimCandidate<'a> {
    pub state: QueueState,
    pub hold: ClaimHold,
    pub attempts: i64,
    pub max_attempts: i64,
    /// The row's wait has ended: a queued row's `not_before` has
    /// passed (or is unset), or a parked row's release is due under
    /// the pick predicate (timer elapsed on a timer-released park, or
    /// the capacity bypass). Condition-only parks are never due here.
    pub wait_due: bool,
    /// A `handed_off` row that is an unbound opencode handoff in the
    /// sink (`HANDOFF_SQL`, #8234).
    pub handoff_sink: bool,
    /// FDR-0004: automatic claim must never select this row.
    pub manual_only: bool,
    /// A Do Now directive on the work. A directive is about urgency,
    /// never eligibility; the model reads it only to prove that.
    pub do_now: bool,
    pub requires: TaskRequirements<'a>,
    pub meta: TaskMeta<'a>,
}

/// How an eligible claim moves a live claim, if it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimTransfer {
    /// The holder gave the task away by naming its own session (#8210).
    HolderConsent,
    /// FDR-0003 `--take-over`: the prior worker/session and the
    /// hand-back signal (or its unavailability) must be recorded.
    TakeOver,
}

/// What an eligible claim must record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimGrant {
    /// FDR-0002: the claim reached the row only through the recorded
    /// owner override (parked, attempt-capped, or not yet due).
    pub override_recorded: bool,
    pub transfer: Option<ClaimTransfer>,
}

/// Why a door refuses a row. Every refusal mutates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRefusal {
    /// Done, cancelled or failed.
    Terminal,
    /// Blocked rows are dependency-gated on every door.
    Blocked,
    /// FDR-0004: the automatic pick never selects a manual-only row.
    ManualOnly,
    /// The task's own requirements bind every door.
    RequirementsUnmet(&'static str),
    /// The claimer's filter excludes the task (automatic pick only).
    FilterExcludes,
    /// A live claim stands; skipped by default (FDR-0003).
    LiveClaimHeld,
    /// The lease lapsed but the sweep has not reclaimed the row yet:
    /// no door takes it until `expire_leases` returns it.
    ExpiredAwaitingReclaim,
    /// The wait has not ended (future `not_before`, or a park whose
    /// release is not due).
    NotYetDue,
    /// Queued at `max_attempts`.
    AttemptCap,
    /// A parked row: only the FDR-0002 override reaches it by id.
    ParkedNeedsOverride,
    /// A `handed_off` row that is not an unbound sink row.
    HandedOffBound,
    /// A take-over needs a live claim to move.
    NoLiveClaimToMove,
    /// Today a transfer must be given by the holder (#8210).
    HolderConsentRequired,
}

/// The result of asking one door about one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimEligibility {
    Eligible(ClaimGrant),
    /// FDR-0002: the requester already holds the live claim; repeating
    /// the claim is idempotent and returns the existing claim.
    AlreadyOwned,
    Refused(ClaimRefusal),
}

/// One claim request: the contract, the door, who asks, and the
/// claimer's filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRequest<'a> {
    pub contract: ClaimContract,
    pub path: ClaimPath,
    pub claimer: ClaimerAttrs<'a>,
    pub filter: ClaimFilter<'a>,
}

fn grant(override_recorded: bool, transfer: Option<ClaimTransfer>) -> ClaimEligibility {
    ClaimEligibility::Eligible(ClaimGrant {
        override_recorded,
        transfer,
    })
}

/// Decide whether `req` may claim `cand`. Pure: the same inputs
/// always give the same answer, and `do_now` never changes it.
pub fn claim_eligibility(req: &ClaimRequest<'_>, cand: &ClaimCandidate<'_>) -> ClaimEligibility {
    use ClaimEligibility::Refused;
    use ClaimRefusal as R;
    let fdr = req.contract == ClaimContract::Fdr9178;

    match cand.state {
        QueueState::Done | QueueState::Cancelled | QueueState::Failed => {
            return Refused(R::Terminal);
        }
        QueueState::Blocked => return Refused(R::Blocked),
        _ => {}
    }
    // The task's own requirements bind every door (#8097): the pick,
    // the by-id claim and the transfer alike.
    if let Some(name) = cand.requires.unmet(&req.claimer) {
        return Refused(R::RequirementsUnmet(name));
    }
    if cand.hold == ClaimHold::Expired {
        return Refused(R::ExpiredAwaitingReclaim);
    }

    match req.path {
        ClaimPath::TakeOver { holder_consents } => match cand.hold {
            ClaimHold::Live { by_requester: true } => ClaimEligibility::AlreadyOwned,
            ClaimHold::Live {
                by_requester: false,
            } => {
                if fdr {
                    grant(false, Some(ClaimTransfer::TakeOver))
                } else if holder_consents {
                    grant(false, Some(ClaimTransfer::HolderConsent))
                } else {
                    Refused(R::HolderConsentRequired)
                }
            }
            _ => Refused(R::NoLiveClaimToMove),
        },
        ClaimPath::Automatic => {
            // FDR-0004: unconditional — priority, age, lane, queue
            // depth and Do Now do not change it. The flag is a
            // persisted claim exclusion, so every automatic picker
            // observes it regardless of the request contract version.
            if cand.manual_only {
                return Refused(R::ManualOnly);
            }
            // The claimer's filter is a pick-time predicate only.
            if !req.filter.matches(&cand.meta) {
                return Refused(R::FilterExcludes);
            }
            if let ClaimHold::Live { .. } = cand.hold {
                return Refused(R::LiveClaimHeld);
            }
            match cand.state {
                QueueState::Queued if cand.attempts >= cand.max_attempts => Refused(R::AttemptCap),
                QueueState::Queued | QueueState::Parked if !cand.wait_due => Refused(R::NotYetDue),
                QueueState::Queued | QueueState::Parked => grant(false, None),
                QueueState::HandedOff if cand.handoff_sink => grant(false, None),
                QueueState::HandedOff => Refused(R::HandedOffBound),
                _ => Refused(R::LiveClaimHeld),
            }
        }
        ClaimPath::ById { owner_override } => {
            // The override exists only under #9178 (FDR-0002).
            let can_override = fdr && owner_override;
            match cand.hold {
                ClaimHold::Live { by_requester: true } if fdr => {
                    return ClaimEligibility::AlreadyOwned;
                }
                ClaimHold::Live { .. } => return Refused(R::LiveClaimHeld),
                _ => {}
            }
            match cand.state {
                QueueState::Queued => {
                    let capped = cand.attempts >= cand.max_attempts;
                    if (capped || !cand.wait_due) && !can_override {
                        return Refused(if capped { R::AttemptCap } else { R::NotYetDue });
                    }
                    grant(capped || !cand.wait_due, None)
                }
                QueueState::Parked if can_override => grant(true, None),
                QueueState::Parked => Refused(R::ParkedNeedsOverride),
                QueueState::HandedOff if cand.handoff_sink => grant(false, None),
                QueueState::HandedOff => Refused(R::HandedOffBound),
                _ => Refused(R::LiveClaimHeld),
            }
        }
    }
}

/// FDR-0003's per-task batch result vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOutcome {
    Claimed(ClaimGrant),
    AlreadyOwnedByThisSession,
    SkippedClaimed,
    NotFound,
    Failed(ClaimRefusal),
}

impl BatchOutcome {
    /// The FDR-0003 spelling of this outcome.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claimed(_) => "claimed",
            Self::AlreadyOwnedByThisSession => "already-owned-by-this-session",
            Self::SkippedClaimed => "skipped-claimed",
            Self::NotFound => "not-found",
            Self::Failed(_) => "failed",
        }
    }
}

/// Why a whole batch is refused before any task is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchRefusal {
    /// A batch names at least one task.
    Empty,
    /// The batch bound is the existing `BATCH_CLAIM_MAX`; #9178 does
    /// not settle a by-id batch cap, so the model reuses the existing
    /// bound and does not invent one.
    OverCap,
}

/// Evaluate an FDR-0003 batch claim: one door, many named tasks, one
/// independent outcome per task (`None` is a task id with no row).
/// A task's outcome never depends on the other tasks in the batch.
pub fn batch_claim_outcomes(
    req: &ClaimRequest<'_>,
    tasks: &[Option<ClaimCandidate<'_>>],
) -> Result<Vec<BatchOutcome>, BatchRefusal> {
    if tasks.is_empty() {
        return Err(BatchRefusal::Empty);
    }
    if tasks.len() as i64 > crate::BATCH_CLAIM_MAX {
        return Err(BatchRefusal::OverCap);
    }
    Ok(tasks
        .iter()
        .map(|task| match task {
            None => BatchOutcome::NotFound,
            Some(cand) => match claim_eligibility(req, cand) {
                ClaimEligibility::Eligible(g) => BatchOutcome::Claimed(g),
                ClaimEligibility::AlreadyOwned => BatchOutcome::AlreadyOwnedByThisSession,
                ClaimEligibility::Refused(ClaimRefusal::LiveClaimHeld) => {
                    BatchOutcome::SkippedClaimed
                }
                ClaimEligibility::Refused(r) => BatchOutcome::Failed(r),
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown_observation<T>() -> ProjectionObservation<T> {
        ProjectionObservation::Unknown {
            provenance: ProjectionProvenance {
                source: "test-fixture".to_string(),
                observed_at: Some("2026-11-23T00:00:00Z".to_string()),
                version: Some("fixture-v1".to_string()),
            },
        }
    }

    fn assert_fixture_provenance<T>(observation: &ProjectionObservation<T>) {
        assert!(matches!(
            observation,
            ProjectionObservation::Unknown { provenance }
                if provenance.source == "test-fixture"
        ));
    }

    fn inspection(authority_id: &str, work_id: &str) -> WorkInspection {
        WorkInspection {
            identity: WorkRef::new(authority_id, work_id).unwrap(),
            workflow: unknown_observation(),
            readiness: unknown_observation(),
            ownership_execution: unknown_observation(),
            remote_resources: unknown_observation(),
        }
    }

    /// Oracle: a bare work ID cannot select arbitrarily when the read
    /// set contains that ID under multiple authorities. Each of the
    /// four independent dimensions also has an explicit provenance
    /// slot, including when its value is unknown.
    #[test]
    fn work_inspect_rejects_ambiguous_cross_repo_reference() {
        let inspections = [
            inspection("developing-today/code", "842"),
            inspection("developing-today/fleet", "842"),
        ];

        assert_eq!(
            resolve_work_inspection_by_id("842", &inspections),
            Err(WorkInspectionLookupError::Ambiguous {
                work_id: "842".to_string(),
                authorities: vec![
                    "developing-today/code".to_string(),
                    "developing-today/fleet".to_string(),
                ],
            })
        );
        assert_fixture_provenance(&inspections[0].workflow);
        assert_fixture_provenance(&inspections[0].readiness);
        assert_fixture_provenance(&inspections[0].ownership_execution);
        assert_fixture_provenance(&inspections[0].remote_resources);
    }

    /// Oracle: an explicit identity must match both authority and work
    /// ID; a same-number result from another repository is not a hit.
    #[test]
    fn work_inspect_by_id_rejects_nonmatching_result() {
        assert_eq!(
            resolve_work_inspection_by_id("842", &[inspection("developing-today/code", "843")]),
            Err(WorkInspectionLookupError::NotFoundById {
                work_id: "842".to_string(),
            })
        );

        let requested = WorkRef::new("developing-today/code", "842").unwrap();
        let result_from_another_repo = [inspection("developing-today/fleet", "842")];

        assert_eq!(
            resolve_work_inspection_by_ref(&requested, &result_from_another_repo),
            Err(WorkInspectionLookupError::NotFoundByRef { requested })
        );
    }

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

    // -----------------------------------------------------------------
    // REL-01 (#9512): claim eligibility against #9178.
    // -----------------------------------------------------------------

    const NO_LABELS: &[&str] = &[];
    const DESIGN: &[&str] = &["kind/design"];

    fn queued() -> ClaimCandidate<'static> {
        ClaimCandidate {
            state: QueueState::Queued,
            hold: ClaimHold::None,
            attempts: 0,
            max_attempts: 3,
            wait_due: true,
            handoff_sink: false,
            manual_only: false,
            do_now: false,
            requires: TaskRequirements::default(),
            meta: TaskMeta {
                repo: "developing-today/code",
                queue: "code",
                labels: DESIGN,
            },
        }
    }

    fn request(contract: ClaimContract, path: ClaimPath) -> ClaimRequest<'static> {
        ClaimRequest {
            contract,
            path,
            claimer: ClaimerAttrs::default(),
            filter: ClaimFilter::default(),
        }
    }

    const CONTRACTS: [ClaimContract; 2] = [ClaimContract::Current, ClaimContract::Fdr9178];

    fn all_paths() -> Vec<ClaimPath> {
        vec![
            ClaimPath::Automatic,
            ClaimPath::ById {
                owner_override: false,
            },
            ClaimPath::ById {
                owner_override: true,
            },
            ClaimPath::TakeOver {
                holder_consents: false,
            },
            ClaimPath::TakeOver {
                holder_consents: true,
            },
        ]
    }

    fn eligible(override_recorded: bool) -> ClaimEligibility {
        ClaimEligibility::Eligible(ClaimGrant {
            override_recorded,
            transfer: None,
        })
    }

    #[test]
    fn pmo_rel_claim_requires_requirements_and_filter() {
        let lane7 = ClaimerAttrs {
            lane: Some("lane-7"),
            platform: Some("openchamber"),
            ..ClaimerAttrs::default()
        };
        let needs_lane7 = TaskRequirements {
            lane: Some("lane-7"),
            ..TaskRequirements::default()
        };
        let needs_muse = TaskRequirements {
            platform: Some("muse"),
            ..TaskRequirements::default()
        };
        let code_only = ClaimFilter {
            queue: Some("code"),
            ..ClaimFilter::default()
        };
        let fleet_only = ClaimFilter {
            queue: Some("fleet"),
            ..ClaimFilter::default()
        };
        for contract in CONTRACTS {
            let pick = |claimer, filter, requires| {
                let req = ClaimRequest {
                    contract,
                    path: ClaimPath::Automatic,
                    claimer,
                    filter,
                };
                claim_eligibility(
                    &req,
                    &ClaimCandidate {
                        requires,
                        ..queued()
                    },
                )
            };
            // Defaults: no requirements and an empty filter are both
            // true, so the pick is a plain priority pick.
            assert!(ClaimFilter::default().is_empty());
            assert_eq!(
                pick(
                    ClaimerAttrs::default(),
                    ClaimFilter::default(),
                    TaskRequirements::default()
                ),
                eligible(false)
            );
            // Both sides hold: eligible.
            assert_eq!(pick(lane7, code_only, needs_lane7), eligible(false));
            // Requirements unmet, filter matches: refused.
            assert_eq!(
                pick(lane7, code_only, needs_muse),
                ClaimEligibility::Refused(ClaimRefusal::RequirementsUnmet("platform"))
            );
            // A claimer that does not declare the attribute does not
            // meet the requirement.
            assert_eq!(
                pick(ClaimerAttrs::default(), code_only, needs_lane7),
                ClaimEligibility::Refused(ClaimRefusal::RequirementsUnmet("lane"))
            );
            // Requirements met, filter excludes: refused.
            assert_eq!(
                pick(lane7, fleet_only, needs_lane7),
                ClaimEligibility::Refused(ClaimRefusal::FilterExcludes)
            );
            // Requirements bind every door; the filter is pick-only.
            for path in [
                ClaimPath::ById {
                    owner_override: false,
                },
                ClaimPath::ById {
                    owner_override: true,
                },
            ] {
                let by_id = |filter, requires| {
                    let req = ClaimRequest {
                        contract,
                        path,
                        claimer: lane7,
                        filter,
                    };
                    claim_eligibility(
                        &req,
                        &ClaimCandidate {
                            requires,
                            ..queued()
                        },
                    )
                };
                assert_eq!(by_id(fleet_only, needs_lane7), eligible(false));
                assert_eq!(
                    by_id(code_only, needs_muse),
                    ClaimEligibility::Refused(ClaimRefusal::RequirementsUnmet("platform"))
                );
            }
            let held = ClaimCandidate {
                state: QueueState::Claimed,
                hold: ClaimHold::Live {
                    by_requester: false,
                },
                requires: needs_muse,
                ..queued()
            };
            let take = ClaimRequest {
                contract,
                path: ClaimPath::TakeOver {
                    holder_consents: true,
                },
                claimer: lane7,
                filter: ClaimFilter::default(),
            };
            assert_eq!(
                claim_eligibility(&take, &held),
                ClaimEligibility::Refused(ClaimRefusal::RequirementsUnmet("platform"))
            );
        }
    }

    #[test]
    fn pmo_rel_manual_only_is_eligibility_not_directive() {
        let manual = ClaimCandidate {
            manual_only: true,
            ..queued()
        };
        let auto = request(ClaimContract::Fdr9178, ClaimPath::Automatic);
        // FDR-0004: the automatic pick never selects it, Do Now or not.
        for do_now in [false, true] {
            assert_eq!(
                claim_eligibility(&auto, &ClaimCandidate { do_now, ..manual }),
                ClaimEligibility::Refused(ClaimRefusal::ManualOnly)
            );
        }
        // A Do Now item that is not manual-only is an ordinary pick:
        // the directive is urgency, never eligibility.
        assert_eq!(
            claim_eligibility(
                &auto,
                &ClaimCandidate {
                    do_now: true,
                    ..queued()
                }
            ),
            eligible(false)
        );
        // The explicit by-id door is the one that reaches it.
        for owner_override in [false, true] {
            let by_id = request(ClaimContract::Fdr9178, ClaimPath::ById { owner_override });
            assert_eq!(claim_eligibility(&by_id, &manual), eligible(false));
        }
        // The persisted exclusion applies to legacy as well as current
        // request contracts; no automatic picker may select the row.
        assert_eq!(
            claim_eligibility(
                &request(ClaimContract::Current, ClaimPath::Automatic),
                &manual
            ),
            ClaimEligibility::Refused(ClaimRefusal::ManualOnly)
        );
        // Do Now never changes any door's answer on any row shape.
        let shapes = [
            queued(),
            manual,
            ClaimCandidate {
                state: QueueState::Parked,
                wait_due: false,
                ..queued()
            },
            ClaimCandidate {
                attempts: 3,
                ..queued()
            },
            ClaimCandidate {
                state: QueueState::Claimed,
                hold: ClaimHold::Live {
                    by_requester: false,
                },
                ..manual
            },
        ];
        for contract in CONTRACTS {
            for path in all_paths() {
                let req = request(contract, path);
                for shape in shapes {
                    assert_eq!(
                        claim_eligibility(
                            &req,
                            &ClaimCandidate {
                                do_now: true,
                                ..shape
                            }
                        ),
                        claim_eligibility(
                            &req,
                            &ClaimCandidate {
                                do_now: false,
                                ..shape
                            }
                        ),
                        "{contract:?} {path:?} {shape:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn pmo_rel_claim_expired_but_unreclaimed_is_taken_by_no_door() {
        let expired = ClaimCandidate {
            state: QueueState::Claimed,
            hold: ClaimHold::Expired,
            ..queued()
        };
        for contract in CONTRACTS {
            for path in all_paths() {
                assert_eq!(
                    claim_eligibility(&request(contract, path), &expired),
                    ClaimEligibility::Refused(ClaimRefusal::ExpiredAwaitingReclaim),
                    "{contract:?} {path:?}"
                );
            }
        }
    }

    #[test]
    fn pmo_rel_claim_parked_capped_and_blocked_rows() {
        let parked = ClaimCandidate {
            state: QueueState::Parked,
            wait_due: false,
            ..queued()
        };
        let capped = ClaimCandidate {
            attempts: 3,
            ..queued()
        };
        let not_due = ClaimCandidate {
            wait_due: false,
            ..queued()
        };
        let blocked = ClaimCandidate {
            state: QueueState::Blocked,
            ..queued()
        };
        let plain = ClaimPath::ById {
            owner_override: false,
        };
        let overriding = ClaimPath::ById {
            owner_override: true,
        };
        let refused = ClaimEligibility::Refused;
        for contract in CONTRACTS {
            let auto = request(contract, ClaimPath::Automatic);
            assert_eq!(
                claim_eligibility(&auto, &parked),
                refused(ClaimRefusal::NotYetDue)
            );
            assert_eq!(
                claim_eligibility(
                    &auto,
                    &ClaimCandidate {
                        wait_due: true,
                        ..parked
                    }
                ),
                eligible(false),
                "a due timer park is released through the pick (FDR-0008)"
            );
            assert_eq!(
                claim_eligibility(&auto, &capped),
                refused(ClaimRefusal::AttemptCap)
            );
            assert_eq!(
                claim_eligibility(&request(contract, plain), &parked),
                refused(ClaimRefusal::ParkedNeedsOverride)
            );
            assert_eq!(
                claim_eligibility(&request(contract, plain), &capped),
                refused(ClaimRefusal::AttemptCap)
            );
            for path in all_paths() {
                assert_eq!(
                    claim_eligibility(&request(contract, path), &blocked),
                    refused(ClaimRefusal::Blocked)
                );
            }
        }
        // FDR-0002: only the recorded override reaches parked, capped
        // and not-yet-due rows by id; today the override does not exist.
        let fdr = request(ClaimContract::Fdr9178, overriding);
        let current = request(ClaimContract::Current, overriding);
        for row in [parked, capped, not_due] {
            assert_eq!(claim_eligibility(&fdr, &row), eligible(true));
            assert!(matches!(
                claim_eligibility(&current, &row),
                ClaimEligibility::Refused(_)
            ));
        }
        // An override on a row that needed none records none.
        assert_eq!(claim_eligibility(&fdr, &queued()), eligible(false));
    }

    #[test]
    fn pmo_rel_claim_live_claims_skip_take_over_and_idempotence() {
        let theirs = ClaimCandidate {
            state: QueueState::Claimed,
            hold: ClaimHold::Live {
                by_requester: false,
            },
            ..queued()
        };
        let mine = ClaimCandidate {
            hold: ClaimHold::Live { by_requester: true },
            ..theirs
        };
        let by_id = ClaimPath::ById {
            owner_override: true,
        };
        let take = |holder_consents| ClaimPath::TakeOver { holder_consents };
        for contract in CONTRACTS {
            // Skip by default on every non-transfer door.
            for path in [ClaimPath::Automatic, by_id] {
                assert_eq!(
                    claim_eligibility(&request(contract, path), &theirs),
                    ClaimEligibility::Refused(ClaimRefusal::LiveClaimHeld)
                );
            }
            // A take-over needs a live claim to move.
            assert_eq!(
                claim_eligibility(&request(contract, take(true)), &queued()),
                ClaimEligibility::Refused(ClaimRefusal::NoLiveClaimToMove)
            );
        }
        // Today: only the holder can give the task away (#8210).
        let current = |path| claim_eligibility(&request(ClaimContract::Current, path), &theirs);
        assert_eq!(
            current(take(false)),
            ClaimEligibility::Refused(ClaimRefusal::HolderConsentRequired)
        );
        assert_eq!(
            current(take(true)),
            ClaimEligibility::Eligible(ClaimGrant {
                override_recorded: false,
                transfer: Some(ClaimTransfer::HolderConsent),
            })
        );
        // #9178: --take-over moves it without consent, recorded as such.
        for consents in [false, true] {
            assert_eq!(
                claim_eligibility(&request(ClaimContract::Fdr9178, take(consents)), &theirs),
                ClaimEligibility::Eligible(ClaimGrant {
                    override_recorded: false,
                    transfer: Some(ClaimTransfer::TakeOver),
                })
            );
        }
        // FDR-0002: repeating one's own claim is idempotent; today the
        // by-id door refuses any claimed row.
        assert_eq!(
            claim_eligibility(&request(ClaimContract::Fdr9178, by_id), &mine),
            ClaimEligibility::AlreadyOwned
        );
        assert_eq!(
            claim_eligibility(&request(ClaimContract::Current, by_id), &mine),
            ClaimEligibility::Refused(ClaimRefusal::LiveClaimHeld)
        );
    }

    #[test]
    fn pmo_rel_claim_batch_outcomes_are_per_task_and_keep_the_existing_cap() {
        let req = request(
            ClaimContract::Fdr9178,
            ClaimPath::ById {
                owner_override: false,
            },
        );
        let theirs = ClaimCandidate {
            state: QueueState::Claimed,
            hold: ClaimHold::Live {
                by_requester: false,
            },
            ..queued()
        };
        let mine = ClaimCandidate {
            hold: ClaimHold::Live { by_requester: true },
            ..theirs
        };
        let parked = ClaimCandidate {
            state: QueueState::Parked,
            ..queued()
        };
        let batch = [Some(queued()), Some(mine), Some(theirs), None, Some(parked)];
        let outcomes = batch_claim_outcomes(&req, &batch).expect("within the cap");
        let spellings: Vec<&str> = outcomes.iter().map(BatchOutcome::as_str).collect();
        assert_eq!(
            spellings,
            [
                "claimed",
                "already-owned-by-this-session",
                "skipped-claimed",
                "not-found",
                "failed"
            ]
        );
        assert_eq!(
            outcomes[4],
            BatchOutcome::Failed(ClaimRefusal::ParkedNeedsOverride)
        );
        // Each outcome is the single-task answer: order and company
        // never change it.
        for (i, task) in batch.iter().enumerate() {
            let alone = batch_claim_outcomes(&req, std::slice::from_ref(task)).unwrap();
            assert_eq!(alone[0], outcomes[i]);
        }
        // The batch bound is the existing BATCH_CLAIM_MAX, left as is.
        let cap = crate::BATCH_CLAIM_MAX as usize;
        assert!(batch_claim_outcomes(&req, &vec![Some(queued()); cap]).is_ok());
        assert_eq!(
            batch_claim_outcomes(&req, &vec![Some(queued()); cap + 1]),
            Err(BatchRefusal::OverCap)
        );
        assert_eq!(batch_claim_outcomes(&req, &[]), Err(BatchRefusal::Empty));
    }

    #[test]
    fn pmo_rel_claim_empty_filter_and_handoff_sink() {
        let auto = request(ClaimContract::Fdr9178, ClaimPath::Automatic);
        // An empty filter matches a task with no labels and any repo.
        let bare = ClaimCandidate {
            meta: TaskMeta {
                repo: "",
                queue: "",
                labels: NO_LABELS,
            },
            ..queued()
        };
        assert_eq!(claim_eligibility(&auto, &bare), eligible(false));
        // A label term against a task without that label excludes it.
        let labelled = ClaimRequest {
            filter: ClaimFilter {
                label: Some("kind/design"),
                ..ClaimFilter::default()
            },
            ..auto
        };
        assert_eq!(
            claim_eligibility(&labelled, &bare),
            ClaimEligibility::Refused(ClaimRefusal::FilterExcludes)
        );
        assert_eq!(claim_eligibility(&labelled, &queued()), eligible(false));
        // Handoff sink rows are collected at +0 by pick and by id;
        // a bound handed_off row is not claimable.
        let sink = ClaimCandidate {
            state: QueueState::HandedOff,
            handoff_sink: true,
            ..queued()
        };
        let bound = ClaimCandidate {
            handoff_sink: false,
            ..sink
        };
        for contract in CONTRACTS {
            for path in [
                ClaimPath::Automatic,
                ClaimPath::ById {
                    owner_override: false,
                },
            ] {
                let req = request(contract, path);
                assert_eq!(claim_eligibility(&req, &sink), eligible(false));
                assert_eq!(
                    claim_eligibility(&req, &bound),
                    ClaimEligibility::Refused(ClaimRefusal::HandedOffBound)
                );
            }
        }
    }
}
