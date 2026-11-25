//! `fleet-pool` — the generic pooling primitive.
//!
//! Design of record: `doc/2026-11-23T21-10-36Z_design_fleet-transport-pooling`
//! (§3, tracking issue #13752), work item 3a (#14721). Five estate codebases
//! re-implemented the same machine — members, walls, strategies — and
//! re-learned the same lessons in production (#13693 wall scoping, #13611
//! unknown load, #13717 walker advance semantics). This crate is that
//! machine, written once.
//!
//! What the primitive owns (design §3.2):
//! - **Members and nested groups.** A member is opaque; groups nest
//!   (member ⊂ provider ⊂ account) because walls bind at group scope as
//!   often as at member scope.
//! - **The state machine.** Per member: `serving → demoted →
//!   walled(scope, until, evidence) → probing → serving`, driven by
//!   classified outcomes and by probes.
//! - **Scope-keyed walls.** A wall is recorded at the narrowest scope the
//!   evidence supports; it never widens past the scope its evidence
//!   supports. A sibling's success during a wide wall narrows it: the
//!   group re-opens, the failing members stay walled.
//! - **Attempt semantics.** `serve-failure` (the member never answered —
//!   the walk advances) vs `served-failure` (the member answered; the
//!   classifier decides whether the walk advances, demotes, or walls).
//! - **Strategies.** Ordered, round-robin, least-loaded, weighted,
//!   and free-first, composable through `Tiered`; unknown load is a
//!   first-class value that never sorts as zero (#13611).
//! - **Recovery.** A wall expires at its recorded reset, or — when no
//!   reset was recorded — at a bounded inferred expiry; either way the
//!   member re-enters through `probing`, and a successful probe
//!   re-seats it at demoted weight before full traffic.
//! - **The decision log.** Every selection, state transition, and
//!   wall placement, narrowing, and removal records (member, state,
//!   strategy, reason). A pool decision you cannot read back is a bug.
//!
//! What stays domain-specific (design §3.3): member identity and
//! grouping, measurement (quota/load/health), classification of raw
//! failures, and policy (weights, cost classes, reserve rules). Those
//! plug in through the [`Member`], [`Classifier`], [`Strategy`], and
//! [`Probe`] traits.
//!
//! Time is a logical `u64` of seconds supplied by the caller (`now`),
//! so the state machine is deterministic and testable without a clock.
//!
//! Slice 1 (#14721) landed the scaffold, the four traits, the state
//! machine, scope-keyed walls, the ordered / round-robin /
//! least-loaded strategies, the selection log, and the first two
//! named-oracle property tests. Slice 2 (#15066) completes the
//! acceptance: the sibling-success and served-failure-advance
//! oracles, the weighted and free-first strategies with `Tiered`
//! composition, decision-log completeness (transitions and wall
//! events are entries too, not only selections), and the #13717
//! route-form walker's acceptance cases bound as oracle tests
//! (tests/walker.rs).

use std::collections::{BTreeMap, BTreeSet};

/// Logical seconds, supplied by the caller.
pub type Seconds = u64;

/// Default lifetime of a wall that carries no recorded reset time.
/// A reset-less wall must still gate (design §3.2, #13693 RC-1/RC-2):
/// it expires at this bound and re-admits through a probe instead of
/// silently never walling.
pub const DEFAULT_INFERRED_WALL_TTL: Seconds = 300;

/// Opaque node identifier. Members and groups share one namespace;
/// a wall's scope is a `NodeId` naming either.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A pool member. The primitive never looks inside a member; the
/// domain declares identity (and grouping, by where it adds the
/// member in the tree).
pub trait Member {
    fn id(&self) -> &str;
}

impl Member for NodeId {
    fn id(&self) -> &str {
        self.as_str()
    }
}

impl Member for String {
    fn id(&self) -> &str {
        self
    }
}

impl Member for &str {
    fn id(&self) -> &str {
        self
    }
}

/// Effective per-member state, as selection sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberState {
    Serving,
    /// Still serving, at reduced standing: one classified failure
    /// away from a wall.
    Demoted,
    /// Gated by a wall at this member's scope or an ancestor's.
    Walled,
    /// Wall expired; awaiting (or undergoing) a re-admission probe.
    Probing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BaseState {
    Serving,
    Demoted,
    Probing,
}

/// Whether an attempt ever reached the member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptKind {
    /// Transport/auth/connect/timeout: the member never answered.
    /// The walk always advances.
    ServeFailure,
    /// The member answered and the answer was a failure. Whether the
    /// walk advances is the classifier's decision, never a default.
    ServedFailure,
}

/// What a classified served-failure does to the member's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServedDisposition {
    /// No state change (e.g. a domain-level rejection that says
    /// nothing about the member's health).
    Hold,
    /// One demotion step.
    Demote,
    /// Wall at the evidence scope.
    Wall,
}

/// A raw outcome, classified by the domain (§3.3) into the only form
/// the primitive consumes.
#[derive(Clone, Debug)]
pub struct Classification {
    pub kind: AttemptKind,
    /// Consulted only for [`AttemptKind::ServedFailure`].
    pub served_disposition: ServedDisposition,
    /// Whether the walk advances past this member after this outcome.
    /// For a serve-failure this is always `true` regardless of the
    /// value supplied here; for a served-failure it is exactly what
    /// the classifier decided (oracle: *a served failure never
    /// advances the walk unless its classifier says so*).
    pub advance: bool,
    /// The widest scope this evidence supports: the member itself or
    /// one of its ancestors. The primitive walls at exactly this
    /// scope and never wider; an evidence scope outside the member's
    /// ancestry is clamped to the member.
    pub evidence_scope: NodeId,
    /// Recorded reset time, when the domain knows one.
    pub reset_hint: Option<Seconds>,
    /// Human-readable evidence, kept on the wall for read-back.
    pub evidence: String,
}

/// Turns a domain outcome into a [`Classification`]. Classification —
/// quota vs transport vs auth, and the scope the evidence supports —
/// is knowledge only the domain has (§3.3).
pub trait Classifier<O> {
    fn classify(&mut self, member: &NodeId, outcome: &O) -> Classification;
}

/// A bounded, cheap re-admission check. The primitive schedules
/// probes; it does not define them (§3.3).
pub trait Probe {
    /// `true` re-seats the member at demoted weight.
    fn probe(&mut self, member: &NodeId) -> bool;
}

impl<F: FnMut(&NodeId) -> bool> Probe for F {
    fn probe(&mut self, member: &NodeId) -> bool {
        self(member)
    }
}

/// A wall, recorded at exactly the scope its evidence supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wall {
    pub scope: NodeId,
    /// Recorded reset time, when known.
    pub until: Option<Seconds>,
    pub evidence: String,
    /// Members whose failures support this wall. A sibling success
    /// narrows the wall to exactly these members.
    pub supporters: BTreeSet<NodeId>,
    placed_at: Seconds,
}

impl Wall {
    /// The wall gates strictly before this time. A wall with no
    /// recorded reset gates for the pool's inferred-wall TTL from
    /// placement — never forever, never not at all.
    pub fn effective_until(&self, inferred_ttl: Seconds) -> Seconds {
        self.until
            .unwrap_or_else(|| self.placed_at.saturating_add(inferred_ttl))
    }

    pub fn is_active(&self, now: Seconds, inferred_ttl: Seconds) -> bool {
        now < self.effective_until(inferred_ttl)
    }
}

/// What a decision-log entry records. Slice 1 wrote selections
/// only; slice 2 (#15066) completes the log: every state transition
/// and every wall placement, narrowing, and removal is an entry
/// too, so a full outcome sequence can be reconstructed from the
/// log alone (tests/log_completeness.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionKind {
    /// A strategy picked (or failed to pick) a member.
    Selection,
    /// A member's base state changed: demotion, restore, probe
    /// scheduling, or probe re-admission.
    Transition,
    /// A wall was placed or reinforced at the entry's scope.
    WallPlaced,
    /// A group wall narrowed: it was replaced by member walls on
    /// its remaining supporters, each recorded as its own
    /// `WallPlaced` entry.
    WallNarrowed,
    /// A wall was removed: it expired, or its last supporter
    /// succeeded.
    WallRemoved,
}

/// One decision-log entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub at: Seconds,
    pub member: Option<NodeId>,
    pub state: Option<MemberState>,
    pub strategy: String,
    pub reason: String,
    pub kind: DecisionKind,
}

/// A candidate as a [`Strategy`] sees it.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: NodeId,
    pub state: MemberState,
    /// Current load, when measured. `None` is *unknown* — a
    /// first-class value that never sorts as zero (#13611).
    pub load: Option<f64>,
    /// Declared cost class (free-first ordering input; policy, §3.3).
    pub cost_class: u32,
}

/// Selection policy. The crate ships [`Ordered`], [`RoundRobin`],
/// [`LeastLoaded`], [`Weighted`], and [`FreeFirst`]; [`Tiered`]
/// composes any of them under free-first tiering (design §3.2).
pub trait Strategy {
    fn name(&self) -> &str;
    /// Index into `candidates`, or `None` to select nothing.
    fn select(&mut self, candidates: &[Candidate]) -> Option<usize>;
}

/// Declaration order: the first available candidate wins.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ordered;

impl Strategy for Ordered {
    fn name(&self) -> &str {
        "ordered"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        if candidates.is_empty() { None } else { Some(0) }
    }
}

/// Rotate through the candidates in order.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoundRobin {
    cursor: usize,
}

impl Strategy for RoundRobin {
    fn name(&self) -> &str {
        "round-robin"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        if candidates.is_empty() {
            return None;
        }
        let idx = self.cursor % candidates.len();
        self.cursor = self.cursor.wrapping_add(1);
        Some(idx)
    }
}

/// Least-loaded first. Unknown load sorts after every known load —
/// never as zero — so a member whose load could not be read can
/// never win a selection against a member with a known non-zero
/// load (named oracle, #13611).
#[derive(Clone, Copy, Debug, Default)]
pub struct LeastLoaded;

impl Strategy for LeastLoaded {
    fn name(&self) -> &str {
        "least-loaded"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        candidates
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| load_key(a.load).partial_cmp(&load_key(b.load)).unwrap())
            .map(|(idx, _)| idx)
    }
}

/// Sort key for least-loaded: known loads ascending, unknown (and
/// NaN, which is a failed read wearing a number) last.
fn load_key(load: Option<f64>) -> f64 {
    match load {
        Some(v) if v.is_finite() => v,
        _ => f64::INFINITY,
    }
}

/// Weighted selection: smooth weighted round-robin over the
/// candidates. Weights are policy (§3.3), supplied by the domain at
/// construction and keyed by member id; a candidate with no
/// declared weight weighs 1, and a weight of 0 parks the member —
/// it is picked only when every candidate weighs 0, where the
/// strategy degrades to declaration order. Over any window of
/// `sum(weights)` consecutive selections against a constant
/// candidate set, each member is picked exactly `weight` times:
/// the shares are exact, not statistical.
#[derive(Clone, Debug, Default)]
pub struct Weighted {
    weights: BTreeMap<NodeId, u32>,
    current: BTreeMap<NodeId, i64>,
}

impl Weighted {
    pub fn new(weights: impl IntoIterator<Item = (NodeId, u32)>) -> Self {
        Self {
            weights: weights.into_iter().collect(),
            current: BTreeMap::new(),
        }
    }

    pub fn weight_of(&self, id: &NodeId) -> u32 {
        self.weights.get(id).copied().unwrap_or(1)
    }
}

impl Strategy for Weighted {
    fn name(&self) -> &str {
        "weighted"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        if candidates.is_empty() {
            return None;
        }
        let total: i64 = candidates
            .iter()
            .map(|c| i64::from(self.weight_of(&c.id)))
            .sum();
        if total <= 0 {
            return Some(0);
        }
        let mut best: Option<usize> = None;
        let mut best_score: i64 = i64::MIN;
        for (idx, candidate) in candidates.iter().enumerate() {
            let weight = i64::from(self.weight_of(&candidate.id));
            if weight == 0 {
                continue;
            }
            let score = {
                let entry = self.current.entry(candidate.id.clone()).or_insert(0);
                *entry += weight;
                *entry
            };
            if score > best_score {
                best_score = score;
                best = Some(idx);
            }
        }
        let picked = best.expect("a positive total implies a weighted candidate");
        *self
            .current
            .get_mut(&candidates[picked].id)
            .expect("the picked candidate was just scored") -= total;
        Some(picked)
    }
}

/// Free-first: the lowest declared cost class wins; within the
/// class, declaration order (design §3.2). Cost class is policy
/// (§3.3), declared per member.
#[derive(Clone, Copy, Debug, Default)]
pub struct FreeFirst;

impl Strategy for FreeFirst {
    fn name(&self) -> &str {
        "free-first"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        candidates
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| c.cost_class)
            .map(|(idx, _)| idx)
    }
}

/// Composition: restrict the field to the lowest cost class
/// present, then let the wrapped strategy choose within it. This
/// is the design's composition shape (§3.2): *free-first then
/// weighted within class* is the model-pool shape
/// (`Tiered::new(Weighted::new(..))`); *ordered then round-robin
/// within tier* is the host shape.
#[derive(Clone, Debug)]
pub struct Tiered<S> {
    inner: S,
    name: String,
}

impl<S: Strategy> Tiered<S> {
    pub fn new(inner: S) -> Self {
        Self {
            name: format!("tiered({})", inner.name()),
            inner,
        }
    }
}

impl<S: Strategy> Strategy for Tiered<S> {
    fn name(&self) -> &str {
        &self.name
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        let best_class = candidates.iter().map(|c| c.cost_class).min()?;
        let tier: Vec<Candidate> = candidates
            .iter()
            .filter(|c| c.cost_class == best_class)
            .cloned()
            .collect();
        let within = self.inner.select(&tier)?;
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.cost_class == best_class)
            .nth(within)
            .map(|(idx, _)| idx)
    }
}

/// The effect of recording an outcome, as the caller (a walker)
/// needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeEffect {
    /// Whether the walk advances past this member.
    pub advance: bool,
    /// The member's effective state after the outcome.
    pub state: MemberState,
    /// The scope walled by this outcome, if any.
    pub walled_scope: Option<NodeId>,
}

struct Node {
    parent: Option<NodeId>,
    group: bool,
    base: BaseState,
    cost_class: u32,
}

/// The pool. Owns the tree of members and groups, the walls, and
/// the decision log.
pub struct Pool {
    nodes: BTreeMap<NodeId, Node>,
    walls: BTreeMap<NodeId, Wall>,
    log: Vec<Decision>,
    inferred_wall_ttl: Seconds,
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl Pool {
    pub fn new() -> Self {
        Self::with_inferred_wall_ttl(DEFAULT_INFERRED_WALL_TTL)
    }

    pub fn with_inferred_wall_ttl(inferred_wall_ttl: Seconds) -> Self {
        Self {
            nodes: BTreeMap::new(),
            walls: BTreeMap::new(),
            log: Vec::new(),
            inferred_wall_ttl,
        }
    }

    pub fn add_group(&mut self, id: impl Into<NodeId>, parent: Option<&NodeId>) {
        self.nodes.insert(
            id.into(),
            Node {
                parent: parent.cloned(),
                group: true,
                base: BaseState::Serving,
                cost_class: 0,
            },
        );
    }

    pub fn add_member(&mut self, id: impl Into<NodeId>, parent: Option<&NodeId>) {
        self.add_member_with_cost(id, parent, 0);
    }

    pub fn add_member_with_cost(
        &mut self,
        id: impl Into<NodeId>,
        parent: Option<&NodeId>,
        cost_class: u32,
    ) {
        self.nodes.insert(
            id.into(),
            Node {
                parent: parent.cloned(),
                group: false,
                base: BaseState::Serving,
                cost_class,
            },
        );
    }

    /// Ancestors of `id`, nearest first, ending at the root.
    fn ancestors(&self, id: &NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(id).and_then(|n| n.parent.clone());
        while let Some(node) = cur {
            cur = self.nodes.get(&node).and_then(|n| n.parent.clone());
            out.push(node);
        }
        out
    }

    /// The active wall covering `id` (its own scope or an
    /// ancestor's), if any.
    fn covering_wall(&self, id: &NodeId, now: Seconds) -> Option<&Wall> {
        for scope in std::iter::once(id.clone()).chain(self.ancestors(id)) {
            if let Some(wall) = self.walls.get(&scope) {
                if wall.is_active(now, self.inferred_wall_ttl) {
                    return Some(wall);
                }
            }
        }
        None
    }

    /// The member's effective state at `now`.
    pub fn state_of(&self, id: &NodeId, now: Seconds) -> MemberState {
        if self.covering_wall(id, now).is_some() {
            return MemberState::Walled;
        }
        match self.nodes.get(id).map(|n| n.base) {
            Some(BaseState::Serving) => MemberState::Serving,
            Some(BaseState::Demoted) => MemberState::Demoted,
            Some(BaseState::Probing) => MemberState::Probing,
            None => MemberState::Serving,
        }
    }

    /// All walls currently recorded (expired ones are removed by
    /// [`Pool::tick`]; an expired wall never gates regardless).
    pub fn walls(&self) -> impl Iterator<Item = &Wall> {
        self.walls.values()
    }

    pub fn decisions(&self) -> &[Decision] {
        &self.log
    }

    /// Place (or reinforce) a wall at exactly `scope` — never wider.
    /// `scope` must already have been validated against the failing
    /// member's ancestry by the caller.
    fn place_wall(&mut self, scope: &NodeId, member: &NodeId, cls: &Classification, now: Seconds) {
        let wall = self.walls.entry(scope.clone()).or_insert_with(|| Wall {
            scope: scope.clone(),
            until: cls.reset_hint,
            evidence: cls.evidence.clone(),
            supporters: BTreeSet::new(),
            placed_at: now,
        });
        wall.supporters.insert(member.clone());
        if cls.reset_hint.is_some() {
            wall.until = cls.reset_hint;
        }
        wall.evidence = cls.evidence.clone();
    }

    /// Append a non-selection entry to the decision log.
    fn log_event(
        &mut self,
        at: Seconds,
        kind: DecisionKind,
        member: Option<NodeId>,
        state: Option<MemberState>,
        reason: String,
    ) {
        self.log.push(Decision {
            at,
            member,
            state,
            strategy: "state-machine".to_owned(),
            reason,
            kind,
        });
    }

    /// Record a wall placement (or reinforcement) in the log.
    fn log_wall_placed(&mut self, scope: &NodeId, cls: &Classification, now: Seconds) {
        let state = self.state_of(scope, now);
        self.log_event(
            now,
            DecisionKind::WallPlaced,
            Some(scope.clone()),
            Some(state),
            format!("wall placed at {scope}: {}", cls.evidence),
        );
    }

    /// Record a classified outcome for `member` at `now`.
    pub fn record_outcome(
        &mut self,
        member: &NodeId,
        cls: &Classification,
        now: Seconds,
    ) -> OutcomeEffect {
        // The evidence scope caps the wall: the member itself or one
        // of its ancestors. Anything else is clamped to the member —
        // a wall never widens past the scope its evidence supports.
        let scope = if cls.evidence_scope == *member
            || self.ancestors(member).contains(&cls.evidence_scope)
        {
            cls.evidence_scope.clone()
        } else {
            member.clone()
        };

        let mut walled_scope = None;
        let advance = match cls.kind {
            AttemptKind::ServeFailure => {
                // One demotion step; a second strike walls.
                match self.nodes.get(member).map(|n| n.base) {
                    Some(BaseState::Serving) => {
                        self.set_base(member, BaseState::Demoted);
                        self.log_event(
                            now,
                            DecisionKind::Transition,
                            Some(member.clone()),
                            Some(MemberState::Demoted),
                            format!("serve-failure demoted {member}"),
                        );
                    }
                    _ => {
                        self.place_wall(&scope, member, cls, now);
                        walled_scope = Some(scope.clone());
                        self.log_wall_placed(&scope, cls, now);
                    }
                }
                true
            }
            AttemptKind::ServedFailure => {
                match cls.served_disposition {
                    ServedDisposition::Hold => {}
                    ServedDisposition::Demote => {
                        if matches!(
                            self.nodes.get(member).map(|n| n.base),
                            Some(BaseState::Serving)
                        ) {
                            self.set_base(member, BaseState::Demoted);
                            self.log_event(
                                now,
                                DecisionKind::Transition,
                                Some(member.clone()),
                                Some(MemberState::Demoted),
                                format!("served-failure demoted {member}"),
                            );
                        }
                    }
                    ServedDisposition::Wall => {
                        self.place_wall(&scope, member, cls, now);
                        walled_scope = Some(scope.clone());
                        self.log_wall_placed(&scope, cls, now);
                    }
                }
                cls.advance
            }
        };

        OutcomeEffect {
            advance,
            state: self.state_of(member, now),
            walled_scope,
        }
    }

    /// Record a success for `member` at `now`: a demoted member
    /// returns to serving, and a wide wall this member does not
    /// support narrows around its remaining supporters.
    pub fn record_success(&mut self, member: &NodeId, now: Seconds) {
        if matches!(
            self.nodes.get(member).map(|n| n.base),
            Some(BaseState::Demoted)
        ) {
            self.set_base(member, BaseState::Serving);
            self.log_event(
                now,
                DecisionKind::Transition,
                Some(member.clone()),
                Some(MemberState::Serving),
                format!("success restored {member} to serving"),
            );
        }

        // Narrowing: a success under an active group wall
        // re-opens the group for everyone but the members whose
        // failures still support the wall — the wall narrows to
        // member walls on exactly those members, whether or not
        // the succeeding member was itself a supporter (oracle: *a
        // sibling success never leaves a group wall wider than the
        // failing member*). If no supporter remains, the wall is
        // gone entirely.
        let ancestors = self.ancestors(member);
        for scope in ancestors {
            let Some(wall) = self.walls.get(&scope).cloned() else {
                continue;
            };
            if !wall.is_active(now, self.inferred_wall_ttl) {
                continue;
            }
            let mut remaining = wall.supporters.clone();
            remaining.remove(member);
            self.walls.remove(&scope);
            if remaining.is_empty() {
                self.log_event(
                    now,
                    DecisionKind::WallRemoved,
                    Some(scope.clone()),
                    None,
                    format!("wall at {scope} removed: its last supporter succeeded"),
                );
            } else {
                self.log_event(
                    now,
                    DecisionKind::WallNarrowed,
                    Some(scope.clone()),
                    None,
                    format!(
                        "success by {member} narrowed the wall at {scope} to its failing members"
                    ),
                );
                for supporter in &remaining {
                    let narrowed = Wall {
                        scope: supporter.clone(),
                        until: wall.until,
                        evidence: wall.evidence.clone(),
                        supporters: BTreeSet::from([supporter.clone()]),
                        placed_at: wall.placed_at,
                    };
                    self.walls.insert(supporter.clone(), narrowed);
                    self.log_event(
                        now,
                        DecisionKind::WallPlaced,
                        Some(supporter.clone()),
                        Some(MemberState::Walled),
                        format!("wall narrowed from {scope} to {supporter}: {}", wall.evidence),
                    );
                }
            }
        }
    }

    /// Advance time: expire walls whose time has come and run
    /// re-admission probes for members left in `probing`.
    pub fn tick(&mut self, now: Seconds, probe: &mut impl Probe) {
        // Expire walls; members they covered (and no other active
        // wall covers) enter probing.
        let expired: Vec<NodeId> = self
            .walls
            .values()
            .filter(|w| !w.is_active(now, self.inferred_wall_ttl))
            .map(|w| w.scope.clone())
            .collect();
        let mut released: Vec<NodeId> = Vec::new();
        for scope in &expired {
            if let Some(wall) = self.walls.remove(scope) {
                released.extend(wall.supporters.iter().cloned());
                // A group wall also covered members who never
                // failed; they simply return to availability.
                self.log_event(
                    now,
                    DecisionKind::WallRemoved,
                    Some(scope.clone()),
                    None,
                    format!("wall at {scope} expired"),
                );
            }
        }
        for member in released {
            if self.covering_wall(&member, now).is_none()
                && !matches!(
                    self.nodes.get(&member).map(|n| n.base),
                    Some(BaseState::Probing)
                )
            {
                self.set_base(&member, BaseState::Probing);
                self.log_event(
                    now,
                    DecisionKind::Transition,
                    Some(member.clone()),
                    Some(MemberState::Probing),
                    format!("wall expired; {member} awaits a re-admission probe"),
                );
            }
        }

        // Probe every probing member no wall covers.
        let probing: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|(id, n)| {
                !n.group
                    && matches!(n.base, BaseState::Probing)
                    && self.covering_wall(id, now).is_none()
            })
            .map(|(id, _)| id.clone())
            .collect();
        for member in probing {
            if probe.probe(&member) {
                // Re-seated at demoted weight before full traffic.
                self.set_base(&member, BaseState::Demoted);
                self.log_event(
                    now,
                    DecisionKind::Transition,
                    Some(member.clone()),
                    Some(MemberState::Demoted),
                    format!("re-admission probe passed; {member} re-seated at demoted weight"),
                );
            } else {
                let cls = Classification {
                    kind: AttemptKind::ServeFailure,
                    served_disposition: ServedDisposition::Hold,
                    advance: true,
                    evidence_scope: member.clone(),
                    reset_hint: None,
                    evidence: "re-admission probe failed".to_owned(),
                };
                self.place_wall(&member, &member, &cls, now);
                self.log_wall_placed(&member, &cls, now);
            }
        }
    }

    /// Select the next member with `strategy`. `load` supplies the
    /// current load read per member (`None` = unknown). Every
    /// selection is written to the decision log.
    pub fn select(
        &mut self,
        strategy: &mut impl Strategy,
        load: &dyn Fn(&NodeId) -> Option<f64>,
        now: Seconds,
    ) -> Option<NodeId> {
        let candidates: Vec<Candidate> = self
            .nodes
            .iter()
            .filter(|(_, n)| !n.group)
            .filter_map(|(id, n)| {
                let state = self.state_of(id, now);
                match state {
                    MemberState::Serving | MemberState::Demoted => Some(Candidate {
                        id: id.clone(),
                        state,
                        load: load(id),
                        cost_class: n.cost_class,
                    }),
                    _ => None,
                }
            })
            .collect();

        let picked = strategy
            .select(&candidates)
            .and_then(|idx| candidates.get(idx));
        let (member, state, reason) = match picked {
            Some(c) => (
                Some(c.id.clone()),
                Some(c.state),
                format!(
                    "{} selected from {} candidate(s)",
                    strategy.name(),
                    candidates.len()
                ),
            ),
            None => (
                None,
                None,
                format!("{} found no available candidate", strategy.name()),
            ),
        };
        self.log.push(Decision {
            at: now,
            member: member.clone(),
            state,
            strategy: strategy.name().to_owned(),
            reason,
            kind: DecisionKind::Selection,
        });
        member
    }

    fn set_base(&mut self, id: &NodeId, base: BaseState) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.base = base;
        }
    }
}
