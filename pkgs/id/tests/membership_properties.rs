//! Property tests for the membership layer (plan P2.1 and P2.2).
//!
//! A scenario is a random causal DAG: each event names some earlier events in `after`, and some of
//! the events are group operations of one of two groups. The mapping is checked against a
//! brute-force walk of each event's full causal past.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use id::membership::frontier::CausalIndex;
use id::membership::{AuthId, CausalEvent, Membership, Outcome, Role};
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
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

/// The identity of event `i`, which is also the ID of the group it creates, for `i` in 0 and 1.
fn id_of(i: usize) -> Hash {
    Hash::digest((i as u32).to_be_bytes())
}

fn key(n: u64) -> AuthId {
    AuthId::Key([n as u8; 32])
}

fn access_of(level: u64) -> Access<()> {
    match level {
        0 => Access::read(),
        1 => Access::write(),
        _ => Access::manage(),
    }
}

fn create_role(members: &[(u64, u64)]) -> Role {
    Role::Auth {
        author: [1; 32],
        group: None,
        action: GroupAction::Create {
            initial_members: members
                .iter()
                .map(|(n, level)| (GroupMember::Individual(key(*n)), access_of(*level)))
                .collect(),
        },
    }
}

/// A random membership scenario of `size` events. Event 0 creates group 0. Event 1 sometimes
/// creates group 1. Later events are membership operations by random authors, some of whom are not
/// members, on random groups, or other events. Each names up to four earlier events. Some
/// operations are invalid where they sit, so the layer refuses them.
fn scenario(seed: u64, size: usize) -> Vec<CausalEvent> {
    let mut rng = Rng(seed);
    let mut events: Vec<CausalEvent> = Vec::with_capacity(size);
    let mut groups = vec![id_of(0)];
    for i in 0..size {
        let mut after: Vec<Hash> = Vec::new();
        if i > 0 {
            for _ in 0..rng.below(5) {
                let pick = events[rng.below(i as u64) as usize].id;
                if !after.contains(&pick) {
                    after.push(pick);
                }
            }
        }
        let role = if i == 0 {
            create_role(&[(1, 2), (2, 2), (3, 2)])
        } else if i == 1 && rng.below(2) == 0 {
            groups.push(id_of(1));
            create_role(&[(1, 2), (2, 0)])
        } else if rng.below(4) == 0 {
            Role::Other
        } else {
            let group = groups[rng.below(groups.len() as u64) as usize];
            let member = key(1 + rng.below(5));
            let access = access_of(rng.below(3));
            let action = match rng.below(4) {
                0 => GroupAction::Add {
                    member: GroupMember::Individual(member),
                    access,
                },
                1 => GroupAction::Remove {
                    member: GroupMember::Individual(member),
                },
                2 => GroupAction::Promote {
                    member: GroupMember::Individual(member),
                    access,
                },
                _ => GroupAction::Demote {
                    member: GroupMember::Individual(member),
                    access,
                },
            };
            Role::Auth {
                author: [1 + rng.below(4) as u8; 32],
                group: Some(group),
                action,
            }
        };
        events.push(CausalEvent {
            id: id_of(i),
            after,
            role,
        });
    }
    events
}

/// What one arrival order produces.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    /// The final outcome of each event that was decided.
    decided: BTreeMap<Hash, Outcome>,
    /// The members of groups 0 and 1, sorted.
    members: Vec<(Hash, Vec<(AuthId, Access<()>)>)>,
    /// The auth frontier the layer derived for each decided event.
    dependencies: BTreeMap<Hash, Vec<Hash>>,
    /// How many events are still held after the last arrival.
    held: usize,
}

/// Delivers `arrivals` to a fresh layer, in order. Each arrival is one event by index.
fn run(events: &[CausalEvent], arrivals: &[usize]) -> Run {
    let mut layer = Membership::new();
    let mut decided = BTreeMap::new();
    for &i in arrivals {
        for (id, outcome) in layer.receive(events[i].clone()).unwrap() {
            if !matches!(outcome, Outcome::Held(_) | Outcome::Duplicate) {
                decided.insert(id, outcome);
            }
        }
    }
    let members = [id_of(0), id_of(1)]
        .into_iter()
        .map(|group| {
            let mut list = layer.members(group);
            list.sort_by_key(|(id, _)| *id);
            (group, list)
        })
        .collect();
    let dependencies = decided
        .keys()
        .filter_map(|id| layer.dependencies(id).map(|deps| (*id, deps.to_vec())))
        .collect();
    Run {
        decided,
        members,
        dependencies,
        held: layer.held_len(),
    }
}

/// Fisher-Yates shuffle with the scenario generator.
fn shuffle(list: &mut [usize], rng: &mut Rng) {
    for i in (1..list.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        list.swap(i, j);
    }
}

proptest! {
    #![proptest_config(config())]

    /// Every arrival order decides every event, and reaches the same outcomes, the same state, and
    /// the same derived dependencies. Arrivals are random, and some are repeated.
    #[test]
    fn test_every_arrival_order_gives_the_same_outcomes_and_state(
        seed in any::<u64>(),
        size in 1usize..=30,
        order_seed in any::<u64>(),
    ) {
        let events = scenario(seed, size);
        let causal: Vec<usize> = (0..events.len()).collect();
        let canonical = run(&events, &causal);
        prop_assert_eq!(canonical.held, 0);
        prop_assert_eq!(canonical.decided.len(), events.len());

        let mut rng = Rng(order_seed);
        for _ in 0..3 {
            let mut arrivals: Vec<usize> = (0..events.len()).collect();
            shuffle(&mut arrivals, &mut rng);
            for _ in 0..rng.below(4) {
                let repeat = rng.below(events.len() as u64) as usize;
                let at = rng.below(arrivals.len() as u64 + 1) as usize;
                arrivals.insert(at, repeat);
            }
            let got = run(&events, &arrivals);
            prop_assert_eq!(got.held, 0, "arrivals {:?}", arrivals);
            prop_assert_eq!(got.decided.len(), events.len(), "arrivals {:?}", arrivals);
            prop_assert_eq!(&got.decided, &canonical.decided, "arrivals {:?}", arrivals);
            prop_assert_eq!(&got.members, &canonical.members, "arrivals {:?}", arrivals);
            prop_assert_eq!(&got.dependencies, &canonical.dependencies, "arrivals {:?}", arrivals);
        }
    }
}
