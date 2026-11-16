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

use std::collections::BTreeMap;

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
        assert!(
            BoundedShareFairness
                .service_order(&[], &FairnessState::new())
                .is_empty()
        );
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
}
