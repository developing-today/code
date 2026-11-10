//! Q4: forward secrecy and post-compromise security, checked with the crate's own derivations.
//!
//! A compromise is a copy of a replica (its secret keys and cached state). The attacker then gets
//! the public ops it asks for and tries to derive keys with the copy.

use crate::party::{build, Op};
use beekem::{cgka::Cgka, pcs_key::PcsKey};
use keyhive_crypto::digest::Digest;
use rand::{rngs::StdRng, SeedableRng};

/// A copy of a replica, given the public ops in order.
fn attacker(snapshot: &Cgka, ops: &[&Op]) -> Cgka {
    let mut a = snapshot.clone();
    for op in ops {
        a.merge_concurrent_operation((*op).clone()).expect("public ops apply in order");
    }
    a
}

/// Whether the attacker derives `key` as the current root.
fn reads_now(a: &mut Cgka, key: &PcsKey) -> bool {
    a.pcs_key_from_tree_root().is_ok_and(|k| k == *key)
}

/// Whether the attacker derives an earlier epoch's key, given the op that created that epoch.
fn reads_epoch(a: &mut Cgka, key: &PcsKey, creating_op: &Op) -> bool {
    a.secret(&Digest::hash(key), &Digest::hash(&**creating_op))
        .is_ok_and(|k| k == *key)
}

/// c is compromised but stays in the group. Its copy must not read a root after c itself commits.
fn in_group_compromise(rng: &mut StdRng) {
    println!("## T1: c's replica is copied (compromise); c stays in the group");
    let (mut ps, _, op0, k0) = build(4, rng);
    // c derives the root as a member does, so the copy holds the path secrets c used.
    assert!(ps[2].root().is_ok_and(|k| k == k0), "c derives the epoch-0 root");
    let copy = ps[2].c.clone();

    let op_b = ps[1].update(rng).expect("b updates");
    for i in [0, 2, 3] {
        ps[i].merge(&op_b).expect("in order");
    }
    let k1 = ps[0].root().expect("root after b");
    let mut a1 = attacker(&copy, &[&op_b]);
    println!("  after b's update, the copy derives the new root: {}", reads_now(&mut a1, &k1));

    let op_c = ps[2].update(rng).expect("c updates");
    for i in [0, 1, 3] {
        ps[i].merge(&op_c).expect("in order");
    }
    let k2 = ps[0].root().expect("root after c");
    let mut a2 = attacker(&copy, &[&op_b, &op_c]);
    println!("  after c's own update, the copy derives the new root: {}", reads_now(&mut a2, &k2));
    println!("  the copy derives the epoch-0 root (earlier than the compromise): {}", reads_epoch(&mut a2, &k0, &op0));
}

/// a removes c. Two attackers: c informed of its removal, and c fed only a's later update.
fn removal(rng: &mut StdRng) {
    println!("## T2: a removes c; c's copy is then fed public ops");
    let (mut ps, _, op0, k0) = build(4, rng);
    assert!(ps[2].root().is_ok_and(|k| k == k0), "c derives the epoch-0 root");
    let copy = ps[2].c.clone();
    let victim = ps[2].id();

    let rm = ps[0].remove(victim).expect("remove").expect("c present");
    for i in [1, 3] {
        ps[i].merge(&rm).expect("in order");
    }
    let op_a = ps[0].update(rng).expect("a updates");
    for i in [1, 3] {
        ps[i].merge(&op_a).expect("in order");
    }
    let k1 = ps[0].root().expect("root after removal");
    println!("  b (a member) derives the post-removal root: {}", ps[1].root().is_ok_and(|k| k == k1));

    let mut informed = attacker(&copy, &[&rm, &op_a]);
    println!("  c, informed of its removal, derives a root: {}", informed.pcs_key_from_tree_root().is_ok());
    // A c that ignores its removal cannot accept a's later update: the update names the removal.
    let mut ignorant = copy.clone();
    println!(
        "  c that ignores its removal, given a's later update: {:?}",
        ignorant.merge_concurrent_operation(op_a.clone()).err()
    );
    println!("  c's copy derives the pre-removal (epoch-0) root: {}", reads_epoch(&mut informed, &k0, &op0));
}

pub fn run() {
    let mut rng = StdRng::seed_from_u64(0x5ec2_0001);
    in_group_compromise(&mut rng);
    println!();
    removal(&mut rng);
}
