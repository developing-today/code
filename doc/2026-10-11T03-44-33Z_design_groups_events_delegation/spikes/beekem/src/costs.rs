//! Q1: bytes and CPU per commit at one group size, plus per-member state and what a joiner gets.
//!
//! The committer is member 0 on one canonical replica. Other members' commits are computed on that
//! replica with the owner swapped to them, which gives the same op a replica of theirs would make
//! (same tree, their leaf key). Member 1 is a real receiver: it merges every op and derives the key.

use crate::common::{block, contains, mean, median, micros, wire_len, Member};
use beekem::{cgka::Cgka, id::TreeId, keys::ShareKeyMap, pcs_key::PcsKey};
use future_form::Local;
use keyhive_crypto::{
    share_key::{ShareKey, ShareSecretKey},
    signed::Signed,
    signer::memory::MemorySigner,
    verifiable::Verifiable,
};
use beekem::operation::CgkaOperation;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{sync::Arc, time::Instant};

type Op = Arc<Signed<CgkaOperation>>;
type Leaf = (ShareKey, ShareSecretKey);

/// Measurements for one kind of commit.
#[derive(Default)]
struct Acc {
    label: &'static str,
    bytes: Vec<usize>,
    committer_us: Vec<f64>,
    receiver_us: Vec<f64>,
}

impl Acc {
    fn new(label: &'static str) -> Self {
        Self { label, ..Default::default() }
    }

    fn add(&mut self, bytes: usize, committer_us: f64, receiver_us: f64) {
        self.bytes.push(bytes);
        self.committer_us.push(committer_us);
        self.receiver_us.push(receiver_us);
    }

    fn row(&mut self, n: usize) -> String {
        format!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {:.0} |",
            self.label,
            self.bytes.len(),
            mean(&self.bytes),
            mean(&self.bytes) * (n - 1) as f64,
            median(&mut self.committer_us),
            median(&mut self.receiver_us),
        )
    }
}

/// Commit as `who` on the canonical replica. Returns the op, the root key it produced, and the
/// committer's CPU time.
fn commit_as(state: &mut Cgka, who: &Member, leaf: &mut Leaf, rng: &mut StdRng) -> (Op, PcsKey, f64) {
    let saved_id = std::mem::replace(&mut state.owner_id, who.id);
    let saved_sks = std::mem::replace(&mut state.owner_sks, ShareKeyMap::new());
    state.owner_sks.insert(leaf.0, leaf.1);
    let sk = ShareSecretKey::generate(rng);
    let pk = sk.share_key();
    let t = Instant::now();
    let (pcs, op, _) = block(state.update::<Local, _, StdRng>(pk, sk, &who.signer, rng)).expect("update");
    let cpu = micros(t.elapsed());
    state.owner_id = saved_id;
    state.owner_sks = saved_sks;
    *leaf = (pk, sk);
    (Arc::new(op), pcs, cpu)
}

/// Merge on a receiver and return its CPU time.
fn receive(replica: &mut Cgka, op: &Op) -> f64 {
    let t = Instant::now();
    replica.merge_concurrent_operation(op.clone()).expect("receiver merges an in-order op");
    micros(t.elapsed())
}

/// Check that the receiver derives the key the committer produced.
fn check_key(receiver: &mut Cgka, expected: &PcsKey, when: &str) {
    let got = receiver
        .pcs_key_from_tree_root()
        .unwrap_or_else(|e| panic!("{when}: receiver cannot derive the root key: {e:?}"));
    assert!(got == *expected, "{when}: receiver derived a different root key");
}

/// `fill` members (1..=fill) commit once in the fill pass. Use fewer than n-1 when the full fill is
/// too slow: the op history grows about with n^1.9, and the rows are then transient, not steady state.
pub fn run(n: usize, fill: usize) {
    let mut rng = StdRng::seed_from_u64(0xbee0 ^ n as u64);
    let doc_id = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let members: Vec<Member> = (0..n).map(|_| Member::new(&mut rng)).collect();
    let mut leaves: Vec<Leaf> = members.iter().map(|m| (m.pk, m.sk)).collect();
    let mut author = Cgka::new(doc_id, members[0].id, members[0].keys());
    let mut observer = Cgka::new(doc_id, members[1].id, members[1].keys());
    let mut epoch_keys: Vec<PcsKey> = Vec::new();
    let mut rows: Vec<String> = Vec::new();

    // 1. Build: one add per member, each merged by the receiver.
    let mut build = Acc::new("add (during build)");
    for m in &members {
        let t = Instant::now();
        let op = block(author.add::<Local, _>(m.id, m.pk, &members[0].signer))
            .expect("add")
            .expect("new member");
        let committer_us = micros(t.elapsed());
        let op = Arc::new(op);
        build.add(wire_len(&op), committer_us, receive(&mut observer, &op));
    }
    rows.push(build.row(n));

    // 2. First commit after the build. Every add blanked its path, so the first path encryption
    //    covers whole sibling subtrees.
    let mut first = Acc::new("update, first after build");
    let (op, key, c) = commit_as(&mut author, &members[0], &mut leaves[0], &mut rng);
    first.add(wire_len(&op), c, receive(&mut observer, &op));
    check_key(&mut observer, &key, "first commit");
    epoch_keys.push(key);
    rows.push(first.row(n));
    let after_create = bincode::serialized_size(&observer).expect("state serializes");

    // 3. Fill pass: members 1..=fill commit once. A full fill makes every inner node hold a secret.
    let mut fill_acc = Acc::new("update, fill pass");
    let mut fill_tail = Acc::new("update, fill pass, last 10%");
    for k in 1..=fill {
        let (op, key, c) = commit_as(&mut author, &members[k], &mut leaves[k], &mut rng);
        let r = receive(&mut observer, &op);
        if k == 1 {
            observer.owner_sks.insert(leaves[1].0, leaves[1].1);
        }
        let bytes = wire_len(&op);
        fill_acc.add(bytes, c, r);
        if k >= fill * 9 / 10 {
            fill_tail.add(bytes, c, r);
        }
        epoch_keys.push(key);
    }
    let fill_total: usize = fill_acc.bytes.iter().sum();
    rows.push(fill_acc.row(n));
    rows.push(fill_tail.row(n));
    check_key(&mut observer, epoch_keys.last().expect("a commit"), "end of fill");

    // 4. Steady state: random committers on the filled tree. Also time signature verification,
    //    which receivers must do because the crate does not.
    let mut steady = Acc::new("update, steady state (20 random committers)");
    let mut verify = Vec::new();
    for _ in 0..20 {
        let k = rng.gen_range(0..n);
        let (op, key, c) = commit_as(&mut author, &members[k], &mut leaves[k], &mut rng);
        let r = receive(&mut observer, &op);
        if k == 1 {
            observer.owner_sks.insert(leaves[1].0, leaves[1].1);
        }
        steady.add(wire_len(&op), c, r);
        let t = Instant::now();
        op.try_verify().expect("signature verifies");
        verify.push(micros(t.elapsed()));
        epoch_keys.push(key);
    }
    rows.push(steady.row(n));
    let verify_us = median(&mut verify);

    // 5. Remove a member at the middle leaf, then the commits that restore a root key.
    let victim = n / 2;
    let mut remove = Acc::new("remove (steady state)");
    let t = Instant::now();
    let rm = block(author.remove::<Local, _>(members[victim].id, &members[0].signer))
        .expect("remove")
        .expect("member present");
    let c = micros(t.elapsed());
    let rm = Arc::new(rm);
    remove.add(wire_len(&rm), c, receive(&mut observer, &rm));
    rows.push(remove.row(n));

    let mut after_rm = Acc::new("update, first after remove");
    let (op, key, c) = commit_as(&mut author, &members[2], &mut leaves[2], &mut rng);
    after_rm.add(wire_len(&op), c, receive(&mut observer, &op));
    check_key(&mut observer, &key, "first commit after remove");
    epoch_keys.push(key);
    rows.push(after_rm.row(n));

    let mut after_rm2 = Acc::new("update, second after remove");
    let (op, key, c) = commit_as(&mut author, &members[3], &mut leaves[3], &mut rng);
    after_rm2.add(wire_len(&op), c, receive(&mut observer, &op));
    epoch_keys.push(key);
    rows.push(after_rm2.row(n));

    // 6. Add after the removal, then the update that restores a root key.
    let newcomer = Member::new(&mut rng);
    let mut add = Acc::new("add (steady state)");
    let t = Instant::now();
    let op = block(author.add::<Local, _>(newcomer.id, newcomer.pk, &members[0].signer))
        .expect("add")
        .expect("new member");
    let c = micros(t.elapsed());
    let op = Arc::new(op);
    add.add(wire_len(&op), c, receive(&mut observer, &op));
    rows.push(add.row(n));

    let mut after_add = Acc::new("update, first after add");
    let (op, key, c) = commit_as(&mut author, &members[4], &mut leaves[4], &mut rng);
    after_add.add(wire_len(&op), c, receive(&mut observer, &op));
    check_key(&mut observer, &key, "first commit after add");
    epoch_keys.push(key);
    rows.push(after_add.row(n));

    // Print the table: bytes per op, the sender's egress if it unicasts to all n-1 members, and CPU.
    println!("## N = {n} (committer: member 0; receiver: member 1); fill pass: members 1..={fill} of {}\n", n - 1);
    if fill < n - 1 {
        println!("NOTE: partial fill. Rows after the fill pass are transient, not steady state.\n");
    }
    println!("| Commit | Samples | Bytes per op (one copy) | Committer egress, unicast to n-1 | Committer CPU median (µs) | Receiver merge median (µs) |");
    println!("|---|---|---|---|---|---|");
    for r in &rows {
        println!("{r}");
    }
    println!("\nSignature verification (receiver, steady-state ops), median: {verify_us:.0} µs");
    println!("Fill pass total bytes ({fill} ops): {fill_total}");

    // 7. Per-member state of the receiver. The op graph holds every op; owner_sks holds this member's
    //    keys. The rest of the bytes (tree, graph index, cached keys) are reported as a residual.
    let state = bincode::serialized_size(&observer).expect("state serializes");
    let ops_bytes: usize = observer
        .ops()
        .expect("ops sort")
        .iter()
        .flat_map(|epoch| epoch.iter())
        .map(|op| wire_len(op))
        .sum();
    let owner_bytes = bincode::serialized_size(&observer.owner_sks).expect("keys serialize");
    println!("\n### Receiver state (member 1)");
    println!("ops in graph: {}; members: {}", observer.ops_count(), observer.group_size());
    println!("serialized state right after creation (adds + first update): {after_create} B");
    println!("total serialized state at end of run: {state} B");
    println!("  op bytes (all ops, wire form): {ops_bytes} B");
    println!("  owner_sks: {owner_bytes} B");
    println!("  residual (tree, graph index, cached keys): {} B", state - ops_bytes as u64 - owner_bytes);

    // 8. What a joiner gets today: a replica of the state, owned by the joiner (keys stripped).
    let joiner = Member::new(&mut rng);
    let welcome = observer
        .with_new_owner(joiner.id, joiner.keys())
        .expect("replica copy");
    let welcome_bytes = bincode::serialized_size(&welcome).expect("serializes");
    println!("\n### Joiner payload");
    println!("replica copy (with_new_owner, serialized): {welcome_bytes} B");
    println!("op log alone (replayed by joiner): {ops_bytes} B");

    // 9. Leak check: a replica copy that is sent as-is carries the sender's secrets and every cached
    //    epoch key. Serialize the committer's own replica, which cached every epoch key it derived.
    let naive = bincode::serialize(&author).expect("serializes");
    let leaked_epoch_keys = epoch_keys
        .iter()
        .filter(|k| contains(&naive, k.0.as_slice()))
        .count();
    println!("\n### Leak check (committer's replica, serialized as-is: {} B)", naive.len());
    println!("sender's own leaf secret key present: {}", contains(&naive, members[0].sk.as_slice()));
    println!("epoch keys present: {leaked_epoch_keys} of {}", epoch_keys.len());
}
