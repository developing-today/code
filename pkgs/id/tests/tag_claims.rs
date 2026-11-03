//! Tags as observed-remove claims over the event store (Phase 5, P5.1; design §9.1, spike S8).
//!
//! The S8 properties, checked on [`EventStore`] through its production path. Each event carries
//! its tag operation in its header, and a receiver gets nothing else (#182):
//!
//! 1. A clear removes the claims it names that its author had seen, and no others.
//! 2. A concurrent add survives a clear that did not see it.
//! 3. A clear that names a claim its author never saw does nothing to that claim.
//! 4. The past is the whole causal past, so a clear whose frontier does not name a claim still
//!    removes it when the frontier's events saw it.
//! 5. The live set is the same under any delivery order, with duplicates.
//!
//! The oracle for 5 is the author's own record. Each writer knows which events it had seen when it
//! wrote a clear, so the expected live set is computed from those records, not from the causal walk.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use id::chain::{ChainExt, Event, sign};
use id::event_store::{EventStore, Ingest, Refusal};
use id::tag_claims::{
    Applied, CLEAR_KIND, Claim, SET_KIND, TagOp, causal_past, compacted_claims, live_claims,
};
use id::waiting::{AuthorKey, WaitingLimits};
use p2panda_core::{Hash, SeqNum, SigningKey};
use proptest::prelude::*;

/// A signer with its own chain. Each event names the one before it, and names its frontier in
/// `after`.
struct Writer {
    key: SigningKey,
    seq: SeqNum,
    last: Option<Hash>,
}

/// One signed event.
#[derive(Clone)]
struct Sent {
    event: Event,
    /// The key that signed it.
    author: AuthorKey,
    /// Which writer signed it.
    writer: usize,
}

impl Sent {
    fn id(&self) -> Hash {
        self.event.id
    }
}

impl Writer {
    fn new(n: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[n; 32]),
            seq: 0,
            last: None,
        }
    }

    fn author(&self) -> AuthorKey {
        *self.key.verifying_key().as_bytes()
    }

    fn send(&mut self, op: &TagOp, after: Vec<Hash>, writer: usize) -> Sent {
        let kind = match op {
            TagOp::Set { .. } => SET_KIND,
            TagOp::Clear { .. } => CLEAR_KIND,
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag: Some(op.clone()),
            document: None,
            removes: None,
            note: None,
        };
        let event = sign(&self.key, self.seq, self.last, None, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        Sent {
            event,
            author: self.author(),
            writer,
        }
    }
}

fn set(subject: Hash, name: &str, value: Option<&str>) -> TagOp {
    TagOp::Set {
        subject,
        name: name.to_owned(),
        value: value.map(str::to_owned),
    }
}

fn clear(claims: &[Hash]) -> TagOp {
    TagOp::Clear {
        claims: claims.to_vec(),
    }
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

/// Delivers one event's bytes, as sync does, and checks that it was not refused.
fn deliver(store: &mut EventStore, sent: &Sent) {
    let outcome = store.ingest(&sent.event.bytes, 0).unwrap();
    assert!(
        !matches!(outcome, Ingest::Refused(_)),
        "refused: {outcome:?}"
    );
}

/// The live claims of `store`, as their IDs.
fn live(store: &EventStore) -> BTreeSet<[u8; 32]> {
    store
        .tag_claims()
        .unwrap()
        .iter()
        .map(|claim| *claim.id.as_bytes())
        .collect()
}

fn ids(sent: &[&Sent]) -> BTreeSet<[u8; 32]> {
    sent.iter().map(|item| *item.id().as_bytes()).collect()
}

#[test]
fn test_a_clear_removes_the_claim_its_author_saw() {
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let seen = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);
    let kept = a.send(&set(subject, "name", None), vec![seen.id()], 0);
    let mut b = Writer::new(2);
    let cleared = b.send(&clear(&[seen.id()]), vec![kept.id()], 1);

    let (_dir, mut store) = open();
    for sent in [&seen, &kept, &cleared] {
        deliver(&mut store, sent);
    }
    assert_eq!(live(&store), ids(&[&kept]));
}

#[test]
fn test_a_concurrent_add_survives_a_clear_that_did_not_see_it() {
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let mut b = Writer::new(2);
    let a_add = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);
    // Written without seeing `a_add`, so it is concurrent with it.
    let b_add = b.send(&set(subject, "city", Some("Seattle")), vec![], 1);
    let a_clear = a.send(&clear(&[a_add.id()]), vec![a_add.id()], 0);

    let (_dir, mut store) = open();
    for sent in [&a_add, &b_add, &a_clear] {
        deliver(&mut store, sent);
    }
    assert_eq!(live(&store), ids(&[&b_add]));
}

#[test]
fn test_a_clear_of_an_unseen_claim_does_nothing_to_it() {
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let mut b = Writer::new(2);
    let unseen = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);
    let own = b.send(&set(subject, "tag", None), vec![], 1);
    // `b` never saw `unseen`, but names it.
    let clears_unseen = b.send(&clear(&[unseen.id()]), vec![own.id()], 1);

    let (_dir, mut store) = open();
    for sent in [&unseen, &own, &clears_unseen] {
        deliver(&mut store, sent);
    }
    assert_eq!(live(&store), ids(&[&unseen, &own]));
}

#[test]
fn test_a_clear_removes_a_claim_its_frontier_reaches_only_through_another_author() {
    // Regression for the whole-causal-past clock (S8). `c` saw `x` through `y`, and its frontier
    // names only `y`. A frontier-only clock would leave `x` live.
    let (s1, s2) = (Hash::digest(b"one"), Hash::digest(b"two"));
    let mut a = Writer::new(1);
    let mut b = Writer::new(2);
    let mut c = Writer::new(3);
    let x = a.send(&set(s1, "city", Some("Seattle")), vec![], 0);
    let y = b.send(&set(s2, "seen", None), vec![x.id()], 1);
    let removes_x = c.send(&clear(&[x.id()]), vec![y.id()], 2);

    let (_dir, mut store) = open();
    for sent in [&x, &y, &removes_x] {
        deliver(&mut store, sent);
    }
    assert_eq!(live(&store), ids(&[&y]));
}

#[test]
fn test_a_clear_delivered_before_the_claim_it_names_still_removes_it() {
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let seen = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);
    let kept = a.send(&set(subject, "name", None), vec![seen.id()], 0);
    let mut b = Writer::new(2);
    let cleared = b.send(&clear(&[seen.id()]), vec![kept.id()], 1);

    let (_dir, mut store) = open();
    for sent in [&cleared, &kept, &seen] {
        deliver(&mut store, sent);
    }
    assert_eq!(live(&store), ids(&[&kept]));
}

#[test]
fn test_a_claim_is_attributed_to_the_key_that_signed_it() {
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let claim = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);

    let (_dir, mut store) = open();
    deliver(&mut store, &claim);
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].author, a.author());
    assert_eq!(claims[0].subject, subject);
    assert_eq!(claims[0].name, "city");
    assert_eq!(claims[0].value.as_deref(), Some("Seattle"));
}

#[test]
fn test_a_tag_event_with_a_payload_body_is_refused() {
    // The operation is in the header, so a body beside a tag event is refused.
    let subject = Hash::digest(b"doc");
    let mut a = Writer::new(1);
    let claim = a.send(&set(subject, "city", Some("Seattle")), vec![], 0);
    let body = b"a body".to_vec();

    let (_dir, mut store) = open();
    let outcome = store
        .ingest_with_body(&claim.event.bytes, Some(&body), 0)
        .unwrap();
    assert!(
        matches!(outcome, Ingest::Refused(Refusal::Malformed(_))),
        "{outcome:?}"
    );
    assert!(live(&store).is_empty());
}

#[test]
fn test_a_tag_event_whose_operation_does_not_fit_its_kind_is_not_signed() {
    let subject = Hash::digest(b"doc");
    let a = Writer::new(1);
    let mismatch = ChainExt {
        kind: CLEAR_KIND.to_owned(),
        group: None,
        after: vec![],
        tag: Some(set(subject, "city", None)),
        document: None,
        removes: None,
        note: None,
    };
    assert!(sign(&a.key, 0, None, None, mismatch).is_err());
    let missing = ChainExt {
        kind: SET_KIND.to_owned(),
        group: None,
        after: vec![],
        tag: None,
        document: None,
        removes: None,
        note: None,
    };
    assert!(sign(&a.key, 0, None, None, missing).is_err());
}

// ---------------------------------------------------------------------------
// Convergence against the writers' own record of what they had seen
// ---------------------------------------------------------------------------

/// SplitMix64. A history is a pure function of its seed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap()).unwrap()
    }
}

/// A generated history: its events in the order they were written, and the expected live set.
struct History {
    events: Vec<Sent>,
    expected: BTreeSet<[u8; 32]>,
    /// For each event, the events its author had seen when it wrote it.
    written: Vec<(Hash, HashSet<Hash>)>,
}

/// Writes a history of three writers, with syncs between them, sets, and clears. A clear names
/// random claims, seen or not. The oracle records the claims each clear's author had seen.
fn history(seed: u64, steps: usize, count: usize) -> History {
    let mut rng = Rng(seed);
    let subjects = [
        Hash::digest(b"s0"),
        Hash::digest(b"s1"),
        Hash::digest(b"s2"),
    ];
    let names = ["city", "name", "a"];
    let values = [None, Some("Seattle"), Some("a\0b"), Some("a"), Some("")];

    let mut writers: Vec<Writer> = (1..=count)
        .map(|n| Writer::new(u8::try_from(n).unwrap()))
        .collect();
    // What each writer has seen: its events by ID, which is causally closed.
    let mut known: Vec<BTreeMap<[u8; 32], Sent>> = vec![BTreeMap::new(); count];
    let mut events: Vec<Sent> = Vec::new();
    let mut sets: Vec<Sent> = Vec::new();
    // Each clear's named claims, and the claims its author had seen when it wrote it.
    let mut clears: Vec<(Vec<Hash>, HashSet<Hash>)> = Vec::new();
    let mut written: Vec<(Hash, HashSet<Hash>)> = Vec::new();

    for _ in 0..steps {
        let w = rng.below(count);
        match rng.below(10) {
            0..=1 => {
                let other = rng.below(count);
                let theirs = known[other].clone();
                known[w].extend(theirs);
            }
            2..=6 => {
                let subject = subjects[rng.below(subjects.len())];
                let name = names[rng.below(names.len())];
                let value = values[rng.below(values.len())];
                let after = frontier(&known[w], count);
                let seen: HashSet<Hash> = known[w].values().map(Sent::id).collect();
                let sent = writers[w].send(&set(subject, name, value), after, w);
                known[w].insert(*sent.id().as_bytes(), sent.clone());
                written.push((sent.id(), seen));
                sets.push(sent.clone());
                events.push(sent);
            }
            _ => {
                if sets.is_empty() {
                    continue;
                }
                let named_count = 1 + rng.below(3);
                let named: Vec<Hash> = (0..named_count)
                    .map(|_| sets[rng.below(sets.len())].id())
                    .collect();
                let seen: HashSet<Hash> = known[w].values().map(Sent::id).collect();
                let after = frontier(&known[w], count);
                let sent = writers[w].send(&clear(&named), after, w);
                known[w].insert(*sent.id().as_bytes(), sent.clone());
                written.push((sent.id(), seen.clone()));
                clears.push((named, seen));
                events.push(sent);
            }
        }
    }

    // The oracle: a set is live unless a clear names it and its author had seen it.
    let removed: HashSet<Hash> = clears
        .iter()
        .flat_map(|(named, seen)| named.iter().filter(|id| seen.contains(id)).copied())
        .collect();
    let expected = sets
        .iter()
        .filter(|sent| !removed.contains(&sent.id()))
        .map(|sent| *sent.id().as_bytes())
        .collect();
    History {
        events,
        expected,
        written,
    }
}

/// The parent links of each event: its `after` hashes and its backlink.
fn parents_of(events: &[Sent]) -> HashMap<Hash, Vec<Hash>> {
    events
        .iter()
        .map(|sent| {
            let header = &sent.event.header;
            (
                sent.id(),
                header
                    .extensions
                    .after
                    .iter()
                    .copied()
                    .chain(header.backlink)
                    .collect(),
            )
        })
        .collect()
}

/// The latest event of each writer that `seen` holds: the frontier an author names in `after`.
fn frontier(seen: &BTreeMap<[u8; 32], Sent>, writers: usize) -> Vec<Hash> {
    (0..writers)
        .filter_map(|writer| {
            seen.values()
                .filter(|sent| sent.writer == writer)
                .max_by_key(|sent| sent.event.header.seq_num)
                .map(Sent::id)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    /// Three stores receive the same history in three orders: as written, reversed (so every event
    /// waits for its causal past), and shuffled with every event repeated. Each reaches the oracle.
    #[test]
    fn test_the_live_set_is_the_oracle_under_any_delivery_order(seed in any::<u64>(), steps in 8usize..32) {
        let history = history(seed, steps, 3);
        let mut rng = Rng(seed ^ 0xD1E5_0C0F_FEE0_0001);

        let mut shuffled: Vec<&Sent> = history.events.iter().flat_map(|sent| [sent, sent]).collect();
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, rng.below(i + 1));
        }
        let orders: [Vec<&Sent>; 3] = [
            history.events.iter().collect(),
            history.events.iter().rev().collect(),
            shuffled,
        ];
        for order in orders {
            let (_dir, mut store) = open();
            for sent in order {
                let outcome = store.ingest(&sent.event.bytes, 0).unwrap();
                prop_assert!(!matches!(outcome, Ingest::Refused(_)), "refused: {outcome:?}");
            }
            prop_assert_eq!(live(&store), history.expected.clone());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

    /// The compacted fold gives the causal walk's live claims, for histories of two or three
    /// writers. Clears name claims their authors had seen and claims they had not, so a clear from
    /// an author who had not seen a claim must leave it live (P5.3, #184).
    #[test]
    fn test_the_compacted_fold_agrees_with_the_causal_walk(seed in any::<u64>(), writers in 2usize..=3, steps in 8usize..40) {
        let history = history(seed, steps, writers);
        let applied: Vec<Applied> = history
            .events
            .iter()
            .map(|sent| Applied {
                id: sent.id(),
                author: sent.author,
                op: sent.event.header.extensions.tag.clone().expect("a tag event carries its operation"),
            })
            .collect();
        let parents: HashMap<Hash, Vec<Hash>> = history
            .events
            .iter()
            .map(|sent| {
                let header = &sent.event.header;
                (sent.id(), header.extensions.after.iter().copied().chain(header.backlink).collect())
            })
            .collect();
        let walked = live_claims(&applied, &parents);
        let folded = compacted_claims(&applied, &parents);
        prop_assert_eq!(&folded, &walked);
        let folded_ids: BTreeSet<[u8; 32]> = folded.iter().map(|claim| *claim.id.as_bytes()).collect();
        prop_assert_eq!(folded_ids, history.expected.clone());
    }
}

/// The histories that shrank the two property failures (P5.1, P5.3), as `(seed, steps, writers)`.
const REGRESSION_HISTORIES: [(u64, usize, usize); 2] = [
    (12_311_497_518_687_219_712, 23, 3),
    (2_831_645_957_844_276_698, 30, 3),
];

#[test]
fn test_each_event_names_every_event_its_author_had_seen() {
    // The causal past of an event, read from its parent links, is the set its author had seen when
    // it wrote it. A frontier that names too few writers leaves part of that set out of the past.
    for (seed, steps, writers) in REGRESSION_HISTORIES {
        let history = history(seed, steps, writers);
        let parents = parents_of(&history.events);
        for (id, seen) in &history.written {
            assert_eq!(
                causal_past(&parents[id], &parents),
                *seen,
                "seed {seed}, event {id:?}"
            );
        }
    }
}

#[test]
fn test_the_regression_histories_reach_the_oracle_through_every_path() {
    for (seed, steps, writers) in REGRESSION_HISTORIES {
        let history = history(seed, steps, writers);
        let (_dir, mut store) = open();
        for sent in &history.events {
            deliver(&mut store, sent);
        }
        assert_eq!(live(&store), history.expected, "seed {seed}: the store");

        let applied: Vec<Applied> = history
            .events
            .iter()
            .map(|sent| Applied {
                id: sent.id(),
                author: sent.author,
                op: sent.event.header.extensions.tag.clone().unwrap(),
            })
            .collect();
        let parents = parents_of(&history.events);
        let ids = |claims: Vec<Claim>| -> BTreeSet<[u8; 32]> {
            claims.iter().map(|claim| *claim.id.as_bytes()).collect()
        };
        assert_eq!(
            ids(live_claims(&applied, &parents)),
            history.expected,
            "seed {seed}: the causal walk"
        );
        assert_eq!(
            ids(compacted_claims(&applied, &parents)),
            history.expected,
            "seed {seed}: the compacted fold"
        );
    }
}
