//! B15: prioritized remote selection, its inputs and ordering, and what
//! a remote's negative answer means (#10229, statements S1.47-S1.53;
//! Part B "Query contract", fan-out design #9144 section 3).
//!
//! A federated query asks the local store first (S1.47), then fans out
//! (S1.48) to a prioritized list of remotes (S1.49), none of which is
//! assumed to hold everything (S1.50):
//!
//! - [`plan`] turns the configured remote list into an order to ask in.
//!   The only ordering input is the configured `priority` (ascending),
//!   with ties broken by list position, so equal inputs always give the
//!   same order (S1.52). Trust class is a gate, not a score: a remote is
//!   eligible only when it has a peer-class policy (B31) in force whose
//!   class allows querying it (`fully_trusted` or `accept_signed_from`)
//!   and it is not disabled; every other remote is skipped with a
//!   reason. A peer listed twice is asked once. Reputation, coverage,
//!   latency and cost (S1.53) are not inputs yet.
//! - Routing advice (B23) may reorder only when the operator opts in by
//!   passing it: remotes the advice reports down are asked after the
//!   rest, in their configured order. Advice never removes a remote and
//!   never makes one eligible. Without it the configured order stands
//!   (the rollback).
//! - A priority decides only the order of asking. It gives no remote
//!   more authority over the answer.
//! - [`Search`] runs the plan: the local answer first, then one remote
//!   at a time, until the requested limit is met (when stopping at the
//!   limit), the list is exhausted, or the deadline passes. Results are
//!   deduplicated by stable result id, so the same result from two
//!   places counts once towards the limit (B21), and each peer answers
//!   at most once.
//! - A remote's "not found" means "not at that peer" (S1.51): it is
//!   recorded against that peer and the search moves on. The
//!   [`Outcome`] has no notion of global absence; when nothing is found
//!   it says which peers answered "not found", which failed and which
//!   were never asked.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use super::b20_request_identity::RequestId;
use super::b31_peer_classes::{PeerClass, PeerPolicy};

/// A peer's local identifier, as in its B31 policy.
pub type PeerId = String;
/// A stable result id (B21).
pub type ResultId = [u8; 32];

/// One configured remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// The remote.
    pub peer_id: PeerId,
    /// Lower is asked earlier.
    pub priority: u32,
}

/// Why a configured remote is not asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No peer-class policy for it.
    NoPolicy,
    /// Its policy is not in force now.
    PolicyNotInForce,
    /// Its class does not allow querying it (send-only or unknown).
    ClassForbidsQuery,
    /// It is disabled or revoked locally.
    Disabled,
    /// It is already in the list at an earlier position.
    DuplicateEntry,
}

/// Advisory routing input the operator chose to apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingAdvice {
    /// Remotes a current routing observation reports down.
    pub down: BTreeSet<PeerId>,
}

/// The order to ask in, and what was left out and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The request this plan serves.
    pub request_id: RequestId,
    /// Eligible remotes, first to ask first.
    pub order: Vec<PeerId>,
    /// Configured remotes not asked, in list order.
    pub skipped: Vec<(PeerId, SkipReason)>,
}

const fn query_allowed(class: PeerClass) -> bool {
    matches!(class, PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom)
}

/// Order the configured remotes for `request_id` at `now`.
pub fn plan(
    request_id: RequestId,
    remotes: &[RemoteEntry],
    policies: &BTreeMap<PeerId, PeerPolicy>,
    disabled: &BTreeSet<PeerId>,
    advice: Option<&RoutingAdvice>,
    now: u64,
) -> Plan {
    let mut seen = BTreeSet::new();
    let mut eligible = Vec::new();
    let mut skipped = Vec::new();
    for (position, entry) in remotes.iter().enumerate() {
        let reason = if seen.insert(entry.peer_id.as_str()) {
            match policies.get(&entry.peer_id) {
                None => Some(SkipReason::NoPolicy),
                Some(policy) if !policy.in_force_at(now) => Some(SkipReason::PolicyNotInForce),
                Some(policy) if !query_allowed(policy.class) => Some(SkipReason::ClassForbidsQuery),
                Some(_) if disabled.contains(&entry.peer_id) => Some(SkipReason::Disabled),
                Some(_) => None,
            }
        } else {
            Some(SkipReason::DuplicateEntry)
        };
        match reason {
            Some(reason) => skipped.push((entry.peer_id.clone(), reason)),
            None => eligible.push((entry.priority, position, entry.peer_id.clone())),
        }
    }
    let down = |peer: &PeerId| advice.is_some_and(|a| a.down.contains(peer));
    eligible.sort_by_key(|(priority, position, peer)| (down(peer), *priority, *position));
    Plan {
        request_id,
        order: eligible.into_iter().map(|(_, _, peer)| peer).collect(),
        skipped,
    }
}

/// One answer, local or remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Matching results.
    Found(BTreeSet<ResultId>),
    /// Nothing matched there.
    NotFound,
    /// No usable answer (timeout, refusal, error).
    Failed,
}

/// Where a result was found.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The local store.
    Local,
    /// A remote.
    Remote(PeerId),
}

/// Why the search stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The requested number of results was reached.
    LimitReached,
    /// Every eligible remote was asked.
    Exhausted,
    /// The query deadline passed.
    Deadline,
}

/// What to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Ask this remote.
    Ask(PeerId),
    /// Stop.
    Stop(StopReason),
}

/// Why an answer was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchError {
    /// Remotes were consulted before the local store answered.
    LocalNotFirst,
    /// The local store already answered.
    LocalAlreadyAnswered,
    /// The answer is from a remote that is not the one being asked.
    NotAsked,
}

/// The search state for one request.
#[derive(Debug, Clone)]
pub struct Search {
    plan: Plan,
    limit: usize,
    stop_at_limit: bool,
    deadline: u64,
    local: Option<Answer>,
    cursor: usize,
    asking: Option<PeerId>,
    results: BTreeMap<ResultId, BTreeSet<Source>>,
    absent_at: Vec<PeerId>,
    failed_at: Vec<PeerId>,
    stopped: Option<StopReason>,
}

/// What a search found, with the per-peer gaps named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Each result found, with every place it was found.
    pub results: BTreeMap<ResultId, BTreeSet<Source>>,
    /// Whether the local store had nothing.
    pub local_not_found: bool,
    /// Remotes that answered "not found": absent at that peer only.
    pub absent_at: Vec<PeerId>,
    /// Remotes that gave no usable answer, including one still being
    /// asked when the deadline passed.
    pub failed_at: Vec<PeerId>,
    /// The remote being asked now, if any.
    pub pending: Option<PeerId>,
    /// Eligible remotes never asked.
    pub not_asked: Vec<PeerId>,
    /// Why the search stopped, if it has.
    pub stopped: Option<StopReason>,
}

impl Search {
    /// Start a search over `plan`, wanting `limit` results by `deadline`.
    pub const fn new(plan: Plan, limit: usize, stop_at_limit: bool, deadline: u64) -> Self {
        Self {
            plan,
            limit,
            stop_at_limit,
            deadline,
            local: None,
            cursor: 0,
            asking: None,
            results: BTreeMap::new(),
            absent_at: Vec::new(),
            failed_at: Vec::new(),
            stopped: None,
        }
    }

    fn absorb(&mut self, source: &Source, ids: &BTreeSet<ResultId>) {
        for id in ids {
            self.results.entry(*id).or_default().insert(source.clone());
        }
    }

    /// Record the local store's answer, which always comes first.
    pub fn record_local(&mut self, answer: Answer) -> Result<(), SearchError> {
        if self.local.is_some() {
            return Err(SearchError::LocalAlreadyAnswered);
        }
        if let Answer::Found(ids) = &answer {
            self.absorb(&Source::Local, ids);
        }
        self.local = Some(answer);
        Ok(())
    }

    /// The next step at `now`. Until the remote being asked answers,
    /// the step is to ask it again; if the deadline passes first, it
    /// counts as failed.
    pub fn next(&mut self, now: u64) -> Result<Next, SearchError> {
        if self.local.is_none() {
            return Err(SearchError::LocalNotFirst);
        }
        if let Some(reason) = self.stopped {
            return Ok(Next::Stop(reason));
        }
        let reason = if self.stop_at_limit && self.results.len() >= self.limit {
            StopReason::LimitReached
        } else if now >= self.deadline {
            StopReason::Deadline
        } else if let Some(peer) = &self.asking {
            return Ok(Next::Ask(peer.clone()));
        } else if let Some(peer) = self.plan.order.get(self.cursor) {
            self.cursor += 1;
            self.asking = Some(peer.clone());
            return Ok(Next::Ask(peer.clone()));
        } else {
            StopReason::Exhausted
        };
        if let Some(peer) = self.asking.take() {
            self.failed_at.push(peer);
        }
        self.stopped = Some(reason);
        Ok(Next::Stop(reason))
    }

    /// Record the answer of the remote being asked.
    pub fn record_remote(&mut self, peer: &str, answer: Answer) -> Result<(), SearchError> {
        if self.asking.as_deref() != Some(peer) {
            return Err(SearchError::NotAsked);
        }
        self.asking = None;
        let source = Source::Remote(peer.to_owned());
        match answer {
            Answer::Found(ids) => self.absorb(&source, &ids),
            Answer::NotFound => self.absent_at.push(peer.to_owned()),
            Answer::Failed => self.failed_at.push(peer.to_owned()),
        }
        Ok(())
    }

    /// What the search has so far.
    pub fn outcome(&self) -> Outcome {
        Outcome {
            results: self.results.clone(),
            local_not_found: self.local == Some(Answer::NotFound),
            absent_at: self.absent_at.clone(),
            failed_at: self.failed_at.clone(),
            pending: self.asking.clone(),
            not_asked: self.plan.order[self.cursor..].to_vec(),
            stopped: self.stopped,
        }
    }
}
