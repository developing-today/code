//! Q2: concurrent commits. Deterministic scenarios first, then a randomized run that checks
//! whether replicas converge with causal buffering and without it.

use crate::{
    common::Member,
    party::{build, Op, Party},
};
use beekem::{error::CgkaError, id::MemberId, pcs_key::PcsKey};
use keyhive_crypto::{share_key::ShareKey, signer::memory::MemorySigner, verifiable::Verifiable};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use std::{
    collections::BTreeMap,
    panic::{self, AssertUnwindSafe},
    sync::Mutex,
};
use beekem::id::TreeId;

fn hex8(k: &PcsKey) -> String {
    k.0.as_slice()[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Deliver `op` to `p` and print what happened.
fn give(p: &mut Party, who: &str, op: &Op) {
    match p.merge(op) {
        Ok(applied) => println!("  {who} <- op: applied={applied}"),
        Err(e) => println!("  {who} <- op: error {e:?}"),
    }
}

/// One line per replica: members it places, and the root key it derives (or why not).
fn status(names: &[&str], ps: &mut [Party]) {
    let mut keys = Vec::new();
    for (name, p) in names.iter().zip(ps.iter_mut()) {
        let size = p.c.group_size();
        let root = match p.root() {
            Ok(k) => {
                keys.push(hex8(&k));
                format!("root {}", hex8(&k))
            }
            Err(e) => format!("no root ({e:?})"),
        };
        println!("  {name}: members={size}, {root}, has_pcs_key={}", p.c.has_pcs_key());
    }
    let all_derive = keys.len() == ps.len();
    let agree = all_derive && keys.windows(2).all(|w| w[0] == w[1]);
    println!("  all replicas derive one key: {agree} (derived by {} of {})", keys.len(), ps.len());
}

fn s0_plain_remove(rng: &mut StdRng) {
    println!("## S0: a plain removal with no concurrent op");
    let (mut ps, _, _, _) = build(4, rng);
    let victim = ps[2].id();
    let rm = ps[0].remove(victim).expect("remove").expect("c present");
    give(&mut ps[1], "b", &rm);
    give(&mut ps[2], "c", &rm);
    give(&mut ps[3], "d", &rm);
    status(&["a", "b", "c", "d"], &mut ps);
}

fn s1_concurrent_updates(rng: &mut StdRng) {
    println!("## S1: a and b commit at once; c and d see the two updates in opposite orders");
    let (mut ps, _, _, _) = build(4, rng);
    let op_a = ps[0].update(rng).expect("a updates");
    let op_b = ps[1].update(rng).expect("b updates");
    give(&mut ps[2], "c", &op_a);
    give(&mut ps[2], "c", &op_b);
    give(&mut ps[3], "d", &op_b);
    give(&mut ps[3], "d", &op_a);
    give(&mut ps[0], "a", &op_b);
    give(&mut ps[1], "b", &op_a);
    status(&["a", "b", "c", "d"], &mut ps);
    println!("  c resolves with one update:");
    let op_c = ps[2].update(rng).expect("c updates");
    for i in [0, 1, 3] {
        give(&mut ps[i], ["a", "b", "", "d"][i], &op_c);
    }
    status(&["a", "b", "c", "d"], &mut ps);
}

fn s2_update_and_remove(rng: &mut StdRng) {
    println!("## S2: a updates while b removes c");
    let (mut ps, _, _, _) = build(4, rng);
    let victim = ps[2].id();
    let op_a = ps[0].update(rng).expect("a updates");
    let op_r = ps[1].remove(victim).expect("remove").expect("c present");
    give(&mut ps[3], "d", &op_r);
    give(&mut ps[3], "d", &op_a);
    give(&mut ps[2], "c", &op_a);
    give(&mut ps[2], "c", &op_r);
    give(&mut ps[0], "a", &op_r);
    give(&mut ps[1], "b", &op_a);
    status(&["a", "b", "c", "d"], &mut ps);
    let op_d = ps[3].update(rng).expect("d updates");
    for i in [0, 1, 2] {
        give(&mut ps[i], ["a", "b", "c", ""][i], &op_d);
    }
    println!("  after d's update:");
    status(&["a", "b", "c", "d"], &mut ps);
}

fn s3_concurrent_add_same_member(rng: &mut StdRng) {
    println!("## S3: a and b both add the same newcomer e");
    let (mut ps, _, _, _) = build(4, rng);
    let e = Member::new(rng);
    let op_a = ps[0].add(e.id, e.pk).expect("a adds").expect("e is new");
    let op_b = ps[1].add(e.id, e.pk).expect("b adds").expect("e is new");
    give(&mut ps[2], "c", &op_a);
    give(&mut ps[2], "c", &op_b);
    give(&mut ps[3], "d", &op_b);
    give(&mut ps[3], "d", &op_a);
    give(&mut ps[0], "a", &op_b);
    give(&mut ps[1], "b", &op_a);
    status(&["a", "b", "c", "d"], &mut ps);
    let op_c = ps[2].update(rng).expect("c updates");
    for i in [0, 1, 3] {
        give(&mut ps[i], ["a", "b", "", "d"][i], &op_c);
    }
    println!("  after c's update:");
    status(&["a", "b", "c", "d"], &mut ps);
}

fn s4_concurrent_add_and_remove(rng: &mut StdRng) {
    println!("## S4: a adds e while b removes c");
    let (mut ps, _, _, _) = build(4, rng);
    let victim = ps[2].id();
    let e = Member::new(rng);
    let op_a = ps[0].add(e.id, e.pk).expect("a adds").expect("e is new");
    let op_b = ps[1].remove(victim).expect("remove").expect("c present");
    give(&mut ps[2], "c", &op_a);
    give(&mut ps[2], "c", &op_b);
    give(&mut ps[3], "d", &op_b);
    give(&mut ps[3], "d", &op_a);
    give(&mut ps[0], "a", &op_b);
    give(&mut ps[1], "b", &op_a);
    let op_d = ps[3].update(rng).expect("d updates");
    for i in [0, 1, 2] {
        give(&mut ps[i], ["a", "b", "c"][i], &op_d);
    }
    println!("  after d's update:");
    status(&["a", "b", "c", "d"], &mut ps);
}

fn s5_out_of_order(rng: &mut StdRng) {
    println!("## S5: an op delivered before its predecessor; a duplicate delivery");
    let (mut ps, _, _, _) = build(4, rng);
    let op1 = ps[0].update(rng).expect("a updates");
    give(&mut ps[1], "b", &op1);
    // b's next update names op1 as a predecessor.
    let op2 = ps[1].update(rng).expect("b updates");
    let before = ps[3].c.ops_count();
    give(&mut ps[3], "d (op2 first)", &op2);
    println!("  d's op count unchanged after the rejected op: {}", ps[3].c.ops_count() == before);
    give(&mut ps[3], "d (op1)", &op1);
    give(&mut ps[3], "d (op2, retry)", &op2);
    give(&mut ps[3], "d (op2, duplicate)", &op2);
    // a and c were never sent the two ops; deliver them in causal order.
    give(&mut ps[0], "a (op2)", &op2);
    give(&mut ps[2], "c (op1)", &op1);
    give(&mut ps[2], "c (op2)", &op2);
    status(&["a", "b", "c", "d"], &mut ps);
}

pub fn scenarios() {
    s0_plain_remove(&mut StdRng::seed_from_u64(0xc0c0_0000));
    println!();
    let mut rng = StdRng::seed_from_u64(0xc0c0_0001);
    s1_concurrent_updates(&mut rng);
    println!();
    s2_update_and_remove(&mut rng);
    println!();
    s3_concurrent_add_same_member(&mut rng);
    println!();
    s4_concurrent_add_and_remove(&mut rng);
    println!();
    s5_out_of_order(&mut rng);
}

// ---- randomized convergence ----

const ROUNDS: usize = 8;
const GROUP: usize = 5;

#[derive(Default)]
struct Stats {
    converged: usize,
    emptied: usize,
    buffered_events: usize,
    dropped: usize,
    max_predecessors: usize,
    failures: BTreeMap<String, usize>,
    errors: BTreeMap<String, usize>,
}

static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Merge `op` into `ps[r]` and retry its buffer. Returns the number of ops dropped.
fn deliver_to(p: &mut Party, op: &Op, buffer: bool, stats: &mut Stats) {
    match p.merge(op) {
        Ok(_) => {}
        Err(CgkaError::OutOfOrderOperation) if buffer => {
            p.buffer.push(op.clone());
            stats.buffered_events += 1;
            return;
        }
        Err(CgkaError::OutOfOrderOperation) => {
            stats.dropped += 1;
            return;
        }
        Err(e) => {
            *stats.errors.entry(format!("{e:?}").chars().take(40).collect()).or_default() += 1;
            return;
        }
    }
    // Something new may now be ready: retry until nothing more applies.
    loop {
        let mut progress = false;
        for pending in std::mem::take(&mut p.buffer) {
            match p.merge(&pending) {
                Err(CgkaError::OutOfOrderOperation) => p.buffer.push(pending),
                Ok(_) => progress = true,
                Err(e) => {
                    *stats.errors.entry(format!("{e:?}").chars().take(40).collect()).or_default() += 1;
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
}

/// Deliver each (op, recipient) still missing with probability `p`, in random order.
/// `flight` holds ops with the recipients still missing them.
fn deliver_some(
    ps: &mut [Party],
    flight: &mut Vec<(Op, Vec<usize>)>,
    p: f64,
    buffer: bool,
    rng: &mut StdRng,
    stats: &mut Stats,
) {
    let mut pairs: Vec<(usize, usize)> = flight
        .iter()
        .enumerate()
        .flat_map(|(fi, (_, missing))| missing.iter().map(move |&r| (fi, r)))
        .collect();
    pairs.shuffle(rng);
    for (fi, r) in pairs {
        if rng.gen_range(0.0..1.0) >= p {
            continue;
        }
        let op = flight[fi].0.clone();
        deliver_to(&mut ps[r], &op, buffer, stats);
        flight[fi].1.retain(|&x| x != r);
    }
    flight.retain(|(_, missing)| !missing.is_empty());
}

fn sorted_ids(p: &Party) -> Vec<MemberId> {
    let mut ids: Vec<MemberId> = p.c.member_ids().collect();
    ids.sort();
    ids
}

/// One randomized run. Ok(true): the replicas converged. Ok(false): nobody was left to settle, so
/// there was nothing to compare. Err: the reason they did not converge.
fn trial(seed: u64, buffer: bool, stats: &mut Stats) -> Result<bool, String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let doc_id = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let mut ps: Vec<Party> = (0..GROUP).map(|_| Party::new(doc_id, Member::new(&mut rng))).collect();
    // Setup, in order, as in `build`.
    for i in 0..GROUP {
        let (id, pk) = (ps[i].id(), ps[i].m.pk);
        let op = ps[0].add(id, pk).expect("add").expect("new member");
        for p in ps.iter_mut().skip(1) {
            p.merge(&op).expect("in-order add");
        }
    }
    let settle = ps[0].update(&mut rng).expect("settle");
    for p in ps.iter_mut().skip(1) {
        p.merge(&settle).expect("in-order update");
    }
    // Spare members wait to be added. They get no replica; only the five replicas are checked.
    let mut spares: Vec<(MemberId, ShareKey)> = (0..3)
        .map(|_| {
            let m = Member::new(&mut rng);
            (m.id, m.pk)
        })
        .collect();

    let mut flight: Vec<(Op, Vec<usize>)> = Vec::new();
    for _ in 0..ROUNDS {
        let mut actors: Vec<usize> = (0..GROUP).collect();
        actors.shuffle(&mut rng);
        actors.truncate(rng.gen_range(1..=3));
        for a in actors {
            let roll = rng.gen_range(0..10);
            let made = if roll < 6 {
                ps[a].update(&mut rng).map(Some)
            } else if roll < 8 {
                let own = ps[a].id();
                let mut view: Vec<MemberId> = sorted_ids(&ps[a]).into_iter().filter(|id| *id != own).collect();
                if view.is_empty() {
                    Ok(None)
                } else {
                    let target = view.swap_remove(rng.gen_range(0..view.len()));
                    ps[a].remove(target)
                }
            } else {
                match spares.pop() {
                    Some((id, pk)) => ps[a].add(id, pk),
                    None => Ok(None),
                }
            };
            match made {
                Ok(Some(op)) => {
                    stats.max_predecessors = stats.max_predecessors.max(op.payload.predecessors().len());
                    let others = (0..GROUP).filter(|&i| i != a).collect();
                    flight.push((op, others));
                }
                Ok(None) => {}
                Err(e) => *stats.errors.entry(format!("{e:?}").chars().take(40).collect()).or_default() += 1,
            }
        }
        deliver_some(&mut ps, &mut flight, 0.5, buffer, &mut rng, stats);
    }
    // Flush: everything still in flight, then anything buffered.
    deliver_some(&mut ps, &mut flight, 1.0, buffer, &mut rng, stats);
    if !flight.is_empty() {
        return Err("ops still in flight after flush".into());
    }
    // Settle: the first member, in random order, that still has a leaf after replaying its pending
    // ops commits, and everyone receives it. A member's view of who is present can be stale until a
    // replay, so the harness asks the replica itself rather than reading member_ids().
    let mut order: Vec<usize> = (0..GROUP).collect();
    order.shuffle(&mut rng);
    let mut settled = None;
    for s in order {
        match ps[s].update(&mut rng) {
            Ok(op) => {
                settled = Some((s, op));
                break;
            }
            Err(CgkaError::IdentifierNotFound) | Err(CgkaError::NoMembers) => {}
            Err(e) => return Err(format!("settle: {e:?}")),
        }
    }
    let Some((s, last)) = settled else {
        return Ok(false);
    };
    let mut settle_flight = vec![(last, (0..GROUP).filter(|&i| i != s).collect())];
    deliver_some(&mut ps, &mut settle_flight, 1.0, buffer, &mut rng, stats);

    if ps.iter().any(|p| !p.buffer.is_empty()) {
        return Err("ops left buffered after settle".into());
    }
    let members = sorted_ids(&ps[s]);
    for p in ps.iter() {
        if sorted_ids(p) != members {
            return Err("membership views differ".into());
        }
    }
    let mut key: Option<PcsKey> = None;
    for p in ps.iter_mut() {
        let is_member = members.contains(&p.id());
        match (is_member, p.root()) {
            (true, Ok(k)) => match key {
                None => key = Some(k),
                Some(first) if first == k => {}
                Some(_) => return Err("member keys differ".into()),
            },
            (true, Err(e)) => return Err(format!("member cannot derive root: {e:?}")),
            (false, Ok(_)) => return Err("removed member derives root".into()),
            (false, Err(_)) => {}
        }
    }
    Ok(true)
}

pub fn fuzz(trials: usize, buffer: bool) {
    let mut stats = Stats::default();
    panic::set_hook(Box::new(|info| {
        PANICS.lock().expect("panic log").push(info.to_string());
    }));
    for t in 0..trials {
        let seed = 0xf0ee_0000 + t as u64;
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| trial(seed, buffer, &mut stats)));
        match outcome {
            Ok(Ok(true)) => stats.converged += 1,
            Ok(Ok(false)) => stats.emptied += 1,
            Ok(Err(reason)) => *stats.failures.entry(reason).or_default() += 1,
            Err(_) => *stats.failures.entry("panic (see below)".into()).or_default() += 1,
        }
    }
    let _ = panic::take_hook();

    println!("## Randomized run: {trials} trials, {ROUNDS} rounds, group of {GROUP}, buffering={buffer}");
    println!("converged: {} of {trials}", stats.converged);
    println!("nobody left to settle (not compared): {}", stats.emptied);
    println!("out-of-order deliveries buffered: {}", stats.buffered_events);
    println!("out-of-order deliveries dropped: {}", stats.dropped);
    println!("max predecessors named by one op: {}", stats.max_predecessors);
    println!("failures by reason:");
    for (reason, count) in &stats.failures {
        println!("  {count:>4}  {reason}");
    }
    println!("merge or op errors (other than out-of-order):");
    for (err, count) in &stats.errors {
        println!("  {count:>4}  {err}");
    }
    let panics = PANICS.lock().expect("panic log");
    println!("panics: {}", panics.len());
    let mut seen: Vec<String> = Vec::new();
    for msg in panics.iter() {
        let first = msg.lines().take(2).collect::<Vec<_>>().join(" | ");
        if !seen.contains(&first) && seen.len() < 5 {
            seen.push(first);
        }
    }
    for msg in seen {
        println!("  {msg}");
    }
}
