// Shared S3 harness. main.rs includes this file once per crate: `enc` is the unmodified
// p2panda-encryption (`base`) or the path copy (`patched`). Both runs execute the same scenarios.
//
// The includer defines `Dgm`, the membership: `MemberDgm` (a copy of `TestDgm` whose `from_welcome`
// keeps the joiner, see its doc) for `base`, and the stock `TestDgm` for `patched`. Ordering is the
// crate's `MessageOrderer`. Both need the crate's `test_utils` feature, enabled in the workspace pin.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cbor_core::Value;
use p2panda_core::cbor::encode_cbor;
use serde::{Deserialize, Serialize};

use enc::data_scheme::dcgka::{Dcgka, DirectMessage, ProcessInput};
use enc::data_scheme::group::{EncryptionGroup, GroupOutput, GroupState};
use enc::data_scheme::group_secret::{GroupSecretId, SecretBundle};
use enc::data_scheme::test_utils::ordering::{MessageOrderer, TestMessage};
use enc::key_bundle::{Lifetime, LongTermKeyBundle};
use enc::key_manager::{KeyManager, KeyManagerState};
use enc::key_registry::KeyRegistry;
use enc::test_utils::{MemberId, MessageId};
use enc::traits::{
    GroupMembership, GroupMessage, GroupMessageContent, IdentityHandle, OperationId, Ordering,
    PreKeyManager, PreKeyRegistry,
};

type Id = MemberId;
type Mid = MessageId;
// `Dgm` is defined by each including module (see main.rs).

/// The crate's `TestDgm`, with one change: `from_welcome` puts the joiner back in the membership.
///
/// `Dcgka::add` builds the welcome's `history` from the adder's membership before the add is
/// applied. The adder applies the add locally afterwards, and the joiner applies it in
/// `process_add`. Then `process_welcome` replaces the joiner's membership with the welcome's copy,
/// which lacks the joiner. `TestDgm` keeps that copy, so the joiner is never a member and is never
/// marked welcomed. Observed in the first run (see RESULTS.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDgm<ID, OP> {
    _marker: PhantomData<(ID, OP)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDgmState<ID, OP>
where
    ID: IdentityHandle,
{
    my_id: ID,
    members: HashSet<ID>,
    _marker: PhantomData<OP>,
}

impl<ID, OP> MemberDgm<ID, OP>
where
    ID: IdentityHandle + Serialize + for<'a> Deserialize<'a>,
{
    pub fn init(my_id: ID) -> MemberDgmState<ID, OP> {
        MemberDgmState {
            my_id,
            members: HashSet::new(),
            _marker: PhantomData,
        }
    }
}

impl<ID, OP> GroupMembership<ID, OP> for MemberDgm<ID, OP>
where
    ID: IdentityHandle + Serialize + for<'a> Deserialize<'a>,
    OP: OperationId + Serialize + for<'a> Deserialize<'a>,
{
    type State = MemberDgmState<ID, OP>;

    type Error = Infallible;

    fn create(my_id: ID, initial_members: &[ID]) -> Result<Self::State, Self::Error> {
        Ok(MemberDgmState {
            my_id,
            members: HashSet::from_iter(initial_members.iter().cloned()),
            _marker: PhantomData,
        })
    }

    fn from_welcome(my_id: ID, mut y: Self::State) -> Result<Self::State, Self::Error> {
        y.my_id = my_id;
        y.members.insert(my_id);
        Ok(y)
    }

    fn add(
        mut y: Self::State,
        _adder: ID,
        added: ID,
        _operation_id: OP,
    ) -> Result<Self::State, Self::Error> {
        y.members.insert(added);
        Ok(y)
    }

    fn remove(
        mut y: Self::State,
        _remover: ID,
        removed: &ID,
        _operation_id: OP,
    ) -> Result<Self::State, Self::Error> {
        y.members.remove(removed);
        Ok(y)
    }

    fn members(y: &Self::State) -> Result<HashSet<ID>, Self::Error> {
        Ok(y.members.clone())
    }
}
type Orderer = MessageOrderer<Dgm>;
type Pki = KeyRegistry<Id>;
type Y = GroupState<Id, Mid, Pki, Dgm, KeyManager, Orderer>;
type Msg = TestMessage<Dgm>;
type Dm = DirectMessage<Id, Mid, Dgm>;

const ALICE: Id = 0;
const BOB: Id = 1;
const CAROL: Id = 2;
const DAN: Id = 3;

/// What a member could read from one message.
#[derive(Clone, Debug, PartialEq)]
enum Got {
    Plain(String),
    Removed,
}

fn readable(output: GroupOutput<Id, Mid, Dgm, Orderer>) -> Option<Got> {
    match output {
        GroupOutput::Application { plaintext } => {
            Some(Got::Plain(String::from_utf8_lossy(&plaintext).into_owned()))
        }
        GroupOutput::Removed => Some(Got::Removed),
        GroupOutput::Control(_) => None,
    }
}

fn ok<T, E: std::fmt::Debug>(result: Result<T, E>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(err) => panic!("{what} failed: {err:?}"),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

fn log_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("logs")
        .join(name)
}

/// The dependency list a published message names. Read from the serialised form because the
/// test orderer keeps its `previous` field private.
fn named(msg: &Msg) -> Vec<Mid> {
    let json = serde_json::to_value(msg).expect("message serialises");
    serde_json::from_value(json["previous"].clone()).expect("previous is a list of message ids")
}

fn is_control(msg: &Msg) -> bool {
    matches!(msg.content(), GroupMessageContent::Control(_))
}

fn others(all: &[Id], skip: Id) -> Vec<Id> {
    all.iter().copied().filter(|id| *id != skip).collect()
}

/// One state per member. Every member's registry holds every member's long-term bundle, each
/// with the lifetime given for that member.
fn init_states(members: &[(Id, Lifetime)], rng: &enc::Rng) -> Vec<Y> {
    let mut managers: BTreeMap<Id, KeyManagerState> = BTreeMap::new();
    let mut bundles: BTreeMap<Id, LongTermKeyBundle> = BTreeMap::new();
    for (id, lifetime) in members {
        let identity = enc::test_utils::crypto::SecretKey::from_bytes(rng.random_array().unwrap());
        let manager = KeyManager::init_and_generate_prekey(&identity, *lifetime, rng).unwrap();
        bundles.insert(*id, KeyManager::prekey_bundle(&manager).unwrap());
        managers.insert(*id, manager);
    }
    members
        .iter()
        .map(|(id, _)| {
            let mut registry = KeyRegistry::<Id>::init();
            for (other, bundle) in &bundles {
                registry =
                    KeyRegistry::add_longterm_bundle(registry, *other, bundle.clone()).unwrap();
            }
            GroupState {
                my_id: *id,
                dcgka: Dcgka::init(*id, managers[id].clone(), registry, Dgm::init(*id)),
                orderer: MessageOrderer::<Dgm>::init(*id),
                secrets: SecretBundle::init(),
                is_welcomed: false,
            }
        })
        .collect()
}

/// Dependency counts and heads, per published message.
#[derive(Clone, Debug)]
struct Sample {
    kind: &'static str,
    label: String,
    author: Id,
    /// Dependencies the message names (what the crate's orderer emits).
    named: usize,
    /// Heads in the author's view: control messages it has seen that no other seen control
    /// message names. This is the smallest set that still names every last-known control message.
    heads: usize,
}

/// The control messages one member has seen, and which of them others name.
#[derive(Default)]
struct View {
    known: HashSet<Mid>,
    referenced: HashSet<Mid>,
}

impl View {
    fn observe(&mut self, id: Mid, deps: &[Mid]) {
        self.known.insert(id);
        self.referenced.extend(deps.iter().copied());
    }

    fn heads(&self) -> usize {
        self.known
            .iter()
            .filter(|id| !self.referenced.contains(id))
            .count()
    }
}

struct Harness {
    rng: enc::Rng,
    ys: BTreeMap<Id, Y>,
    views: BTreeMap<Id, View>,
    deps: HashMap<Mid, Vec<Mid>>,
    samples: Vec<Sample>,
}

impl Harness {
    fn new(states: Vec<Y>, rng: enc::Rng) -> Self {
        let views = states.iter().map(|y| (y.my_id, View::default())).collect();
        let ys = states.into_iter().map(|y| (y.my_id, y)).collect();
        Self {
            rng,
            ys,
            views,
            deps: HashMap::new(),
            samples: Vec::new(),
        }
    }

    fn take(&mut self, id: Id) -> Y {
        self.ys
            .remove(&id)
            .unwrap_or_else(|| panic!("state of member {id} is missing"))
    }

    fn put(&mut self, y: Y) {
        self.ys.insert(y.my_id, y);
    }

    fn deps_of(&mut self, msg: &Msg) -> Vec<Mid> {
        self.deps
            .entry(msg.id())
            .or_insert_with(|| named(msg))
            .clone()
    }

    /// Records a message this member published: a measurement sample, and its view update.
    fn published(&mut self, author: Id, msg: &Msg, label: &str) {
        let deps = self.deps_of(msg);
        let heads = self.views.get(&author).map_or(0, View::heads);
        self.samples.push(Sample {
            kind: if is_control(msg) { "control" } else { "app" },
            label: label.to_string(),
            author,
            named: deps.len(),
            heads,
        });
        if is_control(msg) {
            self.views
                .entry(author)
                .or_default()
                .observe(msg.id(), &deps);
        }
    }

    fn create(&mut self, creator: Id, members: &[Id]) -> Msg {
        let y = self.take(creator);
        let (y, msg) = ok(
            EncryptionGroup::create(y, members.to_vec(), &self.rng),
            "create",
        );
        self.put(y);
        self.published(creator, &msg, "create");
        msg
    }

    fn add(&mut self, adder: Id, added: Id) -> Msg {
        let y = self.take(adder);
        let (y, msg) = ok(EncryptionGroup::add(y, added, &self.rng), "add");
        self.put(y);
        self.published(adder, &msg, "add");
        msg
    }

    fn add_from_join(&mut self, adder: Id, added: Id) -> Msg {
        let y = self.take(adder);
        let (y, msg) = add_from_join(y, added, &self.rng);
        self.put(y);
        self.published(adder, &msg, "add (from_join)");
        msg
    }

    fn remove(&mut self, remover: Id, removed: Id) -> Msg {
        let y = self.take(remover);
        let (y, msg) = ok(EncryptionGroup::remove(y, removed, &self.rng), "remove");
        self.put(y);
        self.published(remover, &msg, "remove");
        msg
    }

    fn update(&mut self, updater: Id) -> Msg {
        let y = self.take(updater);
        let (y, msg) = ok(EncryptionGroup::update(y, &self.rng), "update");
        self.put(y);
        self.published(updater, &msg, "update");
        msg
    }

    fn send(&mut self, sender: Id, plaintext: &[u8]) -> Msg {
        let y = self.take(sender);
        let (y, msg) = ok(EncryptionGroup::send(y, plaintext, &self.rng), "send");
        self.put(y);
        self.published(sender, &msg, "app");
        msg
    }

    /// Hands `msg` to member `to`. An error here consumes the member's state, so it panics.
    fn receive(&mut self, to: Id, msg: &Msg) -> Vec<Got> {
        let y = self.take(to);
        let (y, outputs) = ok(
            EncryptionGroup::receive(y, msg),
            &format!("receive {:?} at member {to}", msg.id()),
        );
        self.put(y);
        if is_control(msg) {
            let deps = self.deps_of(msg);
            self.views.entry(to).or_default().observe(msg.id(), &deps);
        }
        outputs.into_iter().filter_map(readable).collect()
    }

    fn broadcast(&mut self, msg: &Msg, to: &[Id]) -> BTreeMap<Id, Vec<Got>> {
        to.iter().map(|id| (*id, self.receive(*id, msg))).collect()
    }

    /// Runs `receive` on a copy of `to`'s state, so the real state survives an error.
    fn probe(&self, to: Id, msg: &Msg) -> Result<Vec<Got>, String> {
        let y = self.ys[&to].clone();
        EncryptionGroup::receive(y, msg)
            .map(|(_, outputs)| outputs.into_iter().filter_map(readable).collect())
            .map_err(|err| format!("{err:?}"))
    }

    fn secrets_len(&self, id: Id) -> usize {
        self.ys[&id].secrets.len()
    }

    fn latest_secret(&self, id: Id) -> Option<GroupSecretId> {
        self.ys[&id].secrets.latest().map(|secret| secret.id())
    }

    fn describe(&self, id: Id) {
        let y = &self.ys[&id];
        let mut members: Vec<Id> = EncryptionGroup::members(y)
            .expect("members")
            .into_iter()
            .collect();
        members.sort();
        println!(
            "  member {id}: is_welcomed={} members={members:?} secrets={}",
            y.is_welcomed,
            y.secrets.len()
        );
    }

    fn summary(&self, title: &str, kind: &str) {
        let samples: Vec<&Sample> = self.samples.iter().filter(|s| s.kind == kind).collect();
        let max_named = samples.iter().map(|s| s.named).max().unwrap_or(0);
        let max_heads = samples.iter().map(|s| s.heads).max().unwrap_or(0);
        let over = samples.iter().filter(|s| s.named > 32).count();
        println!(
            "  {title}: {} {kind} messages; named max {max_named} (over 32: {over}); heads max {max_heads}",
            samples.len()
        );
    }

    fn print_samples(&self) {
        for s in &self.samples {
            println!(
                "    {:<14} author {:>2}  named {:>2}  heads {:>2}",
                s.label, s.author, s.named, s.heads
            );
        }
    }
}

/// Encodes the welcome direct message as CBOR, dumps its diagnostic form, and returns its size.
fn inspect_welcome(msg: &Msg, joiner: Id, file: &str) -> usize {
    let dm: Dm = msg
        .direct_messages()
        .into_iter()
        .find(|dm| dm.recipient == joiner)
        .expect("a direct message for the joiner");
    let bytes = encode_cbor(&dm).expect("encode the welcome");
    let tree = Value::decode_owned(&bytes).expect("decode the welcome");
    fs::write(log_path(file), format!("{tree}\n")).expect("write welcome diagnostic");
    bytes.len()
}

fn scenario(name: &str, run: fn()) {
    if let Err(panic) = std::panic::catch_unwind(run) {
        let text = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "non-string panic".into());
        println!("!! scenario {name} stopped: {text}");
    }
    println!();
}

// --- (a) full history and post-compromise security ----------------------------------------------

pub fn full_history_and_pcs() {
    println!("## (a) full history (plain add) and post-compromise security");
    let rng = enc::Rng::from_seed([1; 32]);
    let def = Lifetime::default();
    let all = [(ALICE, def), (BOB, def), (CAROL, def), (DAN, def)];
    let mut h = Harness::new(init_states(&all, &rng), rng);

    let create = h.create(ALICE, &[ALICE, BOB, CAROL]);
    h.broadcast(&create, &[BOB, CAROL, DAN]);
    let m1 = h.send(ALICE, b"m1: sent before Dan joins");
    let got = h.broadcast(&m1, &[BOB, CAROL, DAN]);
    println!("m1 read by Bob: {:?}", got[&BOB]);
    println!("m1 read by Carol: {:?}", got[&CAROL]);
    println!("m1 read by Dan (not welcomed yet): {:?}", got[&DAN]);

    let add = h.add(ALICE, DAN);
    let got = h.broadcast(&add, &[BOB, CAROL, DAN]);
    println!("Dan processes the welcome and reads: {:?}", got[&DAN]);
    h.describe(DAN);
    let bytes = inspect_welcome(&add, DAN, "a-full-history.diag");
    println!("  welcome direct message: {bytes} bytes");

    // An attacker copies Carol's device state just before her removal.
    let stolen = h.ys[&CAROL].clone();
    let remove = h.remove(ALICE, CAROL);
    let got = h.broadcast(&remove, &[BOB, DAN, CAROL]);
    println!("Carol processes the remove: {:?}", got[&CAROL]);
    let m2 = h.send(ALICE, b"m2: sent after Carol is removed");
    let got = h.broadcast(&m2, &[BOB, DAN]);
    println!("m2 read by Bob: {:?}", got[&BOB]);
    println!("m2 read by Dan: {:?}", got[&DAN]);
    match h.probe(CAROL, &m2) {
        Ok(out) => println!("Carol reads m2: {out:?} (UNEXPECTED)"),
        Err(err) => println!("Carol cannot read m2: {err}"),
    }
    // The attacker runs the copied state over the same broadcast messages.
    let (stolen, _) = ok(
        EncryptionGroup::receive(stolen, &remove),
        "attacker: remove",
    );
    match EncryptionGroup::receive(stolen, &m2) {
        Ok((_, out)) => println!(
            "attacker with pre-removal state reads m2: {:?} (UNEXPECTED)",
            out.into_iter().filter_map(readable).collect::<Vec<Got>>()
        ),
        Err(err) => println!("attacker with pre-removal state cannot read m2: {err:?}"),
    }
}

/// PCS does not cover a member who stays in the group. Their device still receives each new
/// secret over its own 2SM session, so a copied state reads what the device reads.
pub fn compromised_member_in_group() {
    println!("## (a2) rotation after a compromise, with the compromised member still in the group");
    let rng = enc::Rng::from_seed([8; 32]);
    let def = Lifetime::default();
    let all = [(ALICE, def), (BOB, def), (CAROL, def), (DAN, def)];
    let mut h = Harness::new(init_states(&all, &rng), rng);
    let create = h.create(ALICE, &[ALICE, BOB, CAROL]);
    h.broadcast(&create, &[BOB, CAROL, DAN]);
    let add = h.add(ALICE, DAN);
    h.broadcast(&add, &[BOB, CAROL, DAN]);

    // Carol's device is compromised here. The attacker copies its state.
    let stolen = h.ys[&CAROL].clone();
    let upd = h.update(ALICE);
    let got = h.broadcast(&upd, &[BOB, CAROL, DAN]);
    println!("Carol processes the update: {:?}", got[&CAROL]);
    let m3 = h.send(ALICE, b"m3: sent after the rotation");
    let got = h.broadcast(&m3, &[BOB, CAROL, DAN]);
    println!("m3 read by Carol (still a member): {:?}", got[&CAROL]);
    let (stolen, _) = ok(EncryptionGroup::receive(stolen, &upd), "attacker: update");
    match EncryptionGroup::receive(stolen, &m3) {
        Ok((_, out)) => println!(
            "attacker with the copied state reads m3: {:?}",
            out.into_iter().filter_map(readable).collect::<Vec<Got>>()
        ),
        Err(err) => println!("attacker cannot read m3: {err:?}"),
    }
}

// --- (b) welcome size against past epochs --------------------------------------------------------

pub fn welcome_size_by_epochs() {
    println!("## (b) welcome size against the number of past epochs (full history)");
    for updates in [0usize, 3, 7, 15] {
        let rng = enc::Rng::from_seed([2; 32]);
        let def = Lifetime::default();
        let all = [(ALICE, def), (BOB, def), (CAROL, def), (DAN, def)];
        let mut h = Harness::new(init_states(&all, &rng), rng);
        let create = h.create(ALICE, &[ALICE, BOB, CAROL]);
        h.broadcast(&create, &[BOB, CAROL, DAN]);
        for _ in 0..updates {
            let u = h.update(ALICE);
            h.broadcast(&u, &[BOB, CAROL, DAN]);
        }
        let before = h.secrets_len(ALICE);
        let add = h.add(ALICE, DAN);
        let after = h.secrets_len(ALICE);
        h.broadcast(&add, &[BOB, CAROL, DAN]);
        let bytes = inspect_welcome(&add, DAN, &format!("b-epochs-{}.diag", updates + 1));
        println!(
            "  adder's bundle before add: {before}, after add: {after}; welcome bytes: {bytes}; joiner's bundle after welcome: {}",
            h.secrets_len(DAN)
        );
    }
}

// --- (c) from_join ------------------------------------------------------------------------------

pub fn from_join() {
    println!("## (c) from_join: the welcome carries no earlier secret; one update rotates");
    let rng = enc::Rng::from_seed([3; 32]);
    let def = Lifetime::default();
    let all = [(ALICE, def), (BOB, def), (CAROL, def), (DAN, def)];
    let mut h = Harness::new(init_states(&all, &rng), rng);

    let create = h.create(ALICE, &[ALICE, BOB, CAROL]);
    h.broadcast(&create, &[BOB, CAROL, DAN]);
    let m1 = h.send(ALICE, b"m1: sent before Dan joins");
    // Pre-join application messages are withheld from the joiner (see from_join_hazard).
    h.broadcast(&m1, &[BOB, CAROL]);

    let add = h.add_from_join(ALICE, DAN);
    h.broadcast(&add, &[BOB, CAROL, DAN]);
    println!("after the welcome:");
    h.describe(DAN);
    match h.probe(DAN, &m1) {
        Ok(out) => println!("  Dan reads m1: {out:?} (UNEXPECTED)"),
        Err(err) => println!("  Dan cannot read m1: {err}"),
    }

    let s0 = h.latest_secret(ALICE);
    let gap = h.send(ALICE, b"gap: sent before the rotation");
    h.broadcast(&gap, &[BOB, CAROL]);
    match h.probe(DAN, &gap) {
        Ok(out) => println!("  Dan reads a message from the gap: {out:?}"),
        Err(err) => println!("  Dan cannot read a message sent between add and update: {err}"),
    }

    let upd = h.update(ALICE);
    let got = h.broadcast(&upd, &[BOB, CAROL, DAN]);
    println!("Dan processes the update: {:?}", got[&DAN]);
    println!("after the update:");
    h.describe(DAN);
    if let Some(s0) = s0 {
        println!(
            "  Dan holds Alice's pre-add secret: {}",
            h.ys[&DAN].secrets.contains(&s0)
        );
    }

    let m2 = h.send(ALICE, b"m2: sent after the rotation");
    let got = h.broadcast(&m2, &[BOB, CAROL, DAN]);
    println!("m2 read by Bob: {:?}", got[&BOB]);
    println!("m2 read by Dan: {:?}", got[&DAN]);
    match h.probe(DAN, &m1) {
        Ok(out) => println!("  Dan reads m1 after the rotation: {out:?} (UNEXPECTED)"),
        Err(err) => println!("  Dan still cannot read m1: {err}"),
    }
}

/// Without withholding: the joiner is delivered a pre-join application message.
pub fn from_join_hazard() {
    println!("## (c1) hazard: a pre-join application message reaches the joiner");
    let rng = enc::Rng::from_seed([3; 32]);
    let def = Lifetime::default();
    let all = [(ALICE, def), (BOB, def), (CAROL, def), (DAN, def)];
    let mut h = Harness::new(init_states(&all, &rng), rng);

    let create = h.create(ALICE, &[ALICE, BOB, CAROL]);
    h.broadcast(&create, &[BOB, CAROL, DAN]);
    let m1 = h.send(ALICE, b"m1: sent before Dan joins");
    h.broadcast(&m1, &[BOB, CAROL, DAN]);

    let add = h.add_from_join(ALICE, DAN);
    match h.probe(DAN, &add) {
        Ok(out) => println!("  welcome processed, Dan reads {out:?}"),
        Err(err) => println!("  welcome fails for Dan: {err}"),
    }
}

// --- (d) ordering: dependencies named per control message ---------------------------------------

pub fn ordering_counts() {
    println!("## (d) ordering: dependencies named by control and application messages, 40 members");
    const N: usize = 40;
    let all: Vec<Id> = (0..N).collect();
    let members: Vec<(Id, Lifetime)> = all.iter().map(|id| (*id, Lifetime::default())).collect();

    // d1: members 1..=8 update concurrently; member 9 merges them with one update.
    println!(" d1: 8 concurrent updates, then one merging update");
    {
        let rng = enc::Rng::from_seed([4; 32]);
        let mut h = Harness::new(init_states(&members, &rng), rng);
        let create = h.create(ALICE, &all);
        h.broadcast(&create, &others(&all, ALICE));
        let burst: Vec<Msg> = (1..=8).map(|id| h.update(id)).collect();
        for m in &burst {
            h.broadcast(m, &others(&all, m.sender()));
        }
        let merge = h.update(9);
        h.broadcast(&merge, &others(&all, 9));
        h.print_samples();
        h.summary("d1", "control");
    }

    // d2: all 40 members update concurrently; member 0 merges them with one update.
    println!(" d2: 40 concurrent updates, then one merging update");
    {
        let rng = enc::Rng::from_seed([5; 32]);
        let mut h = Harness::new(init_states(&members, &rng), rng);
        let create = h.create(ALICE, &all);
        h.broadcast(&create, &others(&all, ALICE));
        let burst: Vec<Msg> = all.iter().map(|id| h.update(*id)).collect();
        for m in &burst {
            h.broadcast(m, &others(&all, m.sender()));
        }
        let merge = h.update(ALICE);
        h.broadcast(&merge, &others(&all, ALICE));
        let last = h.samples.last().expect("merge sample").clone();
        println!(
            "    merge by member {}: named {}, heads {}",
            last.author, last.named, last.heads
        );
        h.summary("d2 updates", "control");
    }

    // d3: 40 updates in sequence, each delivered before the next, then 40 application messages.
    println!(" d3: 40 sequential updates, then 40 application messages");
    {
        let rng = enc::Rng::from_seed([6; 32]);
        let mut h = Harness::new(init_states(&members, &rng), rng);
        let create = h.create(ALICE, &all);
        h.broadcast(&create, &others(&all, ALICE));
        for id in &all {
            let m = h.update(*id);
            h.broadcast(&m, &others(&all, *id));
        }
        for id in &all {
            let m = h.send(*id, b"app");
            h.broadcast(&m, &others(&all, *id));
        }
        h.summary("d3", "control");
        h.summary("d3", "app");
        let first_update = h
            .samples
            .iter()
            .find(|s| s.kind == "control" && s.author == 1);
        if let Some(s) = first_update {
            println!(
                "    member 1's update (in sequence): named {}, heads {}",
                s.named, s.heads
            );
        }
    }
}

// --- (e) pre-key lifetime -----------------------------------------------------------------------

pub fn prekey_lifetime() {
    println!("## (e) pre-key lifetime: expired bundles at registry and first contact");
    let now = unix_now();
    let short = Lifetime::from_range(now - 10, now + 2);
    let rng = enc::Rng::from_seed([7; 32]);
    let members = [(ALICE, Lifetime::default()), (BOB, short), (DAN, short)];
    let mut h = Harness::new(init_states(&members, &rng), rng);

    // Dan's bundle, as Alice's registry holds it while it is still valid.
    let registry = h.ys[&ALICE].dcgka.pki.clone();
    let (_, dan_bundle) =
        <Pki as PreKeyRegistry<Id, LongTermKeyBundle>>::key_bundle(registry, &DAN)
            .expect("registry lookup");
    let dan_bundle = dan_bundle.expect("Dan's bundle while valid");

    let create = h.create(ALICE, &[ALICE, BOB]);
    h.broadcast(&create, &[BOB]);
    println!("  Bob's and Dan's bundles expire in about 2 seconds; waiting 4");
    std::thread::sleep(Duration::from_secs(4));

    let upd = h.update(ALICE);
    let got = h.broadcast(&upd, &[BOB]);
    println!(
        "  E1 update with Bob's session after his bundle expired: ok. Bob's secrets: {} ({:?})",
        h.secrets_len(BOB),
        got[&BOB]
    );

    match EncryptionGroup::add(h.ys[&ALICE].clone(), DAN, &h.rng) {
        Ok(_) => println!("  E2 add Dan with an expired bundle: accepted (UNEXPECTED)"),
        Err(err) => println!("  E2 add Dan, first contact: blocked: {err:?}"),
    }

    match KeyRegistry::add_longterm_bundle(KeyRegistry::<Id>::init(), DAN, dan_bundle) {
        Ok(_) => println!("  E3 registry accepts an expired bundle (UNEXPECTED)"),
        Err(err) => println!("  E3 registry rejects an expired bundle on insert: {err:?}"),
    }

    let default = serde_json::to_value(Lifetime::default()).expect("lifetime serialises");
    let span = default["not_after"].as_u64().unwrap() - unix_now();
    println!(
        "  default lifetime: {span} s from now ({:.2} days)",
        span as f64 / 86400.0
    );
}

pub fn run_base() {
    scenario("a", full_history_and_pcs);
    scenario("a2", compromised_member_in_group);
    scenario("b", welcome_size_by_epochs);
    scenario("c", from_join);
    scenario("c1", from_join_hazard);
    scenario("d", ordering_counts);
    scenario("e", prekey_lifetime);
}

pub fn run_patched() {
    scenario("a", full_history_and_pcs);
    scenario("c", from_join);
    scenario("c1", from_join_hazard);
}

// The `from_join` joiner-side and welcome-side steps are the same in both crates, so the one
// function `add_from_join` lives in the including module (see main.rs).
