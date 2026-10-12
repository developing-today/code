//! Property tests for the membership layer (plan P2.1 and P2.2).
//!
//! A scenario is a random causal DAG: each event names some earlier events in `after`, and some of
//! the events are group operations of one of two groups. The mapping is checked against a
//! brute-force walk of each event's full causal past.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeSet, HashMap, HashSet};

use id::membership::frontier::CausalIndex;
use p2panda_core::Hash;
use proptest::prelude::*;

/// SplitMix64. A scenario is a pure function of its seed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Cases per property. `PROPTEST_CASES` raises it.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256);
    ProptestConfig {
        cases,
        ..ProptestConfig::default()
    }
}

/// One event of a random DAG. `group` is `Some` for a group operation.
#[derive(Clone, Debug)]
struct DagEvent {
    id: Hash,
    after: Vec<Hash>,
    group: Option<Hash>,
}

fn group_hash(n: u64) -> Hash {
    Hash::digest([n as u8])
}

/// A random causal DAG of `size` events. Event `i` names up to four earlier events, and about two
/// thirds of the events are group operations of one of two groups.
fn random_dag(seed: u64, size: usize) -> Vec<DagEvent> {
    let mut rng = Rng(seed);
    let mut events: Vec<DagEvent> = Vec::with_capacity(size);
    for i in 0..size {
        let id = Hash::digest((i as u32).to_be_bytes());
        let mut after: Vec<Hash> = Vec::new();
        if i > 0 {
            let wanted = rng.below(4.min(i as u64) + 1);
            for _ in 0..wanted {
                let pick = events[rng.below(i as u64) as usize].id;
                if !after.contains(&pick) {
                    after.push(pick);
                }
            }
        }
        let group = (rng.below(3) != 0).then(|| group_hash(rng.below(2)));
        events.push(DagEvent { id, after, group });
    }
    events
}

/// Every event in the causal past of `id`, walked through `after`. Excludes `id`.
fn past(by_id: &HashMap<Hash, &DagEvent>, id: Hash) -> HashSet<Hash> {
    let mut seen = HashSet::new();
    let mut stack = by_id[&id].after.clone();
    while let Some(next) = stack.pop() {
        if seen.insert(next) {
            stack.extend(by_id[&next].after.iter().copied());
        }
    }
    seen
}

/// The reference frontier: the maximal group operations of `group` in the full causal past of
/// `event`, sorted. Computed by walking the whole past, with no memo.
fn reference_frontier(dag: &[DagEvent], event: &DagEvent, group: Hash) -> Vec<Hash> {
    let by_id: HashMap<Hash, &DagEvent> = dag.iter().map(|e| (e.id, e)).collect();
    let candidates: Vec<Hash> = past_of_after(&by_id, &event.after)
        .into_iter()
        .filter(|id| by_id[id].group == Some(group))
        .collect();
    let mut maximal: Vec<Hash> = candidates
        .iter()
        .filter(|candidate| {
            !candidates
                .iter()
                .any(|other| other != *candidate && past(&by_id, *other).contains(*candidate))
        })
        .copied()
        .collect();
    maximal.sort();
    maximal
}

/// The past of an event whose `after` is `after`: the `after` hashes and everything below them.
fn past_of_after(by_id: &HashMap<Hash, &DagEvent>, after: &[Hash]) -> HashSet<Hash> {
    let mut seen: HashSet<Hash> = HashSet::new();
    let mut stack = after.to_vec();
    while let Some(next) = stack.pop() {
        if seen.insert(next) {
            stack.extend(by_id[&next].after.iter().copied());
        }
    }
    seen
}

proptest! {
    #![proptest_config(config())]

    /// The memoised, incremental frontier equals the full walk for every event, in causal order,
    /// for both groups. The memo is shared across calls, so this also checks that reuse is sound.
    #[test]
    fn test_frontier_equals_the_full_walk_for_every_event_and_group(
        seed in any::<u64>(),
        size in 1usize..=40,
    ) {
        let dag = random_dag(seed, size);
        let mut index = CausalIndex::new();
        for event in &dag {
            for group in [group_hash(0), group_hash(1)] {
                let got = index.frontier(&event.after, group).unwrap();
                let want = reference_frontier(&dag, event, group);
                prop_assert_eq!(got, want, "event {} group {}", event.id, group);
            }
            index.insert(event.id, event.after.clone(), event.group).unwrap();
        }
    }

    /// The frontier never names an event outside the group, never names an event that is in the
    /// past of another frontier member, and never names anything that is not in the past.
    #[test]
    fn test_frontier_is_an_antichain_of_group_operations_in_the_past(
        seed in any::<u64>(),
        size in 1usize..=40,
    ) {
        let dag = random_dag(seed, size);
        let by_id: HashMap<Hash, &DagEvent> = dag.iter().map(|e| (e.id, e)).collect();
        let mut index = CausalIndex::new();
        for event in &dag {
            let group = group_hash(0);
            let frontier = index.frontier(&event.after, group).unwrap();
            let below = past_of_after(&by_id, &event.after);
            let unique: BTreeSet<Hash> = frontier.iter().copied().collect();
            prop_assert_eq!(unique.len(), frontier.len());
            for member in &frontier {
                prop_assert!(below.contains(member));
                prop_assert_eq!(by_id[member].group, Some(group));
                for other in &frontier {
                    if other != member {
                        prop_assert!(!past(&by_id, *other).contains(member));
                    }
                }
            }
            index.insert(event.id, event.after.clone(), event.group).unwrap();
        }
    }
}
