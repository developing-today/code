//! S10 spike: heads orderer and frontier size in a 40-member group. Method and results: RESULTS.md.
//!
//! Every run builds one group of 40 members with p2panda-encryption's DataScheme (the crate's
//! test DGM and key managers), and drives it through one scenario with one orderer. The orderer is
//! either the crate's `MessageOrderer` (per-author latest, no pruning) or `heads::HeadsOrderer`.
//! Each published event records how many dependencies it names, and, for the heads orderer, how
//! many heads its author held (the frontier the event would carry without a cap).

mod heads;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use enc::data_scheme::dcgka::Dcgka;
use enc::data_scheme::group::{EncryptionGroup, GroupOutput, GroupState};
use enc::data_scheme::group_secret::SecretBundle;
use enc::data_scheme::test_utils::dgm::TestDgm;
use enc::data_scheme::test_utils::ordering::MessageOrderer;
use enc::key_bundle::{Lifetime, LongTermKeyBundle};
use enc::key_manager::{KeyManager, KeyManagerState};
use enc::key_registry::KeyRegistry;
use enc::test_utils::{MemberId, MessageId};
use enc::traits::{GroupMessage, Ordering, PreKeyManager};
use p2panda_encryption as enc;
use serde::Serialize;

use heads::HeadsOrderer;

type Id = MemberId;
type Mid = MessageId;
type Dgm = TestDgm<Id, Mid>;
type Pki = KeyRegistry<Id>;

const N: usize = 40;
/// The design's `after` cap.
const DESIGN_CAP: usize = 32;

/// An orderer under test, and how to read its heads and waiting count.
trait Kit {
    type O: Ordering<Id, Mid, Dgm, Message: Clone + Serialize> + std::fmt::Debug;
    fn name(&self) -> String;
    /// The merge threshold the driver enforces. `None` never merges.
    fn cap(&self) -> Option<usize>;
    fn init(&self, id: Id) -> OState<Self>;
    /// (heads, waiting) as the orderer reports them. `None` when it does not expose them.
    fn probe(y: &OState<Self>) -> (Option<usize>, Option<usize>);
    /// (most messages waiting at once, longest wait in arrivals) since the start.
    fn peak(y: &OState<Self>) -> (Option<usize>, Option<usize>);
}

type OState<K> = <<K as Kit>::O as Ordering<Id, Mid, Dgm>>::State;
type Msg<K> = <<K as Kit>::O as Ordering<Id, Mid, Dgm>>::Message;
type Y<K> = GroupState<Id, Mid, Pki, Dgm, KeyManager, <K as Kit>::O>;

/// The crate's orderer, as shipped in test_utils.
struct CrateKit;

impl Kit for CrateKit {
    type O = MessageOrderer<Dgm>;

    fn name(&self) -> String {
        "crate MessageOrderer".into()
    }

    fn cap(&self) -> Option<usize> {
        None
    }

    fn init(&self, id: Id) -> OState<Self> {
        MessageOrderer::<Dgm>::init(id)
    }

    fn probe(_: &OState<Self>) -> (Option<usize>, Option<usize>) {
        (None, None)
    }

    fn peak(_: &OState<Self>) -> (Option<usize>, Option<usize>) {
        (None, None)
    }
}

/// The heads orderer. `track_app`: application messages are heads too (every event).
struct HeadsKit {
    cap: Option<usize>,
    track_app: bool,
    /// Name the author's own latest event first (see heads.rs).
    backlink: bool,
}

impl Kit for HeadsKit {
    type O = HeadsOrderer<Dgm>;

    fn name(&self) -> String {
        let scope = if self.track_app { "all events" } else { "control only" };
        let backlink = if self.backlink { ", backlink" } else { "" };
        match self.cap {
            Some(cap) => format!("heads ({scope}, cap {cap}{backlink})"),
            None => format!("heads ({scope}, no cap{backlink})"),
        }
    }

    fn cap(&self) -> Option<usize> {
        self.cap
    }

    fn init(&self, id: Id) -> OState<Self> {
        HeadsOrderer::<Dgm>::init(id, self.cap, self.track_app, self.backlink)
    }

    fn probe(y: &OState<Self>) -> (Option<usize>, Option<usize>) {
        (Some(y.heads_len()), Some(y.waiting_len()))
    }

    fn peak(y: &OState<Self>) -> (Option<usize>, Option<usize>) {
        let longest = y.waits().iter().copied().max().unwrap_or(0);
        (Some(y.max_waiting()), Some(longest))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Create,
    App,
    Update,
    /// An update published because the author's frontier had reached the cap.
    Merge,
}

#[derive(Clone, Copy, Debug)]
enum Want {
    App,
    Update,
}

struct Sample {
    kind: Kind,
    /// Heads in the author's view before the event (untruncated). `None` for the crate.
    heads: Option<usize>,
    /// Dependencies the event names.
    named: usize,
}

#[derive(Clone, Copy, Default)]
struct Opts {
    /// Each member hears the events of a round in its own order, drawn from this seed.
    reorder: Option<u64>,
    /// Only member 0 merges at the cap. Other members at the cap skip the event.
    merger_only: bool,
}

struct Net<K: Kit> {
    kit: K,
    opts: Opts,
    rng: enc::Rng,
    ys: BTreeMap<Id, Y<K>>,
    /// Every published message, with its author, in publication order.
    log: Vec<(Id, Msg<K>)>,
    /// Application labels with their authors.
    labels: Vec<(Id, String)>,
    /// (member, message) pairs the member has seen or published.
    seen: HashSet<(Id, Mid)>,
    /// Application labels each member has read.
    read: BTreeMap<Id, BTreeSet<String>>,
    samples: Vec<Sample>,
    max_heads: Option<usize>,
    max_waiting: Option<usize>,
    longest_wait: Option<usize>,
}

fn max_opt(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

fn init_states<K: Kit>(kit: &K, members: &[(Id, Lifetime)], rng: &enc::Rng) -> Vec<Y<K>> {
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
                registry = KeyRegistry::add_longterm_bundle(registry, *other, bundle.clone())
                    .unwrap();
            }
            GroupState {
                my_id: *id,
                dcgka: Dcgka::init(*id, managers[id].clone(), registry, Dgm::init(*id)),
                orderer: kit.init(*id),
                secrets: SecretBundle::init(),
                is_welcomed: false,
            }
        })
        .collect()
}

fn all() -> Vec<Id> {
    (0..N).collect()
}

fn shuffle<T>(v: &mut [T], mut seed: u64) {
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

impl<K: Kit> Net<K> {
    fn new(kit: K, seed: u8) -> Self {
        let rng = enc::Rng::from_seed([seed; 32]);
        let members: Vec<(Id, Lifetime)> = (0..N).map(|id| (id, Lifetime::default())).collect();
        let ys = init_states(&kit, &members, &rng)
            .into_iter()
            .map(|y| (y.my_id, y))
            .collect();
        Self {
            kit,
            rng,
            ys,
            log: Vec::new(),
            labels: Vec::new(),
            seen: HashSet::new(),
            read: BTreeMap::new(),
            samples: Vec::new(),
            max_heads: None,
            max_waiting: None,
            longest_wait: None,
            opts: Opts::default(),
        }
    }

    fn take(&mut self, id: Id) -> Y<K> {
        self.ys.remove(&id).expect("member state")
    }

    fn put(&mut self, y: Y<K>) {
        self.ys.insert(y.my_id, y);
    }

    fn probe(&self, id: Id) -> (Option<usize>, Option<usize>) {
        K::probe(&self.ys[&id].orderer)
    }

    /// The frontier has reached the cap, so the next event from this member is a merge.
    fn is_full(&self, id: Id) -> bool {
        match (self.kit.cap(), self.probe(id).0) {
            (Some(cap), Some(heads)) => heads >= cap,
            _ => false,
        }
    }

    /// Hands a published message back to its author's state, and records it.
    fn emit(&mut self, author: Id, kind: Kind, heads: Option<usize>, y: Y<K>, msg: Msg<K>) {
        let named = serde_json::to_value(&msg).expect("message serialises")["previous"]
            .as_array()
            .map_or(0, Vec::len);
        self.samples.push(Sample {
            kind,
            heads,
            named,
        });
        self.seen.insert((author, msg.id()));
        self.log.push((author, msg));
        self.put(y);
    }

    fn create(&mut self, creator: Id, members: &[Id]) -> Result<(), String> {
        let heads = self.probe(creator).0;
        let y = self.take(creator);
        let (y, msg) = EncryptionGroup::create(y, members.to_vec(), &self.rng)
            .map_err(|err| format!("create: {err:?}"))?;
        self.emit(creator, Kind::Create, heads, y, msg);
        let (_, msg) = self.log.last().cloned().expect("just published");
        let to: Vec<Id> = members.iter().copied().filter(|m| *m != creator).collect();
        self.deliver(&msg, &to)
    }

    fn push_app(&mut self, author: Id) -> Result<(), String> {
        let label = format!("app {author}/{}", self.log.len());
        let heads = self.probe(author).0;
        let y = self.take(author);
        let (y, msg) = EncryptionGroup::send(y, label.as_bytes(), &self.rng)
            .map_err(|err| format!("send by {author}: {err:?}"))?;
        self.labels.push((author, label));
        self.emit(author, Kind::App, heads, y, msg);
        Ok(())
    }

    fn push_update(&mut self, author: Id, kind: Kind) -> Result<(), String> {
        let heads = self.probe(author).0;
        let y = self.take(author);
        let (y, msg) = EncryptionGroup::update(y, &self.rng)
            .map_err(|err| format!("update by {author}: {err:?}"))?;
        self.emit(author, kind, heads, y, msg);
        Ok(())
    }

    /// One event from `author`: merges first while the frontier is at the cap.
    fn publish(&mut self, author: Id, want: Want) -> Result<(), String> {
        if self.opts.merger_only && author != 0 && self.is_full(author) {
            return Ok(());
        }
        let mut merged = false;
        while self.is_full(author) {
            self.push_update(author, Kind::Merge)?;
            merged = true;
        }
        match want {
            Want::App => self.push_app(author),
            Want::Update if merged => Ok(()),
            Want::Update => self.push_update(author, Kind::Update),
        }
    }

    /// Hands `msg` to each member of `to` that has not seen it yet.
    fn deliver(&mut self, msg: &Msg<K>, to: &[Id]) -> Result<(), String> {
        for &member in to {
            if !self.seen.insert((member, msg.id())) {
                continue;
            }
            let y = self.take(member);
            let (y, outputs) = EncryptionGroup::receive(y, msg).map_err(|err| {
                format!("receive {} at member {member}: {err:?}", msg.id())
            })?;
            for output in outputs {
                if let GroupOutput::Application { plaintext } = output {
                    let label = String::from_utf8_lossy(&plaintext).into_owned();
                    self.read.entry(member).or_default().insert(label);
                }
            }
            self.put(y);
            self.note_views();
        }
        Ok(())
    }

    fn note_views(&mut self) {
        for y in self.ys.values() {
            let (heads, _) = K::probe(&y.orderer);
            let (peak, longest) = K::peak(&y.orderer);
            self.max_heads = max_opt(self.max_heads, heads);
            self.max_waiting = max_opt(self.max_waiting, peak);
            self.longest_wait = max_opt(self.longest_wait, longest);
        }
    }

    /// Every author in `authors` publishes against its current view, then the new events reach
    /// `audience`. A burst is one call with many authors: nobody sees the others' events first.
    fn round(&mut self, authors: &[Id], want: Want, audience: &[Id]) -> Result<(), String> {
        let start = self.log.len();
        for &author in authors {
            self.publish(author, want)?;
        }
        let new: Vec<(Id, Msg<K>)> = self.log[start..].to_vec();
        if let Some(salt) = self.opts.reorder {
            // Each member hears the round in its own order.
            for &member in audience {
                let mut mine: Vec<&(Id, Msg<K>)> =
                    new.iter().filter(|(author, _)| *author != member).collect();
                let seed = (start as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    ^ (member as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
                    ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03);
                shuffle(mine.as_mut_slice(), seed);
                for (_, msg) in mine {
                    self.deliver(msg, &[member])?;
                }
            }
            return Ok(());
        }
        for (author, msg) in &new {
            let to: Vec<Id> = audience.iter().copied().filter(|m| m != author).collect();
            self.deliver(msg, &to)?;
        }
        Ok(())
    }

    /// Each author in turn: its event reaches the audience before the next author posts.
    fn turns(&mut self, authors: &[Id], want: Want, audience: &[Id]) -> Result<(), String> {
        for &author in authors {
            self.round(&[author], want, audience)?;
        }
        Ok(())
    }

    /// Every member holds the same secrets and membership, and has read every application message
    /// that another member published.
    fn verdict(&self, members: &[Id]) -> String {
        let mut counts = BTreeSet::new();
        let mut id_sets = BTreeSet::new();
        let mut latest = BTreeSet::new();
        let mut members_ok = true;
        for id in members {
            let y = &self.ys[id];
            counts.insert(y.secrets.len());
            id_sets.insert(y.secrets.ids().copied().collect::<BTreeSet<_>>());
            latest.insert(y.secrets.latest().map(|secret| secret.id()));
            members_ok &= EncryptionGroup::members(y).is_ok_and(|set| set.len() == members.len());
        }
        let mut missing = 0;
        for id in members {
            let read = self.read.get(id);
            missing += self
                .labels
                .iter()
                .filter(|(author, label)| author != id && !read.is_some_and(|r| r.contains(label)))
                .count();
        }
        let converged = id_sets.len() == 1 && latest.len() == 1 && members_ok && missing == 0;
        if converged {
            return format!("CONVERGED ({} secrets each)", counts.iter().next().unwrap_or(&0));
        }
        format!(
            "NOT converged (secrets per member {counts:?}, identical id sets: {}, latest ids: {}, membership {}, app msgs missing {missing})",
            id_sets.len() == 1,
            latest.len(),
            if members_ok { "ok" } else { "wrong" },
        )
    }
}

// --- scenarios ------------------------------------------------------------------------------------

/// (a) Members post in turn. Each post reaches everyone before the next.
fn steady<K: Kit>(net: &mut Net<K>, want: Want, cycles: usize) -> Result<String, String> {
    net.create(0, &all())?;
    for _ in 0..cycles {
        net.turns(&all(), want, &all())?;
    }
    Ok(net.verdict(&all()))
}

/// (b) All 40 publish in one round, then each member posts once in turn.
fn burst_then_turns<K: Kit>(net: &mut Net<K>, want: Want) -> Result<String, String> {
    net.create(0, &all())?;
    net.round(&all(), want, &all())?;
    net.turns(&all(), Want::App, &all())?;
    Ok(net.verdict(&all()))
}

/// (b2) `rounds` rounds in which all 40 publish concurrently, then each member posts once in turn.
fn storm<K: Kit>(net: &mut Net<K>, rounds: usize) -> Result<String, String> {
    net.create(0, &all())?;
    for _ in 0..rounds {
        net.round(&all(), Want::Update, &all())?;
    }
    net.turns(&all(), Want::App, &all())?;
    Ok(net.verdict(&all()))
}

/// (c) The group splits into `p` members and 40-p. Each side publishes concurrently for `k` rounds,
/// the partition heals (every event reaches every member), then each member posts once in turn.
fn partition<K: Kit>(net: &mut Net<K>, p: usize, k: usize) -> Result<String, String> {
    net.create(0, &all())?;
    let left: Vec<Id> = all()[..p].to_vec();
    let right: Vec<Id> = all()[p..].to_vec();
    for _ in 0..k {
        net.round(&left, Want::Update, &left)?;
        net.round(&right, Want::Update, &right)?;
    }
    let log = net.log.clone();
    for (author, msg) in &log {
        let to: Vec<Id> = all().into_iter().filter(|m| m != author).collect();
        net.deliver(msg, &to)?;
    }
    net.turns(&all(), Want::App, &all())?;
    Ok(net.verdict(&all()))
}

/// (d) Member 20 is offline for `cycles` cycles. The other 39 post in turn, and once a cycle one of
/// them publishes an update. Then member 20 catches up, in publication order or shuffled, and posts.
fn offline<K: Kit>(net: &mut Net<K>, cycles: usize, shuffled: bool) -> Result<String, String> {
    const Z: Id = 20;
    net.create(0, &all())?;
    let online: Vec<Id> = all().into_iter().filter(|m| *m != Z).collect();
    for cycle in 0..cycles {
        net.turns(&online, Want::App, &online)?;
        net.round(&[online[cycle % online.len()]], Want::Update, &online)?;
    }
    let mut missed: Vec<(Id, Msg<K>)> = net
        .log
        .iter()
        .filter(|(_, msg)| !net.seen.contains(&(Z, msg.id())))
        .cloned()
        .collect();
    let missed_count = missed.len();
    if shuffled {
        shuffle(&mut missed, 0x5eed);
    }
    for (_, msg) in &missed {
        net.deliver(msg, &[Z])?;
    }
    let waiting_after_catch_up = net.probe(Z).1;
    net.round(&[Z], Want::App, &all())?;
    Ok(format!(
        "caught up {missed_count} missed msgs (waiting at Z after catch-up: {}); {}",
        waiting_after_catch_up.map_or("-".into(), |w| w.to_string()),
        net.verdict(&all())
    ))
}

/// (e) One round of posts, then member 5 publishes an update that member 7 never receives. Everyone
/// else keeps posting. Member 7 gets every later message, which names the lost update.
fn lost<K: Kit>(net: &mut Net<K>, rounds: usize) -> Result<String, String> {
    const M: Id = 5;
    const Z: Id = 7;
    net.create(0, &all())?;
    net.turns(&all(), Want::App, &all())?;
    let before_loss = net.labels.len();
    let without_z: Vec<Id> = all().into_iter().filter(|m| *m != Z).collect();
    net.round(&[M], Want::Update, &without_z)?;
    for _ in 0..rounds {
        net.turns(&all(), Want::App, &all())?;
    }
    let total = net.labels.iter().filter(|(author, _)| *author != Z).count();
    let read = net.read.get(&Z).map_or(0, BTreeSet::len);
    Ok(format!(
        "member 7 read {read} of {total} app msgs by others (the {before_loss} before the loss, then none); waiting at 7: {}",
        net.probe(Z).1.map_or("-".into(), |w| w.to_string())
    ))
}

// --- driver ---------------------------------------------------------------------------------------

fn case<K: Kit>(scenario: &str, kit: K, body: impl FnOnce(&mut Net<K>) -> Result<String, String>) {
    case_opts(scenario, kit, Opts::default(), body);
}

fn case_opts<K: Kit>(
    scenario: &str,
    kit: K,
    opts: Opts,
    body: impl FnOnce(&mut Net<K>) -> Result<String, String>,
) {
    let mut label = kit.name();
    if let Some(salt) = opts.reorder {
        label.push_str(&format!(" +reorder{salt}"));
    }
    if opts.merger_only {
        label.push_str(" +merger0");
    }
    let started = std::time::Instant::now();
    let mut net = Net::new(kit, 1);
    net.opts = opts;
    // A panic inside the crate (for example an `expect` on a missing dependency) ends this case only.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut net)))
        .unwrap_or_else(|panic| {
            let text = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            Err(format!("PANIC: {text}"))
        });
    let secs = started.elapsed().as_secs_f64();
    let events: Vec<&Sample> = net
        .samples
        .iter()
        .filter(|s| s.kind != Kind::Create)
        .collect();
    let count = |kind: Kind| events.iter().filter(|s| s.kind == kind).count();
    let named_max = events.iter().map(|s| s.named).max().unwrap_or(0);
    let heads: Vec<usize> = events.iter().filter_map(|s| s.heads).collect();
    let fmt_opt = |v: Option<usize>| v.map_or("-".to_string(), |v| v.to_string());
    let would_max = if heads.is_empty() { None } else { heads.iter().max().copied() };
    let ge = heads.iter().filter(|h| **h >= DESIGN_CAP).count();
    let gt = heads.iter().filter(|h| **h > DESIGN_CAP).count();
    let verdict = match outcome {
        Ok(verdict) => verdict,
        Err(err) => format!("STOPPED: {err}"),
    };
    let last_named = events.last().map_or(0, |s| s.named);
    println!(
        "{scenario:<26}| {label:<26}| {:>4} {:>4} {:>4} | {:>3} {:>3} {:>3} | {:>4} {:>4} | {:>3} {:>3} {:>3} | {secs:>6.1}s | {verdict}",
        events.len(),
        count(Kind::Update),
        count(Kind::Merge),
        named_max,
        last_named,
        fmt_opt(would_max),
        ge,
        gt,
        fmt_opt(net.max_heads),
        fmt_opt(net.max_waiting),
        fmt_opt(net.longest_wait),
    );
}

fn main() {
    println!(
        "{:<26}| {:<26}| {:>4} {:>4} {:>4} | {:>3} {:>3} {:>3} | {:>4} {:>4} | {:>3} {:>3} {:>3} | {:>7} | result",
        "scenario", "orderer", "evts", "upd", "mrg", "nmd", "lst", "hds", ">=32", ">32", "vw", "pk", "lw", "time"
    );
    let heads_all = |cap| HeadsKit { cap, track_app: true, backlink: false };

    // The crate baseline, then the heads orderer uncapped (to measure the frontier), then the
    // design's cap, then control-only heads (the S3 measure).
    run_matrix(|| CrateKit);
    run_matrix(|| heads_all(None));
    run_matrix(|| heads_all(Some(DESIGN_CAP)));
    run_matrix(|| HeadsKit { cap: None, track_app: false, backlink: false });
    run_matrix(|| HeadsKit { cap: Some(DESIGN_CAP), track_app: false, backlink: false });

    // The smallest cap that works: the concurrent scenarios at smaller caps.
    for cap in [2, 4, 8, 16, 24, 40] {
        case("b burst+turns, updates", heads_all(Some(cap)), |n| burst_then_turns(n, Want::Update));
        case("b2 storm x2", heads_all(Some(cap)), |n| storm(n, 2));
        case("c partition p=10 k=2", heads_all(Some(cap)), |n| partition(n, 10, 2));
        case("c partition p=20 k=2", heads_all(Some(cap)), |n| partition(n, 20, 2));
    }

    // Adversarial delivery: each member hears a round in its own order, for two orders per cap,
    // without and with the backlink (own latest event always named first).
    for salt in [1u64, 2] {
        for cap in [Some(2), Some(4), Some(8), Some(16), Some(24), Some(DESIGN_CAP), Some(40), None] {
            for backlink in [false, true] {
                let opts = Opts { reorder: Some(salt), merger_only: false };
                let kit = || HeadsKit { cap, track_app: true, backlink };
                case_opts("b2 storm x3", kit(), opts, |n| storm(n, 3));
                case_opts("c partition p=2 k=3", kit(), opts, |n| partition(n, 2, 3));
            }
        }
    }
    // Designated merger: only member 0 merges at the cap. In publication order, and reordered.
    for opts in [
        Opts { reorder: None, merger_only: true },
        Opts { reorder: Some(1), merger_only: true },
    ] {
        for cap in [Some(16), Some(DESIGN_CAP)] {
            let kit = || HeadsKit { cap, track_app: true, backlink: false };
            case_opts("b2 storm x3", kit(), opts, |n| storm(n, 3));
            case_opts("c partition p=2 k=3", kit(), opts, |n| partition(n, 2, 3));
            case_opts("c partition p=20 k=3", kit(), opts, |n| partition(n, 20, 3));
        }
    }
}

/// The scenario set, run with one orderer.
fn run_matrix<K: Kit>(make: impl Fn() -> K) {
    case("a steady, app posts x2", make(), |n| steady(n, Want::App, 2));
    case("a steady, updates x2", make(), |n| steady(n, Want::Update, 2));
    case("b burst, updates", make(), |n| burst_then_turns(n, Want::Update));
    case("b burst, app posts", make(), |n| burst_then_turns(n, Want::App));
    case("b2 storm x3", make(), |n| storm(n, 3));
    for p in [2, 10, 20] {
        case(&format!("c partition p={p} k=3"), make(), |n| partition(n, p, 3));
    }
    case("d offline, 10 cycles", make(), |n| offline(n, 10, false));
    case("d offline, shuffled", make(), |n| offline(n, 10, true));
    case("e lost update", make(), |n| lost(n, 3));
}
