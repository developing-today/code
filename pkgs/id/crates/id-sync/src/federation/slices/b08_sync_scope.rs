//! Implicit-trust sync scope and tolerance (slice B08, #10209;
//! design: part-b versioned design, component dispositions,
//! statements S1.26–S1.31).
//!
//! Internal peers sync with an implicit-trust posture toward "1:1
//! sameness within reason" (S1.27). This module is the pure
//! planner for one anti-entropy round between the local node and
//! one peer: given both sides' inventories (record id, version,
//! origin, tenant, scope tags, tombstone flag, size, update time —
//! the inventory/hash comparison of the settled default), the
//! authorized scope, the configured limits, and the directions the
//! peer-class matrix (slice B31) permits for this peer, it decides
//! exactly what crosses, what is excluded and why, and where the
//! two sides have diverged.
//!
//! The boundaries, exactly as settled:
//!
//! - **Scope (S1.26, S1.28).** A record version is eligible only
//!   inside the authorized scope: its tenant is one of the scope's
//!   tenants and every scope tag it carries is authorized. Anything
//!   else never crosses — not its data, not its tombstone.
//! - **Deletion and retention (S1.29).** Deleted data never crosses
//!   as live data; a tombstone inside its retention window
//!   propagates the deletion, and a live/deleted disagreement is
//!   divergence, never a silent resurrection. Versions older than
//!   the retention window are outside the convergence target:
//!   they do not cross in either direction.
//! - **Capacity (S1.27, S1.30).** Data movement in one round is
//!   bounded by the configured capacity; what does not fit is
//!   excluded for this round and remains for the next — convergence
//!   is toward 1:1 *within* capacity and retention, never beyond
//!   them. Planning is deterministic (record-id order), so a
//!   resumed round continues where the last stopped.
//! - **Divergence (S1.31).** When both sides hold different live
//!   versions of a record, neither crosses in this round; the
//!   planner emits a durable conflict record (both version ids,
//!   both origins, the scope it was detected under, detection
//!   time) before any resolution — resolution is a later slice.
//!   Unrelated records continue to sync in the same round.
//! - **Absence is not evidence.** A record present on neither
//!   inventory is simply not planned; a record present on one side
//!   only is missing data to converge, never proof of global
//!   absence, and this planner draws no conclusion from it beyond
//!   the plan itself.
//!
//! Directions come from B31's matrix, evaluated by the caller:
//! this slice consumes the outcome (which directions are open for
//! this peer) and never re-decides peer trust. It performs no I/O
//! and no cryptography.

use std::collections::{BTreeMap, BTreeSet};

/// One inventory entry: a record as either side reports it in the
/// anti-entropy inventory exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    /// The record's stable id, shared across peers.
    pub id: String,
    /// The version identifier of this side's copy (an opaque
    /// version/content id; equality is the convergence test).
    pub version: String,
    /// The origin that produced this version (recorded on
    /// conflicts; never used to decide them here).
    pub origin: String,
    /// The tenant the record belongs to.
    pub tenant: String,
    /// Scope tags the record carries; all of them must be
    /// authorized for the record to be eligible.
    pub scopes: BTreeSet<String>,
    /// True when this side's copy is a deletion tombstone.
    pub deleted: bool,
    /// Serialized size of the record's data, for capacity
    /// accounting. Tombstones carry no data and are not charged.
    pub size_bytes: u64,
    /// When this version was produced, unix seconds.
    pub updated_at: u64,
}

/// The authorized scope of one sync relationship: the tenants and
/// scope tags convergence is allowed to touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncScope {
    /// Tenants whose records may converge with this peer.
    pub tenants: BTreeSet<String>,
    /// Scope tags authorized for this peer.
    pub scope_tags: BTreeSet<String>,
}

impl SyncScope {
    /// Whether a record version sits inside the authorized scope:
    /// its tenant is in scope and every tag it carries is
    /// authorized.
    #[must_use]
    pub fn admits(&self, record: &RecordRef) -> bool {
        self.tenants.contains(&record.tenant)
            && record.scopes.iter().all(|t| self.scope_tags.contains(t))
    }
}

/// The configured tolerance of one sync relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncLimits {
    /// Maximum total data bytes moved in one round, both
    /// directions combined.
    pub capacity_bytes: u64,
    /// Versions older than this (relative to the planning
    /// instant) are outside the convergence target.
    pub retention_seconds: u64,
    /// How long a tombstone keeps propagating its deletion.
    pub tombstone_retention_seconds: u64,
}

/// Which data directions the peer-class matrix (B31) leaves open
/// for this peer, as decided by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExchangeDirections {
    /// Local data may be sent to the peer.
    pub send: bool,
    /// Peer data may be accepted from the peer.
    pub accept: bool,
}

/// Why a record did not cross in this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExclusionReason {
    /// The record carries a scope tag outside the authorized set.
    OutOfScope,
    /// The record's tenant is outside the authorized set.
    CrossTenant,
    /// The record is deleted; deleted data never crosses as data.
    Deleted,
    /// The version is older than the retention window.
    ExpiredRetention,
    /// The round's capacity budget ran out before this record.
    OverCapacity,
    /// The peer-class matrix does not open the needed direction.
    DirectionNotPermitted,
}

/// A durable conflict record, emitted before any resolution: both
/// version ids, both origins, the scope and the detection time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRecord {
    /// The diverged record's id.
    pub id: String,
    /// The local side's version id.
    pub local_version: String,
    /// The peer side's version id.
    pub remote_version: String,
    /// The origin of the local version.
    pub local_origin: String,
    /// The origin of the remote version.
    pub remote_origin: String,
    /// The tenant the divergence was detected under.
    pub tenant: String,
    /// When the divergence was detected, unix seconds.
    pub detected_at: u64,
}

/// The outcome of one anti-entropy planning round. All lists are
/// in record-id order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// Record ids to fetch from the peer (data crosses inbound).
    pub fetch: Vec<String>,
    /// Record ids to send to the peer (data crosses outbound).
    pub send: Vec<String>,
    /// Record ids whose tombstone the local side must apply
    /// (learned from the peer).
    pub apply_deletions: Vec<String>,
    /// Record ids whose tombstone the peer must apply (learned
    /// from the local side).
    pub send_deletions: Vec<String>,
    /// Divergences detected this round, reported before any
    /// resolution; the records themselves do not cross.
    pub conflicts: Vec<ConflictRecord>,
    /// Records that did not cross, each with its reason.
    pub excluded: Vec<(String, ExclusionReason)>,
}

fn scope_exclusion(scope: &SyncScope, record: &RecordRef) -> Option<ExclusionReason> {
    if !scope.tenants.contains(&record.tenant) {
        return Some(ExclusionReason::CrossTenant);
    }
    if !record.scopes.iter().all(|t| scope.scope_tags.contains(t)) {
        return Some(ExclusionReason::OutOfScope);
    }
    None
}

fn expired(updated_at: u64, window: u64, now: u64) -> bool {
    now.saturating_sub(updated_at) > window
}

/// Plan one anti-entropy round between the local inventory and
/// one peer's inventory. Pure and deterministic: the same inputs
/// always produce the same plan.
#[must_use]
pub fn plan_sync(
    local: &[RecordRef],
    remote: &[RecordRef],
    scope: &SyncScope,
    limits: &SyncLimits,
    exchange: ExchangeDirections,
    now: u64,
) -> SyncPlan {
    let local_by_id: BTreeMap<&str, &RecordRef> =
        local.iter().map(|r| (r.id.as_str(), r)).collect();
    let remote_by_id: BTreeMap<&str, &RecordRef> =
        remote.iter().map(|r| (r.id.as_str(), r)).collect();
    let ids: BTreeSet<&str> = local_by_id.keys().chain(remote_by_id.keys()).copied().collect();

    let mut plan = SyncPlan::default();
    let mut capacity_left = limits.capacity_bytes;

    for id in ids {
        let l = local_by_id.get(id).copied();
        let r = remote_by_id.get(id).copied();

        // Scope gate, per version: an out-of-scope version is
        // invisible to the round. If the visible versions leave
        // nothing to plan, the exclusion is recorded once.
        let l_vis = l.filter(|v| scope.admits(v));
        let r_vis = r.filter(|v| scope.admits(v));
        if l_vis.is_none() && r_vis.is_none() {
            let any = l.or(r);
            if let Some(any) = any
                && let Some(reason) = scope_exclusion(scope, any)
            {
                plan.excluded.push((id.to_string(), reason));
            }
            continue;
        }

        match (l_vis, r_vis) {
            (Some(a), Some(b)) => {
                if a.deleted && b.deleted {
                    // Converged on the deletion itself.
                    continue;
                }
                if a.deleted != b.deleted {
                    // Live/deleted disagreement is divergence.
                    plan.conflicts.push(ConflictRecord {
                        id: id.to_string(),
                        local_version: a.version.clone(),
                        remote_version: b.version.clone(),
                        local_origin: a.origin.clone(),
                        remote_origin: b.origin.clone(),
                        tenant: a.tenant.clone(),
                        detected_at: now,
                    });
                    continue;
                }
                if a.version == b.version {
                    // Already 1:1 for this record.
                    continue;
                }
                // Both live, different versions: conflict first,
                // no data crosses, unrelated records continue.
                plan.conflicts.push(ConflictRecord {
                    id: id.to_string(),
                    local_version: a.version.clone(),
                    remote_version: b.version.clone(),
                    local_origin: a.origin.clone(),
                    remote_origin: b.origin.clone(),
                    tenant: a.tenant.clone(),
                    detected_at: now,
                });
            }
            (Some(a), None) => {
                // Local only: send, send the tombstone, or exclude.
                if a.deleted {
                    if expired(a.updated_at, limits.tombstone_retention_seconds, now) {
                        plan.excluded.push((id.to_string(), ExclusionReason::ExpiredRetention));
                    } else if exchange.send {
                        plan.send_deletions.push(id.to_string());
                    } else {
                        plan.excluded
                            .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                    }
                } else if expired(a.updated_at, limits.retention_seconds, now) {
                    plan.excluded.push((id.to_string(), ExclusionReason::ExpiredRetention));
                } else if !exchange.send {
                    plan.excluded
                        .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                } else if a.size_bytes > capacity_left {
                    plan.excluded.push((id.to_string(), ExclusionReason::OverCapacity));
                } else {
                    capacity_left -= a.size_bytes;
                    plan.send.push(id.to_string());
                }
            }
            (None, Some(b)) => {
                // Remote only: fetch, apply the tombstone, or exclude.
                if b.deleted {
                    if expired(b.updated_at, limits.tombstone_retention_seconds, now) {
                        plan.excluded.push((id.to_string(), ExclusionReason::ExpiredRetention));
                    } else if exchange.accept {
                        plan.apply_deletions.push(id.to_string());
                    } else {
                        plan.excluded
                            .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                    }
                } else if expired(b.updated_at, limits.retention_seconds, now) {
                    plan.excluded.push((id.to_string(), ExclusionReason::ExpiredRetention));
                } else if !exchange.accept {
                    plan.excluded
                        .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                } else if b.size_bytes > capacity_left {
                    plan.excluded.push((id.to_string(), ExclusionReason::OverCapacity));
                } else {
                    capacity_left -= b.size_bytes;
                    plan.fetch.push(id.to_string());
                }
            }
            (None, None) => unreachable!("id came from one of the inventories"),
        }
    }

    plan
}
