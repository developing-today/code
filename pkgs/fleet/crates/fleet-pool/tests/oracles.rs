//! Named-oracle tests for `fleet-pool` (design #13752 §3.5, #14721).
//!
//! Property tests name their oracles, per the testing doctrine:
//! each `proptest!` block states the invariant it guards, in the
//! design's own words. Slice 1 landed the first two oracles; slice
//! 2 (#15066) adds the sibling-success and served-failure-advance
//! oracles in property form.

use fleet_pool::{
    AttemptKind, Classification, LeastLoaded, MemberState, NodeId, Pool, RoundRobin,
    ServedDisposition, Strategy as PoolStrategy,
};
use proptest::prelude::*;
use std::collections::HashSet;

fn serve_failure(scope: &NodeId, reset_hint: Option<u64>) -> Classification {
    Classification {
        kind: AttemptKind::ServeFailure,
        served_disposition: ServedDisposition::Hold,
        advance: true,
        evidence_scope: scope.clone(),
        reset_hint,
        evidence: "test serve failure".to_owned(),
    }
}

fn served_wall(scope: &NodeId) -> Classification {
    Classification {
        kind: AttemptKind::ServedFailure,
        served_disposition: ServedDisposition::Wall,
        advance: true,
        evidence_scope: scope.clone(),
        reset_hint: None,
        evidence: "test served failure".to_owned(),
    }
}

fn served(scope: &NodeId, disposition: ServedDisposition, advance: bool) -> Classification {
    Classification {
        kind: AttemptKind::ServedFailure,
        served_disposition: disposition,
        advance,
        evidence_scope: scope.clone(),
        reset_hint: None,
        evidence: "test served failure".to_owned(),
    }
}

/// root ─┬─ g1 ─┬─ m1
///       │      └─ m2
///       └─ g2 ─┬─ m3
///              └─ m4
fn tree_pool() -> Pool {
    let mut pool = Pool::new();
    pool.add_group("root", None);
    pool.add_group("g1", Some(&NodeId::new("root")));
    pool.add_group("g2", Some(&NodeId::new("root")));
    pool.add_member("m1", Some(&NodeId::new("g1")));
    pool.add_member("m2", Some(&NodeId::new("g1")));
    pool.add_member("m3", Some(&NodeId::new("g2")));
    pool.add_member("m4", Some(&NodeId::new("g2")));
    pool
}

fn ancestors_of(id: &str) -> Vec<&'static str> {
    match id {
        "m1" | "m2" => vec!["g1", "root"],
        "m3" | "m4" => vec!["g2", "root"],
        "g1" | "g2" => vec!["root"],
        _ => vec![],
    }
}

#[test]
fn state_machine_walks_serving_demoted_walled_probing_serving() {
    let mut pool = Pool::new();
    pool.add_member("m", None);
    let m = NodeId::new("m");

    assert_eq!(pool.state_of(&m, 0), MemberState::Serving);

    // First serve-failure demotes; the walk advances.
    let fx = pool.record_outcome(&m, &serve_failure(&m, Some(10)), 0);
    assert!(fx.advance);
    assert_eq!(pool.state_of(&m, 0), MemberState::Demoted);

    // Second serve-failure walls at the evidence scope.
    let fx = pool.record_outcome(&m, &serve_failure(&m, Some(10)), 0);
    assert_eq!(fx.walled_scope, Some(m.clone()));
    assert_eq!(pool.state_of(&m, 5), MemberState::Walled);

    // A walled member is never selected.
    let mut strategy = LeastLoaded;
    assert_eq!(pool.select(&mut strategy, &|_| Some(0.0), 5), None);

    // Wall expires at the recorded reset; a successful probe
    // re-seats the member at demoted weight.
    pool.tick(11, &mut |_: &NodeId| true);
    assert_eq!(pool.state_of(&m, 11), MemberState::Demoted);

    // A served success restores full standing.
    pool.record_success(&m, 12);
    assert_eq!(pool.state_of(&m, 12), MemberState::Serving);
}

#[test]
fn failed_probe_rewalls_with_inferred_expiry() {
    let mut pool = Pool::new();
    pool.add_member("m", None);
    let m = NodeId::new("m");

    pool.record_outcome(&m, &serve_failure(&m, Some(10)), 0);
    pool.record_outcome(&m, &serve_failure(&m, Some(10)), 0);
    assert_eq!(pool.state_of(&m, 5), MemberState::Walled);

    // Probe fails at expiry: the member re-walls on the inferred
    // bound (default 300s), it does not silently rejoin.
    pool.tick(11, &mut |_: &NodeId| false);
    assert_eq!(pool.state_of(&m, 11), MemberState::Walled);
    assert_eq!(pool.state_of(&m, 300), MemberState::Walled);

    // Inferred expiry passes; a good probe now re-seats it.
    pool.tick(400, &mut |_: &NodeId| true);
    assert_eq!(pool.state_of(&m, 400), MemberState::Demoted);
}

#[test]
fn sibling_success_narrows_a_group_wall() {
    let mut pool = tree_pool();
    let (m1, m2, g1) = (NodeId::new("m1"), NodeId::new("m2"), NodeId::new("g1"));

    // m1 fails twice with group-scope evidence: g1 walls, and m2 is
    // gated by its sibling's wall.
    pool.record_outcome(&m1, &serve_failure(&g1, Some(100)), 0);
    pool.record_outcome(&m1, &serve_failure(&g1, Some(100)), 0);
    assert_eq!(pool.state_of(&m1, 1), MemberState::Walled);
    assert_eq!(pool.state_of(&m2, 1), MemberState::Walled);

    // m2 succeeds (e.g. via a probe path): the group re-opens, the
    // wall narrows to m1 alone.
    pool.record_success(&m2, 2);
    assert_eq!(pool.state_of(&m2, 2), MemberState::Serving);
    assert_eq!(pool.state_of(&m1, 2), MemberState::Walled);
    let scopes: Vec<&NodeId> = pool.walls().map(|w| &w.scope).collect();
    assert_eq!(scopes, vec![&m1]);
}

#[test]
fn supporter_success_narrows_a_group_wall_to_the_remaining_failure() {
    let mut pool = tree_pool();
    let (m1, m2, g1) = (NodeId::new("m1"), NodeId::new("m2"), NodeId::new("g1"));

    // Both siblings fail twice with group-scope evidence: one wall
    // at g1, supported by both.
    for member in [&m1, &m2] {
        pool.record_outcome(member, &serve_failure(&g1, Some(100)), 0);
        pool.record_outcome(member, &serve_failure(&g1, Some(100)), 0);
    }
    assert_eq!(pool.state_of(&m1, 1), MemberState::Walled);
    assert_eq!(pool.state_of(&m2, 1), MemberState::Walled);

    // A supporter's success re-opens the group for it too: the
    // wall narrows to the member still failing (oracle: *a sibling
    // success never leaves a group wall wider than the failing
    // member*). Slice 1 left the group wall in place here, still
    // gating the member that had just succeeded.
    pool.record_success(&m1, 2);
    assert_eq!(pool.state_of(&m1, 2), MemberState::Serving);
    assert_eq!(pool.state_of(&m2, 2), MemberState::Walled);
    let scopes: Vec<&NodeId> = pool.walls().map(|w| &w.scope).collect();
    assert_eq!(scopes, vec![&m2]);
}

#[test]
fn every_selection_is_in_the_decision_log() {
    let mut pool = tree_pool();
    let mut strategy = RoundRobin::default();
    let load = |_: &NodeId| Some(1.0);

    let first = pool.select(&mut strategy, &load, 7);
    let second = pool.select(&mut strategy, &load, 8);
    assert!(first.is_some() && second.is_some());
    assert_ne!(first, second, "round-robin must rotate");

    let log = pool.decisions();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].strategy, "round-robin");
    assert_eq!(log[0].member, first);
    assert_eq!(log[0].state, Some(MemberState::Serving));
    assert_eq!(log[1].member, second);
}

fn load_gen() -> impl proptest::strategy::Strategy<Value = Option<f64>> {
    prop_oneof![Just(None), (-100.0f64..1000.0f64).prop_map(Some),]
}

proptest! {
    /// ORACLE (#13611, design §3.5): *an unknown load never wins a
    /// least-loaded selection against a known non-zero load.*
    /// Stronger form asserted here: whenever any known load exists,
    /// the winner holds the minimum known load.
    #[test]
    fn oracle_unknown_load_never_wins_least_loaded(
        loads in prop::collection::vec(load_gen(), 1..=8)
            .prop_filter("needs a known non-zero load", |loads| {
                loads.iter().any(|l| matches!(l, Some(v) if v.is_finite() && *v != 0.0))
            }),
    ) {
        let candidates: Vec<fleet_pool::Candidate> = loads
            .iter()
            .enumerate()
            .map(|(i, load)| fleet_pool::Candidate {
                id: NodeId::new(format!("m{i}")),
                state: MemberState::Serving,
                load: *load,
                cost_class: 0,
            })
            .collect();

        let mut strategy = LeastLoaded;
        let picked = strategy.select(&candidates).expect("candidates are non-empty");
        let winner = candidates[picked].load;
        let min_known = loads
            .iter()
            .filter_map(|l| *l)
            .filter(|v| v.is_finite())
            .fold(f64::INFINITY, f64::min);
        prop_assert_eq!(winner, Some(min_known));
    }

    /// ORACLE (#13693, design §3.5): *a wall never widens past the
    /// scope its evidence supports.* Every wall in the pool must sit
    /// at a scope some outcome's evidence named, or at a member
    /// scope derived by narrowing from such a wall (a descendant of
    /// a named scope) — never at a wider or unrelated scope.
    #[test]
    fn oracle_wall_never_widens_past_its_evidence(
        ops in prop::collection::vec((0usize..4, 0usize..3, 0u8..3), 1..=24),
    ) {
        let members = ["m1", "m2", "m3", "m4"];
        let mut pool = tree_pool();
        let mut evidence_scopes: HashSet<String> = HashSet::new();

        for (i, (member_idx, scope_choice, kind)) in ops.iter().enumerate() {
            let member = members[*member_idx];
            let group = if matches!(member, "m1" | "m2") { "g1" } else { "g2" };
            let scope = match scope_choice {
                0 => member,
                1 => group,
                _ => "root",
            };
            let now = i as u64;
            match kind {
                0 => {
                    evidence_scopes.insert(scope.to_owned());
                    pool.record_outcome(
                        &NodeId::new(member),
                        &serve_failure(&NodeId::new(scope), Some(50)),
                        now,
                    );
                }
                1 => {
                    evidence_scopes.insert(scope.to_owned());
                    pool.record_outcome(&NodeId::new(member), &served_wall(&NodeId::new(scope)), now);
                }
                _ => pool.record_success(&NodeId::new(member), now),
            }
        }

        for wall in pool.walls() {
            let scope = wall.scope.as_str();
            let named = evidence_scopes.contains(scope);
            let narrowed = ancestors_of(scope)
                .iter()
                .any(|a| evidence_scopes.contains(*a));
            prop_assert!(
                named || narrowed,
                "wall at {scope} is neither evidence-named nor narrowing-derived (evidence: {evidence_scopes:?})",
            );
        }
    }

    /// ORACLE (#13693, design §3.5): *a sibling success never
    /// leaves a group wall wider than the failing member.* Checked
    /// at the moment of each success: afterwards, the succeeding
    /// member is gated only by a wall at its own scope, never by a
    /// group wall.
    #[test]
    fn oracle_sibling_success_never_leaves_a_wider_group_wall(
        ops in prop::collection::vec((0usize..4, 0usize..3, 0u8..3), 1..=24),
    ) {
        let members = ["m1", "m2", "m3", "m4"];
        let mut pool = tree_pool();

        for (i, (member_idx, scope_choice, kind)) in ops.iter().enumerate() {
            let member = members[*member_idx];
            let group = if matches!(member, "m1" | "m2") { "g1" } else { "g2" };
            let scope = match scope_choice {
                0 => member,
                1 => group,
                _ => "root",
            };
            let now = i as u64;
            match kind {
                0 => {
                    pool.record_outcome(
                        &NodeId::new(member),
                        &serve_failure(&NodeId::new(scope), Some(50)),
                        now,
                    );
                }
                1 => {
                    pool.record_outcome(&NodeId::new(member), &served_wall(&NodeId::new(scope)), now);
                }
                _ => {
                    pool.record_success(&NodeId::new(member), now);
                    if pool.state_of(&NodeId::new(member), now) == MemberState::Walled {
                        prop_assert!(
                            pool.walls().any(|w| w.scope == NodeId::new(member)),
                            "success by {member} left it gated by a group wall, \
                             not a wall at its own scope",
                        );
                    }
                }
            }
        }
    }

    /// ORACLE (#13717, design §3.5): *a served failure never
    /// advances the walk unless its classifier says so.* The
    /// effect's `advance` is exactly the classifier's decision for
    /// every served disposition; a serve-failure always advances,
    /// whatever the classification's field carries.
    #[test]
    fn oracle_served_failure_advances_only_as_classified(
        cases in prop::collection::vec((0u8..3, prop::bool::ANY), 1..=16),
    ) {
        for (disposition, advance) in cases.iter() {
            let mut pool = tree_pool();
            let m = NodeId::new("m1");
            let disp = match disposition {
                0 => ServedDisposition::Hold,
                1 => ServedDisposition::Demote,
                _ => ServedDisposition::Wall,
            };
            let fx = pool.record_outcome(&m, &served(&m, disp, *advance), 0);
            prop_assert_eq!(fx.advance, *advance);

            let mut cls = serve_failure(&m, None);
            cls.advance = !advance;
            let fx = pool.record_outcome(&m, &cls, 1);
            prop_assert!(fx.advance, "a serve-failure must always advance the walk");
        }
    }
}
