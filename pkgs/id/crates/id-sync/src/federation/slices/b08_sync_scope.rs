//! B08 — implicit-trust sync scope and tolerance (slice B08, #10209;
//! statements S1.26–S1.31 of program #9106 part B).
//!
//! Design basis: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-b/versioned-design.md`, "Component dispositions and statement
//! coverage" (item 2, #9142) and "Retention, deletion, quarantine and
//! privacy boundary".
//!
//! The contract, in one paragraph: internal peers carry implicit trust
//! — a posture distinct from external federation peers — and sync
//! toward **1:1 sameness** of the records they are both authorized to
//! hold. Sameness is a convergence *target* qualified "within reason",
//! not an absolute invariant: it stops at the authorized scope, the
//! tenant/trust-domain boundary, the configured retention window and
//! the node's storage capacity. Divergence inside those bounds is
//! detected and reported before it is resolved; divergence outside
//! them is legitimate and is never "fixed" by copying data across a
//! boundary. A deletion is a signed scoped event: the tombstone
//! propagates, the deleted content never does, and `deleted_by_policy`
//! is reported separately from `missing` and `not_authorized`.
//! Convergence is a process over time, so temporary divergence is
//! normal and no local absence is ever read as global absence.
//!
//! Shape of the module: like the peer-class matrix (slice B31), this is
//! a pure policy core. It performs no I/O, no cryptography and no
//! resolution of conflicts; it reads two snapshots (what the local node
//! holds, what the remote holds) plus the link's scope configuration
//! and peer policy, and returns an [`ExchangePlan`]: what may cross in
//! each direction, what the local node must retire locally, and the
//! divergence that cannot or must not be transferred. Applying the plan
//! is the caller's job, which keeps the module testable against an
//! independent reference state machine.
//!
//! Scope of the guarantees, stated plainly so the test oracle can be
//! exact: capacity bounds what a node *adds* over the link; it never
//! deletes data the node already retained (eviction is a storage-tier
//! decision outside this slice). Retention and scope/tenant/authorization
//! checks are read at one instant `now`, supplied by the caller, so the
//! state machine is deterministic. A record id present on both sides
//! with different content digests is [`DivergenceKind::ContentMismatch`]:
//! detected, reported, never auto-resolved here (the durable conflict
//! report is slice B07's; the CRDT merge is later). Both peers are
//! assumed to be configured for the same shared scope and trust domain —
//! that is what "the same link" means — so one [`SyncScopeConfig`] and
//! one peer policy snapshot are evaluated for both directions.
//!
//! This module is deliberately not exported from the federation root:
//! slice 9110-02 (#10160) recorded the module path in `B_SLICE_REGISTRY`
//! without exporting it, and slice 9110-09 wires the versioned public
//! exports once module outputs land. The dedicated integration test
//! includes this file by path.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::b31_peer_classes::{
    Decision, DenyReason, Direction, GateEvidence, Operation, PeerPolicy, decide,
};

/// Capacity used when a [`SyncScopeConfig`] does not set one.
pub const DEFAULT_CAPACITY: usize = 1024;

/// The authorized convergence bounds of one shared scope: which scope
/// and trust domain this node syncs, how many records it will hold in
/// that scope, and how long a record stays eligible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncScopeConfig {
    /// The only scope whose records may cross this link.
    pub scope: String,
    /// The only tenant / trust domain whose records may cross this
    /// link. Sameness never crosses this boundary (S1.30).
    pub trust_domain: String,
    /// Maximum number of live records this node holds in the scope.
    /// Capacity bounds what the node adds; it never evicts retained
    /// records (S1.29, "within reason").
    pub capacity: usize,
    /// Retention window in logical seconds, measured from a record's
    /// `admitted_at`. `None` means records are retained until their own
    /// `expires_at` (which may also be `None`). A record is retained
    /// while `now - admitted_at < window`; at exactly `window` it has
    /// expired (S1.29).
    pub retention: Option<u64>,
}

impl SyncScopeConfig {
    /// A configuration for `scope` in `trust_domain`, with
    /// [`DEFAULT_CAPACITY`] and no retention window.
    pub fn new(scope: impl Into<String>, trust_domain: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            trust_domain: trust_domain.into(),
            capacity: DEFAULT_CAPACITY,
            retention: None,
        }
    }

    /// Set the record capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Set a retention window in seconds.
    pub fn with_retention(mut self, seconds: u64) -> Self {
        self.retention = Some(seconds);
        self
    }

    /// Whether `record` has passed the retention bound at `now`: either
    /// its own `expires_at` has been reached, or it is older than the
    /// configured window.
    pub fn expired(&self, record: &ScopedRecord, now: u64) -> bool {
        if record.expires_at.is_some_and(|at| now >= at) {
            return true;
        }
        self.retention
            .is_some_and(|window| now.saturating_sub(record.admitted_at) >= window)
    }
}

/// One record, as seen by one node, with the metadata the scope policy
/// needs. A record is either live (it has content, identified by
/// `digest`) or a tombstone (`digest` is `None`: the deletion is known,
/// the content is physically gone — S1.31's `deleted_by_policy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedRecord {
    /// Stable identity of the record across the scope.
    pub record_id: String,
    /// The scope the record belongs to.
    pub scope: String,
    /// The tenant / trust domain the record belongs to.
    pub trust_domain: String,
    /// Whether the record is authorized for this link's scope. An
    /// unauthorized record never crosses, whatever the peer class says.
    pub authorized: bool,
    /// Logical second at which the record was admitted locally; the
    /// retention window is measured from here.
    pub admitted_at: u64,
    /// The record's own expiry, if it carries one.
    pub expires_at: Option<u64>,
    /// Content digest while the record is live; `None` once it is a
    /// tombstone.
    pub digest: Option<String>,
}

impl ScopedRecord {
    /// A live record carrying `digest`.
    pub fn live(
        record_id: impl Into<String>,
        scope: impl Into<String>,
        trust_domain: impl Into<String>,
        digest: impl Into<String>,
        admitted_at: u64,
    ) -> Self {
        Self {
            record_id: record_id.into(),
            scope: scope.into(),
            trust_domain: trust_domain.into(),
            authorized: true,
            admitted_at,
            expires_at: None,
            digest: Some(digest.into()),
        }
    }

    /// A tombstone: the scoped deletion event for `record_id`, with no
    /// content.
    pub fn tombstone(
        record_id: impl Into<String>,
        scope: impl Into<String>,
        trust_domain: impl Into<String>,
        admitted_at: u64,
    ) -> Self {
        Self {
            record_id: record_id.into(),
            scope: scope.into(),
            trust_domain: trust_domain.into(),
            authorized: true,
            admitted_at,
            expires_at: None,
            digest: None,
        }
    }

    /// Whether this entry is a tombstone (content physically gone).
    pub fn is_tombstone(&self) -> bool {
        self.digest.is_none()
    }
}

/// What kind of object the link would carry for a record: the content
/// itself, or the signed deletion event. Content of a tombstoned record
/// never crosses; the tombstone may (S1.31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    /// The record's content.
    Content,
    /// The record's signed deletion event.
    Tombstone,
}

/// Why a record is outside the authorized convergence bounds. These
/// exclusions are legitimate divergence: they are reported and never
/// transferred across the boundary they violate (S1.29, S1.30, S1.31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Exclusion {
    /// The record belongs to another scope.
    OutsideScope,
    /// The record belongs to another tenant / trust domain.
    TenantBoundary,
    /// The record is not authorized for this link's scope.
    NotAuthorized,
    /// The content is deleted by policy; only the tombstone may cross.
    DeletedByPolicy,
    /// The record is past its retention bound at `now`.
    ExpiredRetention,
}

impl Exclusion {
    /// The divergence kind this exclusion is reported as.
    pub const fn divergence_kind(self) -> DivergenceKind {
        match self {
            Self::OutsideScope => DivergenceKind::OutsideScope,
            Self::TenantBoundary => DivergenceKind::TenantBoundary,
            Self::NotAuthorized => DivergenceKind::NotAuthorized,
            Self::DeletedByPolicy => DivergenceKind::DeletedByPolicy,
            Self::ExpiredRetention => DivergenceKind::ExpiredRetention,
        }
    }
}

impl fmt::Display for Exclusion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::OutsideScope => "outside the authorized scope",
            Self::TenantBoundary => "across a tenant/trust-domain boundary",
            Self::NotAuthorized => "not authorized for this link",
            Self::DeletedByPolicy => "deleted by policy",
            Self::ExpiredRetention => "past its retention bound",
        };
        f.write_str(text)
    }
}

/// Why a record may not cross: either it is outside the authorized
/// bounds, or the peer class / gate refuses the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Withhold {
    /// Outside the authorized convergence bounds.
    Excluded(Exclusion),
    /// Refused by the B31 peer-class matrix (which never waives the
    /// universal signed-and-validated gate).
    PeerDenied(DenyReason),
}

/// The link's verdict for one (record, object kind, direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Crossing {
    /// The object may cross.
    Cross,
    /// The object must not cross.
    Withhold(Withhold),
}

/// The authorized-bounds check for one record and object kind at `now`.
///
/// The order is fixed and part of the contract: scope, then trust
/// domain, then authorization, then deletion (content only), then
/// retention. The first violation names the exclusion.
pub fn exclusion(
    record: &ScopedRecord,
    object: ObjectKind,
    config: &SyncScopeConfig,
    now: u64,
) -> Option<Exclusion> {
    if record.scope != config.scope {
        return Some(Exclusion::OutsideScope);
    }
    if record.trust_domain != config.trust_domain {
        return Some(Exclusion::TenantBoundary);
    }
    if !record.authorized {
        return Some(Exclusion::NotAuthorized);
    }
    if object == ObjectKind::Content && record.is_tombstone() {
        return Some(Exclusion::DeletedByPolicy);
    }
    if config.expired(record, now) {
        return Some(Exclusion::ExpiredRetention);
    }
    None
}

/// Decide whether one record's object may cross the link: first the
/// authorized bounds, then the B31 peer-class matrix for the direction
/// (outbound send / inbound accept), which still requires the full gate
/// evidence for acceptance.
pub fn crossing(
    record: &ScopedRecord,
    object: ObjectKind,
    config: &SyncScopeConfig,
    policy: Option<&PeerPolicy>,
    direction: Direction,
    gate: &GateEvidence,
    now: u64,
) -> Crossing {
    if let Some(excluded) = exclusion(record, object, config, now) {
        return Crossing::Withhold(Withhold::Excluded(excluded));
    }
    let operation = match direction {
        Direction::Outbound => Operation::SendData,
        Direction::Inbound => Operation::AcceptData,
    };
    match decide(policy, operation, direction, gate, now) {
        Decision::Allow => Crossing::Cross,
        Decision::Deny(reason) => Crossing::Withhold(Withhold::PeerDenied(reason)),
    }
}

/// The live records this node may hold in the scope, oldest admission
/// first, and the ids held back because the capacity is full.
///
/// Only live, in-bound, authorized, unexpired records are ordered; a
/// tombstone carries no content and does not consume capacity. Ties
/// break on `record_id`, so the selection is total and deterministic.
pub fn within_capacity<'a>(
    records: &'a [ScopedRecord],
    config: &SyncScopeConfig,
    now: u64,
) -> (Vec<&'a ScopedRecord>, Vec<String>) {
    let mut live: Vec<&ScopedRecord> = unique_by_id(records)
        .into_iter()
        .filter(|r| !r.is_tombstone() && exclusion(r, ObjectKind::Content, config, now).is_none())
        .collect();
    live.sort_by(|a, b| {
        a.admitted_at
            .cmp(&b.admitted_at)
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    let mut held_back = Vec::new();
    let kept = if live.len() <= config.capacity {
        live
    } else {
        held_back = live
            .iter()
            .skip(config.capacity)
            .map(|r| r.record_id.clone())
            .collect();
        live.truncate(config.capacity);
        live
    };
    (kept, held_back)
}

/// One detected difference that is not (yet) resolved by a transfer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Divergence {
    /// The record the difference is about.
    pub record_id: String,
    /// What kind of difference it is.
    pub kind: DivergenceKind,
}

/// What kind of divergence was detected. The first group is *resolvable
/// by transfer* (a pending push/pull/retire accompanies it); the second
/// is *tolerated* — it lies outside the authorized bounds or the
/// capacity, so it is reported, never "fixed" by crossing a boundary
/// (S1.28–S1.31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DivergenceKind {
    /// The local node has the record and the remote does not; a push is
    /// scheduled.
    MissingRemote,
    /// The remote has the record and the local node does not; a pull
    /// is scheduled.
    MissingLocal,
    /// The two sides hold different content for the same record id.
    /// Detected and reported; resolution belongs to the conflict
    /// report (B07) and later CRDT work.
    ContentMismatch,
    /// One side holds a tombstone, the other still holds content; the
    /// tombstone propagates and the content does not.
    DeletedByPolicy,
    /// Outside the authorized scope; never crosses.
    OutsideScope,
    /// Another tenant / trust domain; never crosses.
    TenantBoundary,
    /// Not authorized for this link; never crosses.
    NotAuthorized,
    /// Past the retention bound; never crosses.
    ExpiredRetention,
    /// Admissible, but the receiving side is at capacity; held back and
    /// tolerated as "within reason".
    HeldAtCapacity,
    /// Refused by the peer-class matrix or the gate for this link.
    LinkDenied,
}

impl DivergenceKind {
    /// Whether this divergence is legitimate ("within reason"): it is
    /// excluded by the authorized bounds, the capacity, or the link
    /// policy, so it persists without blocking convergence.
    pub const fn is_tolerated(self) -> bool {
        matches!(
            self,
            Self::OutsideScope
                | Self::TenantBoundary
                | Self::NotAuthorized
                | Self::ExpiredRetention
                | Self::HeldAtCapacity
                | Self::LinkDenied
        )
    }
}

impl fmt::Display for DivergenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::MissingRemote => "missing on the remote",
            Self::MissingLocal => "missing locally",
            Self::ContentMismatch => "content mismatch",
            Self::DeletedByPolicy => "deleted by policy",
            Self::OutsideScope => "outside the authorized scope",
            Self::TenantBoundary => "across a tenant/trust-domain boundary",
            Self::NotAuthorized => "not authorized",
            Self::ExpiredRetention => "past retention",
            Self::HeldAtCapacity => "held at capacity",
            Self::LinkDenied => "denied by the link's peer policy",
        };
        f.write_str(text)
    }
}

/// The plan for one exchange with one peer: what crosses each way, what
/// the local node retires locally, and the divergence detected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExchangePlan {
    /// Record ids whose content the local node sends.
    pub push_content: Vec<String>,
    /// Record ids whose tombstone the local node sends.
    pub push_tombstones: Vec<String>,
    /// Record ids whose content the local node pulls.
    pub pull_content: Vec<String>,
    /// Record ids whose tombstone the local node pulls.
    pub pull_tombstones: Vec<String>,
    /// Record ids whose local content is retired because the remote
    /// holds a scoped tombstone for them.
    pub retire_local: Vec<String>,
    /// Divergence detected this exchange, ordered by record id.
    pub divergence: Vec<Divergence>,
}

impl ExchangePlan {
    /// Whether the plan asks for nothing at all.
    pub fn is_empty(&self) -> bool {
        self.push_content.is_empty()
            && self.push_tombstones.is_empty()
            && self.pull_content.is_empty()
            && self.pull_tombstones.is_empty()
            && self.retire_local.is_empty()
    }

    /// Whether any transfer or local retirement is scheduled.
    pub fn has_pending_transfer(&self) -> bool {
        !self.is_empty()
    }

    /// Whether the exchange reached its fixpoint: nothing left to
    /// transfer, no content mismatch, and the only divergence left is
    /// tolerated divergence (outside the bounds, at capacity, or denied
    /// by the link's peer policy).
    pub fn converged(&self) -> bool {
        !self.has_pending_transfer() && !self.divergence.iter().any(|d| !d.kind.is_tolerated())
    }
}

/// Plan one exchange between the local node's snapshot and the remote's,
/// under one shared scope configuration and one peer policy snapshot.
///
/// Deterministic by construction: ids are visited in sorted order, and
/// the returned vectors are sorted, so two nodes computing the same
/// plan agree on it. Duplicate ids inside a snapshot are a caller bug;
/// the first occurrence wins. Capacity is allocated for the receiving
/// side of each direction, oldest admission first; a record that would
/// exceed it is held back and reported as
/// [`DivergenceKind::HeldAtCapacity`].
pub fn plan_exchange(
    config: &SyncScopeConfig,
    local: &[ScopedRecord],
    remote: &[ScopedRecord],
    policy: Option<&PeerPolicy>,
    gate: &GateEvidence,
    now: u64,
) -> ExchangePlan {
    let local_index = index_by_id(local);
    let remote_index = index_by_id(remote);
    let mut plan = ExchangePlan::default();

    let mut push_content: Vec<&ScopedRecord> = Vec::new();
    let mut pull_content: Vec<&ScopedRecord> = Vec::new();

    let ids: BTreeSet<&str> = local_index
        .keys()
        .chain(remote_index.keys())
        .copied()
        .collect();

    for id in ids {
        let local_record = local_index.get(id).copied();
        let remote_record = remote_index.get(id).copied();
        match (local_record, remote_record) {
            (None, None) => {}
            (Some(l), None) => {
                if l.is_tombstone() {
                    match crossing(
                        l,
                        ObjectKind::Tombstone,
                        config,
                        policy,
                        Direction::Outbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => plan.push_tombstones.push(id.to_string()),
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                } else {
                    match crossing(
                        l,
                        ObjectKind::Content,
                        config,
                        policy,
                        Direction::Outbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => push_content.push(l),
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                }
            }
            (None, Some(r)) => {
                if r.is_tombstone() {
                    match crossing(
                        r,
                        ObjectKind::Tombstone,
                        config,
                        policy,
                        Direction::Inbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => plan.pull_tombstones.push(id.to_string()),
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                } else {
                    match crossing(
                        r,
                        ObjectKind::Content,
                        config,
                        policy,
                        Direction::Inbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => pull_content.push(r),
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                }
            }
            (Some(l), Some(r)) => {
                if l.is_tombstone() && r.is_tombstone() {
                    continue;
                }
                if l.is_tombstone() {
                    // The remote still holds content the local node has
                    // deleted: propagate the tombstone, never content.
                    match crossing(
                        l,
                        ObjectKind::Tombstone,
                        config,
                        policy,
                        Direction::Outbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => {
                            plan.push_tombstones.push(id.to_string());
                            plan.divergence.push(Divergence {
                                record_id: id.to_string(),
                                kind: DivergenceKind::DeletedByPolicy,
                            });
                        }
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                } else if r.is_tombstone() {
                    // The remote has deleted by policy: retire local
                    // content on receipt of the scoped tombstone.
                    match crossing(
                        r,
                        ObjectKind::Tombstone,
                        config,
                        policy,
                        Direction::Inbound,
                        gate,
                        now,
                    ) {
                        Crossing::Cross => {
                            plan.retire_local.push(id.to_string());
                            plan.divergence.push(Divergence {
                                record_id: id.to_string(),
                                kind: DivergenceKind::DeletedByPolicy,
                            });
                        }
                        Crossing::Withhold(w) => plan.divergence.push(Divergence {
                            record_id: id.to_string(),
                            kind: withhold_kind(w),
                        }),
                    }
                } else if l.digest == r.digest {
                    // Both sides hold the same content: converged.
                } else {
                    // Same id, different content: detected, never
                    // auto-resolved by this slice.
                    plan.divergence.push(Divergence {
                        record_id: id.to_string(),
                        kind: DivergenceKind::ContentMismatch,
                    });
                }
            }
        }
    }

    let local_remaining = config
        .capacity
        .saturating_sub(within_capacity(local, config, now).0.len());
    let remote_remaining = config
        .capacity
        .saturating_sub(within_capacity(remote, config, now).0.len());

    push_content.sort_by(capacity_order);
    for (slot, record) in push_content.into_iter().enumerate() {
        if slot < remote_remaining {
            plan.push_content.push(record.record_id.clone());
            plan.divergence.push(Divergence {
                record_id: record.record_id.clone(),
                kind: DivergenceKind::MissingRemote,
            });
        } else {
            plan.divergence.push(Divergence {
                record_id: record.record_id.clone(),
                kind: DivergenceKind::HeldAtCapacity,
            });
        }
    }

    pull_content.sort_by(capacity_order);
    for (slot, record) in pull_content.into_iter().enumerate() {
        if slot < local_remaining {
            plan.pull_content.push(record.record_id.clone());
            plan.divergence.push(Divergence {
                record_id: record.record_id.clone(),
                kind: DivergenceKind::MissingLocal,
            });
        } else {
            plan.divergence.push(Divergence {
                record_id: record.record_id.clone(),
                kind: DivergenceKind::HeldAtCapacity,
            });
        }
    }

    plan.push_content.sort();
    plan.push_tombstones.sort();
    plan.pull_content.sort();
    plan.pull_tombstones.sort();
    plan.retire_local.sort();
    plan.divergence
        .sort_by(|a, b| a.record_id.cmp(&b.record_id).then(a.kind.cmp(&b.kind)));
    plan
}

/// Oldest admission first, ties broken by id: the deterministic order
/// capacity is allocated in.
fn capacity_order(a: &&ScopedRecord, b: &&ScopedRecord) -> std::cmp::Ordering {
    a.admitted_at
        .cmp(&b.admitted_at)
        .then_with(|| a.record_id.cmp(&b.record_id))
}

/// The divergence kind a withhold is reported as. Peer refusals are
/// reported by the receiving side's link policy, not as an exclusion.
fn withhold_kind(withhold: Withhold) -> DivergenceKind {
    match withhold {
        Withhold::Excluded(exclusion) => exclusion.divergence_kind(),
        Withhold::PeerDenied(_) => DivergenceKind::LinkDenied,
    }
}

/// Index a snapshot by record id, first occurrence winning.
fn index_by_id(records: &[ScopedRecord]) -> BTreeMap<&str, &ScopedRecord> {
    let mut index = BTreeMap::new();
    for record in records {
        index.entry(record.record_id.as_str()).or_insert(record);
    }
    index
}

/// The first occurrence of each record id, in input order.
fn unique_by_id(records: &[ScopedRecord]) -> Vec<&ScopedRecord> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for record in records {
        if seen.insert(record.record_id.as_str()) {
            unique.push(record);
        }
    }
    unique
}
