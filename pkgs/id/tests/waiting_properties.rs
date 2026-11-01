//! Property tests for the waiting pool in `src/waiting.rs`.
//!
//! The first property runs the pool and a naive model side by side. The model
//! is a list of held events, scanned on every call, so it is easy to check by
//! eye. Both see the same operations, and their results must match at every
//! step. The second property checks that inserts sharing a timestamp can come in
//! any order. Runs use a fixed seed, so CI draws the same cases every time, and
//! failures are not written to `proptest-regressions/`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeSet, HashMap, HashSet};

use id::waiting::{AuthorKey, WaitRefusal, WaitingLimits, WaitingPool};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

/// Seed for every property in this file. Changing it changes every generated case.
const SEED: u64 = 0x0077_A171_5EED;

/// Cases per property.
const CASES: u32 = 1_024;

/// Authors the generated operations draw from.
const AUTHORS: u8 = 3;

/// Keys per step in the second property. An insert's key is its step times this
/// plus its slot, so keys are unique across the whole run.
const SLOTS_PER_STEP: u32 = 8;

/// The range of keys that dependencies and arrivals are drawn from. Some of these
/// keys are never inserted, so their dependents wait until they expire.
const KEY_SPACE: u32 = 48;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..ProptestConfig::default()
    }
}

/// The 32-byte author key the pool takes, for a small test author number.
const fn author_key(author: u8) -> AuthorKey {
    [author; 32]
}

/// Clock steps are often zero, so many operations share a timestamp.
fn advance() -> impl Strategy<Value = u64> {
    prop_oneof![
        1 => Just(0_u64),
        1 => 1_u64..=12,
    ]
}

/// An event the model holds.
#[derive(Clone, Debug)]
struct Held {
    author: u8,
    key: u32,
    arrived_at: u64,
    deps: Vec<u32>,
    missing: BTreeSet<u32>,
    id: u32,
}

/// A naive reference for the pool: a list of held events, scanned on every call.
struct Model {
    per_author: usize,
    max_age_ms: u64,
    /// Held events in arrival order.
    held: Vec<Held>,
    arrived: BTreeSet<u32>,
    ready: Vec<u32>,
    expired: Vec<(u8, u32)>,
    /// Every event the model has expired, so that none is released later.
    ever_expired: BTreeSet<u32>,
}

impl Model {
    const fn new(per_author: usize, max_age_ms: u64) -> Self {
        Self {
            per_author,
            max_age_ms,
            held: Vec::new(),
            arrived: BTreeSet::new(),
            ready: Vec::new(),
            expired: Vec::new(),
            ever_expired: BTreeSet::new(),
        }
    }

    fn count(&self, author: u8) -> usize {
        self.held
            .iter()
            .filter(|held| held.author == author)
            .count()
    }

    /// Drop the held events whose age has reached the cap at `now`.
    fn expire_due(&mut self, now: u64) {
        let max_age = self.max_age_ms;
        for held in std::mem::take(&mut self.held) {
            if now.saturating_sub(held.arrived_at) >= max_age {
                self.ever_expired.insert(held.id);
                self.expired.push((held.author, held.id));
            } else {
                self.held.push(held);
            }
        }
    }

    /// Returns `true` when the event is refused.
    fn insert(&mut self, author: u8, key: u32, deps: &[u32], id: u32, now: u64) -> bool {
        self.expire_due(now);
        if self.arrived.contains(&key) || self.held.iter().any(|held| held.key == key) {
            return false;
        }
        let missing: BTreeSet<u32> = deps
            .iter()
            .copied()
            .filter(|dep| !self.arrived.contains(dep))
            .collect();
        if missing.is_empty() {
            self.ready.push(id);
            return false;
        }
        if self.count(author) >= self.per_author {
            return true;
        }
        self.held.push(Held {
            author,
            key,
            arrived_at: now,
            deps: deps.to_vec(),
            missing,
            id,
        });
        false
    }

    /// Record an arrival, and return the events it released in arrival order.
    fn arrive(&mut self, key: u32, now: u64) -> Vec<Held> {
        self.expire_due(now);
        self.held.retain(|held| held.key != key);
        self.arrived.insert(key);
        let mut released = Vec::new();
        for mut held in std::mem::take(&mut self.held) {
            held.missing.remove(&key);
            if held.missing.is_empty() {
                released.push(held);
            } else {
                self.held.push(held);
            }
        }
        released
    }

    fn take_ready(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.ready)
    }

    fn expire(&mut self, now: u64) -> Vec<(u8, u32)> {
        self.expire_due(now);
        std::mem::take(&mut self.expired)
    }
}

/// One operation of the first property. The clock advances before each one.
#[derive(Clone, Debug)]
enum Op {
    Insert {
        author: u8,
        key: u32,
        deps: Vec<u32>,
        advance: u64,
    },
    Arrive {
        key: u32,
        advance: u64,
    },
    Expire {
        advance: u64,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (0..AUTHORS, 0_u32..12, prop::collection::vec(0_u32..14, 0..4), advance())
            .prop_map(|(author, key, deps, advance)| Op::Insert { author, key, deps, advance }),
        3 => (0_u32..14, advance()).prop_map(|(key, advance)| Op::Arrive { key, advance }),
        1 => advance().prop_map(|advance| Op::Expire { advance }),
    ]
}

/// Each event is output at most once, by release, by `take_ready`, or by expiry.
fn record_outputs(ids: &[u32], seen: &mut HashSet<u32>) -> Result<(), TestCaseError> {
    for id in ids {
        prop_assert!(seen.insert(*id), "event {id} was output twice");
    }
    Ok(())
}

/// After every operation, no author is over the cap, and each count and the total
/// match the model.
fn check_counts(
    pool: &WaitingPool<u32, u32>,
    model: &Model,
    per_author: usize,
) -> Result<(), TestCaseError> {
    for author in 0..AUTHORS {
        let count = pool.waiting_count(author_key(author));
        prop_assert!(
            count <= per_author,
            "author {author} holds {count}, over the cap of {per_author}"
        );
        prop_assert_eq!(count, model.count(author));
    }
    prop_assert_eq!(pool.len(), model.held.len());
    Ok(())
}

/// Apply `ops` to the pool and the model, and check every result against the model.
fn check_against_model(
    ops: &[Op],
    per_author: usize,
    max_age_ms: u64,
) -> Result<(), TestCaseError> {
    let limits = WaitingLimits::new(per_author, max_age_ms);
    let mut pool: WaitingPool<u32, u32> = WaitingPool::new(limits);
    let mut model = Model::new(per_author, max_age_ms);
    let mut now = 0_u64;
    let mut next_id = 0_u32;
    let mut output = HashSet::new();
    for op in ops {
        match op {
            Op::Insert {
                author,
                key,
                deps,
                advance,
            } => {
                now += advance;
                let id = next_id;
                next_id += 1;
                let refused = model.insert(*author, *key, deps, id, now);
                match pool.insert(author_key(*author), *key, deps.iter().copied(), id, now) {
                    Ok(()) => {
                        prop_assert!(!refused, "the model refused an event the pool accepted");
                    }
                    Err(reason) => {
                        prop_assert_eq!(reason, WaitRefusal::AuthorFull);
                        prop_assert!(refused, "the pool refused an event the model accepted");
                    }
                }
                let ready = pool.take_ready();
                prop_assert_eq!(&ready, &model.take_ready());
                record_outputs(&ready, &mut output)?;
            }
            Op::Arrive { key, advance } => {
                now += advance;
                let released = model.arrive(*key, now);
                let expected: Vec<u32> = released.iter().map(|held| held.id).collect();
                let got = pool.arrived(*key, now);
                prop_assert_eq!(&got, &expected);
                for held in &released {
                    prop_assert!(
                        held.deps.iter().all(|dep| model.arrived.contains(dep)),
                        "event {} was released before all its dependencies arrived",
                        held.id
                    );
                    prop_assert!(
                        !model.ever_expired.contains(&held.id),
                        "expired event {} was released",
                        held.id
                    );
                }
                record_outputs(&got, &mut output)?;
            }
            Op::Expire { advance } => {
                now += advance;
                let expected = model.expire(now);
                let got: Vec<(u8, u32)> = pool
                    .expire(now)
                    .into_iter()
                    .map(|(author, id)| (author[0], id))
                    .collect();
                prop_assert_eq!(&got, &expected);
                let ids: Vec<u32> = got.iter().map(|(_, id)| *id).collect();
                record_outputs(&ids, &mut output)?;
            }
        }
        check_counts(&pool, &model, per_author)?;
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// The pool agrees with the model on every result. The count never exceeds the
    /// cap, a released event has all its dependencies, and an expired event is never
    /// released.
    #[test]
    fn test_random_operations_match_the_model(
        ops in prop::collection::vec(op(), 0..80),
        per_author in 1_usize..=4,
        max_age_ms in 1_u64..=40,
    ) {
        check_against_model(&ops, per_author, max_age_ms)?;
    }
}

/// An insert in the second property. `order` is a sort key, so that the same
/// inserts can be reordered within their step.
#[derive(Clone, Debug)]
struct Entry {
    author: u8,
    key: u32,
    deps: Vec<u32>,
    order: u32,
}

/// One timestamp of the second property: its inserts, then its arrivals, then an expiry
/// sweep, all at the same time.
#[derive(Clone, Debug)]
struct Step {
    advance: u64,
    inserts: Vec<Entry>,
    arrivals: Vec<u32>,
}

fn schedule() -> impl Strategy<Value = Vec<Step>> {
    let entry = (
        0..AUTHORS,
        prop::collection::vec(0..KEY_SPACE, 0..3),
        any::<u32>(),
    );
    let step = (
        advance(),
        prop::collection::vec(entry, 0..=6),
        prop::collection::vec(0..KEY_SPACE, 0..=3),
    );
    prop::collection::vec(step, 1..=6).prop_map(|steps| {
        steps
            .into_iter()
            .enumerate()
            .map(|(index, (advance, entries, arrivals))| {
                let base = u32::try_from(index).unwrap() * SLOTS_PER_STEP;
                let inserts = entries
                    .into_iter()
                    .enumerate()
                    .map(|(slot, (author, deps, order))| Entry {
                        author,
                        key: base + u32::try_from(slot).unwrap(),
                        deps,
                        order,
                    })
                    .collect();
                Step {
                    advance,
                    inserts,
                    arrivals,
                }
            })
            .collect()
    })
}

/// What one step produced. Each list is sorted, so the order of calls does not show.
#[derive(Debug, PartialEq, Eq)]
struct StepResult {
    ready: Vec<u32>,
    released: Vec<u32>,
    expired: Vec<u32>,
}

fn sorted(mut ids: Vec<u32>) -> Vec<u32> {
    ids.sort_unstable();
    ids
}

/// The documented rule: every list the pool returns is in insert order. For a
/// timestamp that is the order the events were inserted.
fn in_insert_order(ids: &[u32], position: &HashMap<u32, usize>) -> Result<(), TestCaseError> {
    let positions: Vec<usize> = ids
        .iter()
        .map(|id| *position.get(id).expect("only inserted events are output"))
        .collect();
    prop_assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{ids:?} is not in insert order"
    );
    Ok(())
}

/// Run `steps` through a pool that never refuses, and return what each step
/// produced.
fn run(steps: &[Step], max_age_ms: u64) -> Result<Vec<StepResult>, TestCaseError> {
    let limits = WaitingLimits::new(1_000, max_age_ms);
    let mut pool: WaitingPool<u32, u32> = WaitingPool::new(limits);
    let mut position: HashMap<u32, usize> = HashMap::new();
    let mut now = 0_u64;
    let mut results = Vec::new();
    for step in steps {
        now += step.advance;
        for entry in &step.inserts {
            let index = position.len();
            position.insert(entry.key, index);
            pool.insert(
                author_key(entry.author),
                entry.key,
                entry.deps.iter().copied(),
                entry.key,
                now,
            )
            .map_err(|reason| TestCaseError::fail(format!("unexpected refusal: {reason}")))?;
        }
        let ready = pool.take_ready();
        in_insert_order(&ready, &position)?;
        let mut released = Vec::new();
        for key in &step.arrivals {
            let batch = pool.arrived(*key, now);
            in_insert_order(&batch, &position)?;
            released.extend(batch);
        }
        let expired: Vec<u32> = pool.expire(now).into_iter().map(|(_, id)| id).collect();
        in_insert_order(&expired, &position)?;
        results.push(StepResult {
            ready: sorted(ready),
            released: sorted(released),
            expired: sorted(expired),
        });
    }
    Ok(results)
}

proptest! {
    #![proptest_config(config())]

    /// Inserts that share a timestamp can come in any order. The ready, released and
    /// expired events are the same as sets, and each returned list is in insert order.
    #[test]
    fn test_equal_timestamp_inserts_give_the_same_result_in_any_order(
        steps in schedule(),
        max_age_ms in 1_u64..=30,
    ) {
        let drawn = run(&steps, max_age_ms)?;
        let reordered: Vec<Step> = steps
            .iter()
            .map(|step| {
                let mut step = step.clone();
                step.inserts.sort_by_key(|entry| entry.order);
                step
            })
            .collect();
        prop_assert_eq!(run(&reordered, max_age_ms)?, drawn);
    }
}

/// The event store's path (#1730): holds go through `restore`, releases
/// through `release`. Releasing any number of applied keys must leave the
/// pool's retained state exactly as it was — the pool used to record every
/// released key in its arrived set forever (+37 MB per 2M arrivals in a
/// pool reporting itself empty). Held events aside, pool state after N
/// releases is independent of N.
#[test]
fn store_path_releases_do_not_grow_the_pool() {
    let mut pool: WaitingPool<u32, u32> = WaitingPool::new(WaitingLimits::default());
    // One held event waiting on key 9, restored the way the store holds.
    pool.restore(author_key(1), 1_000_000, [9], 1, 0).unwrap();
    assert_eq!(pool.len(), 1);
    // A long stream of applied events, none of them key 9.
    for key in (0..10_000u32).filter(|key| *key != 9) {
        assert!(pool.release(key, 0).is_empty());
    }
    assert_eq!(pool.len(), 1, "releases retained state in the pool");
    // The one real dependency still releases the held event...
    assert_eq!(pool.release(9, 0), vec![1]);
    assert!(pool.is_empty());
    // ...and none of the released keys was recorded as arrived: a later
    // insert depending on one still waits for it.
    pool.insert(author_key(1), 1_000_001, [5], 2, 0).unwrap();
    assert!(pool.take_ready().is_empty());
    assert_eq!(pool.len(), 1);
}
