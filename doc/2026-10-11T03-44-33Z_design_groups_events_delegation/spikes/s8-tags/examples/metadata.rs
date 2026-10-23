//! Metadata cost tables for RESULTS.md. Run: `cargo run --release --example metadata`.
//!
//! Three sizes per case. `state` is the live dots plus the version vector (the design). `tomb` is
//! the same state plus one tombstone per removed add, grouped by device (the baseline). `log` is
//! every op with its 32-byte BLAKE3 key.

use std::collections::{BTreeMap, BTreeSet};

use cbor_core::Value;
use s8_tags::{Body, Device, Dot, Entry, Op, Replica, Rng, State, encode_op, encode_state, op_id, reference};

fn device(i: usize) -> Device {
    let mut d = [0u8; 32];
    d[0] = u8::try_from(i + 1).expect("few devices");
    d
}

const ENTRIES: usize = 24;

/// Entry `idx` of a fixed space of 24: 4 subjects x 2 names x 3 values.
fn entry_of(idx: usize) -> Entry {
    let mut subject = [0u8; 32];
    subject[0] = u8::try_from(idx % 4).expect("small");
    Entry {
        subject,
        name: ["tag", "kind"][idx / 4 % 2].to_owned(),
        value: [None, Some("v1".to_owned()), Some("v2".to_owned())][idx / 8 % 3].clone(),
    }
}

/// A unique entry per index, with a BLAKE3 subject as an event hash would be.
fn distinct_entry(i: usize) -> Entry {
    Entry {
        subject: *blake3::hash(&(i as u64).to_be_bytes()).as_bytes(),
        name: "tag".to_owned(),
        value: Some("v".to_owned()),
    }
}

fn tombstone_bytes(state: &State, applied: &BTreeMap<Dot, Op>) -> usize {
    let mut by_device: BTreeMap<Device, Vec<u64>> = BTreeMap::new();
    for (dot, op) in applied {
        if matches!(op.body, Body::Set(_)) && !state.live.contains_key(dot) {
            by_device.entry(dot.device).or_default().push(dot.seq);
        }
    }
    let tombs = Value::array(
        by_device
            .into_iter()
            .map(|(d, seqs)| {
                Value::array(vec![
                    Value::byte_string(d.to_vec()),
                    Value::array(seqs.into_iter().map(Value::from).collect::<Vec<_>>()),
                ])
            })
            .collect::<Vec<_>>(),
    );
    encode_state(state).len() + tombs.encode().len()
}

fn log_bytes(ops: &[Op]) -> usize {
    ops.iter().map(|op| encode_op(op).len() + op_id(op).len()).sum()
}

/// Table 1: one entry, one device, `cycles` of set-then-clear, then one final set.
fn table_churn() {
    println!("## T1. One tag entry, one device: add then clear, repeated\n");
    println!("| cycles | ops | live dots | state B | tomb B | log B |");
    println!("|---:|---:|---:|---:|---:|---:|");
    for cycles in [0usize, 1, 10, 100, 1000, 10000] {
        let mut rep = Replica::new(device(0));
        let mut ops = Vec::new();
        for _ in 0..cycles {
            let add = rep.set(entry_of(0));
            ops.push(add.clone());
            ops.push(rep.clear(BTreeSet::from([add.dot])));
        }
        ops.push(rep.set(entry_of(0)));
        println!(
            "| {cycles} | {} | {} | {} | {} | {} |",
            ops.len(),
            rep.state.live.len(),
            encode_state(&rep.state).len(),
            tombstone_bytes(&rep.state, &rep.applied),
            log_bytes(&ops),
        );
    }
    println!();
}

/// Table 2: N distinct entries, each set once and the first half cleared. Per-live-entry cost.
fn table_distinct() {
    println!("## T2. Distinct entries, one device: set each once, clear half\n");
    println!("| entries | live | state B | state B per live entry | tomb B |");
    println!("|---:|---:|---:|---:|---:|");
    for n in [1usize, 10, 100, 1000] {
        let mut rep = Replica::new(device(0));
        let mut dots = Vec::new();
        for i in 0..n {
            dots.push(rep.set(distinct_entry(i)).dot);
        }
        rep.clear(dots[..n / 2].iter().copied().collect());
        let live = rep.state.live.len();
        let state = encode_state(&rep.state).len();
        println!(
            "| {n} | {live} | {state} | {:.1} | {} |",
            state as f64 / live.max(1) as f64,
            tombstone_bytes(&rep.state, &rep.applied),
        );
    }
    println!();
}

/// A random run with partial gossip. Returns every replica and every issued op, in issue order.
fn random_run(devices: usize, steps: usize, seed: u64) -> (Vec<Replica>, Vec<Op>) {
    let mut rng = Rng::new(seed);
    let mut reps: Vec<Replica> = (0..devices).map(|i| Replica::new(device(i))).collect();
    let mut issued = Vec::new();
    for _ in 0..steps {
        let r = rng.below(devices);
        if rng.below(10) < 6 {
            issued.push(reps[r].set(entry_of(rng.below(ENTRIES))));
        } else {
            let live: Vec<Dot> = reps[r].state.live.keys().copied().collect();
            if !live.is_empty() {
                let names: BTreeSet<Dot> = (0..=rng.below(3)).map(|_| live[rng.below(live.len())]).collect();
                issued.push(reps[r].clear(names));
            }
        }
        if rng.below(10) < 3 {
            let other = rng.below(devices);
            if other != r {
                let src = reps[r].clone();
                reps[other].merge(&src);
            }
        }
    }
    (reps, issued)
}

/// Table 3: random multi-device runs. Final state is the join of all devices.
fn table_multi_device() {
    println!("## T3. Random run, several devices, partial gossip (3000 steps, seed 20261011)\n");
    println!("| devices | ops | live dots | vv entries | state B | tomb B | log B | join = reference |");
    println!("|---:|---:|---:|---:|---:|---:|---:|---|");
    for devices in [2usize, 5] {
        let (reps, issued) = random_run(devices, 3000, 20261011);
        let mut full = Replica::new(device(devices));
        for rep in &reps {
            full.merge(rep);
        }
        let ok = full.state == reference(issued.iter());
        println!(
            "| {devices} | {} | {} | {} | {} | {} | {} | {ok} |",
            issued.len(),
            full.state.live.len(),
            full.state.vv.len(),
            encode_state(&full.state).len(),
            tombstone_bytes(&full.state, &full.applied),
            log_bytes(&issued),
        );
    }
    println!();
}

/// Table 4: size of one clear, as the names and the clock grow.
fn table_clear_size() {
    println!("## T4. One clear op: bytes by number of names and devices in the clock\n");
    println!("| names | devices in clock | clear op B |");
    println!("|---:|---:|---:|");
    for names_n in [1usize, 10, 100] {
        for devices in [1usize, 5, 20] {
            let names: BTreeSet<Dot> = (0..names_n)
                .map(|k| Dot {
                    device: device(0),
                    seq: u64::try_from(k + 1).expect("small"),
                })
                .collect();
            let clock = (0..devices).map(|d| (device(d), 1000u64)).collect();
            let op = Op {
                dot: Dot { device: device(0), seq: 5000 },
                body: Body::Clear { names, clock },
            };
            println!("| {names_n} | {devices} | {} |", encode_op(&op).len() + op_id(&op).len());
        }
    }
    println!();
}

/// Table 5: compaction at a cut. The snapshot is the reference over the prefix, and the tail is the rest.
fn table_compaction() {
    println!("## T5. Compaction of the 5-device run at a causal cut (cut = prefix of issue order)\n");
    println!("| cut at | dropped ops | snapshot B | tail B | dropped headers B (32 each) | total B | uncompacted log B |");
    println!("|---:|---:|---:|---:|---:|---:|---:|");
    let (_reps, issued) = random_run(5, 3000, 20261011);
    let full = log_bytes(&issued);
    for pct in [50usize, 90, 99] {
        let cut = issued.len() * pct / 100;
        let snapshot = encode_state(&reference(issued[..cut].iter())).len();
        let tail = log_bytes(&issued[cut..]);
        let headers = 32 * cut;
        println!(
            "| {pct}% | {cut} | {snapshot} | {tail} | {headers} | {} | {full} |",
            snapshot + tail + headers
        );
    }
    println!();
}

fn main() {
    table_churn();
    table_distinct();
    table_multi_device();
    table_clear_size();
    table_compaction();
}
