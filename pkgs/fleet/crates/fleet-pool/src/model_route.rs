//! The model-route boundary adapter (design #13752 §3.5 step 3,
//! #14723): the boundary between the model-route proxy/fork and
//! `fleet-pool`.
//!
//! The proxy/fork keeps what is its own (§3.3): its classifiers
//! and plants, and the #13693 scoping layer itself — this adapter
//! is explicitly *not* a rewrite of that layer. What crosses the
//! boundary in each direction is narrow:
//!
//! - **In:** recorded outcomes. The domain classifies a raw result
//!   (quota vs transport vs served failure, and the scope the
//!   evidence supports) and hands the adapter a [`RouteOutcome`];
//!   the adapter turns it into the [`Classification`] the primitive
//!   consumes, with #13693's wall rules expressed as *grouping
//!   declarations* (below) rather than config tricks.
//! - **Out:** selections. [`ModelRoute::select`] returns the next
//!   leg to try, or `None` when every leg is walled — an
//!   all-walled pool stops walking (#13693 RC-4).
//!
//! Grouping declarations: a route is a group; each provider is a
//! group under it; each (provider, model) leg is a member under
//! its provider. Providers that share one cap (the #13693
//! alias-provider pattern — e.g. a plan alias and a free alias of
//! the same underlying account) are additionally declared under a
//! shared *cap group*. A quota outcome's evidence scope is the
//! leg's cap scope — the cap group when one is declared, else the
//! provider group — so the wall binds at the cap's scope, not at
//! the reader's shape (#13693 RC-3). A quota wall with no recorded
//! reset still gates, for the pool's inferred-wall TTL, and
//! re-admits through probing (RC-1/RC-2 — the primitive's own
//! semantics; the adapter supplies no exception to them).

use std::collections::BTreeMap;

use crate::{
    AttemptKind, Candidate, Classification, NodeId, OutcomeEffect, Pool, Seconds,
    ServedDisposition, Strategy,
};

/// The route's own selection cursor: declaration order over the
/// legs the pool currently offers. The pool supplies availability
/// (walled legs never become candidates), state, and the decision
/// log; the route's preference order is the boundary's, exactly as
/// the walker's cursor is the walker's (#14722).
struct DeclarationOrder {
    order: Vec<NodeId>,
    attempted: std::collections::BTreeSet<NodeId>,
}

impl Strategy for DeclarationOrder {
    fn name(&self) -> &str {
        "route-declared"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        self.order.iter().find_map(|id| {
            if self.attempted.contains(id) {
                return None;
            }
            candidates.iter().position(|c| &c.id == id)
        })
    }
}

/// One (provider, model) leg of a model route.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteLeg {
    pub provider: String,
    pub model: String,
}

impl RouteLeg {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }

    /// The leg's pool member id: `provider/model`.
    pub fn node_id(&self) -> NodeId {
        NodeId::new(format!("{}/{}", self.provider, self.model))
    }
}

/// A recorded outcome at the boundary, already classified by the
/// domain (the proxy/fork's classifiers stay on its side, §3.3).
#[derive(Clone, Debug)]
pub enum RouteOutcome {
    /// The leg served the call (any exit status the domain counts
    /// as served).
    Served,
    /// A quota-class answer: the cap is exhausted. `reset_hint` is
    /// the recorded reset time when the domain knows one; `None`
    /// still walls, for the pool's inferred TTL (RC-1/RC-2).
    Quota {
        reset_hint: Option<Seconds>,
        evidence: String,
    },
    /// Transport/auth/connect/timeout: the leg never answered.
    /// The walk advances; no wall at any scope.
    Transport { evidence: String },
    /// The leg answered and the answer was a failure whose
    /// classifier judges the walk may (`advance`) or may not
    /// advance past this leg. No state change beyond the verdict.
    ServedFailure { advance: bool, evidence: String },
}

/// The boundary: one route's pool, its grouping declarations, and
/// the map from a leg to the scope its quota evidence walls.
pub struct ModelRoute {
    pool: Pool,
    legs: Vec<RouteLeg>,
    cap_scope: BTreeMap<RouteLeg, NodeId>,
    /// Legs attempted in the current walk. A walk attempts each
    /// leg at most once (the walker's rule, #13717): recording an
    /// outcome marks the leg attempted, a served call ends the
    /// walk and clears the set, and a selection that finds nothing
    /// ends an exhausted walk and clears it too. Pool state
    /// (demotions, walls) persists across walks; this set does not.
    attempted: std::collections::BTreeSet<NodeId>,
}

impl ModelRoute {
    /// Declare a route: `legs` in the route's preference order.
    /// `cap_groups` declares shared caps: each entry names a cap
    /// and the providers whose legs all draw on it (the #13693
    /// alias-provider pattern as a grouping declaration).
    pub fn new(route: &str, legs: Vec<RouteLeg>, cap_groups: &[(&str, &[&str])]) -> Self {
        let mut pool = Pool::new();
        let route_id = NodeId::new(route);
        pool.add_group(route_id.clone(), None);

        // Cap groups sit between the route and their providers.
        let mut provider_parent: BTreeMap<String, NodeId> = BTreeMap::new();
        let mut cap_scope: BTreeMap<RouteLeg, NodeId> = BTreeMap::new();
        for (cap, providers) in cap_groups {
            let cap_id = NodeId::new(format!("{route}/cap:{cap}"));
            pool.add_group(cap_id.clone(), Some(&route_id));
            for provider in *providers {
                provider_parent.insert((*provider).to_owned(), cap_id.clone());
            }
        }

        let mut providers_seen: Vec<String> = Vec::new();
        for leg in &legs {
            if !providers_seen.contains(&leg.provider) {
                providers_seen.push(leg.provider.clone());
                let parent = provider_parent
                    .get(&leg.provider)
                    .cloned()
                    .unwrap_or_else(|| route_id.clone());
                pool.add_group(
                    NodeId::new(format!("{route}/{}", leg.provider)),
                    Some(&parent),
                );
            }
            let provider_group = NodeId::new(format!("{route}/{}", leg.provider));
            pool.add_member(leg.node_id(), Some(&provider_group));
            // Quota evidence walls at the cap's scope: the cap
            // group when the provider draws on a declared cap,
            // else the provider group itself.
            let scope = provider_parent
                .get(&leg.provider)
                .cloned()
                .unwrap_or(provider_group);
            cap_scope.insert(leg.clone(), scope);
        }

        Self {
            pool,
            legs,
            cap_scope,
            attempted: std::collections::BTreeSet::new(),
        }
    }

    /// The classification the primitive consumes for one recorded
    /// outcome — `None` for a served call, which is recorded as a
    /// success instead.
    pub fn classify(&self, leg: &RouteLeg, outcome: &RouteOutcome) -> Option<Classification> {
        match outcome {
            RouteOutcome::Served => None,
            RouteOutcome::Quota {
                reset_hint,
                evidence,
            } => Some(Classification {
                kind: AttemptKind::ServedFailure,
                served_disposition: ServedDisposition::Wall,
                advance: true,
                evidence_scope: self
                    .cap_scope
                    .get(leg)
                    .cloned()
                    .unwrap_or_else(|| leg.node_id()),
                reset_hint: *reset_hint,
                evidence: evidence.clone(),
            }),
            RouteOutcome::Transport { evidence } => Some(Classification {
                kind: AttemptKind::ServeFailure,
                served_disposition: ServedDisposition::Hold,
                advance: true,
                evidence_scope: leg.node_id(),
                reset_hint: None,
                evidence: evidence.clone(),
            }),
            RouteOutcome::ServedFailure { advance, evidence } => Some(Classification {
                kind: AttemptKind::ServedFailure,
                served_disposition: ServedDisposition::Hold,
                advance: *advance,
                evidence_scope: leg.node_id(),
                reset_hint: None,
                evidence: evidence.clone(),
            }),
        }
    }

    /// Consume one recorded outcome for `leg` at `now`. Returns
    /// the primitive's effect (notably `advance`) for non-served
    /// outcomes; a served call records a success and returns
    /// `None`.
    pub fn record(
        &mut self,
        leg: &RouteLeg,
        outcome: &RouteOutcome,
        now: Seconds,
    ) -> Option<OutcomeEffect> {
        self.attempted.insert(leg.node_id());
        match self.classify(leg, outcome) {
            None => {
                self.pool.record_success(&leg.node_id(), now);
                self.attempted.clear();
                None
            }
            Some(cls) => Some(self.pool.record_outcome(&leg.node_id(), &cls, now)),
        }
    }

    /// The next leg to try at `now`, in declaration order over the
    /// legs the pool currently offers — or `None` when no leg is
    /// available, which is the walk's stop signal (RC-4).
    pub fn select(&mut self, now: Seconds) -> Option<RouteLeg> {
        let mut cursor = DeclarationOrder {
            order: self.legs.iter().map(RouteLeg::node_id).collect(),
            attempted: self.attempted.clone(),
        };
        let picked = match self.pool.select(&mut cursor, &|_| None, now) {
            Some(p) => p,
            None => {
                self.attempted.clear();
                return None;
            }
        };
        self.legs
            .iter()
            .find(|leg| leg.node_id() == picked)
            .cloned()
    }

    /// Advance time on the underlying pool (wall expiry and
    /// re-admission probes; RC-2's rejoin path).
    pub fn tick(&mut self, now: Seconds, probe: &mut impl crate::Probe) {
        self.pool.tick(now, probe);
    }

    /// The underlying pool, for read-back (walls, decision log).
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}
