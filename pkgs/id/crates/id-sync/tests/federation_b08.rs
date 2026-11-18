//! B08 model test: implicit-trust sync scope/tolerance (#10209,
//! statements S1.26–S1.31).
//!
//! Oracle: model — within authorized scope peers converge toward
//! 1:1 within configured capacity/retention; excluded/deleted/
//! unauthorized data never crosses. The verifier is the
//! independent reference planner below (`reference_plan`): it is
//! written directly from the settled design statements, as a
//! per-record classification, and shares no code with the
//! implementation under test. The peer directions fed to both
//! planners are derived from the B31 peer-class matrix, so the
//! test also pins how B08 consumes B31's interface. The
//! implementations are loaded by direct path because slice
//! 9110-09 — not this slice — wires the versioned public exports
//! after the leaf modules land (9110-02 registry rule).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b31_peer_classes.rs"]
mod b31;
#[path = "../src/federation/slices/b08_sync_scope.rs"]
mod b08;

use std::collections::BTreeSet;

use b08::{
    ConflictRecord, ExchangeDirections, ExclusionReason, RecordRef, SyncLimits, SyncPlan,
    SyncScope, plan_sync,
};
use b31::{Decision, Direction, GateEvidence, Operation, PeerClass, PeerPolicy};

const NOW: u64 = 10_000;

fn rec(
    id: &str,
    version: &str,
    tenant: &str,
    tags: &[&str],
    deleted: bool,
    size: u64,
    updated_at: u64,
) -> RecordRef {
    RecordRef {
        id: id.to_string(),
        version: version.to_string(),
        origin: "origin".to_string(),
        tenant: tenant.to_string(),
        scopes: tags.iter().map(|t| t.to_string()).collect(),
        deleted,
        size_bytes: size,
        updated_at,
    }
}

fn scope() -> SyncScope {
    SyncScope {
        tenants: BTreeSet::from(["tenant-a".to_string()]),
        scope_tags: BTreeSet::from(["public".to_string(), "shared".to_string()]),
    }
}

fn limits() -> SyncLimits {
    SyncLimits {
        capacity_bytes: 1_000,
        retention_seconds: 1_000,
        tombstone_retention_seconds: 500,
    }
}

fn both_ways() -> ExchangeDirections {
    ExchangeDirections {
        send: true,
        accept: true,
    }
}

// ---------------------------------------------------------------------------
// Independent reference planner, written from the design statements:
// scope first (an out-of-scope version is invisible); deleted data
// never crosses as data — a fresh tombstone propagates, a stale one
// does not, and live-vs-deleted is divergence; two different live
// versions are a conflict, reported before resolution, and neither
// crosses while unrelated records continue; a one-sided live record
// crosses only in an open direction, inside retention, and while
// the round's capacity lasts (record-id order); absence on both
// sides plans nothing.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RefPlan {
    fetch: Vec<String>,
    send: Vec<String>,
    apply_deletions: Vec<String>,
    send_deletions: Vec<String>,
    conflicts: Vec<ConflictRecord>,
    excluded: Vec<(String, ExclusionReason)>,
}

fn visible(scope: &SyncScope, r: &RecordRef) -> bool {
    scope.tenants.contains(&r.tenant) && r.scopes.iter().all(|t| scope.scope_tags.contains(t))
}

fn reference_plan(
    local: &[RecordRef],
    remote: &[RecordRef],
    scope: &SyncScope,
    limits: &SyncLimits,
    exchange: ExchangeDirections,
    now: u64,
) -> SyncPlan {
    let mut ids: Vec<&str> = local.iter().map(|r| r.id.as_str()).collect();
    ids.extend(remote.iter().map(|r| r.id.as_str()));
    ids.sort_unstable();
    ids.dedup();

    let mut out = RefPlan::default();
    let mut spent: u64 = 0;

    for id in ids {
        let l = local.iter().find(|r| r.id == id);
        let r = remote.iter().find(|r| r.id == id);
        let lv = l.filter(|v| visible(scope, v));
        let rv = r.filter(|v| visible(scope, v));

        if lv.is_none() && rv.is_none() {
            let any = l.or(r).expect("id from an inventory");
            let reason = if !scope.tenants.contains(&any.tenant) {
                ExclusionReason::CrossTenant
            } else {
                ExclusionReason::OutOfScope
            };
            out.excluded.push((id.to_string(), reason));
            continue;
        }

        let stale = |v: &RecordRef, window: u64| now.saturating_sub(v.updated_at) > window;

        match (lv, rv) {
            (Some(a), Some(b)) => {
                if a.deleted == b.deleted && (a.deleted || a.version == b.version) {
                    // Same version, or both tombstones: converged.
                } else {
                    out.conflicts.push(ConflictRecord {
                        id: id.to_string(),
                        local_version: a.version.clone(),
                        remote_version: b.version.clone(),
                        local_origin: a.origin.clone(),
                        remote_origin: b.origin.clone(),
                        tenant: a.tenant.clone(),
                        detected_at: now,
                    });
                }
            }
            (Some(a), None) => {
                if a.deleted {
                    if stale(a, limits.tombstone_retention_seconds) {
                        out.excluded
                            .push((id.to_string(), ExclusionReason::ExpiredRetention));
                    } else if exchange.send {
                        out.send_deletions.push(id.to_string());
                    } else {
                        out.excluded
                            .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                    }
                } else if stale(a, limits.retention_seconds) {
                    out.excluded
                        .push((id.to_string(), ExclusionReason::ExpiredRetention));
                } else if !exchange.send {
                    out.excluded
                        .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                } else if spent + a.size_bytes > limits.capacity_bytes {
                    out.excluded.push((id.to_string(), ExclusionReason::OverCapacity));
                } else {
                    spent += a.size_bytes;
                    out.send.push(id.to_string());
                }
            }
            (None, Some(b)) => {
                if b.deleted {
                    if stale(b, limits.tombstone_retention_seconds) {
                        out.excluded
                            .push((id.to_string(), ExclusionReason::ExpiredRetention));
                    } else if exchange.accept {
                        out.apply_deletions.push(id.to_string());
                    } else {
                        out.excluded
                            .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                    }
                } else if stale(b, limits.retention_seconds) {
                    out.excluded
                        .push((id.to_string(), ExclusionReason::ExpiredRetention));
                } else if !exchange.accept {
                    out.excluded
                        .push((id.to_string(), ExclusionReason::DirectionNotPermitted));
                } else if spent + b.size_bytes > limits.capacity_bytes {
                    out.excluded.push((id.to_string(), ExclusionReason::OverCapacity));
                } else {
                    spent += b.size_bytes;
                    out.fetch.push(id.to_string());
                }
            }
            (None, None) => unreachable!(),
        }
    }

    SyncPlan {
        fetch: out.fetch,
        send: out.send,
        apply_deletions: out.apply_deletions,
        send_deletions: out.send_deletions,
        conflicts: out.conflicts,
        excluded: out.excluded,
    }
}

fn assert_matches_reference(
    local: &[RecordRef],
    remote: &[RecordRef],
    scope: &SyncScope,
    limits: &SyncLimits,
    exchange: ExchangeDirections,
    now: u64,
) {
    let got = plan_sync(local, remote, scope, limits, exchange, now);
    let want = reference_plan(local, remote, scope, limits, exchange, now);
    assert_eq!(got, want);
}

// ---------------------------------------------------------------------------
// Directed scenarios, one oracle clause at a time.
// ---------------------------------------------------------------------------

#[test]
fn in_scope_one_sided_records_converge_both_directions() {
    let local = vec![rec("a", "v1", "tenant-a", &["public"], false, 10, NOW)];
    let remote = vec![rec("b", "v1", "tenant-a", &["shared"], false, 10, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert_eq!(plan.send, ["a"]);
    assert_eq!(plan.fetch, ["b"]);
    assert!(plan.conflicts.is_empty() && plan.excluded.is_empty());
    assert_matches_reference(&local, &remote, &scope(), &limits(), both_ways(), NOW);
}

#[test]
fn same_version_is_already_one_to_one() {
    let local = vec![rec("a", "v1", "tenant-a", &["public"], false, 10, NOW)];
    let remote = vec![rec("a", "v1", "tenant-a", &["public"], false, 10, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert_eq!(plan, SyncPlan::default());
}

#[test]
fn out_of_scope_and_cross_tenant_data_never_crosses() {
    let local = vec![
        rec("a", "v1", "tenant-a", &["secret"], false, 10, NOW),
        rec("b", "v1", "tenant-b", &["public"], false, 10, NOW),
    ];
    let remote = vec![rec("c", "v1", "tenant-b", &["public"], false, 10, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert!(plan.send.is_empty() && plan.fetch.is_empty());
    assert_eq!(
        plan.excluded,
        [
            ("a".to_string(), ExclusionReason::OutOfScope),
            ("b".to_string(), ExclusionReason::CrossTenant),
            ("c".to_string(), ExclusionReason::CrossTenant),
        ]
    );
    assert_matches_reference(&local, &remote, &scope(), &limits(), both_ways(), NOW);
}

#[test]
fn deleted_data_never_crosses_but_fresh_tombstones_propagate() {
    let local = vec![rec("gone-local", "v2", "tenant-a", &["public"], true, 0, NOW)];
    let remote = vec![rec("gone-remote", "v2", "tenant-a", &["public"], true, 0, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert!(plan.send.is_empty() && plan.fetch.is_empty());
    assert_eq!(plan.send_deletions, ["gone-local"]);
    assert_eq!(plan.apply_deletions, ["gone-remote"]);
    assert_matches_reference(&local, &remote, &scope(), &limits(), both_ways(), NOW);
}

#[test]
fn divergence_is_reported_before_resolution_and_unrelated_records_continue() {
    let local = vec![
        rec("clash", "v-local", "tenant-a", &["public"], false, 10, NOW),
        rec("fine", "v1", "tenant-a", &["public"], false, 10, NOW),
    ];
    let remote = vec![rec("clash", "v-remote", "tenant-a", &["public"], false, 10, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert_eq!(plan.conflicts.len(), 1);
    let c = &plan.conflicts[0];
    assert_eq!(c.id, "clash");
    assert_eq!(c.local_version, "v-local");
    assert_eq!(c.remote_version, "v-remote");
    assert_eq!(c.detected_at, NOW);
    // The diverged record crosses in neither direction; the
    // unrelated one still syncs in the same round.
    assert_eq!(plan.send, ["fine"]);
    assert!(plan.fetch.is_empty());
    assert_matches_reference(&local, &remote, &scope(), &limits(), both_ways(), NOW);
}

#[test]
fn live_versus_deleted_is_divergence_not_resurrection() {
    let local = vec![rec("a", "v2", "tenant-a", &["public"], false, 10, NOW)];
    let remote = vec![rec("a", "v3", "tenant-a", &["public"], true, 0, NOW)];
    let plan = plan_sync(&local, &remote, &scope(), &limits(), both_ways(), NOW);
    assert_eq!(plan.conflicts.len(), 1);
    assert!(plan.fetch.is_empty() && plan.send.is_empty());
    assert!(plan.apply_deletions.is_empty());
    assert_matches_reference(&local, &remote, &scope(), &limits(), both_ways(), NOW);
}

#[test]
fn capacity_bounds_the_round_in_record_id_order() {
    let tight = SyncLimits {
        capacity_bytes: 25,
        ..limits()
    };
    let local: Vec<RecordRef> = ["a", "b", "c"]
        .iter()
        .map(|id| rec(id, "v1", "tenant-a", &["public"], false, 10, NOW))
        .collect();
    let plan = plan_sync(&local, &[], &scope(), &tight, both_ways(), NOW);
    assert_eq!(plan.send, ["a", "b"]);
    assert_eq!(
        plan.excluded,
        [("c".to_string(), ExclusionReason::OverCapacity)]
    );
    assert_matches_reference(&local, &[], &scope(), &tight, both_ways(), NOW);
}

#[test]
fn expired_versions_and_stale_tombstones_stay_put() {
    let local = vec![
        rec("old", "v1", "tenant-a", &["public"], false, 10, NOW - 2_000),
        rec("stale-gone", "v2", "tenant-a", &["public"], true, 0, NOW - 900),
    ];
    let plan = plan_sync(&local, &[], &scope(), &limits(), both_ways(), NOW);
    assert!(plan.send.is_empty() && plan.send_deletions.is_empty());
    assert_eq!(
        plan.excluded,
        [
            ("old".to_string(), ExclusionReason::ExpiredRetention),
            ("stale-gone".to_string(), ExclusionReason::ExpiredRetention),
        ]
    );
    assert_matches_reference(&local, &[], &scope(), &limits(), both_ways(), NOW);
}

// ---------------------------------------------------------------------------
// B31 consumption: the directions come from the peer-class matrix.
// ---------------------------------------------------------------------------

fn directions_for(class: PeerClass) -> ExchangeDirections {
    let policy = PeerPolicy {
        peer_id: "peer-1".to_string(),
        class,
        operations: BTreeSet::from([Operation::AcceptData, Operation::SendData]),
        relay_granted: false,
        scope: "scope-1".to_string(),
        policy_version: 1,
        valid_from: None,
        expires_at: None,
    };
    let gate = GateEvidence::all_pass();
    ExchangeDirections {
        send: b31::decide(
            Some(&policy),
            Operation::SendData,
            Direction::Outbound,
            &gate,
            NOW,
        ) == Decision::Allow,
        accept: b31::decide(
            Some(&policy),
            Operation::AcceptData,
            Direction::Inbound,
            &gate,
            NOW,
        ) == Decision::Allow,
    }
}

#[test]
fn peer_class_gates_which_directions_converge() {
    let local = vec![rec("a", "v1", "tenant-a", &["public"], false, 10, NOW)];
    let remote = vec![rec("b", "v1", "tenant-a", &["public"], false, 10, NOW)];

    // Unknown peer: nothing crosses either way.
    let plan = plan_sync(
        &local,
        &remote,
        &scope(),
        &limits(),
        directions_for(PeerClass::Unknown),
        NOW,
    );
    assert!(plan.send.is_empty() && plan.fetch.is_empty());
    assert_eq!(plan.excluded.len(), 2);
    assert!(
        plan.excluded
            .iter()
            .all(|(_, r)| *r == ExclusionReason::DirectionNotPermitted)
    );

    // Send-only peer: local data goes out, nothing is accepted.
    let plan = plan_sync(
        &local,
        &remote,
        &scope(),
        &limits(),
        directions_for(PeerClass::SendOnly),
        NOW,
    );
    assert_eq!(plan.send, ["a"]);
    assert!(plan.fetch.is_empty());

    // Accept-signed-from peer: data comes in, nothing goes out.
    let plan = plan_sync(
        &local,
        &remote,
        &scope(),
        &limits(),
        directions_for(PeerClass::AcceptSignedFrom),
        NOW,
    );
    assert_eq!(plan.fetch, ["b"]);
    assert!(plan.send.is_empty());
}

// ---------------------------------------------------------------------------
// Deterministic sweep: pseudo-random inventories, full-plan
// comparison against the reference.
// ---------------------------------------------------------------------------

#[test]
fn sweep_matches_reference() {
    // A tiny LCG keeps the sweep deterministic without a rand dep.
    let mut state: u64 = 0x5eed;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };

    for round in 0..40_u64 {
        let mut local = Vec::new();
        let mut remote = Vec::new();
        for i in 0..8 {
            let id = format!("r{i}");
            for inv in [&mut local, &mut remote] {
                if next() % 3 == 0 {
                    continue; // absent from this inventory
                }
                let tenant = if next() % 5 == 0 { "tenant-b" } else { "tenant-a" };
                let tags: &[&str] = match next() % 4 {
                    0 => &["public"],
                    1 => &["shared"],
                    2 => &["secret"],
                    _ => &["public", "shared"],
                };
                let deleted = next() % 6 == 0;
                let version = format!("v{}", next() % 3);
                let age = u64::from(next() % 1_500);
                inv.push(rec(
                    &id,
                    &version,
                    tenant,
                    tags,
                    deleted,
                    u64::from(next() % 60),
                    NOW - age,
                ));
            }
        }
        let round_limits = SyncLimits {
            capacity_bytes: u64::from(40 + (round * 37) % 200),
            ..limits()
        };
        let exchange = match round % 4 {
            0 => both_ways(),
            1 => ExchangeDirections {
                send: true,
                accept: false,
            },
            2 => ExchangeDirections {
                send: false,
                accept: true,
            },
            _ => ExchangeDirections::default(),
        };
        assert_matches_reference(&local, &remote, &scope(), &round_limits, exchange, NOW);
    }
}
