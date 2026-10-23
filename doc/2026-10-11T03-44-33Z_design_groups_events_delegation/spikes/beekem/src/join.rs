//! Q3: what a joiner gets, and whether it can start from a welcome without history.

use crate::{
    common::{contains, Member},
    party::build,
};
use beekem::cgka::Cgka;
use keyhive_crypto::digest::Digest;
use rand::{rngs::StdRng, SeedableRng};

pub fn run() {
    let mut rng = StdRng::seed_from_u64(0x501a_0001);
    let (mut ps, doc_id, op0, k0) = build(4, &mut rng);

    // The adder adds the joiner, then updates: an add blanks the path, so the root needs an update.
    let joiner = Member::new(&mut rng);
    let add = ps[0].add(joiner.id, joiner.pk).expect("add").expect("new member");
    for i in 1..4 {
        ps[i].merge(&add).expect("in order");
    }
    let upd = ps[0].update(&mut rng).expect("update after add");
    for i in 1..4 {
        ps[i].merge(&upd).expect("in order");
    }
    let k1 = ps[0].root().expect("root");

    // The welcome the crate allows today: a copy of the adder's replica, owned by the joiner.
    let welcome = ps[0].c.with_new_owner(joiner.id, joiner.keys()).expect("copy");
    let welcome_wire = bincode::serialize(&welcome).expect("serializes");
    println!("## Welcome as a replica copy");
    println!("  serialized size: {} B", welcome_wire.len());
    println!("  adder's leaf secret key present: {}", contains(&welcome_wire, ps[0].m.sk.as_slice()));
    println!("  epoch-0 root key present: {}", contains(&welcome_wire, k0.0.as_slice()));

    let mut j = welcome;
    println!("  joiner derives the current root: {}", j.pcs_key_from_tree_root().is_ok_and(|k| k == k1));
    println!(
        "  joiner derives the epoch-0 root (it was not a member then): {}",
        j.secret(&Digest::hash(&k0), &Digest::hash(&*op0)).is_ok_and(|k| k == k0)
    );

    let op2 = ps[1].update(&mut rng).expect("b updates");
    for i in [0, 2, 3] {
        ps[i].merge(&op2).expect("in order");
    }
    let k2 = ps[0].root().expect("root");
    let merged = j.merge_concurrent_operation(op2.clone()).is_ok();
    println!(
        "  joiner merges the next update ({merged}) and derives the new root: {}",
        j.pcs_key_from_tree_root().is_ok_and(|k| k == k2)
    );

    // No welcome: a replica that starts from its own add and the latest update.
    let mut fresh = Cgka::new(doc_id, joiner.id, joiner.keys());
    println!("## Fresh replica, no history");
    println!("  merging the latest update: {:?}", fresh.merge_concurrent_operation(upd.clone()).err());
    println!("  merging the add: {:?}", fresh.merge_concurrent_operation(add.clone()).err());
}
