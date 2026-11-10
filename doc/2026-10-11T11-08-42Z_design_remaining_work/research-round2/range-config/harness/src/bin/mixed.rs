//! Mixed and extreme K and T on real redb stores over iroh, plus the failure-mode checks that
//! RESULTS.md cites. Each wire run is checked against the in-memory run with the same parameters,
//! and against the ground truth taken from the two stores before the sync.
//!
//! Usage (under `unshare -Urn`, after `ip link set lo up`):
//!   mixed pairs     <dir> <chains> <repeats>   K/T pairs, new events on both sides
//!   mixed bootstrap <dir> <chains>             one side empty, both directions
//!   mixed fork      <dir> <chains>             two different events at one (author, seq)
//!   mixed tzero                                in memory: parameter values the code rejects
//!   mixed frame     <events>                   in memory: one leaf reply, and index memory
//!   mixed gap                                  compare_logs on heights, for the same cases

use anyhow::{Context, Result, bail};
use p2panda_core::logs::{LogHeights, compare_logs};
use p2panda_core::{Hash, VerifyingKey};
use s7_range_real::chain::{self, BODY_LEN, EVENTS_PER_CHAIN, chain_events, chain_key, event_with_body};
use s7_range_real::range::{self, Entry, Mode, Party, simulate_mixed};
use s7_range_real::session::{Cut, Params, Side, sync_pair_mixed};
use s7_range_real::store::{Id, Item, KEY_LEN, Key, Store, key_of};
use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `MAX_FRAME` in session.rs. `write_frame` refuses a payload larger than this.
const MAX_FRAME: usize = 16 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Empty {
    Neither,
    Dialer,
    Acceptor,
}

/// `dial` and `acc` are (K, T), in the order the brief writes them.
#[derive(Clone)]
struct Case {
    label: String,
    dial: (usize, usize),
    acc: (usize, usize),
    dial_new: usize,
    acc_new: usize,
    empty: Empty,
}

#[derive(Default, Clone)]
struct Rec {
    flights: (u32, u32),
    control: usize,
    data: usize,
    wall: Duration,
    commit: Duration,
    index: Duration,
    learned: usize,
    inserted: (usize, usize),
    dups: usize,
    rows: usize,
    rows_b: usize,
    union: usize,
    sim_flights: u32,
    sim_ok: bool,
    failures: Vec<String>,
}

impl Rec {
    fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// The base store, as `gen` builds it: `chains` chains of 50 events. Reused if it exists.
fn base(dir: &Path, chains: usize) -> Result<PathBuf> {
    let path = dir.join(format!("base-{chains}.redb"));
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    let store = Store::open(&path)?;
    let mut pending = Vec::new();
    for i in 0..chains {
        for ev in chain_events(&chain_key(i), EVENTS_PER_CHAIN) {
            let v = chain::verify(&ev.header, &ev.body)?;
            pending.push(Item { key: key_of(&v.author, v.seq), id: v.id, record: chain::record(&ev.header, &ev.body) });
            if pending.len() == 10_000 {
                store.insert(&pending)?;
                pending.clear();
            }
        }
    }
    store.insert(&pending)?;
    drop(store);
    Ok(path)
}

/// The next event (seq 50) of chain `c`, as a store item.
fn next_item(c: usize) -> Result<Item> {
    let ev = chain_events(&chain_key(c), EVENTS_PER_CHAIN + 1).pop().context("no next event")?;
    let v = chain::verify(&ev.header, &ev.body)?;
    Ok(Item { key: key_of(&v.author, v.seq), id: v.id, record: chain::record(&ev.header, &ev.body) })
}

/// Adds `n` next events to the store at `path`, on chains `j * step`, or `j * step + step / 2`
/// when `offset` is set, with `step = chains / n`. The two sides use different offsets, so their
/// new events sit on different chains, and no (author, seq) is written twice.
fn add_new(path: &Path, chains: usize, n: usize, offset: bool) -> Result<()> {
    if n == 0 {
        return Ok(());
    }
    let step = chains / n;
    let items = (0..n)
        .map(|j| next_item(j * step + if offset { step / 2 } else { 0 }))
        .collect::<Result<Vec<_>>>()?;
    let store = Store::open(path)?;
    store.insert(&items)?;
    drop(store);
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

/// Copies the base into two stores, leaving out an empty side, and adds the new events. The
/// copies are flushed first, as in S7's bench, so that fsync does not wait on the copies.
fn prepare(dir: &Path, base: &Path, tag: &str, chains: usize, c: &Case) -> Result<(PathBuf, PathBuf)> {
    let a = dir.join(format!("a-{tag}.redb"));
    let b = dir.join(format!("b-{tag}.redb"));
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    if c.empty != Empty::Dialer {
        std::fs::copy(base, &a)?;
    }
    if c.empty != Empty::Acceptor {
        std::fs::copy(base, &b)?;
    }
    add_new(&a, chains, c.dial_new, false)?;
    add_new(&b, chains, c.acc_new, true)?;
    for p in [&a, &b] {
        if p.exists() {
            std::fs::File::open(p)?.sync_all()?;
        }
    }
    Ok((a, b))
}

/// Syncs the stores at `a_path` (dialer, K/T `dial`) and `b_path` (acceptor, `acc`), and checks
/// the run against the in-memory run and the ground truth.
async fn sync_check(a_path: &Path, b_path: &Path, dial: (usize, usize), acc: (usize, usize)) -> Result<Rec> {
    let mut rec = Rec::default();
    let store_a = Arc::new(Store::open(a_path)?);
    let store_b = Arc::new(Store::open(b_path)?);
    let pre_a = store_a.pairs()?;
    let pre_b = store_b.pairs()?;
    let ids_a: HashSet<Id> = pre_a.iter().map(|p| p.1).collect();
    let ids_b: HashSet<Id> = pre_b.iter().map(|p| p.1).collect();
    let truth: HashSet<Id> = ids_a.symmetric_difference(&ids_b).copied().collect();
    let a_needs = ids_b.difference(&ids_a).count();
    let b_needs = ids_a.difference(&ids_b).count();
    rec.union = ids_a.union(&ids_b).count();

    let started = Instant::now();
    let party_a = Arc::new(store_a.index()?);
    let party_b = Arc::new(store_b.index()?);
    rec.index = started.elapsed();

    // The dialer is `a`. `simulate_mixed` takes (T, K).
    let sim = simulate_mixed(&party_a, &party_b, (dial.1, dial.0), (acc.1, acc.0));
    rec.sim_flights = sim.flights;

    let run = sync_pair_mixed(
        Side { store: store_a.clone(), party: party_a.clone() },
        Side { store: store_b.clone(), party: party_b.clone() },
        Params { k: dial.0, t: dial.1, batch: 256 },
        Params { k: acc.0, t: acc.1, batch: 256 },
        Cut::Never,
    )
    .await?;
    rec.wall = run.wall;

    let out_a = match run.dialer {
        Ok(o) => Some(o),
        Err(e) => {
            rec.failures.push(format!("dialer: {e:#}"));
            None
        }
    };
    let out_b = match run.acceptor {
        Ok(o) => Some(o),
        Err(e) => {
            rec.failures.push(format!("acceptor: {e:#}"));
            None
        }
    };
    if let (Some(a), Some(b)) = (&out_a, &out_b) {
        rec.flights = (a.flights, b.flights);
        rec.control = a.control_bytes + b.control_bytes;
        rec.data = a.data_bytes + b.data_bytes;
        rec.commit = a.commit + b.commit;
        rec.learned = a.learned.len();
        rec.inserted = (a.inserted, b.inserted);
        rec.dups = a.duplicates + b.duplicates;
        if a.learned != truth {
            rec.failures.push("dialer did not learn the true difference".into());
        }
        if b.learned != truth {
            rec.failures.push("acceptor did not learn the true difference".into());
        }
        if a.inserted != a_needs {
            rec.failures.push(format!("dialer inserted {} of {a_needs}", a.inserted));
        }
        if b.inserted != b_needs {
            rec.failures.push(format!("acceptor inserted {} of {b_needs}", b.inserted));
        }
        if rec.dups != 0 {
            rec.failures.push(format!("{} duplicate inserts", rec.dups));
        }
        rec.sim_ok = rec.flights.0 + rec.flights.1 == sim.flights
            && rec.control == sim.bytes_a + sim.bytes_b + 4 * sim.flights as usize;
        if !rec.sim_ok {
            rec.failures.push(format!(
                "wire differs from in-memory run: flights {} vs {}, control {} vs {}",
                rec.flights.0 + rec.flights.1,
                sim.flights,
                rec.control,
                sim.bytes_a + sim.bytes_b + 4 * sim.flights as usize
            ));
        }
    }

    let post_a = store_a.pairs()?;
    let post_b = store_b.pairs()?;
    rec.rows = post_a.len();
    rec.rows_b = post_b.len();
    if post_a != post_b {
        rec.failures.push(format!("stores differ after sync: {} rows vs {} rows", post_a.len(), post_b.len()));
    }
    if post_a.len() != rec.union {
        rec.failures.push(format!("{} rows after sync, union is {}", post_a.len(), rec.union));
    }
    Ok(rec)
}

async fn run_case(dir: &Path, base: &Path, tag: &str, chains: usize, c: &Case) -> Result<Rec> {
    let (a, b) = prepare(dir, base, tag, chains, c)?;
    let rec = sync_check(&a, &b, c.dial, c.acc).await?;
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    Ok(rec)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1e3)
}

async fn pairs(dir: &Path, chains: usize, repeats: usize) -> Result<()> {
    let base = base(dir, chains)?;
    let c = |label: &str, dial, acc, dial_new, acc_new| Case {
        label: label.into(),
        dial,
        acc,
        dial_new,
        acc_new,
        empty: Empty::Neither,
    };
    let cases = vec![
        c("calibration: 40 new on the dialer only (S7's N=40 row)", (16, 16), (16, 16), 40, 0),
        c("S7 default on both sides", (16, 16), (16, 16), 20, 20),
        c("mixed: dialer K16 T16, acceptor K4 T8", (16, 16), (4, 8), 20, 20),
        c("mixed: dialer K4 T8, acceptor K16 T16", (4, 8), (16, 16), 20, 20),
        c("K2 T4 on both sides", (2, 4), (2, 4), 20, 20),
        c("K4 T8 on both sides (S7's bulk setting)", (4, 8), (4, 8), 20, 20),
        c("K32 T32 on both sides", (32, 32), (32, 32), 20, 20),
        c("mixed: dialer K2 T4, acceptor K32 T32", (2, 4), (32, 32), 20, 20),
        c("mixed: dialer K32 T32, acceptor K2 T4", (32, 32), (2, 4), 20, 20),
    ];
    println!("## pairs: chains={chains}, base {} events, repeats={repeats}", chains * EVENTS_PER_CHAIN as usize);
    println!("| case | dialer K,T | acceptor K,T | new events, dialer / acceptor | flights A+B | control B | data B | wall ms, median | commit ms, median | index ms, median | checks |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (i, case) in cases.iter().enumerate() {
        let mut runs = Vec::new();
        for r in 0..repeats {
            let rec = run_case(dir, &base, &format!("p{i}-{r}"), chains, case).await?;
            for f in &rec.failures {
                eprintln!("FAILED {} run {}: {f}", case.label, r + 1);
            }
            runs.push(rec);
        }
        let first = runs[0].clone();
        let same = runs.iter().all(|r| r.flights == first.flights && r.control == first.control && r.data == first.data);
        let all_ok = runs.iter().all(|r| r.ok() && r.sim_ok);
        let checks = if all_ok && same {
            format!("exact; {} runs identical; wire = in-memory", runs.len())
        } else {
            "FAILED (see stderr)".to_string()
        };
        println!(
            "| {} | K{} T{} | K{} T{} | {} / {} | {} ({}+{}) | {} | {} | {} | {} | {} | {} |",
            case.label,
            case.dial.0,
            case.dial.1,
            case.acc.0,
            case.acc.1,
            case.dial_new,
            case.acc_new,
            first.flights.0 + first.flights.1,
            first.flights.0,
            first.flights.1,
            first.control,
            first.data,
            ms(median(runs.iter().map(|r| r.wall).collect())),
            ms(median(runs.iter().map(|r| r.commit).collect())),
            ms(median(runs.iter().map(|r| r.index).collect())),
            checks
        );
    }
    println!("base store: {} events per side before the new events", chains * EVENTS_PER_CHAIN as usize);
    Ok(())
}

async fn bootstrap(dir: &Path, chains: usize) -> Result<()> {
    let base = base(dir, chains)?;
    let cases = [
        Case { label: "full dials empty, both K16 T16".into(), dial: (16, 16), acc: (16, 16), dial_new: 0, acc_new: 0, empty: Empty::Acceptor },
        Case { label: "empty dials full, both K16 T16".into(), dial: (16, 16), acc: (16, 16), dial_new: 0, acc_new: 0, empty: Empty::Dialer },
        Case { label: "full (K4 T8) dials empty (K16 T16)".into(), dial: (4, 8), acc: (16, 16), dial_new: 0, acc_new: 0, empty: Empty::Acceptor },
        Case { label: "empty (K16 T16) dials full (K4 T8)".into(), dial: (16, 16), acc: (4, 8), dial_new: 0, acc_new: 0, empty: Empty::Dialer },
    ];
    println!("## bootstrap: chains={chains}, {} events on the full side, one run each", chains * EVENTS_PER_CHAIN as usize);
    println!("| case | flights A+B | control B | data B | control / data | wall ms | checks |");
    println!("|---|---|---|---|---|---|---|");
    for (i, c) in cases.iter().enumerate() {
        let rec = run_case(dir, &base, &format!("boot{i}"), chains, c).await?;
        for f in &rec.failures {
            eprintln!("FAILED {}: {f}", c.label);
        }
        println!(
            "| {} | {} ({}+{}) | {} | {} | {:.3} | {} | {} |",
            c.label,
            rec.flights.0 + rec.flights.1,
            rec.flights.0,
            rec.flights.1,
            rec.control,
            rec.data,
            rec.control as f64 / rec.data.max(1) as f64,
            ms(rec.wall),
            if rec.ok() && rec.sim_ok { "exact; wire = in-memory".to_string() } else { format!("FAILED: {}", rec.failures.join("; ")) }
        );
    }
    Ok(())
}

async fn fork(dir: &Path, chains: usize) -> Result<()> {
    let base = base(dir, chains)?;
    // Chain 3, seq 25: the fork is signed by the same key, with another body, linked to seq 24.
    let key = chain_key(3);
    let author = *key.verifying_key().as_bytes();
    let events = chain_events(&key, EVENTS_PER_CHAIN);
    let fork_ev = event_with_body(&key, 25, Some(Hash::digest(&events[24].header)), &vec![0xAB; BODY_LEN]);
    let a = dir.join("fork-a.redb");
    let b = dir.join("fork-b.redb");
    println!("## fork: chains={chains}; chain 3 at seq 25 is the original on the dialer and a different event on the acceptor");
    println!("| unrelated new events on the dialer | parameters, both sides | failures, verbatim | rows after, dialer / acceptor |");
    println!("|---|---|---|---|");
    for extra in [0usize, 20] {
        for (label, kt) in [("K16 T16", (16, 16)), ("K4 T8", (4, 8))] {
            let _ = std::fs::remove_file(&a);
            let _ = std::fs::remove_file(&b);
            std::fs::copy(&base, &a)?;
            std::fs::copy(&base, &b)?;
            let sb = Store::open(&b)?;
            sb.remove(&key_of(&author, 25))?;
            sb.insert(&[Item { key: key_of(&author, 25), id: fork_ev.id(), record: chain::record(&fork_ev.header, &fork_ev.body) }])?;
            drop(sb);
            // Unrelated events on other chains, in the dialer's store only.
            add_new(&a, chains, extra, false)?;
            let rec = sync_check(&a, &b, kt, kt).await?;
            println!("| {extra} | {label} | {} | {} / {} |", rec.failures.join(" / "), rec.rows, rec.rows_b);
        }
    }
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    Ok(())
}

/// Text of a panic payload.
fn panic_text(e: &Box<dyn Any + Send>) -> String {
    e.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic".into())
}

fn tzero() {
    // 300 keys on the acceptor side; the dialer lacks three of them.
    let mut rows: Vec<(Key, Id)> = (0..300u64)
        .map(|i| {
            let author = *blake3::hash(&i.to_be_bytes()).as_bytes();
            let key = key_of(&author, 0);
            (key, *blake3::hash(&key).as_bytes())
        })
        .collect();
    rows.sort();
    let (ak, ai): (Vec<Key>, Vec<Id>) = rows.iter().copied().unzip();
    let (bk, bi): (Vec<Key>, Vec<Id>) = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| ![10, 150, 290].contains(i))
        .map(|(_, r)| *r)
        .unzip();
    let a = Party::new(ak, ai);
    let b = Party::new(bk, bi);
    println!("| parameters (T, K) on both sides | result |");
    println!("|---|---|");
    for (t, k) in [(16usize, 16usize), (1, 2), (0, 16), (16, 1)] {
        let r = catch_unwind(AssertUnwindSafe(|| simulate_mixed(&a, &b, (t, k), (t, k))));
        match r {
            Ok(sim) => println!(
                "| T={t}, K={k} | ok: {} flights, {} and {} learned |",
                sim.flights,
                sim.learned_a.len(),
                sim.learned_b.len()
            ),
            Err(e) => println!("| T={t}, K={k} | panic: {} |", panic_text(&e)),
        }
    }
    // One key on each side, and the keys differ. The smallest possible mismatch.
    let kx = key_of(blake3::hash(b"x").as_bytes(), 0);
    let ky = key_of(blake3::hash(b"y").as_bytes(), 0);
    let one_x = Party::new(vec![kx], vec![*blake3::hash(&kx).as_bytes()]);
    let one_y = Party::new(vec![ky], vec![*blake3::hash(&ky).as_bytes()]);
    for (t, k) in [(0usize, 16usize), (1, 2)] {
        let r = catch_unwind(AssertUnwindSafe(|| simulate_mixed(&one_x, &one_y, (t, k), (t, k))));
        match r {
            Ok(sim) => println!("| one key each, differing: T={t}, K={k} | ok: {} flights |", sim.flights),
            Err(e) => println!("| one key each, differing: T={t}, K={k} | panic: {} |", panic_text(&e)),
        }
    }
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn frame(n: usize) {
    let before = rss_kib();
    let keys: Vec<Key> = (0..n)
        .map(|i| {
            let mut k = [0u8; KEY_LEN];
            k[..8].copy_from_slice(&(i as u64).to_be_bytes());
            k
        })
        .collect();
    let ids: Vec<Id> = (0..n).map(|i| *blake3::hash(&(i as u64).to_be_bytes()).as_bytes()).collect();
    let party = Party::new(keys, ids);
    let after = rss_kib();
    // The responder is empty, so it answers with an empty ID list for the whole range. The
    // initiator then replies with every ID it holds in that range: one leaf message, unbounded.
    let mut learned = HashSet::new();
    let reply = party.respond(&[Entry { upper: None, mode: Mode::Ids { ids: vec![], reply: true } }], 16, 16, &mut learned);
    let bytes = range::encode(&reply).len();
    let rss_kib_delta = after.saturating_sub(before);
    println!("| events | leaf reply, bytes | payload limit, bytes | over the limit | index RSS, MiB | RSS bytes per event |");
    println!("|---|---|---|---|---|---|");
    println!(
        "| {n} | {bytes} | {MAX_FRAME} | {} | {:.1} | {:.0} |",
        bytes > MAX_FRAME,
        rss_kib_delta as f64 / 1024.0,
        rss_kib_delta as f64 * 1024.0 / n as f64
    );
}

/// The same synthetic index as `frame`.
fn synthetic(n: usize) -> Party {
    let keys: Vec<Key> = (0..n)
        .map(|i| {
            let mut k = [0u8; KEY_LEN];
            k[..8].copy_from_slice(&(i as u64).to_be_bytes());
            k
        })
        .collect();
    let ids: Vec<Id> = (0..n).map(|i| *blake3::hash(&(i as u64).to_be_bytes()).as_bytes()).collect();
    Party::new(keys, ids)
}

/// Bootstrap with the empty side dialing. Each flight is one frame, and it carries the replies
/// for every entry it answers, so the total for the flight is what the limit applies to.
fn frame_empty_dialer(n: usize, k: usize, t: usize) {
    let full = synthetic(n);
    let empty = Party::new(vec![], vec![]);
    let mut learned = HashSet::new();
    let open = vec![empty.full_range()];
    let split = full.respond(&open, t, k, &mut learned);
    let ask = empty.respond(&split, t, k, &mut learned);
    let reply = full.respond(&ask, t, k, &mut learned);
    let bytes = range::encode(&reply).len();
    println!("| events | K, T | entries in flight 2 | flight 4 bytes (one frame) | over the limit |");
    println!("|---|---|---|---|---|");
    println!("| {n} | K{k} T{t} | {} | {bytes} | {} |", split.len(), bytes > MAX_FRAME);
}

fn gap() {
    let author: VerifyingKey = chain_key(7).verifying_key();
    let heights = |h: Option<u32>| -> LogHeights<VerifyingKey, u64> {
        match h {
            Some(h) => BTreeMap::from([(author.clone(), BTreeMap::from([(0u64, h)]))]),
            None => BTreeMap::new(),
        }
    };
    let cases = [
        ("gap at seq 20 on the remote (has 0..19 and 21..49): heights 49 and 49", heights(Some(49)), heights(Some(49))),
        ("same heights, different event at seq 25 (fork): heights 49 and 49", heights(Some(49)), heights(Some(49))),
        ("remote lags: heights 49 and 30", heights(Some(49)), heights(Some(30))),
        ("remote has no entries for this chain: heights 49 and none", heights(Some(49)), heights(None)),
    ];
    println!("| case | compare_logs(local, remote) for the chain | entries sent |");
    println!("|---|---|---|");
    for (label, local, remote) in cases {
        let ranges = compare_logs(&local, &remote);
        let entry = ranges.get(&author).and_then(|m| m.get(&0u64)).copied();
        let sent = ranges.values().map(|m| m.len()).sum::<usize>();
        println!("| {label} | {:?} | {sent} |", entry);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| -> Result<&str> { args.get(i).map(String::as_str).context("missing argument") };
    match arg(0)? {
        "pairs" => pairs(&PathBuf::from(arg(1)?), arg(2)?.parse()?, arg(3)?.parse()?).await,
        "bootstrap" => bootstrap(&PathBuf::from(arg(1)?), arg(2)?.parse()?).await,
        "fork" => fork(&PathBuf::from(arg(1)?), arg(2)?.parse()?).await,
        "tzero" => {
            tzero();
            Ok(())
        }
        "frame" => {
            frame(arg(1)?.parse()?);
            Ok(())
        }
        "gap" => {
            gap();
            Ok(())
        }
        "frame-empty" => {
            frame_empty_dialer(arg(1)?.parse()?, 16, 16);
            Ok(())
        }
        other => bail!("unknown command {other}"),
    }
}
