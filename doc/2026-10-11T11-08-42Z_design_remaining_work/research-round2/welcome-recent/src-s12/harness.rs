// Shared S12 harness. main.rs includes this file once per build. Before the include, the module
// defines `enc` (the crate under test) and `Dgm` (its test membership).
//
// Only the adder is driven. Its welcome depends on its own state and the joiner's key bundle. In
// this scenario Bob and Carol never send, so the adder's state is the same as it would be if they
// processed every message.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cbor_core::Value;
use p2panda_core::cbor::{decode_cbor, encode_cbor};

use enc::data_scheme::dcgka::{
    ControlMessage, Dcgka, DcgkaState, DirectMessage, DirectMessageContent, GroupSecretOutput,
    ProcessInput,
};
use enc::data_scheme::group::{EncryptionGroup, GroupState};
use enc::data_scheme::group_secret::{GroupSecret, SecretBundle, SecretBundleState};
use enc::data_scheme::test_utils::ordering::{MessageOrderer, TestMessage};
use enc::key_bundle::Lifetime;
use enc::key_manager::KeyManager;
use enc::key_registry::KeyRegistry;
use enc::test_utils::{MemberId, MessageId};
use enc::traits::{GroupMessage, Ordering, PreKeyManager, PreKeyRegistry};

type Id = MemberId;
type Mid = MessageId;
type Orderer = MessageOrderer<Dgm>;
type Pki = KeyRegistry<Id>;
type Kmg = KeyManager;
type Y = GroupState<Id, Mid, Pki, Dgm, Kmg, Orderer>;
type Dcgka0 = DcgkaState<Id, Mid, Pki, Dgm, Kmg>;
type Msg = TestMessage<Dgm>;
type Dm = DirectMessage<Id, Mid, Dgm>;

const ADMIN: Id = 0;
const BOB: Id = 1;
const CAROL: Id = 2;
// Joiner for the size probes and the joiner-side check. Probes run on clones of the adder's state.
const PROBE: Id = 3;
// Joiner for the final add, which goes through `EncryptionGroup::add`.
const JOIN: Id = 4;

// Each rotation mints one secret. Odd rotations are `update`. Even rotations `remove` Carol and
// add her back, so the group stays at three members. Re-adding mints no secret.
const ROTATIONS: usize = 1000;
const SAMPLES: [usize; 11] = [0, 1, 2, 5, 10, 50, 100, 250, 500, 750, 1000];
// Recent-N welcomes: only the N newest secrets go into the bundle that `Dcgka::add` receives.
const RECENT: [usize; 3] = [1, 8, 32];

fn ok<T, E: std::fmt::Debug>(result: Result<T, E>, what: &str) -> T {
    result.unwrap_or_else(|err| panic!("{what} failed: {err:?}"))
}

fn log_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("logs")
        .join(name)
}

// The adder's state, and the probe joiner's own `DcgkaState` for the joiner-side check. Every
// identity the run uses has a long-term key bundle in the adder's registry.
fn init_adder(rng: &enc::Rng) -> (Y, Dcgka0) {
    let lifetime = Lifetime::default();
    let mut registry = KeyRegistry::<Id>::init();
    let mut adder_keys = None;
    let mut probe_keys = None;
    for id in [ADMIN, BOB, CAROL, PROBE, JOIN] {
        let identity = enc::test_utils::crypto::SecretKey::from_bytes(rng.random_array().unwrap());
        let keys = ok(KeyManager::init_and_generate_prekey(&identity, lifetime, rng), "prekey");
        let bundle = ok(KeyManager::prekey_bundle(&keys), "prekey bundle");
        registry = ok(KeyRegistry::add_longterm_bundle(registry, id, bundle), "register");
        match id {
            ADMIN => adder_keys = Some(keys),
            PROBE => probe_keys = Some(keys),
            _ => {}
        }
    }
    let joiner = Dcgka::init(
        PROBE,
        probe_keys.expect("probe keys"),
        registry.clone(),
        Dgm::init(PROBE),
    );
    let adder = EncryptionGroup::init(
        ADMIN,
        adder_keys.expect("adder keys"),
        registry,
        Dgm::init(ADMIN),
        Orderer::init(ADMIN),
    );
    (adder, joiner)
}

// The welcome for `joiner` that `Dcgka::add` builds from `bundle`. Runs on a clone of the state.
fn welcome(y: &Y, joiner: Id, bundle: &SecretBundleState, rng: &enc::Rng) -> Dm {
    let (_, output) = ok(Dcgka::add(y.dcgka.clone(), joiner, bundle, rng), "welcome");
    output
        .direct_messages
        .into_iter()
        .find(|dm| dm.recipient == joiner)
        .expect("a welcome for the joiner")
}

fn welcome_in(msg: &Msg, joiner: Id) -> Dm {
    msg.direct_messages()
        .into_iter()
        .find(|dm| dm.recipient == joiner)
        .expect("a welcome for the joiner")
}

// Bytes of the whole welcome, of its history (the DGM state), and of its ciphertext.
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

// What the joiner learns from a welcome, processed with the joiner's own keys.
fn learned(joiner: &Dcgka0, dm: Dm, seq: Mid) -> SecretBundleState {
    let input = ProcessInput {
        seq,
        sender: ADMIN,
        control_message: ControlMessage::Add { added: PROBE },
        direct_message: Some(dm),
    };
    match ok(Dcgka::process(joiner.clone(), input), "joiner processes the welcome").1 {
        GroupSecretOutput::Bundle(bundle) => bundle,
        _ => panic!("a welcome carries a bundle"),
    }
}

// The bundle of the `n` newest secrets. Timestamps increase within a bundle (`SecretBundle::generate`
// bumps them), so sorting by timestamp picks the newest.
fn newest(y: &Y, n: usize) -> SecretBundleState {
    let mut secrets: Vec<GroupSecret> = y.secrets.secrets().cloned().collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.timestamp()));
    secrets.truncate(n);
    SecretBundle::from_secrets(secrets)
}

// Median and minimum, in microseconds.
fn stats_us(mut times: Vec<Duration>) -> (f64, f64) {
    times.sort();
    (
        times[times.len() / 2].as_secs_f64() * 1e6,
        times[0].as_secs_f64() * 1e6,
    )
}

// Time of `Dcgka::add` for the welcome: bundle encoding and 2SM/X3DH encryption.
fn time_welcome(y: &Y, bundle: &SecretBundleState, rng: &enc::Rng, reps: usize) -> Vec<Duration> {
    (0..reps)
        .map(|_| {
            let dcgka = y.dcgka.clone();
            let start = Instant::now();
            let output = ok(Dcgka::add(dcgka, PROBE, bundle, rng), "timed welcome");
            let elapsed = start.elapsed();
            drop(output);
            elapsed
        })
        .collect()
}

// Time of CBOR-encoding the welcome message.
fn time_encode(dm: &Dm, reps: usize) -> Vec<Duration> {
    (0..reps)
        .map(|_| {
            let start = Instant::now();
            let bytes = encode_cbor(dm).expect("encode the welcome");
            let elapsed = start.elapsed();
            drop(bytes);
            elapsed
        })
        .collect()
}

// Time of `EncryptionGroup::add`: the welcome plus local processing and ordering.
fn time_group_add(y: &Y, rng: &enc::Rng, reps: usize) -> Vec<Duration> {
    (0..reps)
        .map(|_| {
            let state = y.clone();
            let start = Instant::now();
            let output = ok(EncryptionGroup::add(state, JOIN, rng), "timed group add");
            let elapsed = start.elapsed();
            drop(output);
            elapsed
        })
        .collect()
}

struct Row {
    k: usize,
    secrets: usize,
    welcome: usize,
    history: usize,
    ciphertext: usize,
    recent: [usize; 3],
    empty: usize,
    welcome_us: (f64, f64),
    encode_us: (f64, f64),
    readd_total: usize,
}

fn probe(y: &Y, k: usize, readd_total: usize, rng: &enc::Rng) -> (Row, Dm) {
    let full = welcome(y, PROBE, &y.secrets, rng);
    let (total, history, ciphertext) = sizes(&full);
    let recent = RECENT.map(|n| sizes(&welcome(y, PROBE, &newest(y, n), rng)).0);
    let empty = sizes(&welcome(y, PROBE, &SecretBundle::init(), rng)).0;
    let reps = if k == ROTATIONS { 50 } else { 15 };
    let row = Row {
        k,
        secrets: y.secrets.len(),
        welcome: total,
        history,
        ciphertext,
        recent,
        empty,
        welcome_us: stats_us(time_welcome(y, &y.secrets, rng, reps)),
        encode_us: stats_us(time_encode(&full, reps)),
        readd_total,
    };
    (row, full)
}

// Least-squares line through (x, y): slope, intercept, and the largest absolute residual.
fn fit(points: &[(f64, f64)]) -> (f64, f64, f64) {
    let n = points.len() as f64;
    let sx: f64 = points.iter().map(|p| p.0).sum();
    let sy: f64 = points.iter().map(|p| p.1).sum();
    let sxx: f64 = points.iter().map(|p| p.0 * p.0).sum();
    let sxy: f64 = points.iter().map(|p| p.0 * p.1).sum();
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    let intercept = (sy - slope * sx) / n;
    let worst = points
        .iter()
        .map(|(x, y)| (y - (intercept + slope * x)).abs())
        .fold(0.0, f64::max);
    (slope, intercept, worst)
}

// Runs the scenario and prints one table. Returns the K=1 welcome and the first re-add welcome,
// for the cross-decode.
pub fn run(name: &str, seed: u8) -> (Vec<u8>, Vec<u8>) {
    println!("## build: {name}, seed {seed}");
    let rng = enc::Rng::from_seed([seed; 32]);
    let (adder, joiner) = init_adder(&rng);
    let (mut y, _) = ok(
        EncryptionGroup::create(adder, vec![ADMIN, BOB, CAROL], &rng),
        "create",
    );

    let file_name = name.replace(' ', "-");
    let mut rows = Vec::new();
    let mut readd_total = 0;
    let mut readd_count = 0;
    let mut readd_first = Vec::new();
    let mut k1 = Vec::new();
    let mut last_seq = None;
    for k in 0..=ROTATIONS {
        if k > 0 && k % 2 == 1 {
            let (next, msg) = ok(EncryptionGroup::update(y, &rng), "update");
            last_seq = Some(msg.id());
            y = next;
        } else if k > 0 {
            let (without, _) = ok(EncryptionGroup::remove(y, CAROL, &rng), "remove");
            let (next, msg) = ok(EncryptionGroup::add(without, CAROL, &rng), "add back");
            last_seq = Some(msg.id());
            let readd_bytes = encode_cbor(&welcome_in(&msg, CAROL)).expect("encode the re-add");
            readd_total += readd_bytes.len();
            readd_count += 1;
            if readd_count == 1 {
                let tree = Value::decode_owned(&readd_bytes).expect("decode the re-add welcome");
                fs::write(
                    log_path(&format!("readd-first-{file_name}.diag")),
                    format!("{tree}\n"),
                )
                .expect("write diagnostic");
                readd_first = readd_bytes;
            }
            y = next;
        }
        if SAMPLES.contains(&k) {
            let (row, full) = probe(&y, k, readd_total, &rng);
            if k == 1 {
                k1 = encode_cbor(&full).expect("encode the K=1 welcome");
                let tree = Value::decode_owned(&k1).expect("decode the K=1 welcome");
                fs::write(
                    log_path(&format!("welcome-k1-{file_name}.diag")),
                    format!("{tree}\n"),
                )
                .expect("write diagnostic");
            }
            rows.push(row);
        }
    }

    println!(
        "| K | secrets | welcome B | history B | ciphertext B | recent 1 | recent 8 | recent 32 | empty B | B/secret since prev | welcome us (med) | encode us (med) | re-add B so far |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    let mut prev: Option<&Row> = None;
    for row in &rows {
        let per_secret = match prev {
            Some(p) => format!(
                "{:.1}",
                (row.welcome - p.welcome) as f64 / (row.secrets - p.secrets) as f64
            ),
            None => "-".to_string(),
        };
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.0} | {:.0} | {} |",
            row.k,
            row.secrets,
            row.welcome,
            row.history,
            row.ciphertext,
            row.recent[0],
            row.recent[1],
            row.recent[2],
            row.empty,
            per_secret,
            row.welcome_us.0,
            row.encode_us.0,
            row.readd_total,
        );
        prev = Some(row);
    }

    let points: Vec<(f64, f64)> = rows
        .iter()
        .map(|r| (r.secrets as f64, r.welcome as f64))
        .collect();
    let (slope, intercept, worst) = fit(&points);
    println!(
        "fit welcome = a + b*secrets over {} samples: b = {slope:.2} B/secret, a = {intercept:.1} B, max residual = {worst:.1} B",
        points.len()
    );

    // What the joiner learns, at K=ROTATIONS. Each welcome is processed with the joiner's keys.
    let seq = last_seq.expect("a rotation happened");
    let full_secrets = y.secrets.clone();
    let learned_full = learned(&joiner, welcome(&y, PROBE, &full_secrets, &rng), seq);
    println!(
        "joiner learns full history: {} secrets, same ids as the adder's bundle: {}",
        learned_full.len(),
        same_ids(&learned_full, &full_secrets)
    );
    for n in RECENT {
        let expected = newest(&y, n);
        let learned_n = learned(&joiner, welcome(&y, PROBE, &expected, &rng), seq);
        println!(
            "joiner learns recent {n}: {} secrets, same ids as the adder's {n} newest: {}",
            learned_n.len(),
            same_ids(&learned_n, &expected)
        );
    }

    // The full API at K=ROTATIONS. This consumes the adder's state, so it runs last.
    let (group_add_med, group_add_min) = stats_us(time_group_add(&y, &rng, 50));
    let (_, final_msg) = ok(EncryptionGroup::add(y, JOIN, &rng), "final add");
    let final_dm = welcome_in(&final_msg, JOIN);
    let (total, history, ciphertext) = sizes(&final_dm);
    println!(
        "final EncryptionGroup::add (full, K={ROTATIONS}): welcome {total} B, history {history} B, ciphertext {ciphertext} B"
    );
    let last = rows.last().expect("a sample at K=ROTATIONS");
    println!(
        "time at K={ROTATIONS} (50 runs): Dcgka::add median {:.0} us, min {:.0} us; encode median {:.0} us, min {:.0} us; EncryptionGroup::add median {group_add_med:.0} us, min {group_add_min:.0} us",
        last.welcome_us.0, last.welcome_us.1, last.encode_us.0, last.encode_us.1
    );
    println!("re-adds of Carol: {readd_count} welcomes, {readd_total} B in total");
    println!();
    (k1, readd_first)
}

fn same_ids(a: &SecretBundleState, b: &SecretBundleState) -> bool {
    a.len() == b.len() && a.ids().all(|id| b.contains(id))
}

// Decodes a welcome with this build's types. Returns the error text when decoding fails.
pub fn decode_welcome(bytes: &[u8]) -> Result<(), String> {
    decode_cbor::<Dm, _>(bytes)
        .map(|_| ())
        .map_err(|err| err.to_string())
}
