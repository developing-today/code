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
//! before it, and every `after` hash is present: applied, or a fork proof header. The fork
//! point is the lowest sequence number where the author has two distinct events, and the proof
//! holds the two lowest IDs there.
//!
//! Failures are not written to `proptest-regressions/`. The minimal input goes into
//! `tests/regressions/ingest.txt` instead (see `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::ingest::{Canonical, DELIVERY_LIMIT, MemStorage, canonical, deliver_within};
use id::chain::{ChainExt, Event, sign};
use id::event_store::{Chain, EventStore, Ingest, Refusal};
use id::waiting::{AuthorKey, WaitingLimits};
use p2panda_core::{Hash, Header, SeqNum, SigningKey};
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseError};

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

/// The runner's RNG seed. A fixed seed draws the same cases on every run, so CI checks one seed
/// set. `PROPTEST_RNG_SEED` overrides it, to search for new failures.
const RNG_SEED: u64 = 0x1D5E_ED20_2610_1012;

/// Cases per property. A debug build runs redb unoptimised, at about a second a case, so the
/// default is small. `PROPTEST_CASES` raises it. A release run (`--release`) goes much further.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(32);
    let rng_seed = std::env::var("PROPTEST_RNG_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(RngSeed::Fixed(RNG_SEED), RngSeed::Fixed);
    ProptestConfig {
        cases,
        failure_persistence: None,
        rng_seed,
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
        tag: None,
        document: None,
        removes: None,
        note: None,
    };
    sign(key, seq, backlink, None, ext).unwrap()
}

/// Where each author forks, if it does: the sequence number of the fork point, and how many
/// rivals sit there beside the original (one or two).
type ForkPlan = BTreeMap<AuthorKey, (SeqNum, usize)>;

/// The events a later event may name: those that are present in every delivery order. An
/// author with a fork is referable up to its fork point. At the fork point it is referable only
/// when there are two events there, which the fork proof holds. Above the fork point, an event
/// is refused as frozen in some orders and kept in others, so no event names it.
fn referable(events: &[Event], plan: &ForkPlan) -> Vec<Event> {
    events
        .iter()
        .filter(|event| {
            let seq = event.header.seq_num;
            match plan.get(&author_of(event)) {
                None => true,
                Some(&(fork, rivals)) => seq < fork || (seq == fork && rivals == 1),
            }
        })
        .cloned()
        .collect()
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
    let publics: Vec<AuthorKey> = keys
        .iter()
        .map(|key| *key.verifying_key().as_bytes())
        .collect();
    // Each author forks at most once, below the end of its chain. Deciding this up front is what
    // lets `referable` know which events are stable while the scenario is still being built.
    let mut plan: ForkPlan = BTreeMap::new();
    for (public, len) in publics.iter().zip(&lengths) {
        if rng.chance(2) {
            let fork = SeqNum::try_from(rng.below(u64::from(*len))).unwrap();
            plan.insert(*public, (fork, 1 + rng.below(2) as usize));
        }
    }

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
        let known = referable(&events, &plan);
        let original = signed(&mut rng, &keys[author], seq, backlink, &known, "message");
        originals[author].push(original.clone());
        events.push(original);
        // The fork point: the rivals, each with its own backlink.
        if let Some(&(_, rivals)) = plan.get(&publics[author]).filter(|(fork, _)| *fork == seq) {
            for n in 0..rivals {
                let link = rival_backlink(&mut rng, seq, backlink, &events);
                let known = referable(&events, &plan);
                let rival = signed(
                    &mut rng,
                    &keys[author],
                    seq,
                    link,
                    &known,
                    &format!("rival {n}"),
                );
                events.push(rival);
            }
        }
    }

    let expected = model(&events, &publics);
    Scenario {
        events,
        authors: publics,
        expected,
    }
}

/// A scenario in which some authors fork once and the rest never fork. Every `after` hash names an
/// honest event, or a forked author's event at or below its fork point, so each one is present in
/// every delivery order and nothing stalls except what a fork voids. Returns the honest authors too.
fn honest_scenario(seed: u64) -> (Scenario, Vec<AuthorKey>) {
    let mut rng = Rng(seed);
    let count = 2 + rng.below(3) as usize;
    let keys: Vec<SigningKey> = (0..count)
        .map(|_| SigningKey::from_bytes(&std::array::from_fn(|_| rng.next_u64() as u8)))
        .collect();
    let lengths: Vec<SeqNum> = (0..count)
        .map(|_| 1 + SeqNum::try_from(rng.below(4)).unwrap())
        .collect();
    // The first `forkers` authors fork and the rest are honest, so at least one is honest.
    let forkers = rng.below(count as u64) as usize;
    let forks: Vec<Option<SeqNum>> = (0..count)
        .map(|author| {
            (author < forkers)
                .then(|| SeqNum::try_from(rng.below(u64::from(lengths[author]))).unwrap())
        })
        .collect();
    let publics: Vec<AuthorKey> = keys
        .iter()
        .map(|key| *key.verifying_key().as_bytes())
        .collect();

    let mut events: Vec<Event> = Vec::new();
    let mut good: Vec<Event> = Vec::new();
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
        let original = present_event(&mut rng, &keys[author], seq, backlink, &good, "message");
        originals[author].push(original.clone());
        if forks[author].is_none_or(|fork| seq <= fork) {
            good.push(original.clone());
        }
        events.push(original);
        if forks[author] == Some(seq) {
            let rival = present_event(&mut rng, &keys[author], seq, backlink, &good, "rival");
            good.push(rival.clone());
            events.push(rival);
        }
    }

    let honest = publics
        .iter()
        .zip(&forks)
        .filter(|(_, fork)| fork.is_none())
        .map(|(public, _)| *public)
        .collect();
    let expected = model(&events, &publics);
    let scenario = Scenario {
        events,
        authors: publics,
        expected,
    };
    (scenario, honest)
}

/// Up to three `after` hashes, each one of `good`. Unlike [`pick_after`], it never names a hash that
/// is never delivered.
fn pick_present(rng: &mut Rng, good: &[Event]) -> Vec<Hash> {
    let mut picked = Vec::new();
    if good.is_empty() {
        return picked;
    }
    for _ in 0..rng.below(4) {
        let hash = good[rng.below(good.len() as u64) as usize].id;
        if !picked.contains(&hash) {
            picked.push(hash);
        }
    }
    picked
}

/// Signs an event whose `after` hashes are drawn from `good` by [`pick_present`].
fn present_event(
    rng: &mut Rng,
    key: &SigningKey,
    seq: SeqNum,
    backlink: Option<Hash>,
    good: &[Event],
    kind: &str,
) -> Event {
    let ext = ChainExt {
        kind: kind.to_owned(),
        group: None,
        after: pick_present(rng, good),
        tag: None,
        document: None,
        removes: None,
        note: None,
    };
    sign(key, seq, backlink, None, ext).unwrap()
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

    // The two lowest IDs at each fork point. A fork keeps its proof headers, so they are present
    // even though neither is applied.
    let mut proof_ids: BTreeSet<Hash> = BTreeSet::new();
    for (&author, &point) in &fork_point {
        let mut ids = at.get(&(author, point)).cloned().unwrap_or_default();
        ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        proof_ids.extend(ids.into_iter().take(2));
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
        // An `after` hash is present when it is applied, or when it is a proof header.
        let deps_present = event
            .header
            .extensions
            .after
            .iter()
            .all(|dep| applied.contains(dep) || proof_ids.contains(dep));
        if below_fork && linked && deps_present {
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

/// An index in `0..n`, drawn from `rng`.
fn index(rng: &mut Rng, n: usize) -> usize {
    usize::try_from(rng.below(n as u64)).unwrap()
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

    /// Honest authors never stall. Some authors fork, at random depths, and every `after` hash names
    /// an event that is present in every order. In every order, each honest author's chain applies
    /// in full, and the state matches the model. This is the liveness rule until P2.3 replaces
    /// fork voiding with authority-based voiding.
    #[test]
    fn test_a_fork_stalls_no_honest_chain_in_any_order_until_p2_3(seed in any::<u64>()) {
        let (scenario, honest) = honest_scenario(seed);
        for author in &honest {
            let applied = scenario.expected.chains.get(author).map(|chain| chain.next_seq);
            let written = scenario.events.iter().filter(|event| author_of(event) == *author).count();
            prop_assert_eq!(applied, Some(SeqNum::try_from(written).unwrap()));
        }
        let mut rng = Rng(seed ^ 0x2545_F491_4F6C_DD1D);
        for order in orders(&scenario.events, &mut rng) {
            let state = deliver_within(order, scenario.authors.clone(), DELIVERY_LIMIT)
                .map_err(TestCaseError::fail)?;
            prop_assert_eq!(&state, &scenario.expected);
        }
    }

    /// A restart changes nothing. One delivery order, run with the store closed and reopened at
    /// random points, gives every delivery the same outcome, and leaves the same applied state and
    /// held events, as the same order run without restarts. The caps are small and the clock runs
    /// past the age limit, so restarts also land on refusals and on expiries.
    #[test]
    fn test_restarts_at_random_points_change_no_outcome(seed in any::<u64>()) {
        restarts_change_no_outcome(seed)?;
    }
}

/// The body of [`test_restarts_at_random_points_change_no_outcome`], for one seed. The pinned
/// seeds in [`PINNED_RESTART_SEEDS`] run it on every test run.
fn restarts_change_no_outcome(seed: u64) -> Result<(), TestCaseError> {
    let scenario = scenario(seed);
    let mut rng = Rng(seed ^ 0x5851_F42D_4C95_7F2D);
    let mut all_orders = orders(&scenario.events, &mut rng);
    let order = all_orders.swap_remove(index(&mut rng, 4));
    let mut clock = 0_u64;
    let times: Vec<u64> = order
        .iter()
        .map(|_| {
            clock += rng.below(1_000);
            clock
        })
        .collect();
    let max_age_ms = [1_500, 6_000, u64::MAX / 2][index(&mut rng, 3)];
    let per_author = [1, 2, 3, 256][index(&mut rng, 4)];
    let limits = WaitingLimits::new(per_author, max_age_ms);
    // Half the cases restart before every delivery. The rest restart before about one in three,
    // so events are often held across a restart.
    let every_delivery = rng.chance(2);
    let restarts: Vec<usize> = (0..order.len())
        .filter(|_| every_delivery || rng.chance(3))
        .collect();

    let plain = {
        let (order, times, authors) = (order.clone(), times.clone(), scenario.authors.clone());
        within_limit(move || deliver_with_restarts(&order, &times, &[], limits, &authors))
    };
    let restarted = within_limit(move || {
        deliver_with_restarts(&order, &times, &restarts, limits, &scenario.authors)
    });
    let (plain_outcomes, plain_state, plain_held) = plain.map_err(TestCaseError::fail)?;
    let (outcomes, state, held) = restarted.map_err(TestCaseError::fail)?;
    prop_assert_eq!(outcomes, plain_outcomes);
    prop_assert_eq!(state, plain_state);
    prop_assert_eq!(held, plain_held);
    Ok(())
}

/// Seeds that [`restarts_change_no_outcome`] must pass on every run, so a failure that a random
/// draw found stays covered. The first one failed before the fix in `decide_in`, on a backlink
/// that was present and never applied, and a restart then found it held.
const PINNED_RESTART_SEEDS: &[u64] = &[3_392_703_367_498_350_025];

#[test]
fn test_pinned_restart_seeds_change_no_outcome() {
    for &seed in PINNED_RESTART_SEEDS {
        restarts_change_no_outcome(seed)
            .map_err(|err| format!("seed {seed}: {err}"))
            .unwrap();
    }
}

/// Delivers `order` to one store, which is closed and reopened before each delivery whose
/// position is in `restarts`. Delivery `i` runs at clock `times[i]`, and a reopen runs at the
/// clock of the delivery it precedes. Returns each delivery's outcome, the applied state of
/// `authors`, and the IDs the store holds at the end. The invariants must hold after the last
/// delivery.
fn deliver_with_restarts(
    order: &[Vec<u8>],
    times: &[u64],
    restarts: &[usize],
    limits: WaitingLimits,
    authors: &[AuthorKey],
) -> Result<(Vec<Ingest>, Canonical, Vec<Hash>), String> {
    let storage = MemStorage::default();
    let open = |at: u64| EventStore::open_with(&storage, limits, at).map_err(|err| err.to_string());
    let first = times.first().copied().unwrap_or(0);
    let mut store = open(first)?;
    let mut outcomes = Vec::new();
    for (i, bytes) in order.iter().enumerate() {
        if restarts.contains(&i) {
            drop(store);
            store = open(times[i])?;
        }
        let outcome = store
            .ingest(bytes, times[i])
            .map_err(|err| format!("ingest failed: {err}"))?;
        outcomes.push(outcome);
    }
    store
        .check_invariants()
        .map_err(|err| format!("invariants after the deliveries: {err}"))?;
    let state = canonical(&store, authors)?;
    let held = store.waiting_ids().map_err(|err| err.to_string())?;
    Ok((outcomes, state, held))
}

/// Delivers `order` to one store, which is closed and reopened before each delivery in `restarts`,
/// and checks the waiting states after each step. Delivery `i` runs at clock `times[i]`.
///
/// An event that is due at a step's clock expires during that step, so it is taken out of the
/// pending set first. The store drops it, or holds it again if it is delivered again.
fn check_each_step(
    order: &[Vec<u8>],
    times: &[u64],
    restarts: &[usize],
    limits: WaitingLimits,
    events: &BTreeMap<Hash, Event>,
) -> Result<(), TestCaseError> {
    let storage = MemStorage::default();
    let open = |at: u64| {
        EventStore::open_with(&storage, limits, at)
            .map_err(|err| TestCaseError::fail(err.to_string()))
    };
    let mut store = open(times.first().copied().unwrap_or(0))?;
    // Each waiting event the store acknowledged, with the clock it arrived at. Its age counts from
    // there.
    let mut pending: BTreeMap<Hash, (u64, Event)> = BTreeMap::new();
    for (i, bytes) in order.iter().enumerate() {
        let now = times[i];
        pending.retain(|_, (arrived_at, _)| !limits.is_due(*arrived_at, now));
        if restarts.contains(&i) {
            drop(store);
            store = open(now)?;
            settle_pending(&store, &mut pending)?;
        }
        let outcome = store
            .ingest(bytes, now)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        if let Ingest::Waiting { id } = outcome
            && let Some(event) = events.get(&id)
        {
            pending.entry(id).or_insert_with(|| (now, event.clone()));
        }
        store
            .check_invariants()
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        settle_pending(&store, &mut pending)?;
    }
    Ok(())
}

/// Checks each pending waiting event. An event that is held is in one state. One that is stored
/// has applied or been voided. One that a fork froze at or below its sequence number is refused,
/// and leaves no row: its chain can never apply it. A fork proof header never applies, so it must
/// stay held. Any other event that is neither held nor stored is lost, and fails the test.
/// Settled events leave the pending set.
fn settle_pending(
    store: &EventStore,
    pending: &mut BTreeMap<Hash, (u64, Event)>,
) -> Result<(), TestCaseError> {
    let mut settled = Vec::new();
    for (id, (arrived_at, event)) in pending.iter() {
        if store.is_waiting(id) {
            continue;
        }
        if store
            .event_by_id(id)
            .map_err(|err| TestCaseError::fail(err.to_string()))?
            .is_some()
        {
            settled.push(*id);
            continue;
        }
        let seq = event.header.seq_num;
        let proof = store
            .fork_proof(&author_of(event))
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let header = proof
            .as_ref()
            .is_some_and(|proof| proof.first.id == *id || proof.second.id == *id);
        let frozen = proof
            .as_ref()
            .is_some_and(|proof| proof.seq <= seq && !header);
        if frozen {
            settled.push(*id);
            continue;
        }
        return Err(TestCaseError::fail(format!(
            "an acknowledged waiting event at sequence {seq} (arrived at {arrived_at}) is neither held nor stored{}",
            if header {
                ", and it is a fork proof header"
            } else {
                ""
            }
        )));
    }
    for id in settled {
        pending.remove(&id);
    }
    Ok(())
}

/// Runs `run` on a helper thread. Fails if it has not returned within [`DELIVERY_LIMIT`], so a
/// store that spins fails the test instead of hanging it.
fn within_limit<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // The receiver is gone only once the wait below has given up.
        let _ = sent.send(run());
    });
    match received.recv_timeout(DELIVERY_LIMIT) {
        Ok(result) => result,
        Err(_) => Err(format!(
            "the deliveries did not finish within {DELIVERY_LIMIT:?}: an event made the store spin"
        )),
    }
}

proptest! {
    #![proptest_config(config())]

    /// After every delivery and every reopen, each waiting event the store acknowledged is in one
    /// state: held, or stored (applied or voided), or refused because a fork froze its chain at or
    /// below it, or dropped by the age cap. Random orders, repeats, forks at any depth, small caps,
    /// and restarts all vary the path there.
    #[test]
    fn test_every_step_keeps_each_acknowledged_waiting_event_in_one_state(seed in any::<u64>()) {
        let scenario = scenario(seed);
        let events: BTreeMap<Hash, Event> = scenario
            .events
            .iter()
            .map(|event| (event.id, event.clone()))
            .collect();
        let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        let max_age_ms = [1_500, 6_000, u64::MAX / 2][index(&mut rng, 3)];
        let per_author = [1, 2, 3, 256][index(&mut rng, 4)];
        let limits = WaitingLimits::new(per_author, max_age_ms);
        for order in orders(&scenario.events, &mut rng) {
            let mut clock = 0_u64;
            let times: Vec<u64> = order
                .iter()
                .map(|_| {
                    clock += rng.below(1_000);
                    clock
                })
                .collect();
            let every_delivery = rng.chance(2);
            let restarts: Vec<usize> = (0..order.len())
                .filter(|_| every_delivery || rng.chance(3))
                .collect();
            check_each_step(&order, &times, &restarts, limits, &events)?;
        }
    }

    /// At ingest, a header that breaks the sequence and backlink rule is refused as malformed,
    /// whatever its sequence number. It is not parked or held, so it takes no waiting slot and
    /// raises no conflict. A header that keeps the rule is not refused for it.
    #[test]
    fn test_a_header_breaking_the_sequence_rule_is_refused_at_ingest(
        seq in 0u32..40,
        backlink in prop::option::of(any::<[u8; 32]>()),
    ) {
        let raw = Header::<ChainExt>::builder()
            .seq_num(seq)
            .backlink(backlink.map(Hash::from))
            .build(
                &SigningKey::from_bytes(&[7; 32]),
                ChainExt {
                    kind: "m".to_owned(),
                    group: None,
                    after: Vec::new(), tag: None, document: None, removes: None, note: None,
                },
            )
            .encode();
        let mut store = EventStore::open_with(&MemStorage::default(), WaitingLimits::default(), 0)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let outcome = store
            .ingest(&raw, 0)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let agrees = (seq == 0) == backlink.is_none();
        if agrees {
            prop_assert!(!matches!(outcome, Ingest::Refused(_)), "refused: {outcome:?}");
        } else {
            prop_assert!(
                matches!(outcome, Ingest::Refused(Refusal::Malformed(_))),
                "sequence {seq} with backlink {backlink:?} gave {outcome:?}"
            );
            prop_assert_eq!(store.waiting_len(), 0, "a malformed header was held");
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
                tag: None,
                document: None,
                removes: None,
                note: None,
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
    // A fork at sequence 1 of `b` voids `b1`. An event of `c` ahead of its chain names it.
    let c = key(3);
    let b1 = sign_event(&b, 1, Some(b0.id), Vec::new(), "message");
    let b1_stray = sign_event(
        &b,
        1,
        Some(Hash::digest(b"not the head")),
        Vec::new(),
        "stray",
    );
    let c1_on_b1 = sign_event(&c, 1, Some(b1.id), Vec::new(), "message");
    // A fork at sequence 0 of `b` makes `b0` a proof header. An event of `c` ahead names it.
    let b0_rival = sign_event(&b, 0, None, Vec::new(), "rival");
    let c1_on_b0 = sign_event(&c, 1, Some(b0.id), Vec::new(), "message");
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
        (
            "an event ahead of a voided backlink is parked, not waited for",
            vec![b0.bytes.clone(), b1.bytes, b1_stray.bytes, c1_on_b1.bytes],
        ),
        (
            "an event ahead of a fork proof header is parked, not waited for",
            vec![b0.bytes, b0_rival.bytes, c1_on_b0.bytes],
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
