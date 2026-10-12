//! Property tests for ingest in `src/event_store.rs` (plan P1.8).
//!
//! A scenario is a set of valid chains, with forks injected. Each fork is a second
//! event at a sequence number, signed with the same backlink as the original. The
//! scenario is delivered in three orders, with repeats, to fresh stores. Every store
//! must reach the applied state that the chains determine: the chain up to each
//! author's fork point, frozen if there is one, and the fork proof at the lowest fork.
//!
//! The `after` hashes are drawn from events that a fork can never take out of the
//! applied prefix. A fork can unapply events at and above its own sequence number, and
//! an event that names one of those would make the expected state depend on the order
//! of arrival. The regression list covers the orders the generator does not reach.
//!
//! Failures are not written to `proptest-regressions/`. The minimal input goes into
//! `tests/regressions/ingest.txt` instead (see `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::ingest::{Canonical, deliver};
use id::chain::{ChainExt, Event, sign};
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

/// A scenario: every signed event, and the applied state every delivery must reach.
struct Scenario {
    /// Every signed event, originals and forks, in creation order.
    events: Vec<Event>,
    /// The authors' public keys.
    authors: Vec<AuthorKey>,
    /// The applied state of every author.
    expected: Canonical,
}

/// Up to three `after` hashes, drawn from the stable events created so far.
fn pick_after(rng: &mut Rng, stable: &[Hash]) -> Vec<Hash> {
    let mut picked = Vec::new();
    if stable.is_empty() {
        return picked;
    }
    for _ in 0..rng.below(4) {
        let hash = stable[rng.below(stable.len() as u64) as usize];
        if !picked.contains(&hash) {
            picked.push(hash);
        }
    }
    picked
}

/// Signs an event with a random group and frontier, and the given kind.
fn signed(
    rng: &mut Rng,
    key: &SigningKey,
    seq: SeqNum,
    backlink: Option<Hash>,
    stable: &[Hash],
    kind: &str,
) -> Event {
    let group = rng
        .chance(3)
        .then(|| Hash::digest([u8::try_from(rng.below(3)).unwrap()]));
    let ext = ChainExt {
        kind: kind.to_owned(),
        group,
        after: pick_after(rng, stable),
    };
    sign(key, seq, backlink, None, ext).unwrap()
}

/// Orders two IDs as the fork proof does: lower ID first.
fn ordered(a: Hash, b: Hash) -> (Hash, Hash) {
    if a.as_bytes() <= b.as_bytes() {
        (a, b)
    } else {
        (b, a)
    }
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
    // The forks are chosen first: at most one per sequence number. The lowest one is
    // the author's fork point.
    let forked: Vec<Vec<bool>> = lengths
        .iter()
        .map(|&len| (0..len).map(|_| rng.chance(4)).collect())
        .collect();
    let fork_point: Vec<Option<SeqNum>> = forked
        .iter()
        .map(|row| {
            row.iter()
                .position(|&is_forked| is_forked)
                .map(|at| SeqNum::try_from(at).unwrap())
        })
        .collect();

    let mut events: Vec<Event> = Vec::new();
    let mut chains: Vec<Vec<Event>> = vec![Vec::new(); count];
    let mut rivals: Vec<Option<Event>> = vec![None; count];
    // The IDs that can be named by `after`: events below their author's fork point.
    let mut stable: Vec<Hash> = Vec::new();
    let mut progress: Vec<SeqNum> = vec![0; count];
    while progress.iter().zip(&lengths).any(|(done, len)| done < len) {
        let author = rng.below(count as u64) as usize;
        let seq = progress[author];
        if seq >= lengths[author] {
            continue;
        }
        progress[author] += 1;
        let backlink = chains[author].last().map(|event| event.id);
        let original = signed(&mut rng, &keys[author], seq, backlink, &stable, "message");
        if fork_point[author].is_none_or(|point| seq < point) {
            stable.push(original.id);
        }
        chains[author].push(original.clone());
        events.push(original);
        if forked[author][seq as usize] {
            let rival = signed(&mut rng, &keys[author], seq, backlink, &stable, "rival");
            events.push(rival.clone());
            if fork_point[author] == Some(seq) {
                rivals[author] = Some(rival);
            }
        }
    }

    let mut expected = Canonical::default();
    for author in 0..count {
        let who: AuthorKey = *keys[author].verifying_key().as_bytes();
        match fork_point[author] {
            None => {
                expected.chains.insert(
                    who,
                    Chain {
                        next_seq: lengths[author],
                        head: chains[author].last().map(|event| event.id),
                        frozen: false,
                    },
                );
            }
            Some(point) => {
                let head = point
                    .checked_sub(1)
                    .map(|before| chains[author][before as usize].id);
                expected.chains.insert(
                    who,
                    Chain {
                        next_seq: point,
                        head,
                        frozen: true,
                    },
                );
                let original = chains[author][point as usize].id;
                let rival = rivals[author].as_ref().unwrap().id;
                let (first, second) = ordered(original, rival);
                expected.proofs.insert(who, (point, first, second));
            }
        }
    }
    let authors = keys
        .iter()
        .map(|key| *key.verifying_key().as_bytes())
        .collect();
    Scenario {
        events,
        authors,
        expected,
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// Three delivery orders of the scenario's events, each with some events repeated:
/// the creation order, the reverse, and a shuffle.
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
    let mut shuffled = repeated;
    shuffle(&mut shuffled, rng);
    vec![forward, reversed, shuffled]
}

proptest! {
    #![proptest_config(config())]

    /// Random valid chains with forks, delivered in three orders with repeats, reach
    /// the applied state that the chains determine.
    #[test]
    fn test_random_orders_reach_the_state_the_chains_determine(seed in any::<u64>()) {
        let scenario = scenario(seed);
        let mut rng = Rng(seed ^ 0xD1B5_4A32_D192_ED03);
        for order in orders(&scenario.events, &mut rng) {
            let state = deliver(&order, &scenario.authors).map_err(TestCaseError::fail)?;
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
/// event the fork unapplied, a repeat, and bytes that fail verification.
/// Run by hand with `-- --ignored --nocapture`.
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
    let b0_after_a0 = sign_event(&b, 0, None, vec![a0.id], "message");
    let b0_after_a1 = sign_event(&b, 0, None, vec![a1.id], "message");
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
