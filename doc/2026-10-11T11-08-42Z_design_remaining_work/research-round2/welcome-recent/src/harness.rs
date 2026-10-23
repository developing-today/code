// Shared harness for the joiner's history modes. main.rs includes this file once per build. Before the
// include, the module defines `enc`, the crate under test.
//
// Scenario (S12's): a group of three (Admin, Bob, Carol), then K rotations. Odd rotations are `update`.
// Even rotations remove Carol and add her back. A rotation mints one epoch (one group secret); a re-add
// mints none. The admin sends one application message under each epoch, so there are K + 1 epochs and
// K + 1 messages. A joiner then joins by an add, once per history mode, and is checked against those
// messages.
//
// Orderer: `QOrderer`, a plain queue. The crate's `MessageOrderer` waits for every dependency a message
// names. A joiner never saw the pre-join control messages those dependencies point at, so the crate's
// orderer would keep the pre-join application messages pending. Decryption does not depend on the
// orderer: `EncryptionGroup::receive` looks the secret up in the bundle, and that lookup is what the
// joiner check measures.
//
// Membership: `MemberDgm`, a copy of the crate's `TestDgm` whose `from_welcome` keeps the joiner. The
// stock `from_welcome` drops the joiner (S3's membership bug in `dcgka.rs`). Our adapter's DGM needs the
// same fix. Both builds use this copy, so they run the same code apart from the welcome fields.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use p2panda_core::cbor::encode_cbor;
use serde::{Deserialize, Serialize};

use enc::crypto::xchacha20::XAeadNonce;
use enc::data_scheme::dcgka::{ControlMessage, Dcgka, DirectMessage, DirectMessageContent, ProcessInput};
use enc::data_scheme::group::{EncryptionGroup, GroupError, GroupOutput, GroupState};
use enc::data_scheme::group_secret::{GroupSecret, GroupSecretId, SecretBundle, SecretBundleState};
use enc::key_bundle::Lifetime;
use enc::key_manager::{KeyManager, KeyManagerState};
use enc::key_registry::{KeyRegistry, KeyRegistryState};
use enc::test_utils::{MemberId, MessageId};
use enc::traits::{
    GroupMembership, GroupMessage, GroupMessageContent, IdentityHandle, OperationId, Ordering,
    PreKeyManager, PreKeyRegistry,
};

type Id = MemberId;
type Mid = MessageId;

// --- membership -----------------------------------------------------------------------------------

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

    type Error = std::convert::Infallible;

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

type Dgm = MemberDgm<Id, Mid>;
type Dm = DirectMessage<Id, Mid, Dgm>;

// --- orderer --------------------------------------------------------------------------------------

/// One control or application message, as this harness publishes and delivers it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QMsg {
    id: Mid,
    control: Option<ControlMessage<Id>>,
    direct: Vec<Dm>,
    app: Option<(GroupSecretId, XAeadNonce, Vec<u8>)>,
}

impl GroupMessage<Id, Mid, Dgm> for QMsg {
    fn id(&self) -> Mid {
        self.id
    }

    fn sender(&self) -> Id {
        self.id.sender
    }

    fn content(&self) -> GroupMessageContent<Id> {
        match (&self.control, &self.app) {
            (Some(control), _) => GroupMessageContent::Control(control.clone()),
            (None, Some((group_secret_id, nonce, ciphertext))) => {
                GroupMessageContent::Application {
                    group_secret_id: *group_secret_id,
                    nonce: *nonce,
                    ciphertext: ciphertext.clone(),
                }
            }
            (None, None) => unreachable!("a message is control or application"),
        }
    }

    fn direct_messages(&self) -> Vec<Dm> {
        self.direct.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QState {
    my_id: Id,
    next_seq: usize,
    welcomed: bool,
    queue: VecDeque<QMsg>,
}

#[derive(Debug)]
pub struct QError;

impl fmt::Display for QError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "queue orderer error")
    }
}

impl std::error::Error for QError {}

/// Delivers messages in arrival order, after the welcome. No dependency checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct QOrderer;

impl QOrderer {
    pub fn init(my_id: Id) -> QState {
        QState {
            my_id,
            next_seq: 0,
            welcomed: false,
            queue: VecDeque::new(),
        }
    }
}

impl Ordering<Id, Mid, Dgm> for QOrderer {
    type State = QState;

    type Error = QError;

    type Message = QMsg;

    fn next_control_message(
        mut y: QState,
        control_message: &ControlMessage<Id>,
        direct_messages: &[Dm],
    ) -> Result<(QState, QMsg), QError> {
        let msg = QMsg {
            id: MessageId {
                sender: y.my_id,
                seq: y.next_seq,
            },
            control: Some(control_message.clone()),
            direct: direct_messages.to_vec(),
            app: None,
        };
        y.next_seq += 1;
        Ok((y, msg))
    }

    fn next_application_message(
        mut y: QState,
        group_secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    ) -> Result<(QState, QMsg), QError> {
        let msg = QMsg {
            id: MessageId {
                sender: y.my_id,
                seq: y.next_seq,
            },
            control: None,
            direct: Vec::new(),
            app: Some((group_secret_id, nonce, ciphertext)),
        };
        y.next_seq += 1;
        Ok((y, msg))
    }

    fn queue(mut y: QState, message: &QMsg) -> Result<QState, QError> {
        y.queue.push_back(message.clone());
        Ok(y)
    }

    fn set_welcome(mut y: QState, _message: &QMsg) -> Result<QState, QError> {
        y.welcomed = true;
        Ok(y)
    }

    fn next_ready_message(mut y: QState) -> Result<(QState, Option<QMsg>), QError> {
        if !y.welcomed {
            return Ok((y, None));
        }
        let next = y.queue.pop_front();
        Ok((y, next))
    }
}

type Orderer = QOrderer;
type Pki = KeyRegistry<Id>;
type Kmg = KeyManager;
type Y = GroupState<Id, Mid, Pki, Dgm, Kmg, Orderer>;

// --- scenario parameters --------------------------------------------------------------------------

const ADMIN: Id = 0;
const BOB: Id = 1;
const CAROL: Id = 2;
// The joiner for the checks. JOIN2 is only used by the op-count run (the from_join row).
const JOIN: Id = 3;
const JOIN2: Id = 4;

/// `Full` is the owner's default. `Recent(n)` is the `recent:n` option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Full,
    Recent(usize),
}

pub const MODES: [Mode; 6] = [
    Mode::Full,
    Mode::Recent(1),
    Mode::Recent(8),
    Mode::Recent(32),
    Mode::Recent(128),
    Mode::Recent(512),
];

impl Mode {
    fn label(self) -> String {
        match self {
            Mode::Full => "full".to_string(),
            Mode::Recent(n) => format!("recent:{n}"),
        }
    }

    /// The bundle a welcome in this mode carries, given the adder's bundle.
    fn bundle(self, secrets: &SecretBundleState) -> SecretBundleState {
        match self {
            Mode::Full => secrets.clone(),
            Mode::Recent(n) => newest(secrets, n),
        }
    }
}

// --- helpers --------------------------------------------------------------------------------------

fn ok<T, E: fmt::Debug>(result: Result<T, E>, what: &str) -> T {
    result.unwrap_or_else(|err| panic!("{what} failed: {err:?}"))
}

fn hex_id(id: &GroupSecretId) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn secret_id(msg: &QMsg) -> GroupSecretId {
    match msg.content() {
        GroupMessageContent::Application {
            group_secret_id, ..
        } => group_secret_id,
        GroupMessageContent::Control(_) => panic!("an application message was expected"),
    }
}

/// The `n` newest secrets. Timestamps increase with each new secret (`SecretBundle::generate`), so the
/// newest by timestamp is the last epoch minted.
fn newest(secrets: &SecretBundleState, n: usize) -> SecretBundleState {
    let mut list: Vec<GroupSecret> = secrets.secrets().cloned().collect();
    list.sort_by_key(|secret| std::cmp::Reverse(secret.timestamp()));
    list.truncate(n);
    SecretBundle::from_secrets(list)
}

fn median_us(mut times: Vec<Duration>) -> f64 {
    times.sort();
    times[times.len() / 2].as_secs_f64() * 1e6
}

/// Long-term key bundles for every party, and the registry all parties share.
fn cast(rng: &enc::Rng) -> (BTreeMap<Id, KeyManagerState>, KeyRegistryState<Id>) {
    let lifetime = Lifetime::default();
    let mut keys = BTreeMap::new();
    let mut registry = KeyRegistry::<Id>::init();
    for id in [ADMIN, BOB, CAROL, JOIN, JOIN2] {
        let identity = enc::test_utils::crypto::SecretKey::from_bytes(rng.random_array().unwrap());
        let manager = ok(KeyManager::init_and_generate_prekey(&identity, lifetime, rng), "prekey");
        let bundle = ok(KeyManager::prekey_bundle(&manager), "prekey bundle");
        registry = ok(KeyRegistry::add_longterm_bundle(registry, id, bundle), "register");
        keys.insert(id, manager);
    }
    (keys, registry)
}

fn party(keys: &BTreeMap<Id, KeyManagerState>, registry: &KeyRegistryState<Id>, id: Id) -> Y {
    EncryptionGroup::init(
        id,
        keys[&id].clone(),
        registry.clone(),
        Dgm::init(id),
        Orderer::init(id),
    )
}

fn create(keys: &BTreeMap<Id, KeyManagerState>, registry: &KeyRegistryState<Id>, rng: &enc::Rng) -> Y {
    let (y, _) = ok(
        EncryptionGroup::create(party(keys, registry, ADMIN), vec![ADMIN, BOB, CAROL], rng),
        "create",
    );
    y
}

/// The local steps of `EncryptionGroup::add`, with the bundle given by the caller. This is S3's
/// `add_from_join` with a bundle argument. It uses only public API: `Dcgka::add`,
/// `Ordering::next_control_message` and `Dcgka::process`.
fn add_with_bundle(mut y: Y, joiner: Id, bundle: &SecretBundleState, rng: &enc::Rng) -> (Y, QMsg) {
    let (dcgka, output) = ok(Dcgka::add(y.dcgka, joiner, bundle, rng), "add with bundle");
    let (orderer, msg) = ok(
        <Orderer as Ordering<Id, Mid, Dgm>>::next_control_message(
            y.orderer,
            &output.control_message,
            &output.direct_messages,
        ),
        "order the add",
    );
    let (dcgka, _) = ok(
        Dcgka::process(
            dcgka,
            ProcessInput {
                seq: msg.id(),
                sender: msg.sender(),
                control_message: output.control_message,
                direct_message: None,
            },
        ),
        "process the add locally",
    );
    y.dcgka = dcgka;
    y.orderer = orderer;
    (y, msg)
}

/// The add that brings the joiner in under a history mode.
fn join_message(admin: Y, mode: Mode, rng: &enc::Rng) -> (Y, QMsg) {
    match mode {
        // The crate's own add, which always sends the full bundle.
        Mode::Full => ok(EncryptionGroup::add(admin, JOIN, rng), "add"),
        Mode::Recent(n) => {
            let bundle = newest(&admin.secrets, n);
            add_with_bundle(admin, JOIN, &bundle, rng)
        }
    }
}

fn welcome_for(msg: &QMsg, joiner: Id) -> Dm {
    msg.direct_messages()
        .into_iter()
        .find(|dm| dm.recipient == joiner)
        .expect("a welcome for the joiner")
}

/// Bytes of the whole welcome, of its history (the DGM state), and of its ciphertext.
fn sizes(dm: &Dm) -> (usize, usize, usize) {
    let DirectMessageContent::Welcome {
        ciphertext,
        history,
    } = &dm.content
    else {
        panic!("expected a welcome");
    };
    (
        encode_cbor(dm).expect("encode the welcome").len(),
        encode_cbor(history).expect("encode the history").len(),
        encode_cbor(ciphertext).expect("encode the ciphertext").len(),
    )
}

// --- the scenario ---------------------------------------------------------------------------------

struct Rotated {
    /// The adder's state after K rotations.
    admin: Y,
    /// One application message per epoch, epoch 0 (after create) first.
    epochs: Vec<QMsg>,
    /// Total bytes of Carol's re-add welcomes, per mode (indexed like `MODES`).
    readd_total: Vec<usize>,
    readd_count: usize,
}

fn rotate(keys: &BTreeMap<Id, KeyManagerState>, registry: &KeyRegistryState<Id>, k_max: usize, rng: &enc::Rng) -> Rotated {
    let mut y = create(keys, registry, rng);
    let mut epochs = Vec::new();
    let (next, msg) = ok(EncryptionGroup::send(y, b"epoch 0", rng), "send");
    y = next;
    epochs.push(msg);

    let mut readd_total = vec![0usize; MODES.len()];
    let mut readd_count = 0;
    for k in 1..=k_max {
        if k % 2 == 1 {
            y = ok(EncryptionGroup::update(y, rng), "update").0;
        } else {
            let (without, _) = ok(EncryptionGroup::remove(y, CAROL, rng), "remove");
            // Carol's re-add welcome under each mode. The adder's state is the same for every mode, so
            // only the welcome bytes differ. The mode's welcome goes to a clone, not to the state.
            for (i, mode) in MODES.iter().enumerate() {
                let bundle = mode.bundle(&without.secrets);
                let (_, output) = ok(Dcgka::add(without.dcgka.clone(), CAROL, &bundle, rng), "re-add");
                let dm = welcome_for_output(output, CAROL);
                readd_total[i] += encode_cbor(&dm).expect("encode the re-add").len();
            }
            readd_count += 1;
            y = ok(EncryptionGroup::add(without, CAROL, rng), "add back").0;
        }
        let (next, msg) = ok(EncryptionGroup::send(y, format!("epoch {k}").as_bytes(), rng), "send");
        y = next;
        epochs.push(msg);
    }
    Rotated {
        admin: y,
        epochs,
        readd_total,
        readd_count,
    }
}

fn welcome_for_output(
    output: enc::data_scheme::dcgka::OperationOutput<Id, Mid, Dgm>,
    joiner: Id,
) -> Dm {
    output
        .direct_messages
        .into_iter()
        .find(|dm| dm.recipient == joiner)
        .expect("a welcome for the joiner")
}

// --- the joiner's check ---------------------------------------------------------------------------

struct Check {
    welcome_b: usize,
    history_b: usize,
    ciphertext_b: usize,
    bundle_len: usize,
    build_us: f64,
    encode_us: f64,
    /// The joiner processed the welcome, is a member, and holds exactly the bundle the mode promises.
    welcome_ok: bool,
    decrypt_pass: usize,
    refuse_pass: usize,
    fail: usize,
    expected_decrypt: usize,
    total: usize,
    /// The crate's error for the first older message the joiner was refused.
    refusal: Option<String>,
}

fn check(run: &Rotated, keys: &BTreeMap<Id, KeyManagerState>, registry: &KeyRegistryState<Id>, mode: Mode, reps: usize, rng: &enc::Rng) -> Check {
    let total = run.epochs.len();
    let admin = &run.admin;

    // Welcome size. The DM is the one the joiner gets from this mode's add.
    let (_, join) = join_message(admin.clone(), mode, rng);
    let dm = welcome_for(&join, JOIN);
    let (welcome_b, history_b, ciphertext_b) = sizes(&dm);

    // Time to build the welcome: `Dcgka::add` with the mode's bundle, on a clone of the adder's
    // state (as in S12). The clone is outside the timer.
    let bundle = mode.bundle(&admin.secrets);
    let build: Vec<Duration> = (0..reps)
        .map(|_| {
            let dcgka = admin.dcgka.clone();
            let start = Instant::now();
            let output = ok(Dcgka::add(dcgka, JOIN, &bundle, rng), "timed welcome");
            let elapsed = start.elapsed();
            drop(output);
            elapsed
        })
        .collect();
    let encode: Vec<Duration> = (0..reps)
        .map(|_| {
            let start = Instant::now();
            let bytes = encode_cbor(&dm).expect("encode the welcome");
            let elapsed = start.elapsed();
            drop(bytes);
            elapsed
        })
        .collect();

    // The joiner processes the welcome with its own keys.
    let joiner0 = party(keys, registry, JOIN);
    let (joiner, _) = ok(EncryptionGroup::receive(joiner0, &join), "joiner processes the welcome");
    let welcome_ok = joiner.is_welcomed
        && joiner.secrets.len() == bundle.len()
        && bundle.ids().all(|id| joiner.secrets.contains(id));

    // The joiner's check, per epoch. The last `expected_decrypt` epochs must decrypt to their own text.
    // Each older message must be refused with `UnknownGroupSecret` for its own secret. Each message goes
    // to a clone, because `receive` returns no state on error.
    let expected_decrypt = match mode {
        Mode::Full => total,
        Mode::Recent(n) => n.min(total),
    };
    let first_decrypt = total - expected_decrypt;
    let (mut decrypt_pass, mut refuse_pass, mut fail) = (0, 0, 0);
    let mut refusal = None;
    for (epoch, msg) in run.epochs.iter().enumerate() {
        let should = epoch >= first_decrypt;
        let want = format!("epoch {epoch}").into_bytes();
        match EncryptionGroup::receive(joiner.clone(), msg) {
            Ok((_, outputs)) => {
                let plain = matches!(
                    outputs.as_slice(),
                    [GroupOutput::Application { plaintext }] if *plaintext == want
                );
                if should && plain {
                    decrypt_pass += 1;
                } else {
                    fail += 1;
                }
            }
            Err(GroupError::UnknownGroupSecret(id)) => {
                if !should && id == hex_id(&secret_id(msg)) {
                    refuse_pass += 1;
                    if refusal.is_none() {
                        refusal = Some(format!("UnknownGroupSecret({id})"));
                    }
                } else {
                    fail += 1;
                }
            }
            Err(_) => fail += 1,
        }
    }

    Check {
        welcome_b,
        history_b,
        ciphertext_b,
        bundle_len: bundle.len(),
        build_us: median_us(build),
        encode_us: median_us(encode),
        welcome_ok,
        decrypt_pass,
        refuse_pass,
        fail,
        expected_decrypt,
        total,
        refusal,
    }
}

/// The joiner receives every epoch message before the welcome (as a sync would deliver history), then
/// the welcome. The crate queues the messages, then fails on the welcome for any it cannot decrypt.
fn pre_welcome(run: &Rotated, keys: &BTreeMap<Id, KeyManagerState>, registry: &KeyRegistryState<Id>, mode: Mode, rng: &enc::Rng) -> String {
    let (_, join) = join_message(run.admin.clone(), mode, rng);
    let mut joiner = party(keys, registry, JOIN);
    for msg in &run.epochs {
        joiner = ok(EncryptionGroup::receive(joiner, msg), "queue before the welcome").0;
    }
    match EncryptionGroup::receive(joiner, &join) {
        Ok((_, outputs)) => format!("welcome ok, {} application outputs", outputs.len()),
        Err(err) => format!("welcome fails: {err}"),
    }
}

// --- op counts ------------------------------------------------------------------------------------

/// Prints the adder's bundle size before and after one operation, and returns the size after it.
fn report_op(name: &str, before: usize, y: &Y) -> usize {
    println!("OP|{name}|{before}|{}", y.secrets.len());
    y.secrets.len()
}

/// Epochs minted by each operation: the change in the adder's bundle size.
pub fn op_counts(rng: &enc::Rng) {
    let (keys, registry) = cast(rng);
    let mut y = create(&keys, &registry, rng);
    let mut n = report_op("create", 0, &y);

    let (next, _) = ok(EncryptionGroup::add(y, JOIN, rng), "add");
    y = next;
    n = report_op("add (new member, full bundle)", n, &y);

    let (next, _) = ok(EncryptionGroup::remove(y, CAROL, rng), "remove");
    y = next;
    n = report_op("remove", n, &y);

    let (next, _) = ok(EncryptionGroup::update(y, rng), "update");
    y = next;
    n = report_op("update", n, &y);

    let (next, _) = ok(EncryptionGroup::add(y, CAROL, rng), "re-add");
    y = next;
    n = report_op("add back after remove (re-add)", n, &y);

    let (next, _) = add_with_bundle(y, JOIN2, &SecretBundle::init(), rng);
    y = next;
    n = report_op("from_join welcome (empty bundle)", n, &y);

    let (next, _) = ok(EncryptionGroup::update(y, rng), "update");
    y = next;
    report_op("update after from_join", n, &y);
}

// --- driver ---------------------------------------------------------------------------------------

pub fn run(name: &str, seed: u8, k_max: usize) {
    let rng = enc::Rng::from_seed([seed; 32]);
    let (keys, registry) = cast(&rng);
    let start = Instant::now();
    let run = rotate(&keys, &registry, k_max, &rng);
    let rotate_ms = start.elapsed().as_secs_f64() * 1e3;
    println!(
        "RUN|{name}|{seed}|{k_max}|epochs {}|re-adds {}|rotate ms {rotate_ms:.0}",
        run.epochs.len(),
        run.readd_count
    );

    let reps = if k_max >= 1000 { 50 } else { 25 };
    for (i, mode) in MODES.iter().enumerate() {
        let c = check(&run, &keys, &registry, *mode, reps, &rng);
        let pre = pre_welcome(&run, &keys, &registry, *mode, &rng);
        println!(
            "ROW|{name}|{seed}|{k_max}|{}|{}|{}|{}|{}|{:.1}|{:.1}|{}|{}|{}|{}|{}|{}|{}|{}",
            mode.label(),
            c.welcome_b,
            c.history_b,
            c.ciphertext_b,
            c.bundle_len,
            c.build_us,
            c.encode_us,
            c.decrypt_pass,
            c.refuse_pass,
            c.fail,
            c.expected_decrypt,
            c.total,
            u8::from(c.welcome_ok),
            run.readd_total[i],
            run.readd_count,
        );
        if let Some(text) = &c.refusal {
            println!("ERR|{name}|{seed}|{k_max}|{}|{text}", mode.label());
        }
        println!("PRE|{name}|{seed}|{k_max}|{}|{pre}", mode.label());
    }
}
