//! Property tests for the S8 observed-remove tag set. Each property says why it matters.

use std::collections::BTreeSet;

use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use s8_tags::{Body, Device, Dot, Entry, Op, Replica, Rng, State, join, reference};

/// Cases per property. Set S8_CASES for a longer run.
fn cases() -> u32 {
    std::env::var("S8_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(300)
}

fn device(i: usize) -> Device {
    let mut d = [0u8; 32];
    d[0] = u8::try_from(i + 1).expect("few devices");
    d
}

/// Subjects 0..2, names "a"/"ab"/"b", values none/"x"/"y".
fn entry(subject: u8, name: u8, value: u8) -> Entry {
    let mut s = [0u8; 32];
    s[0] = subject;
    Entry {
        subject: s,
        name: ["a", "ab", "b"][usize::from(name) % 3].to_owned(),
        value: [None, Some("x"), Some("y")][usize::from(value) % 3].map(str::to_owned),
    }
}

/// Deliver `ops` to `rep` in a random order, with about a third of them sent twice.
fn deliver(rep: &mut Replica, ops: &[Op], rng: &mut Rng) {
    let mut batch = ops.to_vec();
    batch.extend(ops.iter().filter(|_| rng.below(3) == 0).cloned());
    rng.shuffle(&mut batch);
    for op in batch {
        rep.receive(op);
    }
}

#[derive(Debug, Clone)]
enum Act {
    Set {
        r: usize,
        subject: u8,
        name: u8,
        value: u8,
    },
    /// Names some of the replica's live dots (observed) and some dots issued anywhere (maybe unseen).
    Clear {
        r: usize,
        observed: Vec<u8>,
        unseen: Vec<u8>,
    },
    Sync {
        from: usize,
        to: usize,
        by_state: bool,
        seed: u64,
    },
    Toggle {
        r: usize,
    },
    Heal,
}

fn act(n: usize) -> impl Strategy<Value = Act> {
    prop_oneof![
        3 => (0..n, 0u8..2, 0u8..3, 0u8..3)
            .prop_map(|(r, subject, name, value)| Act::Set { r, subject, name, value }),
        2 => (0..n, vec(any::<u8>(), 0..3), vec(any::<u8>(), 0..3))
            .prop_map(|(r, observed, unseen)| Act::Clear { r, observed, unseen }),
        4 => (0..n, 0..n, any::<bool>(), any::<u64>())
            .prop_map(|(from, to, by_state, seed)| Act::Sync { from, to, by_state, seed }),
        1 => (0..n).prop_map(|r| Act::Toggle { r }),
        1 => Just(Act::Heal),
    ]
}

/// A random run. Replicas issue ops; syncs run only inside a partition, as either op or state exchange.
struct Sim {
    reps: Vec<Replica>,
    part: Vec<bool>,
    issued: Vec<Op>,
    /// Every replica state after every step. The merge-law test picks from these.
    snapshots: Vec<State>,
    rng: Rng,
}

impl Sim {
    fn new(n: usize, seed: u64) -> Self {
        Self {
            reps: (0..n).map(|i| Replica::new(device(i))).collect(),
            part: vec![false; n],
            issued: Vec::new(),
            snapshots: Vec::new(),
            rng: Rng::new(seed),
        }
    }

    /// Apply one action, then check each replica against the reference over its applied ops.
    fn step(&mut self, act: &Act) -> Result<(), String> {
        match *act {
            Act::Set { r, subject, name, value } => {
                let op = self.reps[r].set(entry(subject, name, value));
                self.issued.push(op);
            }
            Act::Clear { r, ref observed, ref unseen } => {
                let live: Vec<Dot> = self.reps[r].state.live.keys().copied().collect();
                let every_set: Vec<Dot> = self
                    .issued
                    .iter()
                    .filter(|op| matches!(op.body, Body::Set(_)))
                    .map(|op| op.dot)
                    .collect();
                let mut names = BTreeSet::new();
                if !live.is_empty() {
                    names.extend(observed.iter().map(|b| live[usize::from(*b) % live.len()]));
                }
                if !every_set.is_empty() {
                    names.extend(unseen.iter().map(|b| every_set[usize::from(*b) % every_set.len()]));
                }
                let op = self.reps[r].clear(names);
                self.issued.push(op);
            }
            Act::Sync { from, to, by_state, seed } => {
                if from != to && self.part[from] == self.part[to] {
                    if by_state {
                        let src = self.reps[from].clone();
                        self.reps[to].merge(&src);
                    } else {
                        let ops: Vec<Op> = self.reps[from].applied.values().cloned().collect();
                        deliver(&mut self.reps[to], &ops, &mut Rng::new(seed));
                    }
                }
            }
            Act::Toggle { r } => self.part[r] = !self.part[r],
            Act::Heal => self.part.iter_mut().for_each(|p| *p = false),
        }
        self.snapshots.extend(self.reps.iter().map(|rep| rep.state.clone()));
        self.check()
    }

    fn check(&self) -> Result<(), String> {
        for (i, rep) in self.reps.iter().enumerate() {
            let want = reference(rep.applied.values());
            if rep.state != want {
                return Err(format!(
                    "replica {i} differs from the reference over its applied ops\n state: {:?}\n want:  {:?}",
                    rep.state, want
                ));
            }
        }
        Ok(())
    }

    /// Join every replica's state into a fresh observer in random order, then heal and deliver
    /// every issued op to every replica in random order. Everything must match the reference.
    fn settle(&mut self) -> Result<(), String> {
        let mut order: Vec<usize> = (0..self.reps.len()).collect();
        self.rng.shuffle(&mut order);
        let mut observer = Replica::new(device(self.reps.len()));
        for i in order {
            observer.merge(&self.reps[i]);
        }
        if observer.state != reference(observer.applied.values()) {
            return Err("state-based join differs from the reference over the union of applied ops".into());
        }

        self.part.iter_mut().for_each(|p| *p = false);
        let want = reference(self.issued.iter());
        for rep in &mut self.reps {
            deliver(rep, &self.issued, &mut self.rng);
        }
        for (i, rep) in self.reps.iter().enumerate() {
            if rep.state != want || rep.pending_len() != 0 {
                return Err(format!("replica {i} did not converge\n state: {:?}\n want:  {:?}", rep.state, want));
            }
        }
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Convergence. Partitions create concurrency. Every replica matches the reference at every step,
    /// and all converge after a heal, whatever order (with duplicates) the ops arrive in.
    #[test]
    fn convergence_under_partitions_and_any_delivery_order(
        (n, acts, seed) in (3usize..=5).prop_flat_map(|n| (Just(n), vec(act(n), 1..80), any::<u64>())),
    ) {
        let mut sim = Sim::new(n, seed);
        for a in &acts {
            sim.step(a).map_err(TestCaseError::fail)?;
        }
        sim.settle().map_err(TestCaseError::fail)?;
    }

    /// Add-wins. A clear removes only the add its author saw. Adds concurrent with it survive,
    /// whatever the order. Without this, a tag set on one device could be erased by a clear
    /// issued on another device that never saw it.
    #[test]
    fn concurrent_adds_survive_a_clear_that_did_not_see_them(k in 1usize..=3, seed in any::<u64>()) {
        let n = k + 2; // 0: author, 1: clearer, 2..n: concurrent adders. Index n is an observer.
        let mut reps: Vec<Replica> = (0..=n).map(|i| Replica::new(device(i))).collect();
        let e = entry(0, 0, 0);
        let first = reps[0].set(e.clone());
        for rep in &mut reps[1..n] {
            rep.receive(first.clone());
        }
        let clear = reps[1].clear(BTreeSet::from([first.dot]));
        let mut ops = vec![first.clone(), clear];
        let mut expected = BTreeSet::new();
        for rep in &mut reps[2..n] {
            let add = rep.set(e.clone());
            expected.insert(add.dot);
            ops.push(add);
        }
        let mut rng = Rng::new(seed);
        for rep in &mut reps {
            deliver(rep, &ops, &mut rng);
        }
        for (i, rep) in reps.iter().enumerate() {
            let live: BTreeSet<Dot> = rep.state.live.keys().copied().collect();
            prop_assert_eq!(&live, &expected, "replica {} disagrees", i);
        }
    }

    /// Unobserved removal. A clear may name dots its author never saw. Those must survive, and the
    /// dots it did see must go. Without this, a stale or mistaken claim ID could delete a later add.
    #[test]
    fn clear_naming_unseen_dots_does_not_remove_them(
        seen in 0usize..=3,
        unseen in 1usize..=3,
        seed in any::<u64>(),
    ) {
        let mut author = Replica::new(device(0));
        let mut clearer = Replica::new(device(1));
        let mut ops = Vec::new();
        let mut seen_dots = BTreeSet::new();
        for i in 0..seen {
            let op = author.set(entry(0, i as u8, 0));
            clearer.receive(op.clone());
            seen_dots.insert(op.dot);
            ops.push(op);
        }
        let mut unseen_dots = BTreeSet::new();
        for i in 0..unseen {
            let op = author.set(entry(1, i as u8, 0));
            unseen_dots.insert(op.dot);
            ops.push(op);
        }
        let names: BTreeSet<Dot> = seen_dots.union(&unseen_dots).copied().collect();
        ops.push(clearer.clear(names));

        let mut rng = Rng::new(seed);
        let mut observer = Replica::new(device(2));
        deliver(&mut observer, &ops, &mut rng);
        deliver(&mut author, &ops, &mut rng);
        for rep in [&observer, &author] {
            let live: BTreeSet<Dot> = rep.state.live.keys().copied().collect();
            prop_assert_eq!(live, unseen_dots.clone());
        }
    }

    /// Merge laws. The join is commutative, associative and idempotent on reachable states.
    /// Without these, replicas that sync in different groupings could end in different states.
    #[test]
    fn join_is_commutative_associative_and_idempotent(
        (n, acts, seed, picks) in (3usize..=5).prop_flat_map(|n| {
            (Just(n), vec(act(n), 1..60), any::<u64>(), any::<[usize; 3]>())
        }),
    ) {
        let mut sim = Sim::new(n, seed);
        for a in &acts {
            sim.step(a).map_err(TestCaseError::fail)?;
        }
        let s = &sim.snapshots;
        let a = &s[picks[0] % s.len()];
        let b = &s[picks[1] % s.len()];
        let c = &s[picks[2] % s.len()];
        prop_assert_eq!(join(a, b), join(b, a));
        prop_assert_eq!(join(&join(a, b), c), join(a, &join(b, c)));
        prop_assert_eq!(join(a, a), a.clone());
    }
}

/// A late copy of a removed add must not come back. This is the case a tombstone would cover,
/// so it checks that the version vector covers it instead.
#[test]
fn late_duplicate_of_a_removed_add_is_dropped() {
    let mut x = Replica::new(device(0));
    let mut y = Replica::new(device(1));
    let add = x.set(entry(0, 0, 0));
    y.receive(add.clone());
    let clear = y.clear(BTreeSet::from([add.dot]));

    let mut z = Replica::new(device(2));
    z.receive(clear);
    assert_eq!(z.pending_len(), 1, "the clear waits for the add it names");
    z.receive(add.clone());
    assert!(z.state.live.is_empty());
    z.receive(add);
    assert!(z.state.live.is_empty());
    assert_eq!(z.pending_len(), 0);
}
