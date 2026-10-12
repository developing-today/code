//! Property tests for ingest in `src/event_store/ingest.rs` (plan P1.8).
//!
//! A scenario is a set of signed events: valid chains, forks at any depth, and rivals whose
//! backlink is anything: the original's, an earlier event's, or a hash that is never
//! delivered. An `after` hash names any earlier event, or one that is never delivered. The
//! scenario is delivered in four orders, with repeats, to fresh stores.
//!
//! Every store must reach the applied state that [`model`] computes from the set of events
//! alone, so the answer cannot depend on the order. The model is the specification. An event
//! is applied when it sits below its author's fork point, its backlink is the applied event
//! before it, and every `after` hash is applied. The fork point is the lowest sequence number
//! where the author has two distinct events, and the proof holds the two lowest IDs there.
//!
//! Failures are not written to `proptest-regressions/`. The minimal input goes into
//! `tests/regressions/ingest.txt` instead (see `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::ingest::{deliver_within, Canonical, DELIVERY_LIMIT};
use id::chain::{sign, ChainExt, Event};
use id::event_store::Chain;
use id::waiting::AuthorKey;
use p2panda_core::{Hash, SeqNum, SigningKey};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

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

    /// True in about one case in `n`.
    fn chance(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }
}

/// Cases per property. A debug build runs redb unoptimised, at about a second a case, so the
/// default is small. `PROPTEST_CASES` raises it. A release run (`--release`) goes much further.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(32);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A scenario: every distinct signed event, and the applied state they determine.
struct Scenario {
    /// Every signed event, originals and rivals, in creation order. Each one names only
    /// events created before it.
    events: Vec<Event>,
    /// The authors' public keys.
    authors: Vec<AuthorKey>,
    /// The applied state that [`model`] determines from `events`.
    expected: Canonical,
}

fn author_of(event: &Event) -> AuthorKey {
    *event.header.verifying_key.as_bytes()
}

/// A hash that no event in the scenario has: a dependency that is never delivered.
fn undelivered(rng: &mut Rng) -> Hash {
    Hash::digest(rng.next_u64().to_be_bytes())
}

/// Up to three `after` hashes: earlier events, or hashes that are never delivered.
fn pick_after(rng: &mut Rng, known: &[Event]) -> Vec<Hash> {
    let mut picked = Vec::new();
    for _ in 0..rng.below(4) {
        let hash = if known.is_empty() || rng.chance(5) {
            undelivered(rng)
        } else {
            known[rng.below(known.len() as u64) as usize].id
        };
        if !picked.contains(&hash) {
            picked.push(hash);
        }
    }
    picked
}

/// The backlink of a rival at `seq`. It is the original's own backlink, or the ID of an
/// earlier event (of this author or another, applied or not), or a hash never delivered.
fn rival_backlink(
    rng: &mut Rng,
    seq: SeqNum,
    original: Option<Hash>,
    known: &[Event],
) -> Option<Hash> {
    if seq == 0 {
        return None;
    }
    match rng.below(4) {
        0 | 1 => original,
        2 if !known.is_empty() => Some(known[rng.below(known.len() as u64) as usize].id),
        _ => Some(undelivered(rng)),
    }
}

/// Signs an event with a random group and `after` hashes, drawn from `known`.
fn signed(
    rng: &mut Rng,
    key: &SigningKey,
    seq: SeqNum,
    backlink: Option<Hash>,
    known: &[Event],
    kind: &str,
) -> Event {
    let group = rng
        .chance(3)
        .then(|| Hash::digest([u8::try_from(rng.below(3)).unwrap()]));
    let ext = ChainExt {
        kind: kind.to_owned(),
        group,
        after: pick_after(rng, known),
    };
    sign(key, seq, backlink, None, ext).unwrap()
}

fn scenario(seed: u64) -> Scenario {
    let mut rng = Rng(seed);
    let count = 2 + rng.below(2) as usize;
    let keys: Vec<SigningKey> = (0..count)
        .map(|_| SigningKey::from_bytes(&std::array::from_fn(|_| rng.next_u64() as u8)))
        .collect();
    let lengths: Vec<SeqNum> = (0..count)
        .map(|_| 1 + SeqNum::try_from(rng.below(4)).unwrap())
        .collect();

    let mut events: Vec<Event> = Vec::new();
    let mut originals: Vec<Vec<Event>> = vec![Vec::new(); count];
    let mut progress: Vec<SeqNum> = vec![0; count];
    while progress.iter().zip(&lengths).any(|(done, len)| done < len) {
        let author = rng.below(count as u64) as usize;
        let seq = progress[author];
        if seq >= lengths[author] {
            continue;
        }
        progress[author] += 1;
        let backlink = originals[author].last().map(|event| event.id);
        let original = signed(&mut rng, &keys[author], seq, backlink, &events, "message");
        originals[author].push(original.clone());
        events.push(original);
        // A fork at this sequence number: one or two rivals, each with its own backlink.
        if rng.chance(3) {
            for _ in 0..=rng.below(2) {
                let link = rival_backlink(&mut rng, seq, backlink, &events);
                let rival = signed(&mut rng, &keys[author], seq, link, &events, "rival");
                if events.iter().all(|event| event.id != rival.id) {
                    events.push(rival);
                }
            }
        }
    }

    let authors: Vec<AuthorKey> = keys
        .iter()
        .map(|key| *key.verifying_key().as_bytes())
        .collect();
    let expected = model(&events, &authors);
    Scenario {
        events,
        authors,
        expected,
    }
}

/// The applied state that a set of events determines, whatever order they arrive in. Every
/// event names only earlier events, so one pass in creation order settles each event.
fn model(events: &[Event], authors: &[AuthorKey]) -> Canonical {
    let mut at: BTreeMap<(AuthorKey, SeqNum), Vec<Hash>> = BTreeMap::new();
    for event in events {
        at.entry((author_of(event), event.header.seq_num))
            .or_default()
            .push(event.id);
    }
    // The lowest sequence number with two distinct events. Keys are sorted by sequence
    // number within an author, so the first conflict seen is the lowest.
    let mut fork_point: BTreeMap<AuthorKey, SeqNum> = BTreeMap::new();
    for (&(author, seq), ids) in &at {
        if ids.len() >= 2 {
            fork_point.entry(author).or_insert(seq);
        }
    }

    let mut applied_at: BTreeMap<(AuthorKey, SeqNum), Hash> = BTreeMap::new();
    let mut applied: BTreeSet<Hash> = BTreeSet::new();
    for event in events {
        let author = author_of(event);
        let seq = event.header.seq_num;
        let below_fork = fork_point.get(&author).is_none_or(|point| seq < *point);
        let linked = match seq.checked_sub(1) {
            None => true,
            Some(before) => applied_at.get(&(author, before)) == event.header.backlink.as_ref(),
        };
        let deps_applied = event
            .header
            .extensions
            .after
            .iter()
            .all(|dep| applied.contains(dep));
        if below_fork && linked && deps_applied {
            applied_at.insert((author, seq), event.id);
            applied.insert(event.id);
        }
    }

    let mut expected = Canonical {
        applied,
        ..Canonical::default()
    };
    for author in authors {
        let mut next: SeqNum = 0;
        while applied_at.contains_key(&(*author, next)) {
            next += 1;
        }
        let fork = fork_point.get(author).copied();
        if next > 0 || fork.is_some() {
            let head = next
                .checked_sub(1)
                .and_then(|before| applied_at.get(&(*author, before)).copied());
            expected.chains.insert(
                *author,
                Chain {
                    next_seq: next,
                    head,
                    frozen: fork.is_some(),
                },
            );
        }
        if let Some(point) = fork {
            let mut ids = at.get(&(*author, point)).cloned().unwrap_or_default();
            ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            expected.proofs.insert(*author, (point, ids[0], ids[1]));
        }
    }
    expected
}

fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// Four delivery orders of the scenario's events, each with some events repeated: the
/// creation order, its reverse, and two shuffles.
fn orders(events: &[Event], rng: &mut Rng) -> Vec<Vec<Vec<u8>>> {
    let mut repeated: Vec<Vec<u8>> = Vec::new();
    for event in events {
        repeated.push(event.bytes.clone());
        for _ in 0..rng.below(3) {
            repeated.push(event.bytes.clone());
        }
    }
    let forward = repeated.clone();
    let mut reversed = repeated.clone();
    reversed.reverse();
    let mut first_shuffle = repeated.clone();
    shuffle(&mut first_shuffle, rng);
    let mut second_shuffle = repeated;
    shuffle(&mut second_shuffle, rng);
    vec![forward, reversed, first_shuffle, second_shuffle]
}

proptest! {
    #![proptest_config(config())]

    /// Random chains with forks at any depth, rivals with any backlink, and `after` hashes on
    /// any event, delivered in four orders with repeats: every order reaches the applied state
    /// that the model determines from the events.
    #[test]
    fn test_random_orders_reach_the_state_the_chains_determine(seed in any::<u64>()) {
        let scenario = scenario(seed);
        let mut rng = Rng(seed ^ 0xD1B5_4A32_D192_ED03);
        for order in orders(&scenario.events, &mut rng) {
            let state = deliver_within(order, scenario.authors.clone(), DELIVERY_LIMIT)
                .map_err(TestCaseError::fail)?;
            prop_assert_eq!(&state, &scenario.expected);
        }
    }
}

/// Escapes every byte as `\xHH`, which the replay format reads back.
fn escape(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("\\x{byte:02x}")).collect()
}

/// Prints the seed lines for `tests/regressions/ingest.txt`. The seeds sit on the ingest
/// boundaries: a fork at each position, an event ahead of its chain, an `after` hash on an
/// event a fork unapplied, a repeat, bytes that fail verification, and the conflicts that
/// arrive in one order but not another. Run by hand with `-- --ignored --nocapture`.
#[test]
#[ignore = "prints the seed lines for tests/regressions/ingest.txt"]
fn print_ingest_seeds() {
    let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
    let sign_event =
        |key: &SigningKey, seq: SeqNum, backlink: Option<Hash>, after: Vec<Hash>, kind: &str| {
            let ext = ChainExt {
                kind: kind.to_owned(),
                group: None,
                after,
            };
            sign(key, seq, backlink, None, ext).unwrap()
        };
    let (a, b) = (key(1), key(2));
    let a0 = sign_event(&a, 0, None, Vec::new(), "message");
    let a1 = sign_event(&a, 1, Some(a0.id), Vec::new(), "message");
    let a2 = sign_event(&a, 2, Some(a1.id), Vec::new(), "message");
    let a0_rival = sign_event(&a, 0, None, Vec::new(), "rival");
    let a1_rival = sign_event(&a, 1, Some(a0.id), Vec::new(), "rival");
    let a2_rival = sign_event(&a, 2, Some(a1.id), Vec::new(), "rival");
    let b0 = sign_event(&b, 0, None, Vec::new(), "message");
    let b0_after_a0 = sign_event(&b, 0, None, vec![a0.id], "message");
    let b0_after_a1 = sign_event(&b, 0, None, vec![a1.id], "message");
    let missing = Hash::digest(b"an event that is never delivered");
    let a1_stray = sign_event(
        &a,
        1,
        Some(Hash::digest(b"not the head")),
        Vec::new(),
        "stray",
    );
    let a1_held = sign_event(&a, 1, Some(a0.id), vec![missing], "message");
    let a2_stray = sign_event(&a, 2, Some(missing), Vec::new(), "stray");
    let a2_on_b0 = sign_event(&a, 2, Some(b0.id), Vec::new(), "message");
    // Two rivals at sequence 1 whose IDs are below a1's, so that a1 is the highest of the three.
    let low_rivals: Vec<Event> = (0..16)
        .map(|n| sign_event(&a, 1, Some(a0.id), Vec::new(), &format!("rival {n}")))
        .filter(|rival| rival.id.as_bytes() < a1.id.as_bytes())
        .collect();
    let mut tampered = a0.bytes.clone();
    tampered[48] ^= 0x01;
    let seeds: Vec<(&str, Vec<Vec<u8>>)> = vec![
        ("a valid event", vec![a0.bytes.clone()]),
        (
            "a valid chain, in order",
            vec![a0.bytes.clone(), a1.bytes.clone(), a2.bytes.clone()],
        ),
        (
            "a valid chain, reversed",
            vec![a2.bytes.clone(), a1.bytes.clone(), a0.bytes.clone()],
        ),
        (
            "a fork one step up",
            vec![a0.bytes.clone(), a1.bytes.clone(), a1_rival.bytes.clone()],
        ),
        (
            "a fork at sequence zero",
            vec![a0.bytes.clone(), a0_rival.bytes.clone()],
        ),
        (
            "a fork below a higher fork",
            vec![
                a0.bytes.clone(),
                a1.bytes.clone(),
                a2.bytes.clone(),
                a2_rival.bytes.clone(),
                a1_rival.bytes.clone(),
            ],
        ),
        (
            "an after hash released by its event",
            vec![b0_after_a0.bytes.clone(), a0.bytes.clone()],
        ),
        (
            "an after hash on an event the fork unapplied",
            vec![
                a0.bytes.clone(),
                a1.bytes.clone(),
                a1_rival.bytes.clone(),
                b0_after_a1.bytes.clone(),
            ],
        ),
        (
            "the same event twice",
            vec![a0.bytes.clone(), a0.bytes.clone()],
        ),
        (
            "a signature with one bit flipped",
            vec![tampered, a0.bytes.clone()],
        ),
        (
            "bytes that are not CBOR",
            vec![b"\xff\x00 not an event".to_vec(), a0.bytes.clone()],
        ),
        (
            "a fork at sequence zero unapplies an event after it",
            vec![
                a0.bytes.clone(),
                b0_after_a0.bytes.clone(),
                a0_rival.bytes.clone(),
            ],
        ),
        (
            "a conflict with an unmatched backlink, after the event it conflicts with",
            vec![a0.bytes.clone(), a1.bytes.clone(), a1_stray.bytes.clone()],
        ),
        (
            "a conflict held on a missing dependency, after its rival applies",
            vec![
                a0.bytes.clone(),
                a1_rival.bytes.clone(),
                a1_held.bytes.clone(),
            ],
        ),
        (
            "two events ahead of the chain at one position",
            vec![
                a0.bytes.clone(),
                a1.bytes.clone(),
                a2.bytes.clone(),
                a2_stray.bytes.clone(),
            ],
        ),
        (
            "an event ahead of its chain whose backlink is applied elsewhere",
            vec![b0.bytes.clone(), a2_on_b0.bytes.clone()],
        ),
        (
            "three rivals at one position",
            vec![
                a0.bytes.clone(),
                a1.bytes.clone(),
                low_rivals[0].bytes.clone(),
                low_rivals[1].bytes.clone(),
            ],
        ),
    ];
    for (name, events) in seeds {
        let fields: Vec<String> = events
            .iter()
            .map(|bytes| format!("b:{}", escape(bytes)))
            .collect();
        println!("# {name}");
        println!("ingest\t{}\tok", fields.join(" "));
    }
}
