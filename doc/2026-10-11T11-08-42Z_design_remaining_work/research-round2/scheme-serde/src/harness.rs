// Shared harness for scheme-serde. main.rs includes this file once per build. Before the include,
// the module defines `enc` (the crate under test) and `Dgm` (its test membership).
//
// The scenario and setup are those of S12 (spikes-next/s12-welcome/src/harness.rs): the adder
// creates a group of three, then rotates K times (odd K: update; even K: remove Carol and add her
// back). This copy measures three things at each sample K:
//   - the full welcome for a fresh joiner (X3DH path), as in S12;
//   - the direct message to Bob for one update (2SM path, one secret, one recipient);
//   - the re-add welcome for Carol (HPKE path, full bundle).
// It writes the K=1 and K=1000 bytes to `out/` so that main.rs can cross-decode them and run the
// transcoder and the compression tests.

use std::fmt::Debug;
use std::fs;
use std::path::Path;

use p2panda_core::cbor::{decode_cbor, encode_cbor};

use enc::data_scheme::dcgka::{
    ControlMessage, Dcgka, DcgkaState, DirectMessage, DirectMessageContent, GroupSecretOutput,
    ProcessInput,
};
use enc::data_scheme::group::{EncryptionGroup, GroupState};
use enc::data_scheme::group_secret::{SecretBundle, SecretBundleState};
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
// Joiner for the welcome probes. Probes run on clones of the adder's state.
const PROBE: Id = 3;
// Joiner for the joiner-side check, which processes the full welcome with its own keys.
const JOIN: Id = 4;

const ROTATIONS: usize = 1000;
const SAMPLES: [usize; 11] = [0, 1, 2, 5, 10, 50, 100, 250, 500, 750, 1000];

fn ok<T, E: Debug>(result: Result<T, E>, what: &str) -> T {
    result.unwrap_or_else(|err| panic!("{what} failed: {err:?}"))
}

// The adder's state, plus a key bundle in the adder's registry for every identity in the run.
fn init_adder(rng: &enc::Rng) -> (Y, Dcgka0) {
    let lifetime = Lifetime::default();
    let mut registry = KeyRegistry::<Id>::init();
    let mut adder_keys = None;
    let mut probe_keys = None;
    for id in [ADMIN, BOB, CAROL, PROBE, JOIN] {
        let identity = enc::test_utils::crypto::SecretKey::from_bytes(rng.random_array().unwrap());
        let keys = ok(
            KeyManager::init_and_generate_prekey(&identity, lifetime, rng),
            "prekey",
        );
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

fn dm_for(msg: &Msg, recipient: Id) -> Dm {
    msg.direct_messages()
        .into_iter()
        .find(|dm| dm.recipient == recipient)
        .expect("a direct message for the recipient")
}

// Full-history welcome for a fresh joiner (X3DH, PreKey path).
fn full_welcome(y: &Y, rng: &enc::Rng) -> Dm {
    welcome(y, PROBE, &y.secrets, rng)
}

// The direct message to Bob for one update (2SM, HPKE path, one new secret).
fn update_dm(y: &Y, rng: &enc::Rng) -> Dm {
    let (_, msg) = ok(EncryptionGroup::update(y.clone(), rng), "update probe");
    dm_for(&msg, BOB)
}

// Carol's re-add welcome after remove and add (HPKE path, full bundle).
fn readd_welcome(y: &Y, rng: &enc::Rng) -> Dm {
    let (without, _) = ok(EncryptionGroup::remove(y.clone(), CAROL, rng), "remove probe");
    let (_, msg) = ok(EncryptionGroup::add(without, CAROL, rng), "re-add probe");
    dm_for(&msg, CAROL)
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

pub struct Dumps {
    pub k1_welcome: Vec<u8>,
    pub k1000_welcome: Vec<u8>,
    pub k1000_update_dm: Vec<u8>,
    pub k1000_readd: Vec<u8>,
}

// Runs the scenario for one build and seed, prints one table, and writes the dumps to `out_dir`.
pub fn run(name: &str, seed: u8, out_dir: &Path) -> Dumps {
    println!("## build: {name}, seed {seed}");
    let rng = enc::Rng::from_seed([seed; 32]);
    let (adder, _) = init_adder(&rng);
    let (mut y, _) = ok(
        EncryptionGroup::create(adder, vec![ADMIN, BOB, CAROL], &rng),
        "create",
    );

    let mut rows: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
    let mut dumps = Dumps {
        k1_welcome: Vec::new(),
        k1000_welcome: Vec::new(),
        k1000_update_dm: Vec::new(),
        k1000_readd: Vec::new(),
    };
    for k in 0..=ROTATIONS {
        if k > 0 && k % 2 == 1 {
            let (next, _) = ok(EncryptionGroup::update(y, &rng), "update");
            y = next;
        } else if k > 0 {
            let (without, _) = ok(EncryptionGroup::remove(y, CAROL, &rng), "remove");
            let (next, _) = ok(EncryptionGroup::add(without, CAROL, &rng), "add back");
            y = next;
        }
        if SAMPLES.contains(&k) {
            let full = full_welcome(&y, &rng);
            let upd = update_dm(&y, &rng);
            let readd = readd_welcome(&y, &rng);
            let full_bytes = encode_cbor(&full).expect("encode the welcome");
            let upd_bytes = encode_cbor(&upd).expect("encode the update message");
            let readd_bytes = encode_cbor(&readd).expect("encode the re-add welcome");
            if k == 1 {
                dumps.k1_welcome = full_bytes.clone();
            }
            if k == ROTATIONS {
                dumps.k1000_welcome = full_bytes.clone();
                dumps.k1000_update_dm = upd_bytes.clone();
                dumps.k1000_readd = readd_bytes.clone();
            }
            rows.push((
                k,
                y.secrets.len(),
                full_bytes.len(),
                upd_bytes.len(),
                readd_bytes.len(),
            ));
        }
    }

    println!(
        "| K | secrets | welcome B (PreKey, full) | update DM to Bob B (HPKE, 1 secret) | re-add welcome B (HPKE, full) |"
    );
    println!("|---|---|---|---|---|");
    for (k, secrets, welcome, upd, readd) in &rows {
        println!("| {k} | {secrets} | {welcome} | {upd} | {readd} |");
    }
    let welcome_pts: Vec<(f64, f64)> = rows.iter().map(|r| (r.1 as f64, r.2 as f64)).collect();
    let (slope, intercept, worst) = fit(&welcome_pts);
    println!(
        "fit welcome = a + b*secrets over {} samples: b = {slope:.2} B/secret, a = {intercept:.1} B, max residual = {worst:.1} B",
        welcome_pts.len()
    );
    let readd_pts: Vec<(f64, f64)> = rows.iter().map(|r| (r.1 as f64, r.4 as f64)).collect();
    let (rslope, rintercept, _) = fit(&readd_pts);
    println!("fit re-add = a + b*secrets: b = {rslope:.2} B/secret, a = {rintercept:.1} B");
    let upd_min = rows.iter().map(|r| r.3).min().unwrap_or(0);
    let upd_max = rows.iter().map(|r| r.3).max().unwrap_or(0);
    println!("update DM to Bob over all samples: min {upd_min} B, max {upd_max} B");

    let file_name = name.replace(' ', "-");
    let write = |suffix: &str, bytes: &[u8]| {
        fs::write(out_dir.join(format!("{file_name}-s{seed}-{suffix}.bin")), bytes)
            .expect("write dump");
    };
    write("welcome-k1", &dumps.k1_welcome);
    write("welcome-k1000", &dumps.k1000_welcome);
    write("update-dm-k1000", &dumps.k1000_update_dm);
    write("readd-k1000", &dumps.k1000_readd);
    println!();
    dumps
}

// Decodes a welcome with this build's types. Returns the error text when decoding fails.
pub fn decode_welcome(bytes: &[u8]) -> Result<(), String> {
    decode_cbor::<Dm, _>(bytes)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

// Decodes a welcome with this build's types and encodes it again.
pub fn reencode_welcome(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let dm = decode_cbor::<Dm, _>(bytes).map_err(|err| err.to_string())?;
    encode_cbor(&dm).map_err(|err| err.to_string())
}
