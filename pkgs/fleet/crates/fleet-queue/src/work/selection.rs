//! Lane fairness policy interface — SEL-06
//! (developing-today/code#9496; design parent #9204, program #9196).
//!
//! Fairness is separate from selection (#9204, "Fairness is
//! separate"): the urgency comparator decides what goes first; a
//! fairness policy only allocates finite service opportunities among
//! lanes. This module is that policy's interface plus model
//! policies. It is a pure model — no I/O, no state of its own, and
//! no change to live selection: the broker keeps choosing by its
//! existing path until a later slice wires a policy in.
//!
//! The contract every policy under this interface must keep:
//!
//! * **Urgent order is exact.** Candidates flagged urgent keep
//!   their input relative order, and no non-urgent candidate is
//!   ever placed ahead of an urgent one. Fairness never reorders
//!   ahead of urgent work and never delays it.
//! * **No eligibility bypass.** Input candidates are the already
//!   filtered eligible set: #8097's filter contract (a task's
//!   declared requirements AND the claim-time filter both apply;
//!   claimer identity is recorded, not a filter) runs before this
//!   interface. A policy sees only eligible work and can neither
//!   admit nor exclude a candidate — it orders service
//!   opportunities only.
//! * **No intrinsic-score change.** A policy never mutates a
//!   candidate's effective urgency or any other intrinsic field;
//!   it returns an ordering of the input, nothing else.
//! * **Within a lane, order is preserved.** Fairness acts between
//!   lanes; the comparator's order inside one lane is untouched.
//!
//! SEL-01 (below the fairness interface) is the pure reference
//! selector itself: a candidate is eligible only when BOTH the
//! task-requirement predicate AND the caller-filter predicate
//! apply (#8097's filter contract), after every hard gate has
//! passed. Exclusions carry typed reason codes; eligibility is a
//! gate, never a score. The selector is a pure model — no
//! persistence, no I/O, no claim — and live broker selection is
//! unchanged until a later slice wires it in.

use std::collections::{BTreeMap, BTreeSet};

use super::contracts::WorkRef;
use super::priority::{compare_candidates, CandidateKey, PriorityError, PriorityProfile};

/// One eligible candidate, in the order the normal selection
/// comparator produced (effective urgency ascending, then exact
/// peer rank, creation time, canonical identity — #9204). The
/// policy does not re-sort by these fields; they are carried for
/// provenance and explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FairCandidate {
    /// Canonical work identity (`work_id` in the selection input).
    pub work_id: String,
    /// The lane this candidate is eligible for and would be
    /// served by.
    pub lane_id: String,
    /// Effective urgency from the comparator's inputs; lower is
    /// more urgent. Read-only to a fairness policy.
    pub effective_urgency: i64,
    /// True when the candidate belongs to a qualifying urgent
    /// class under the active policy profile (the urgent-
    /// protection gate's set). Urgent candidates are exempt from
    /// fairness reordering.
    pub urgent: bool,
}

/// Per-lane service history: the fairness measure's only input
/// besides the candidate set. Counts are completed service
/// opportunities per lane, as recorded by the caller; the state is
/// a snapshot, and a policy never writes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FairnessState {
    served: BTreeMap<String, u64>,
}

impl FairnessState {
    /// An empty history: no lane has been served yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one completed service opportunity for `lane_id`.
    pub fn record_service(&mut self, lane_id: &str) {
        *self.served.entry(lane_id.to_string()).or_insert(0) += 1;
    }

    /// Completed service opportunities for `lane_id` (0 if the
    /// lane has never been served).
    pub fn served_count(&self, lane_id: &str) -> u64 {
        self.served.get(lane_id).copied().unwrap_or(0)
    }
}

/// A lane fairness policy: given the comparator-ordered eligible
/// candidates and the service history, return the service order
/// as indices into `candidates`. The result must be a permutation
/// of `0..candidates.len()` and must satisfy the module contract
/// (urgent order exact, within-lane order preserved).
pub trait LaneFairnessPolicy {
    /// Compute the service order for one selection round.
    fn service_order(&self, candidates: &[FairCandidate], state: &FairnessState) -> Vec<usize>;
}

/// The null policy: service order is the comparator's order,
/// unchanged. This is the interface's representation of today's
/// live behaviour (no fairness allocation) and the baseline every
/// real policy is compared against.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFairness;

impl LaneFairnessPolicy for NoFairness {
    fn service_order(&self, candidates: &[FairCandidate], _state: &FairnessState) -> Vec<usize> {
        (0..candidates.len()).collect()
    }
}

/// Bounded-share fairness: urgent candidates are served first in
/// their exact input order; the remaining service opportunities
/// are allocated round-robin across lanes, always favouring the
/// lane with the fewest recorded services (history plus this
/// round's allocations), ties broken by lane id. The bound this
/// models: while at least two lanes still have eligible
/// non-urgent work, no lane receives a second opportunity in a
/// round before every other such lane has received one, beyond
/// what the recorded history already skews.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundedShareFairness;

impl LaneFairnessPolicy for BoundedShareFairness {
    fn service_order(&self, candidates: &[FairCandidate], state: &FairnessState) -> Vec<usize> {
        let mut order: Vec<usize> = Vec::with_capacity(candidates.len());
        // Urgent work first, in exact input order — fairness does
        // not touch it.
        for (idx, c) in candidates.iter().enumerate() {
            if c.urgent {
                order.push(idx);
            }
        }
        // Non-urgent candidates grouped by lane, input order kept
        // inside each lane.
        let mut by_lane: BTreeMap<&str, std::collections::VecDeque<usize>> = BTreeMap::new();
        for (idx, c) in candidates.iter().enumerate() {
            if !c.urgent {
                by_lane
                    .entry(c.lane_id.as_str())
                    .or_default()
                    .push_back(idx);
            }
        }
        // Round-robin by least total service (recorded history +
        // this round's allocations), lane id as the tie-break, so
        // the order is deterministic for a given input.
        let mut allocated: BTreeMap<&str, u64> = BTreeMap::new();
        loop {
            let next_lane = by_lane
                .iter()
                .filter(|(_, q)| !q.is_empty())
                .map(|(lane, _)| {
                    let total =
                        state.served_count(lane) + allocated.get(lane).copied().unwrap_or(0);
                    (total, *lane)
                })
                .min()
                .map(|(_, lane)| lane);
            let Some(lane) = next_lane else { break };
            let idx = by_lane
                .get_mut(lane)
                .and_then(|q| q.pop_front())
                .expect("lane was filtered to non-empty queues");
            *allocated.entry(lane).or_insert(0) += 1;
            order.push(idx);
        }
        order
    }
}

// ---------------------------------------------------------------------------
// SEL-01 — Pure reference selector (developing-today/code#9491;
// design parent #9204, program #9196).
//
// A candidate reaches the comparator only after three independent
// checks, in this order:
//
// 1. **Hard gates.** Terminal lifecycle, the persistent #9178
//    `manual_only` flag, an unresolved hard dependency, an
//    incomplete graph evaluation (fails closed — an unseen blocker
//    is never dropped), a policy disable, an active lease, a
//    reached concurrency limit, or incompatible ownership each
//    exclude unconditionally, whatever the candidate's urgency.
// 2. **Task-requirement predicate.** What the work itself demands:
//    every required capability must be offered by the caller, and
//    a task pinned to a platform matches only a caller on that
//    platform (a Linux task excludes a Windows claimer).
// 3. **Caller-filter predicate.** What the caller asked to see:
//    today, a repository filter. A candidate outside the filter
//    is excluded even when its requirements match the caller.
//
// Both predicates must apply — either one alone is not enough
// (#8097). Every exclusion is recorded with a typed
// [`ExclusionReason`]; nothing is silently dropped and nothing is
// admitted by a low weight. Eligible candidates are ordered by
// the shared V2 comparator ([`compare_candidates`]): effective
// urgency ascending, exact peer rank, creation time, canonical
// identity.

/// Lifecycle states the selector distinguishes. Only
/// [`SelectorLifecycle::Ready`] can be eligible; the terminal
/// states are hard gates, kept distinct from lifecycle `hold`
/// (an operational state, not modelled here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorLifecycle {
    /// Open and claimable in principle.
    Ready,
    /// Closed without completion.
    Closed,
    /// Completed.
    Completed,
    /// Cancelled.
    Cancelled,
}

/// What a work item itself requires of any claimer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRequirements {
    /// Capabilities the claimer must offer (all of them).
    pub required_capabilities: BTreeSet<String>,
    /// Platform the task is pinned to (e.g. `"linux"`), if any.
    /// `None` is unpinned: any platform may claim.
    pub platform: Option<String>,
}

/// The hard-gate state of one candidate, as evaluated by the
/// caller against its pinned snapshot. The selector reads these
/// facts; it never fetches or mutates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateState {
    /// Lifecycle state of the work item.
    pub lifecycle: SelectorLifecycle,
    /// The persistent #9178 `manual_only` eligibility flag.
    pub manual_only: bool,
    /// False while any hard dependency is unresolved.
    pub hard_dependencies_satisfied: bool,
    /// True when graph evaluation did not complete; the selector
    /// fails closed on an incomplete evaluation.
    pub graph_evaluation_complete: bool,
    /// True when policy disables this work for selection.
    pub policy_disabled: bool,
    /// True while another claimer holds a lease on the work.
    pub active_lease: bool,
    /// True when the work's concurrency limit is reached.
    pub concurrency_limit_reached: bool,
    /// The lane that owns the work, when ownership is pinned.
    /// A caller from another lane is incompatible.
    pub owner_lane: Option<String>,
}

impl GateState {
    /// A candidate with every hard gate passing.
    pub fn ready() -> Self {
        GateState {
            lifecycle: SelectorLifecycle::Ready,
            manual_only: false,
            hard_dependencies_satisfied: true,
            graph_evaluation_complete: true,
            policy_disabled: false,
            active_lease: false,
            concurrency_limit_reached: false,
            owner_lane: None,
        }
    }
}

/// One candidate for selection: the comparator key plus the
/// facts the two predicates and the hard gates read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorCandidate {
    /// Comparator inputs (identity, urgency, peer rank, creation).
    pub key: CandidateKey,
    /// Repository the work belongs to (caller-filter input).
    pub repo: String,
    /// What the work requires of a claimer.
    pub requirements: TaskRequirements,
    /// Hard-gate facts.
    pub gates: GateState,
}

/// The claiming lane/worker, as the task-requirement predicate
/// sees it. Identity is recorded by callers, never a filter
/// (#8097); what filters is capability, platform and ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerContext {
    /// The claiming lane.
    pub lane_id: String,
    /// Capabilities the caller offers.
    pub capabilities: BTreeSet<String>,
    /// The caller's platform (e.g. `"linux"`, `"windows"`).
    pub platform: Option<String>,
}

/// The caller-side filter: which work the caller asked to see.
/// An empty repository set means no repository restriction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFilter {
    /// Repositories the caller accepts.
    pub repos: BTreeSet<String>,
}

/// Why a candidate was excluded. Every exclusion from
/// [`select`] carries exactly one reason — the first that
/// applies, in gate → requirement → filter order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusionReason {
    /// A second occurrence of an already-seen canonical identity.
    DuplicateIdentity,
    /// Lifecycle is closed, completed or cancelled.
    TerminalState,
    /// The persistent `manual_only` flag is set.
    ManualOnly,
    /// A hard dependency is unresolved.
    UnresolvedHardDependency,
    /// Graph evaluation did not complete; failed closed.
    GraphEvaluationIncomplete,
    /// Policy disables this work for selection.
    PolicyDisabled,
    /// Another claimer holds a lease.
    ActiveLease,
    /// The work's concurrency limit is reached.
    ConcurrencyLimitReached,
    /// The work is owned by a different lane.
    OwnershipIncompatible,
    /// The caller does not offer a required capability.
    MissingCapability(String),
    /// The task is pinned to a platform the caller is not on.
    PlatformMismatch {
        /// Platform the task requires.
        required: String,
        /// Platform the caller offered (`None` = unreported).
        offered: Option<String>,
    },
    /// The candidate's repository is outside the caller filter.
    CallerFilterRepoMismatch,
}

/// One excluded candidate and its typed reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusion {
    /// Canonical identity of the excluded work.
    pub work: WorkRef,
    /// Why it was excluded.
    pub reason: ExclusionReason,
}

/// The selector's result: the eligible candidates in comparator
/// order, and every exclusion with its reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Eligible candidates, comparator-ordered; the head is the
    /// normal-selection pick.
    pub eligible: Vec<SelectorCandidate>,
    /// Excluded candidates, in input order.
    pub excluded: Vec<Exclusion>,
}

impl Selection {
    /// The normal-selection pick: the first eligible candidate.
    pub fn head(&self) -> Option<&SelectorCandidate> {
        self.eligible.first()
    }
}

/// The hard-gate check: `Some(reason)` when a gate excludes the
/// candidate, `None` when every gate passes.
pub fn hard_gate_exclusion(
    candidate: &SelectorCandidate,
    caller: &CallerContext,
) -> Option<ExclusionReason> {
    let gates = &candidate.gates;
    if gates.lifecycle != SelectorLifecycle::Ready {
        return Some(ExclusionReason::TerminalState);
    }
    if gates.manual_only {
        return Some(ExclusionReason::ManualOnly);
    }
    if !gates.hard_dependencies_satisfied {
        return Some(ExclusionReason::UnresolvedHardDependency);
    }
    if !gates.graph_evaluation_complete {
        return Some(ExclusionReason::GraphEvaluationIncomplete);
    }
    if gates.policy_disabled {
        return Some(ExclusionReason::PolicyDisabled);
    }
    if gates.active_lease {
        return Some(ExclusionReason::ActiveLease);
    }
    if gates.concurrency_limit_reached {
        return Some(ExclusionReason::ConcurrencyLimitReached);
    }
    if let Some(owner) = &gates.owner_lane {
        if owner != &caller.lane_id {
            return Some(ExclusionReason::OwnershipIncompatible);
        }
    }
    None
}

/// The task-requirement predicate: `None` when the caller meets
/// everything the work requires, otherwise the reason it does
/// not. Capabilities are checked in sorted order so the reported
/// missing capability is deterministic.
pub fn task_requirement_exclusion(
    candidate: &SelectorCandidate,
    caller: &CallerContext,
) -> Option<ExclusionReason> {
    for capability in &candidate.requirements.required_capabilities {
        if !caller.capabilities.contains(capability) {
            return Some(ExclusionReason::MissingCapability(capability.clone()));
        }
    }
    if let Some(required) = &candidate.requirements.platform {
        if caller.platform.as_ref() != Some(required) {
            return Some(ExclusionReason::PlatformMismatch {
                required: required.clone(),
                offered: caller.platform.clone(),
            });
        }
    }
    None
}

/// The caller-filter predicate: true when the candidate is
/// inside what the caller asked to see.
pub fn caller_filter_matches(candidate: &SelectorCandidate, filter: &CallerFilter) -> bool {
    filter.repos.is_empty() || filter.repos.contains(&candidate.repo)
}

/// The urgent class names the default urgent predicate reads
/// from the active profile: the two classes above `high` in
/// `fleet.queue` v1. The classes are named from the profile,
/// never numeric thresholds (#9204).
pub const DEFAULT_URGENT_CLASS_NAMES: [&str; 2] = ["critical", "expedite"];

/// The urgent predicate: true when the candidate's urgency is a
/// member of one of the qualifying classes named in the active
/// profile. Untriaged work is never urgent. A candidate whose
/// urgency belongs to a different profile (or version) is a
/// typed error, never a numeric comparison.
pub fn is_urgent(
    candidate: &SelectorCandidate,
    profile: &PriorityProfile,
    qualifying_class_names: &[&str],
) -> Result<bool, PriorityError> {
    let Some(value) = &candidate.key.urgency else {
        return Ok(false);
    };
    if value.profile_id != profile.profile_id || value.profile_version != profile.version {
        return Err(PriorityError::ProfileMismatch {
            expected: format!("{}@{}", profile.profile_id, profile.version),
            found: format!("{}@{}", value.profile_id, value.profile_version),
        });
    }
    Ok(profile.entries.iter().any(|entry| {
        entry.value == value.value && qualifying_class_names.contains(&entry.name.as_str())
    }))
}

/// True when any candidate in `eligible` is urgent under the
/// urgent predicate — the exploration gate's input: bounded
/// exploration may run only when no qualifying urgent work that
/// requires this lane would be delayed, and `eligible` is
/// already exactly the set that requires this caller.
pub fn any_urgent(
    eligible: &[SelectorCandidate],
    profile: &PriorityProfile,
    qualifying_class_names: &[&str],
) -> Result<bool, PriorityError> {
    for candidate in eligible {
        if is_urgent(candidate, profile, qualifying_class_names)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Run the reference selector over one pinned candidate set.
///
/// Duplicate canonical identities are excluded after their first
/// occurrence. Mixed priority profiles among eligible candidates
/// are a typed [`PriorityError`], rejected before ordering
/// rather than partially ordered.
pub fn select(
    candidates: Vec<SelectorCandidate>,
    caller: &CallerContext,
    filter: &CallerFilter,
) -> Result<Selection, PriorityError> {
    let mut seen: BTreeSet<WorkRef> = BTreeSet::new();
    let mut eligible: Vec<SelectorCandidate> = Vec::new();
    let mut excluded: Vec<Exclusion> = Vec::new();
    for candidate in candidates {
        let work = candidate.key.work.clone();
        if !seen.insert(work.clone()) {
            excluded.push(Exclusion {
                work,
                reason: ExclusionReason::DuplicateIdentity,
            });
            continue;
        }
        let reason = hard_gate_exclusion(&candidate, caller)
            .or_else(|| task_requirement_exclusion(&candidate, caller))
            .or_else(|| {
                if caller_filter_matches(&candidate, filter) {
                    None
                } else {
                    Some(ExclusionReason::CallerFilterRepoMismatch)
                }
            });
        match reason {
            Some(reason) => excluded.push(Exclusion { work, reason }),
            None => eligible.push(candidate),
        }
    }
    // One profile version across the eligible set, checked before
    // sorting (the comparator's own rule).
    let mut profile: Option<&super::priority::PriorityValue> = None;
    for candidate in &eligible {
        if let Some(value) = &candidate.key.urgency {
            match profile {
                None => profile = Some(value),
                Some(first) => {
                    first.cmp_urgency(value)?;
                }
            }
        }
    }
    eligible.sort_by(|a, b| compare_candidates(&a.key, &b.key).expect("profiles checked above"));
    Ok(Selection { eligible, excluded })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(work_id: &str, lane_id: &str, effective_urgency: i64, urgent: bool) -> FairCandidate {
        FairCandidate {
            work_id: work_id.to_string(),
            lane_id: lane_id.to_string(),
            effective_urgency,
            urgent,
        }
    }

    fn ids<'a>(candidates: &'a [FairCandidate], order: &[usize]) -> Vec<&'a str> {
        order
            .iter()
            .map(|&i| candidates[i].work_id.as_str())
            .collect()
    }

    /// The SEL-06 oracle: under the fairness interface, urgent
    /// order is preserved exactly — the urgent subsequence of the
    /// service order equals the urgent subsequence of the input,
    /// every urgent candidate precedes every non-urgent one, and
    /// the result is a permutation of the input (nothing admitted,
    /// nothing dropped, no score consulted or changed).
    #[test]
    fn pmo_selection_fairness_preserves_urgent_order() {
        // Comparator-ordered input with urgent work interleaved
        // among lanes and non-urgent work.
        let candidates = vec![
            cand("n-a1", "lane-a", 50, false),
            cand("u-b1", "lane-b", 10, true),
            cand("n-b1", "lane-b", 60, false),
            cand("u-a1", "lane-a", 5, true),
            cand("n-a2", "lane-a", 70, false),
            cand("u-c1", "lane-c", 20, true),
        ];
        let state = FairnessState::new();
        let order = BoundedShareFairness.service_order(&candidates, &state);

        // Permutation: same multiset of indices.
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..candidates.len()).collect::<Vec<_>>());

        let result = ids(&candidates, &order);
        // Urgent subsequence is exactly the input's urgent
        // subsequence, in order.
        let urgent_in: Vec<&str> = candidates
            .iter()
            .filter(|c| c.urgent)
            .map(|c| c.work_id.as_str())
            .collect();
        let urgent_out: Vec<&str> = order
            .iter()
            .map(|&i| &candidates[i])
            .filter(|c| c.urgent)
            .map(|c| c.work_id.as_str())
            .collect();
        assert_eq!(urgent_out, urgent_in);
        assert_eq!(urgent_out, vec!["u-b1", "u-a1", "u-c1"]);
        // No non-urgent candidate is ahead of any urgent one.
        let last_urgent_pos = result
            .iter()
            .rposition(|id| urgent_in.contains(id))
            .expect("urgent work present");
        let first_nonurgent_pos = result
            .iter()
            .position(|id| !urgent_in.contains(id))
            .expect("non-urgent work present");
        assert!(last_urgent_pos < first_nonurgent_pos);
    }

    #[test]
    fn fairness_interleaves_lanes_with_bounded_share() {
        let candidates = vec![
            cand("a1", "lane-a", 50, false),
            cand("a2", "lane-a", 51, false),
            cand("a3", "lane-a", 52, false),
            cand("b1", "lane-b", 53, false),
            cand("b2", "lane-b", 54, false),
            cand("b3", "lane-b", 55, false),
        ];
        let order = BoundedShareFairness.service_order(&candidates, &FairnessState::new());
        // Equal history: strict alternation, within-lane order kept.
        assert_eq!(
            ids(&candidates, &order),
            vec!["a1", "b1", "a2", "b2", "a3", "b3"]
        );
    }

    #[test]
    fn fairness_respects_service_history() {
        let candidates = vec![
            cand("a1", "lane-a", 50, false),
            cand("a2", "lane-a", 51, false),
            cand("b1", "lane-b", 52, false),
        ];
        let mut state = FairnessState::new();
        state.record_service("lane-a");
        state.record_service("lane-a");
        let order = BoundedShareFairness.service_order(&candidates, &state);
        // lane-b is behind on service, so its candidate is served
        // first; lane-a's candidates follow in their input order.
        assert_eq!(ids(&candidates, &order), vec!["b1", "a1", "a2"]);
    }

    #[test]
    fn fairness_preserves_within_lane_order_and_empty_input() {
        let candidates = vec![
            cand("a1", "lane-a", 50, false),
            cand("a2", "lane-a", 51, false),
        ];
        let order = BoundedShareFairness.service_order(&candidates, &FairnessState::new());
        assert_eq!(ids(&candidates, &order), vec!["a1", "a2"]);
        assert!(BoundedShareFairness
            .service_order(&[], &FairnessState::new())
            .is_empty());
    }

    #[test]
    fn no_fairness_is_the_comparator_order() {
        let candidates = vec![
            cand("n1", "lane-a", 50, false),
            cand("u1", "lane-b", 10, true),
        ];
        let order = NoFairness.service_order(&candidates, &FairnessState::new());
        assert_eq!(order, vec![0, 1]);
    }
    // --- SEL-01 reference selector -------------------------------------

    fn sel_cand(
        id: &str,
        urgency_name: Option<&str>,
        repo: &str,
        platform: Option<&str>,
        capabilities: &[&str],
    ) -> SelectorCandidate {
        let profile = PriorityProfile::queue_v1();
        SelectorCandidate {
            key: CandidateKey {
                work: WorkRef::new("fleet-local", id).unwrap(),
                urgency: urgency_name.map(|name| profile.resolve_name(name).unwrap()),
                peer_rank: None,
                created_at: 100,
            },
            repo: repo.to_string(),
            requirements: TaskRequirements {
                required_capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                platform: platform.map(|p| p.to_string()),
            },
            gates: GateState::ready(),
        }
    }

    fn linux_caller() -> CallerContext {
        CallerContext {
            lane_id: "lane-a".to_string(),
            capabilities: ["rust"].iter().map(|c| c.to_string()).collect(),
            platform: Some("linux".to_string()),
        }
    }

    fn code_filter() -> CallerFilter {
        CallerFilter {
            repos: ["code"].iter().map(|r| r.to_string()).collect(),
        }
    }

    fn reason_for(selection: &Selection, id: &str) -> ExclusionReason {
        selection
            .excluded
            .iter()
            .find(|e| e.work.work_id == id)
            .map(|e| e.reason.clone())
            .unwrap_or_else(|| panic!("candidate {id} was not excluded"))
    }

    /// The SEL-01 oracle: both predicates must apply, independently.
    /// A candidate whose task requirements match but that falls
    /// outside the caller filter is excluded; a candidate inside
    /// the filter whose requirements the caller cannot meet is
    /// excluded — including the design's counterexample: a
    /// Linux task with a `repo=code` filter still excludes a
    /// Windows claimer.
    #[test]
    fn pmo_selection_requirements_and_filter_both_apply() {
        let eligible = sel_cand("1", Some("high"), "code", Some("linux"), &["rust"]);
        // Requirements met, filter missed (repo outside the filter).
        let filter_miss = sel_cand(
            "2",
            Some("critical"),
            "other-repo",
            Some("linux"),
            &["rust"],
        );
        // Filter hit, requirements missed (capability).
        let requirement_miss = sel_cand("3", Some("critical"), "code", Some("linux"), &["zig"]);
        let selection = select(
            vec![eligible, filter_miss, requirement_miss],
            &linux_caller(),
            &code_filter(),
        )
        .unwrap();
        // Only the both-predicates candidate is eligible, and it is
        // the head despite the excluded candidates' higher urgency:
        // eligibility is a gate, not a score.
        assert_eq!(selection.eligible.len(), 1);
        assert_eq!(selection.head().unwrap().key.work.work_id, "1");
        assert_eq!(
            reason_for(&selection, "2"),
            ExclusionReason::CallerFilterRepoMismatch
        );
        assert_eq!(
            reason_for(&selection, "3"),
            ExclusionReason::MissingCapability("zig".to_string())
        );

        // The counterexample: task requires Linux, the caller's
        // repo=code filter matches the candidate, but the claimer
        // is on Windows — the task-requirement predicate excludes.
        let mut windows_caller = linux_caller();
        windows_caller.platform = Some("windows".to_string());
        let selection = select(
            vec![sel_cand(
                "4",
                Some("critical"),
                "code",
                Some("linux"),
                &["rust"],
            )],
            &windows_caller,
            &code_filter(),
        )
        .unwrap();
        assert!(selection.eligible.is_empty());
        assert_eq!(
            reason_for(&selection, "4"),
            ExclusionReason::PlatformMismatch {
                required: "linux".to_string(),
                offered: Some("windows".to_string()),
            }
        );
    }

    #[test]
    fn hard_ineligible_never_selected() {
        // Every hard gate, each on an otherwise-critical candidate,
        // plus one ready normal candidate.
        let mut closed = sel_cand("1", Some("critical"), "code", None, &[]);
        closed.gates.lifecycle = SelectorLifecycle::Closed;
        let mut manual = sel_cand("2", Some("critical"), "code", None, &[]);
        manual.gates.manual_only = true;
        let mut dep = sel_cand("3", Some("critical"), "code", None, &[]);
        dep.gates.hard_dependencies_satisfied = false;
        let mut graph = sel_cand("4", Some("critical"), "code", None, &[]);
        graph.gates.graph_evaluation_complete = false;
        let mut disabled = sel_cand("5", Some("critical"), "code", None, &[]);
        disabled.gates.policy_disabled = true;
        let mut leased = sel_cand("6", Some("critical"), "code", None, &[]);
        leased.gates.active_lease = true;
        let mut capped = sel_cand("7", Some("critical"), "code", None, &[]);
        capped.gates.concurrency_limit_reached = true;
        let mut owned = sel_cand("8", Some("critical"), "code", None, &[]);
        owned.gates.owner_lane = Some("lane-b".to_string());
        let ready = sel_cand("9", Some("normal"), "code", None, &[]);

        let selection = select(
            vec![
                closed, manual, dep, graph, disabled, leased, capped, owned, ready,
            ],
            &linux_caller(),
            &CallerFilter::default(),
        )
        .unwrap();
        assert_eq!(selection.eligible.len(), 1);
        assert_eq!(selection.head().unwrap().key.work.work_id, "9");
        assert_eq!(reason_for(&selection, "1"), ExclusionReason::TerminalState);
        assert_eq!(reason_for(&selection, "2"), ExclusionReason::ManualOnly);
        assert_eq!(
            reason_for(&selection, "3"),
            ExclusionReason::UnresolvedHardDependency
        );
        assert_eq!(
            reason_for(&selection, "4"),
            ExclusionReason::GraphEvaluationIncomplete
        );
        assert_eq!(reason_for(&selection, "5"), ExclusionReason::PolicyDisabled);
        assert_eq!(reason_for(&selection, "6"), ExclusionReason::ActiveLease);
        assert_eq!(
            reason_for(&selection, "7"),
            ExclusionReason::ConcurrencyLimitReached
        );
        assert_eq!(
            reason_for(&selection, "8"),
            ExclusionReason::OwnershipIncompatible
        );
    }

    #[test]
    fn urgent_predicate_names_classes_from_profile() {
        let profile = PriorityProfile::queue_v1();
        let critical = sel_cand("1", Some("critical"), "code", None, &[]);
        let expedite = sel_cand("2", Some("expedite"), "code", None, &[]);
        let high = sel_cand("3", Some("high"), "code", None, &[]);
        let untriaged = sel_cand("4", None, "code", None, &[]);
        assert!(is_urgent(&critical, &profile, &DEFAULT_URGENT_CLASS_NAMES).unwrap());
        assert!(is_urgent(&expedite, &profile, &DEFAULT_URGENT_CLASS_NAMES).unwrap());
        assert!(!is_urgent(&high, &profile, &DEFAULT_URGENT_CLASS_NAMES).unwrap());
        assert!(!is_urgent(&untriaged, &profile, &DEFAULT_URGENT_CLASS_NAMES).unwrap());
        // The classes are the caller's policy choice, named from
        // the profile — not a numeric threshold baked into the
        // predicate.
        assert!(is_urgent(&high, &profile, &["high"]).unwrap());

        // An urgent item owned by another lane is excluded before
        // the urgent gate reads the eligible set, so it does not
        // trigger this lane's urgent gate.
        let mut foreign_urgent = sel_cand("5", Some("critical"), "code", None, &[]);
        foreign_urgent.gates.owner_lane = Some("lane-b".to_string());
        let selection = select(
            vec![
                foreign_urgent,
                sel_cand("6", Some("normal"), "code", None, &[]),
            ],
            &linux_caller(),
            &CallerFilter::default(),
        )
        .unwrap();
        assert!(!any_urgent(&selection.eligible, &profile, &DEFAULT_URGENT_CLASS_NAMES).unwrap());
    }

    #[test]
    fn selection_orders_by_comparator_and_dedupes_identity() {
        let selection = select(
            vec![
                sel_cand("2", Some("normal"), "code", None, &[]),
                sel_cand("1", Some("critical"), "code", None, &[]),
                sel_cand("1", Some("critical"), "code", None, &[]),
            ],
            &linux_caller(),
            &CallerFilter::default(),
        )
        .unwrap();
        let ids: Vec<&str> = selection
            .eligible
            .iter()
            .map(|c| c.key.work.work_id.as_str())
            .collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(
            reason_for(&selection, "1"),
            ExclusionReason::DuplicateIdentity
        );
    }
}
