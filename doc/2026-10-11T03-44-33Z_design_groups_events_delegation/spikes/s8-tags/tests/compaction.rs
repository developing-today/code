//! Compacting clear bodies, and the snapshot-then-tail rule proposed in RESULTS.md.

use std::collections::BTreeSet;

use s8_tags::{Body, Device, Dot, Entry, Op, Replica, Rng, covers, reference};

fn device(i: usize) -> Device {
    let mut d = [0u8; 32];
    d[0] = u8::try_from(i + 1).expect("few devices");
    d
}

fn entry(subject: usize) -> Entry {
    let mut s = [0u8; 32];
    s[0] = u8::try_from(subject).expect("small");
    Entry {
        subject: s,
        name: "tag".into(),
        value: Some("v".into()),
    }
}

/// Devices 0, 1 and 2 all hold the add and the clear that removed it, so the clear is stable.
/// Device 2 (A) is the one that compacts. Device 3 (N) joins later.
fn stable_removal() -> (Replica, Replica, Replica) {
    let mut x = Replica::new(device(0));
    let mut y = Replica::new(device(1));
    let mut a = Replica::new(device(2));
    let add = x.set(entry(9));
    y.receive(add.clone());
    let clear = y.clear(BTreeSet::from([add.dot]));
    x.receive(clear.clone());
    a.receive(add);
    a.receive(clear);
    (x, a, Replica::new(device(3)))
}

#[test]
fn dropping_a_stable_clear_body_resurrects_the_add_for_a_new_device() {
    let (_x, a, mut n) = stable_removal();
    assert!(a.state.live.is_empty(), "A removed the add");

    // Naive rule: keep the add body, drop the clear body (keep only its header). N replays A's bodies.
    let add_body = a
        .applied
        .values()
        .find(|op| matches!(op.body, Body::Set(_)))
        .cloned();
    n.receive(add_body.expect("A holds the add"));

    assert_eq!(n.state.live.len(), 1, "N holds the removed add: divergence");
    assert_ne!(n.state, a.state);
}

#[test]
fn serving_the_snapshot_to_a_new_device_is_correct() {
    let (_x, a, mut n) = stable_removal();
    // Snapshot rule: a peer that does not cover the cut gets A's state, then the tail.
    n.merge(&a);
    assert_eq!(n.state, a.state);
    assert_eq!(n.pending_len(), 0);
}

/// Three devices issue sets and clears in causal order, with partial gossip. Returns the replicas
/// as they stand (partly synced) and every op in issue order.
fn random_history(seed: u64) -> (Vec<Replica>, Vec<Op>) {
    let mut rng = Rng::new(seed);
    let n = 3;
    let mut reps: Vec<Replica> = (0..n).map(|i| Replica::new(device(i))).collect();
    let mut issued = Vec::new();
    for _ in 0..120 {
        let r = rng.below(n);
        if rng.below(10) < 6 {
            issued.push(reps[r].set(entry(rng.below(4))));
        } else {
            let live: Vec<Dot> = reps[r].state.live.keys().copied().collect();
            if !live.is_empty() {
                let names: BTreeSet<Dot> = (0..=rng.below(2)).map(|_| live[rng.below(live.len())]).collect();
                issued.push(reps[r].clear(names));
            }
        }
        if rng.below(10) < 3 {
            let other = rng.below(n);
            if other != r {
                let src = reps[r].clone();
                reps[other].merge(&src);
            }
        }
    }
    (reps, issued)
}

/// The proposed rule, on random histories. For every cut and every peer: a peer that covers the
/// cut gets only the tail. Any other peer first takes the snapshot (the reference over the cut),
/// then the tail. Either way it must end at the reference over the whole history.
#[test]
fn snapshot_then_tail_is_correct_for_every_peer_and_cut() {
    for seed in 0..200 {
        let (reps, issued) = random_history(seed);
        let want = reference(&issued);
        for cut in [0, issued.len() / 3, issued.len() / 2, issued.len()] {
            let prefix = &issued[..cut];
            let mut snapshot = Replica::new(device(99));
            snapshot.state = reference(prefix);
            for peer in &reps {
                let mut p = peer.clone();
                if !prefix.iter().all(|op| covers(&p.state.vv, &op.dot)) {
                    p.merge(&snapshot);
                }
                for op in &issued[cut..] {
                    p.receive(op.clone());
                }
                assert_eq!(p.state, want, "seed {seed}, cut {cut}");
                assert_eq!(p.pending_len(), 0, "seed {seed}, cut {cut}");
            }
        }
    }
}
