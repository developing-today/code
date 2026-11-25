//! Strategy tests for `fleet-pool` slice 2 (#15066): the weighted
//! and free-first strategies and `Tiered` composition (design
//! #13752 §3.2). Property tests name their oracles.

use fleet_pool::{
    Candidate, FreeFirst, MemberState, NodeId, Pool, RoundRobin, Strategy as PoolStrategy, Tiered,
    Weighted,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn cand(id: &str, cost_class: u32) -> Candidate {
    Candidate {
        id: NodeId::new(id),
        state: MemberState::Serving,
        load: None,
        cost_class,
    }
}

fn count_picks<S: PoolStrategy>(
    strategy: &mut S,
    candidates: &[Candidate],
    n: usize,
) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..n {
        let idx = strategy
            .select(candidates)
            .expect("candidates are non-empty");
        *counts.entry(candidates[idx].id.to_string()).or_insert(0) += 1;
    }
    counts
}

#[test]
fn weighted_gives_exact_shares_over_full_periods() {
    let candidates = vec![cand("a", 0), cand("b", 0)];
    let mut strategy = Weighted::new([(NodeId::new("a"), 2), (NodeId::new("b"), 1)]);

    // Smooth weighted round-robin: the first period is a, b, a.
    let period: Vec<String> = (0..3)
        .map(|_| {
            let idx = strategy.select(&candidates).expect("non-empty");
            candidates[idx].id.to_string()
        })
        .collect();
    assert_eq!(period, vec!["a", "b", "a"]);

    // Ten more periods: shares stay exact (2:1), never statistical.
    let counts = count_picks(&mut strategy, &candidates, 30);
    assert_eq!(counts["a"], 20);
    assert_eq!(counts["b"], 10);
}

#[test]
fn weighted_zero_weight_parks_the_member() {
    let candidates = vec![cand("a", 0), cand("b", 0)];
    let mut strategy = Weighted::new([(NodeId::new("a"), 0), (NodeId::new("b"), 1)]);
    let counts = count_picks(&mut strategy, &candidates, 5);
    assert_eq!(counts.get("a"), None);
    assert_eq!(counts["b"], 5);

    // Every weight zero degrades to declaration order.
    let mut strategy = Weighted::new([(NodeId::new("a"), 0), (NodeId::new("b"), 0)]);
    assert_eq!(strategy.select(&candidates), Some(0));
}

#[test]
fn weighted_unlisted_member_weighs_one() {
    let candidates = vec![cand("a", 0), cand("b", 0)];
    let mut strategy = Weighted::new([(NodeId::new("a"), 3)]);
    assert_eq!(strategy.weight_of(&NodeId::new("b")), 1);
    let counts = count_picks(&mut strategy, &candidates, 8);
    assert_eq!(counts["a"], 6);
    assert_eq!(counts["b"], 2);
}

#[test]
fn free_first_picks_the_lowest_class_then_declaration_order() {
    let candidates = vec![cand("paid", 1), cand("free-a", 0), cand("free-b", 0)];
    let mut strategy = FreeFirst;
    assert_eq!(strategy.select(&candidates), Some(1));
    // Stateless: the same field gives the same answer.
    assert_eq!(strategy.select(&candidates), Some(1));
}

#[test]
fn tiered_weighted_is_the_model_pool_shape() {
    // Free-first, then weighted within class (design §3.2): the
    // paid member's weight of 100 buys it nothing while any free
    // member is available.
    let candidates = vec![cand("free-a", 0), cand("free-b", 0), cand("paid", 1)];
    let mut strategy = Tiered::new(Weighted::new([
        (NodeId::new("free-a"), 1),
        (NodeId::new("free-b"), 3),
        (NodeId::new("paid"), 100),
    ]));
    let counts = count_picks(&mut strategy, &candidates, 40);
    assert_eq!(counts.get("paid"), None);
    assert_eq!(counts["free-a"], 10);
    assert_eq!(counts["free-b"], 30);
}

#[test]
fn tiered_round_robin_is_the_host_shape() {
    // Ordered tiers, round-robin within the tier (design §3.2).
    let candidates = vec![cand("x", 0), cand("y", 0), cand("z", 1)];
    let mut strategy = Tiered::new(RoundRobin::default());
    let picks: Vec<String> = (0..4)
        .map(|_| {
            let idx = strategy.select(&candidates).expect("non-empty");
            candidates[idx].id.to_string()
        })
        .collect();
    assert_eq!(picks, vec!["x", "y", "x", "y"]);
}

#[test]
fn tiered_falls_through_when_the_best_tier_is_absent() {
    let candidates = vec![cand("z", 1), cand("w", 1)];
    let mut strategy = Tiered::new(FreeFirst);
    assert_eq!(strategy.select(&candidates), Some(0));
}

#[test]
fn pool_select_with_tiered_weighted_stays_in_the_free_class() {
    let mut pool = Pool::new();
    pool.add_member_with_cost("free-a", None, 0);
    pool.add_member_with_cost("free-b", None, 0);
    pool.add_member_with_cost("paid-c", None, 1);
    let mut strategy = Tiered::new(Weighted::new([
        (NodeId::new("free-a"), 1),
        (NodeId::new("free-b"), 3),
        (NodeId::new("paid-c"), 100),
    ]));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for i in 0..40 {
        let picked = pool
            .select(&mut strategy, &|_| None, i)
            .expect("free members are available");
        *counts.entry(picked.to_string()).or_insert(0) += 1;
    }
    assert_eq!(counts.get("paid-c"), None);
    assert_eq!(counts["free-a"], 10);
    assert_eq!(counts["free-b"], 30);
    // The decision log names the composed strategy.
    assert!(
        pool.decisions()
            .iter()
            .all(|d| d.strategy == "tiered(weighted)")
    );
}

proptest! {
    /// ORACLE (design §3.2): *weighted shares are the declared
    /// weights.* Over whole periods against a constant field, each
    /// member's pick count is exactly weight × periods.
    #[test]
    fn oracle_weighted_shares_are_exact(
        weights in prop::collection::vec(1u32..=4, 2..=4),
        periods in 1usize..=5,
    ) {
        let candidates: Vec<Candidate> = weights
            .iter()
            .enumerate()
            .map(|(i, _)| cand(&format!("m{i}"), 0))
            .collect();
        let mut strategy = Weighted::new(
            weights
                .iter()
                .enumerate()
                .map(|(i, w)| (NodeId::new(format!("m{i}")), *w)),
        );
        let total: usize = weights.iter().map(|w| *w as usize).sum();
        let counts = count_picks(&mut strategy, &candidates, total * periods);
        for (i, w) in weights.iter().enumerate() {
            prop_assert_eq!(counts[&format!("m{i}")], *w as usize * periods);
        }
    }

    /// ORACLE (design §3.2): *free-first never selects outside the
    /// lowest cost class present.*
    #[test]
    fn oracle_free_first_never_leaves_the_lowest_class(
        classes in prop::collection::vec(0u32..=3, 1..=8),
    ) {
        let candidates: Vec<Candidate> = classes
            .iter()
            .enumerate()
            .map(|(i, c)| cand(&format!("m{i}"), *c))
            .collect();
        let mut strategy = FreeFirst;
        let picked = strategy.select(&candidates).expect("candidates are non-empty");
        let min_class = classes.iter().min().copied().unwrap();
        prop_assert_eq!(candidates[picked].cost_class, min_class);
        // Within the class, declaration order: the first minimum.
        prop_assert_eq!(picked, classes.iter().position(|c| *c == min_class).unwrap());
    }
}
