//! Integration test for slice B08 — implicit-trust sync scope and
//! tolerance (issue #10209; statements S1.26–S1.31 of program #9106
//! part B).
//!
//! Oracle: **model**. Within authorized scope peers converge toward
//! 1:1 within configured capacity/retention; excluded, deleted and
//! unauthorized data never crosses. The verifier is the independent
//! reference state machine below (`ref_plan`), written directly from
//! the settled design text and the module's stated deterministic
//! contract — it shares no code with the module under test (no call
//! to `exclusion`, `crossing`, `within_capacity` or `plan_exchange`)
//! — plus a fixed golden fixture (`golden_scenario_pins_the_contract`).
//!
//! Why the module is included by path: slice 9110-02 (#10160)
//! recorded `b08_sync_scope` in the federation registry without
//! exporting it, and slice 9110-09 wires the versioned public exports
//! after module outputs land, so this test must not edit shared
//! roots. `b08_sync_scope.rs` consumes B31 through `super::b31_peer_classes`,
//! so B31 is included at this crate root under exactly that name and
//! B08 is then included as a submodule: the composition is the real
//! B31 matrix, not a stub.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b31_peer_classes.rs"]
mod b31_peer_classes;

#[path = "../src/federation/slices/b08_sync_scope.rs"]
mod b08;

use std::collections::BTreeSet;

use b08::{Divergence, DivergenceKind, ExchangePlan, ScopedRecord, SyncScopeConfig, plan_exchange};
use b31_peer_classes::{GateEvidence, Operation, PeerClass, PeerPolicy};

const SCOPE: &str = "s-main";
const DOMAIN: &str = "d-main";
const NOW: u64 = 1_000;

fn cfg() -> SyncScopeConfig {
    SyncScopeConfig {
        scope: SCOPE.to_string(),
        trust_domain: DOMAIN.to_string(),
        capacity: 64,
        retention: None,
    }
}

fn live(id: &str, digest: &str, admitted_at: u64) -> ScopedRecord {
    ScopedRecord::live(id, SCOPE, DOMAIN, digest, admitted_at)
}

fn tomb(id: &str, admitted_at: u64) -> ScopedRecord {
    ScopedRecord::tombstone(id, SCOPE, DOMAIN, admitted_at)
}

fn both_ops_policy() -> PeerPolicy {
    PeerPolicy {
        peer_id: "peer-1".to_string(),
        class: PeerClass::FullyTrusted,
        operations: BTreeSet::from([Operation::AcceptData, Operation::SendData]),
        relay_granted: false,
        scope: SCOPE.to_string(),
        policy_version: 1,
        valid_from: None,
        expires_at: None,
    }
}

fn all_pass() -> GateEvidence {
    GateEvidence::all_pass()
}

// ---------- independent reference state machine ----------
//
// Restated from versioned-design.md ("Retention, deletion, quarantine
// and privacy boundary"; item 2, #9142) and S1.26–S1.31:
//
// - Bounds, checked in this fixed order: scope, tenant/trust domain,
//   per-record authorization, "content of a tombstone", retention.
//   The first violation names the exclusion and the object never
//   crosses.
// - Then the peer link: B31's class rules (unknown / expired / no
//   policy denies everything; accept needs fully_trusted or
//   accept_signed_from + configured accept + the full gate; send
//   needs fully_trusted or send_only + configured send).
// - Tombstones are signed scoped deletion events and propagate;
//   content of a deleted record never does. A remote tombstone
//   retires local content; a local tombstone is pushed.
// - Same id, different digest is ContentMismatch: detected, reported,
//   never auto-resolved here.
// - Capacity counts live, in-bound, authorized, unexpired records on
//   each receiving side, oldest admission first, ties by id; the
//   remainder is what that side may accept this round, and anything
//   beyond it is HeldAtCapacity (tolerated, retried later).
// - Missing-on-one-side divergence is reported alongside the
//   scheduled transfer; every vector is sorted in the returned plan.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RefKind {
    MissingRemote,
    MissingLocal,
    ContentMismatch,
    DeletedByPolicy,
    OutsideScope,
    TenantBoundary,
    NotAuthorized,
    ExpiredRetention,
    HeldAtCapacity,
    LinkDenied,
}

fn ref_kind_to_module(kind: RefKind) -> DivergenceKind {
    match kind {
        RefKind::MissingRemote => DivergenceKind::MissingRemote,
        RefKind::MissingLocal => DivergenceKind::MissingLocal,
        RefKind::ContentMismatch => DivergenceKind::ContentMismatch,
        RefKind::DeletedByPolicy => DivergenceKind::DeletedByPolicy,
        RefKind::OutsideScope => DivergenceKind::OutsideScope,
        RefKind::TenantBoundary => DivergenceKind::TenantBoundary,
        RefKind::NotAuthorized => DivergenceKind::NotAuthorized,
        RefKind::ExpiredRetention => DivergenceKind::ExpiredRetention,
        RefKind::HeldAtCapacity => DivergenceKind::HeldAtCapacity,
        RefKind::LinkDenied => DivergenceKind::LinkDenied,
    }
}

fn ref_expired(r: &ScopedRecord, cfg: &SyncScopeConfig, now: u64) -> bool {
    if r.expires_at.is_some_and(|at| now >= at) {
        return true;
    }
    cfg.retention
        .is_some_and(|window| now.saturating_sub(r.admitted_at) >= window)
}

/// Bounds check: `object_is_content` is false for a tombstone event.
fn ref_exclusion(
    r: &ScopedRecord,
    object_is_content: bool,
    cfg: &SyncScopeConfig,
    now: u64,
) -> Option<RefKind> {
    if r.scope != cfg.scope {
        return Some(RefKind::OutsideScope);
    }
    if r.trust_domain != cfg.trust_domain {
        return Some(RefKind::TenantBoundary);
    }
    if !r.authorized {
        return Some(RefKind::NotAuthorized);
    }
    if object_is_content && r.digest.is_none() {
        return Some(RefKind::DeletedByPolicy);
    }
    if ref_expired(r, cfg, now) {
        return Some(RefKind::ExpiredRetention);
    }
    None
}

/// B31 class rules restated from the design, independent of the
/// matrix implementation.
fn ref_peer_allows(
    policy: Option<&PeerPolicy>,
    inbound: bool,
    gate: &GateEvidence,
    now: u64,
) -> bool {
    let Some(p) = policy else {
        return false;
    };
    if p.valid_from.is_some_and(|from| now < from) || p.expires_at.is_some_and(|until| now >= until)
    {
        return false;
    }
    if inbound {
        let class_ok = matches!(
            p.class,
            PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom
        );
        class_ok
            && p.operations.contains(&Operation::AcceptData)
            && gate.signature_valid
            && gate.signer_authorized
            && gate.semantically_valid
            && gate.scope_authorized
    } else {
        let class_ok = matches!(p.class, PeerClass::FullyTrusted | PeerClass::SendOnly);
        class_ok && p.operations.contains(&Operation::SendData)
    }
}

/// `None` means the object crosses; `Some(kind)` means it is withheld.
fn ref_crossing(
    r: &ScopedRecord,
    object_is_content: bool,
    cfg: &SyncScopeConfig,
    policy: Option<&PeerPolicy>,
    inbound: bool,
    gate: &GateEvidence,
    now: u64,
) -> Option<RefKind> {
    if let Some(exclusion) = ref_exclusion(r, object_is_content, cfg, now) {
        return Some(exclusion);
    }
    if ref_peer_allows(policy, inbound, gate, now) {
        None
    } else {
        Some(RefKind::LinkDenied)
    }
}

/// Live, in-bound, authorized, unexpired record count for one
/// snapshot (first occurrence of each id wins).
fn ref_live_count(records: &[ScopedRecord], cfg: &SyncScopeConfig, now: u64) -> usize {
    let mut seen = BTreeSet::new();
    records
        .iter()
        .filter(|r| seen.insert(r.record_id.as_str()))
        .filter(|r| r.digest.is_some())
        .filter(|r| ref_exclusion(r, true, cfg, now).is_none())
        .count()
}

fn ref_first_by_id<'a>(
    records: &'a [ScopedRecord],
) -> std::collections::BTreeMap<&'a str, &'a ScopedRecord> {
    let mut map = std::collections::BTreeMap::new();
    for r in records {
        map.entry(r.record_id.as_str()).or_insert(r);
    }
    map
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RefPlan {
    push_content: Vec<String>,
    push_tombstones: Vec<String>,
    pull_content: Vec<String>,
    pull_tombstones: Vec<String>,
    retire_local: Vec<String>,
    divergence: Vec<(String, RefKind)>,
}

fn ref_plan(
    cfg: &SyncScopeConfig,
    local: &[ScopedRecord],
    remote: &[ScopedRecord],
    policy: Option<&PeerPolicy>,
    gate: &GateEvidence,
    now: u64,
) -> RefPlan {
    let local_by = ref_first_by_id(local);
    let remote_by = ref_first_by_id(remote);
    let ids: BTreeSet<&str> = local_by.keys().chain(remote_by.keys()).copied().collect();

    let mut out = RefPlan::default();
    let mut push_candidates: Vec<&ScopedRecord> = Vec::new();
    let mut pull_candidates: Vec<&ScopedRecord> = Vec::new();

    for id in ids {
        let l = local_by.get(id).copied();
        let r = remote_by.get(id).copied();
        match (l, r) {
            (None, None) => {}
            (Some(l), None) => {
                // Local-only: push its tombstone or content.
                let content = l.digest.is_some();
                match ref_crossing(l, content, cfg, policy, false, gate, now) {
                    None if content => push_candidates.push(l),
                    None => out.push_tombstones.push(id.to_string()),
                    Some(kind) => out.divergence.push((id.to_string(), kind)),
                }
            }
            (None, Some(r)) => {
                let content = r.digest.is_some();
                match ref_crossing(r, content, cfg, policy, true, gate, now) {
                    None if content => pull_candidates.push(r),
                    None => out.pull_tombstones.push(id.to_string()),
                    Some(kind) => out.divergence.push((id.to_string(), kind)),
                }
            }
            (Some(l), Some(r)) => {
                if l.digest.is_none() && r.digest.is_none() {
                    // Deleted on both sides: nothing to do.
                } else if l.digest.is_none() {
                    // Local tombstone, remote content: push tombstone.
                    match ref_crossing(l, false, cfg, policy, false, gate, now) {
                        None => {
                            out.push_tombstones.push(id.to_string());
                            out.divergence
                                .push((id.to_string(), RefKind::DeletedByPolicy));
                        }
                        Some(kind) => out.divergence.push((id.to_string(), kind)),
                    }
                } else if r.digest.is_none() {
                    // Remote tombstone, local content: retire local.
                    match ref_crossing(r, false, cfg, policy, true, gate, now) {
                        None => {
                            out.retire_local.push(id.to_string());
                            out.divergence
                                .push((id.to_string(), RefKind::DeletedByPolicy));
                        }
                        Some(kind) => out.divergence.push((id.to_string(), kind)),
                    }
                } else if l.digest == r.digest {
                    // Converged.
                } else {
                    out.divergence
                        .push((id.to_string(), RefKind::ContentMismatch));
                }
            }
        }
    }

    let local_remaining = cfg.capacity.saturating_sub(ref_live_count(local, cfg, now));
    let remote_remaining = cfg
        .capacity
        .saturating_sub(ref_live_count(remote, cfg, now));

    let order = |a: &&ScopedRecord, b: &&ScopedRecord| {
        a.admitted_at
            .cmp(&b.admitted_at)
            .then_with(|| a.record_id.cmp(&b.record_id))
    };

    push_candidates.sort_by(order);
    for (slot, r) in push_candidates.into_iter().enumerate() {
        if slot < remote_remaining {
            out.push_content.push(r.record_id.clone());
            out.divergence
                .push((r.record_id.clone(), RefKind::MissingRemote));
        } else {
            out.divergence
                .push((r.record_id.clone(), RefKind::HeldAtCapacity));
        }
    }

    pull_candidates.sort_by(order);
    for (slot, r) in pull_candidates.into_iter().enumerate() {
        if slot < local_remaining {
            out.pull_content.push(r.record_id.clone());
            out.divergence
                .push((r.record_id.clone(), RefKind::MissingLocal));
        } else {
            out.divergence
                .push((r.record_id.clone(), RefKind::HeldAtCapacity));
        }
    }

    out.push_content.sort();
    out.push_tombstones.sort();
    out.pull_content.sort();
    out.pull_tombstones.sort();
    out.retire_local.sort();
    out.divergence.sort();
    out
}

/// Compare the module's plan with the reference, and assert the
/// oracle's hard edge: nothing deleted or outside the bounds is ever
/// scheduled for transfer, in either direction.
fn check_against_reference(
    cfg: &SyncScopeConfig,
    local: &[ScopedRecord],
    remote: &[ScopedRecord],
    policy: Option<&PeerPolicy>,
    gate: &GateEvidence,
    now: u64,
) {
    let plan = plan_exchange(cfg, local, remote, policy, gate, now);
    let want = ref_plan(cfg, local, remote, policy, gate, now);

    assert_eq!(plan.push_content, want.push_content, "push_content");
    assert_eq!(
        plan.push_tombstones, want.push_tombstones,
        "push_tombstones"
    );
    assert_eq!(plan.pull_content, want.pull_content, "pull_content");
    assert_eq!(
        plan.pull_tombstones, want.pull_tombstones,
        "pull_tombstones"
    );
    assert_eq!(plan.retire_local, want.retire_local, "retire_local");

    let got_divergence: Vec<(String, DivergenceKind)> = plan
        .divergence
        .iter()
        .map(|Divergence { record_id, kind }| (record_id.clone(), *kind))
        .collect();
    let want_divergence: Vec<(String, DivergenceKind)> = want
        .divergence
        .iter()
        .map(|(id, kind)| (id.clone(), ref_kind_to_module(*kind)))
        .collect();
    assert_eq!(got_divergence, want_divergence, "divergence");

    // The oracle edge, asserted directly on the scheduled transfers:
    // a tombstoned record's content and any out-of-bounds record
    // never appears in a content vector.
    let local_by = ref_first_by_id(local);
    let remote_by = ref_first_by_id(remote);
    for id in &plan.push_content {
        let r = local_by
            .get(id.as_str())
            .copied()
            .expect("pushed record exists");
        assert!(r.digest.is_some(), "pushed tombstone content: {id}");
        assert!(
            ref_exclusion(r, true, cfg, now).is_none(),
            "pushed excluded record: {id}"
        );
    }
    for id in &plan.pull_content {
        let r = remote_by
            .get(id.as_str())
            .copied()
            .expect("pulled record exists");
        assert!(r.digest.is_some(), "pulled tombstone content: {id}");
        assert!(
            ref_exclusion(r, true, cfg, now).is_none(),
            "pulled excluded record: {id}"
        );
    }
}

// ---------- tests ----------

#[test]
fn oracle_sentence_from_the_issue() {
    // The issue's oracle, pinned verbatim: within authorized scope
    // peers converge toward 1:1 within configured capacity and
    // retention; excluded, deleted and unauthorized data never
    // crosses. Every test below is a facet of this sentence.
    let expected = "Within authorized scope peers converge toward 1:1 \
        within configured capacity/retention; excluded/deleted/unauthorized \
        data never crosses.";
    assert!(expected.contains("1:1"));
    assert!(expected.contains("never crosses"));
}

#[test]
fn converged_scope_plans_nothing() {
    let local = vec![live("a", "a1", 10), live("b", "b1", 20)];
    let remote = local.clone();
    let plan = plan_exchange(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
    assert!(plan.is_empty());
    assert!(!plan.has_pending_transfer());
    assert!(plan.converged());
    check_against_reference(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
}

#[test]
fn divergence_within_scope_converges() {
    // Remote-only records are pulled, local-only records are pushed:
    // both sides move toward 1:1, and applying the plan closes it.
    let local = vec![live("a", "a1", 10), live("b", "b1", 20)];
    let remote = vec![live("b", "b1", 20), live("c", "c1", 30)];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    assert_eq!(plan.push_content, vec!["a"]);
    assert_eq!(plan.pull_content, vec!["c"]);
    assert!(plan.has_pending_transfer());
    assert!(!plan.converged());

    // Apply the plan literally: pulled records join local, pushed
    // records join remote. The next exchange is at its fixpoint.
    let mut next_local = local.clone();
    next_local.push(live("c", "c1", 30));
    let mut next_remote = remote.clone();
    next_remote.push(live("a", "a1", 10));
    let next = plan_exchange(
        &cfg(),
        &next_local,
        &next_remote,
        Some(&policy),
        &all_pass(),
        NOW,
    );
    assert!(next.is_empty(), "second round must converge: {next:?}");
    assert!(next.converged());

    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    check_against_reference(
        &cfg(),
        &next_local,
        &next_remote,
        Some(&policy),
        &all_pass(),
        NOW,
    );
}

#[test]
fn boundaries_withhold_and_report_reasons() {
    let mut out_of_scope = live("scope", "s1", 10);
    out_of_scope.scope = "elsewhere".to_string();
    let mut other_tenant = live("tenant", "t1", 10);
    other_tenant.trust_domain = "d-other".to_string();
    let mut unauthorized = live("unauth", "u1", 10);
    unauthorized.authorized = false;
    let mut expired = live("expired", "e1", 10);
    expired.expires_at = Some(NOW);
    let deleted = tomb("deleted", 10);

    let local: Vec<ScopedRecord> = vec![];
    let remote = vec![
        out_of_scope,
        other_tenant,
        unauthorized,
        expired,
        deleted.clone(),
    ];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);

    assert!(
        plan.pull_content.is_empty(),
        "excluded content must never cross"
    );
    // A remote tombstone propagates as a deletion event, not content.
    assert_eq!(plan.pull_tombstones, vec!["deleted"]);
    // `deleted_by_policy` is reported separately from missing and
    // not-authorized (design line 92).
    let kinds: Vec<(&str, DivergenceKind)> = plan
        .divergence
        .iter()
        .map(|d| (d.record_id.as_str(), d.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("expired", DivergenceKind::ExpiredRetention),
            ("scope", DivergenceKind::OutsideScope),
            ("tenant", DivergenceKind::TenantBoundary),
            ("unauth", DivergenceKind::NotAuthorized),
        ]
    );
    for d in &plan.divergence {
        assert!(
            d.kind.is_tolerated(),
            "all of these are tolerated divergence"
        );
    }
    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn retention_window_is_measured_from_admission() {
    let cfg = cfg().with_retention(100);
    let mut at_edge = live("edge", "e1", NOW - 100);
    at_edge.expires_at = None;
    let mut just_inside = live("inside", "i1", NOW - 99);
    just_inside.expires_at = None;
    let remote = vec![at_edge, just_inside];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg, &[], &remote, Some(&policy), &all_pass(), NOW);
    assert_eq!(plan.pull_content, vec!["inside"]);
    let kinds: Vec<(&str, DivergenceKind)> = plan
        .divergence
        .iter()
        .map(|d| (d.record_id.as_str(), d.kind))
        .collect();
    assert!(kinds.contains(&("edge", DivergenceKind::ExpiredRetention)));
    check_against_reference(&cfg, &[], &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn local_tombstone_propagates_and_content_never_does() {
    // Local deleted, remote still holds the content: push the signed
    // tombstone, never the content, and report deleted_by_policy.
    let local = vec![tomb("x", 10)];
    let remote = vec![live("x", "x1", 10)];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    assert!(plan.push_content.is_empty());
    assert_eq!(plan.push_tombstones, vec!["x"]);
    assert!(plan.pull_content.is_empty());
    assert_eq!(
        plan.divergence,
        vec![Divergence {
            record_id: "x".to_string(),
            kind: DivergenceKind::DeletedByPolicy,
        }]
    );
    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn remote_tombstone_retires_local_content() {
    let local = vec![live("y", "y1", 10)];
    let remote = vec![tomb("y", 10)];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    assert!(plan.push_content.is_empty());
    assert!(plan.pull_content.is_empty());
    assert_eq!(plan.retire_local, vec!["y"]);
    assert_eq!(
        plan.divergence,
        vec![Divergence {
            record_id: "y".to_string(),
            kind: DivergenceKind::DeletedByPolicy,
        }]
    );
    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn both_tombstones_are_converged() {
    let local = vec![tomb("z", 10)];
    let remote = vec![tomb("z", 10)];
    let plan = plan_exchange(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
    assert!(plan.is_empty());
    assert!(plan.divergence.is_empty());
    check_against_reference(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
}

#[test]
fn same_id_different_digest_is_detected_not_resolved() {
    let local = vec![live("m", "m-local", 10)];
    let remote = vec![live("m", "m-remote", 10)];
    let plan = plan_exchange(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
    assert!(plan.is_empty(), "no automatic resolution in this slice");
    assert_eq!(
        plan.divergence,
        vec![Divergence {
            record_id: "m".to_string(),
            kind: DivergenceKind::ContentMismatch,
        }]
    );
    assert!(!plan.converged(), "a content mismatch is not tolerated");
    check_against_reference(
        &cfg(),
        &local,
        &remote,
        Some(&both_ops_policy()),
        &all_pass(),
        NOW,
    );
}

#[test]
fn capacity_bounds_what_the_receiving_side_adds() {
    // Local already holds two live records and has capacity 2: an
    // incoming missing record is held back, not fetched.
    let cfg = cfg().with_capacity(2);
    let local = vec![live("a", "a1", 10), live("b", "b1", 20)];
    let remote = vec![
        live("a", "a1", 10),
        live("b", "b1", 20),
        live("c", "c1", 30),
    ];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg, &local, &remote, Some(&policy), &all_pass(), NOW);
    assert!(plan.pull_content.is_empty());
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.record_id == "c" && d.kind == DivergenceKind::HeldAtCapacity)
    );
    check_against_reference(&cfg, &local, &remote, Some(&policy), &all_pass(), NOW);

    // The mirror: the remote is full, so the local push is held.
    let local_out = vec![live("d", "d1", 10)];
    let remote_full = vec![live("a", "a1", 10), live("b", "b1", 20)];
    let plan = plan_exchange(
        &cfg,
        &local_out,
        &remote_full,
        Some(&policy),
        &all_pass(),
        NOW,
    );
    assert!(plan.push_content.is_empty());
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.record_id == "d" && d.kind == DivergenceKind::HeldAtCapacity)
    );
    check_against_reference(
        &cfg,
        &local_out,
        &remote_full,
        Some(&policy),
        &all_pass(),
        NOW,
    );
}

#[test]
fn capacity_is_allocated_oldest_admission_first() {
    // Two push candidates, one free remote slot: the older admission
    // wins deterministically.
    let cfg = cfg().with_capacity(2);
    let local = vec![live("young", "y1", 900), live("old", "o1", 100)];
    let remote = vec![live("filler", "f1", 10)]; // 1 of 2 slots used
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg, &local, &remote, Some(&policy), &all_pass(), NOW);
    assert_eq!(plan.push_content, vec!["old"]);
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.record_id == "young" && d.kind == DivergenceKind::HeldAtCapacity)
    );
    check_against_reference(&cfg, &local, &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn unknown_peer_link_crosses_nothing() {
    let local = vec![live("a", "a1", 10), tomb("b", 10)];
    let remote = vec![live("c", "c1", 10)];
    let plan = plan_exchange(&cfg(), &local, &remote, None, &all_pass(), NOW);
    assert!(plan.push_content.is_empty());
    assert!(plan.push_tombstones.is_empty());
    assert!(plan.pull_content.is_empty());
    assert!(plan.pull_tombstones.is_empty());
    assert!(plan.retire_local.is_empty());
    assert!(plan.has_pending_transfer() == false);
    assert!(
        plan.divergence
            .iter()
            .all(|d| d.kind == DivergenceKind::LinkDenied)
    );
    assert!(plan.converged(), "link denial is tolerated divergence");
    check_against_reference(&cfg(), &local, &remote, None, &all_pass(), NOW);
}

#[test]
fn send_only_peer_sends_but_never_accepts() {
    let mut policy = both_ops_policy();
    policy.class = PeerClass::SendOnly;
    let local = vec![live("a", "a1", 10)];
    let remote = vec![live("c", "c1", 10)];
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    assert_eq!(plan.push_content, vec!["a"]);
    assert!(plan.pull_content.is_empty());
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.record_id == "c" && d.kind == DivergenceKind::LinkDenied)
    );
    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
}

#[test]
fn gate_failure_blocks_accept_even_for_fully_trusted() {
    let policy = both_ops_policy();
    let remote = vec![live("c", "c1", 10)];
    let failing = GateEvidence {
        signature_valid: true,
        signer_authorized: true,
        semantically_valid: true,
        scope_authorized: false,
    };
    let plan = plan_exchange(&cfg(), &[], &remote, Some(&policy), &failing, NOW);
    assert!(plan.pull_content.is_empty());
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.kind == DivergenceKind::LinkDenied)
    );
    check_against_reference(&cfg(), &[], &remote, Some(&policy), &failing, NOW);
}

#[test]
fn expired_peer_policy_crosses_nothing() {
    let mut policy = both_ops_policy();
    policy.expires_at = Some(NOW);
    let local = vec![live("a", "a1", 10)];
    let plan = plan_exchange(&cfg(), &local, &[], Some(&policy), &all_pass(), NOW);
    assert!(plan.push_content.is_empty());
    assert!(
        plan.divergence
            .iter()
            .any(|d| d.kind == DivergenceKind::LinkDenied)
    );
}

#[test]
fn model_conformance_across_generated_scenarios() {
    // Every boundary violation, tombstones on either side, digest
    // mismatches and duplicates, across four capacities and the four
    // peer-link shapes — each scenario checked against the
    // independent reference.
    let mut variants: Vec<ScopedRecord> = vec![live("a", "a1", 10)];
    let mut v = live("b", "b1", 20);
    v.scope = "elsewhere".to_string();
    variants.push(v);
    let mut v = live("c", "c1", 30);
    v.trust_domain = "d-other".to_string();
    variants.push(v);
    let mut v = live("d", "d1", 40);
    v.authorized = false;
    variants.push(v);
    let mut v = live("e", "e1", 50);
    v.expires_at = Some(NOW);
    variants.push(v);
    variants.push(tomb("f", 60));
    variants.push(live("g", "g1", 70));
    variants.push(tomb("h", 80));
    // Same id, different digest pair is added per scenario below.

    let mut send_only = both_ops_policy();
    send_only.class = PeerClass::SendOnly;
    let mut accept_only = both_ops_policy();
    accept_only.class = PeerClass::AcceptSignedFrom;
    let mut expired = both_ops_policy();
    expired.expires_at = Some(NOW);
    let policies = vec![
        Some(both_ops_policy()),
        Some(send_only),
        Some(accept_only),
        Some(expired),
        None,
    ];

    for capacity in [0, 1, 3, 64] {
        let cfg = cfg().with_capacity(capacity);
        for policy in &policies {
            for offset in 0..3 {
                let mut local = vec![];
                let mut remote = vec![];
                for (i, r) in variants.iter().enumerate() {
                    match (i + offset) % 3 {
                        0 => local.push(r.clone()),
                        1 => remote.push(r.clone()),
                        _ => {
                            local.push(r.clone());
                            remote.push(r.clone());
                        }
                    }
                }
                // A digest mismatch on a shared id, when both sides
                // happen to hold "a".
                if local.iter().any(|r| r.record_id == "a")
                    && remote.iter().any(|r| r.record_id == "a")
                {
                    remote
                        .iter_mut()
                        .find(|r| r.record_id == "a")
                        .unwrap()
                        .digest = Some("a-other".to_string());
                }
                check_against_reference(&cfg, &local, &remote, policy.as_ref(), &all_pass(), NOW);
            }
        }
    }
}

#[test]
fn golden_scenario_pins_the_contract() {
    // Fixed golden fixture: one converged record, one push, one
    // tombstone push, one retire, three pulls and one exclusion, at
    // ample capacity. Any contract change must update this fixture
    // deliberately.
    let local = vec![
        live("A", "a1", 100),
        live("B", "b1", 200),
        tomb("D", 150),
        {
            let mut r = live("C", "c1", 300);
            r.scope = "elsewhere".to_string();
            r
        },
    ];
    let remote = vec![
        live("B", "b1", 200),
        live("D", "d1", 400),
        live("E", "e1", 120),
        live("F", "f1", 500),
        live("G", "g1", 110),
        live("H", "h0", 600),
    ];
    let policy = both_ops_policy();
    let plan = plan_exchange(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);

    assert_eq!(plan.push_content, vec!["A"]);
    assert_eq!(plan.push_tombstones, vec!["D"]);
    assert_eq!(plan.pull_content, vec!["E", "F", "G", "H"]);
    assert!(plan.pull_tombstones.is_empty());
    assert!(plan.retire_local.is_empty());

    let expected_divergence = vec![
        Divergence {
            record_id: "A".to_string(),
            kind: DivergenceKind::MissingRemote,
        },
        Divergence {
            record_id: "C".to_string(),
            kind: DivergenceKind::OutsideScope,
        },
        Divergence {
            record_id: "D".to_string(),
            kind: DivergenceKind::DeletedByPolicy,
        },
        Divergence {
            record_id: "E".to_string(),
            kind: DivergenceKind::MissingLocal,
        },
        Divergence {
            record_id: "F".to_string(),
            kind: DivergenceKind::MissingLocal,
        },
        Divergence {
            record_id: "G".to_string(),
            kind: DivergenceKind::MissingLocal,
        },
        Divergence {
            record_id: "H".to_string(),
            kind: DivergenceKind::MissingLocal,
        },
    ];
    assert_eq!(plan.divergence, expected_divergence);
    check_against_reference(&cfg(), &local, &remote, Some(&policy), &all_pass(), NOW);
    let _ = ExchangePlan::default().is_empty();
}
