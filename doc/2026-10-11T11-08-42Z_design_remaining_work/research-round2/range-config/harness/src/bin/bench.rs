//! Range sync over iroh on real redb stores: the main matrix, and the K/T sweep.
//!
//! Each run copies the base store twice. The dialer's copy gets N new events on N distinct
//! chains (spread evenly, as S1 did). Both sides then sync over localhost with
//! `RelayMode::Disabled`, and the run is checked against the ground truth.
//!
//! Usage:
//!   bench matrix <dir> <chains> <repeats> <K> <T> <N,N,...>
//!   bench sweep  <dir> <chains> <K,K,...> <T,T,...> <N,N,...>

use anyhow::{Context, Result, bail};
use s7_range_real::chain::{self, EVENTS_PER_CHAIN, chain_events, chain_key};
use s7_range_real::range::simulate;
use s7_range_real::session::{Cut, Params, Side, sync_pair};
use s7_range_real::store::{Id, Item, Store, key_of};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// S1's LogSync control bytes, CBOR: two Have messages, PreSync and two Done. Their size depends
/// on the chain count only, so they do not change with N. Taken from S1's log_sizes run and
/// not re-measured here.
fn logsync_control(chains: usize) -> Option<usize> {
    match chains {
        1000 => Some(76_071),
        10_000 => Some(760_071),
        _ => None,
    }
}

/// Copies the base store to two files, adds `n_new` events to the first, and returns the paths
/// and the IDs of the new events (the ground truth for the symmetric difference).
fn prepare(dir: &Path, base: &Path, tag: &str, chains: usize, n_new: usize) -> Result<(PathBuf, PathBuf, HashSet<Id>)> {
    let a = dir.join(format!("a-{tag}.redb"));
    let b = dir.join(format!("b-{tag}.redb"));
    std::fs::copy(base, &a)?;
    std::fs::copy(base, &b)?;
    let store = Store::open(&a)?;
    let mut items = Vec::with_capacity(n_new);
    let mut truth = HashSet::with_capacity(n_new);
    for j in 0..n_new {
        let chain = j * chains / n_new;
        let next = chain_events(&chain_key(chain), EVENTS_PER_CHAIN + 1)
            .pop()
            .context("chain has no next event")?;
        let v = chain::verify(&next.header, &next.body)?;
        truth.insert(v.id);
        items.push(Item {
            key: key_of(&v.author, v.seq),
            id: v.id,
            record: chain::record(&next.header, &next.body),
        });
    }
    store.insert(&items)?;
    // Flush the copies, so that a commit's fsync inside the timed run does not wait for them.
    std::fs::File::open(&a)?.sync_all()?;
    std::fs::File::open(&b)?.sync_all()?;
    Ok((a, b, truth))
}

#[derive(Clone, Default)]
struct Record {
    flights: u32,
    flights_a: u32,
    flights_b: u32,
    control: usize,
    data: usize,
    wall: Duration,
    /// Redb commit time on the acceptor and dialer, summed. Included in `wall`.
    commit: Duration,
    index: Duration,
    transport: u64,
    sim_flights: u32,
    failures: Vec<String>,
}

impl Record {
    fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

async fn sync_once(dir: &Path, base: &Path, chains: usize, n_new: usize, p: Params, tag: &str) -> Result<Record> {
    let (a_path, b_path, truth) = prepare(dir, base, tag, chains, n_new)?;
    let mut rec = Record::default();

    let started = Instant::now();
    let store_a = Arc::new(Store::open(&a_path)?);
    let party_a = Arc::new(store_a.index()?);
    let store_b = Arc::new(Store::open(&b_path)?);
    let party_b = Arc::new(store_b.index()?);
    rec.index = started.elapsed();

    // The in-memory run of the same scheme. Its bytes must match the wire run.
    let sim = simulate(&party_a, &party_b, p.t, p.k);
    rec.sim_flights = sim.flights;

    let run = sync_pair(
        Side { store: store_a.clone(), party: party_a.clone() },
        Side { store: store_b.clone(), party: party_b.clone() },
        p,
        Cut::Never,
    )
    .await?;
    rec.wall = run.wall;
    rec.transport = run.transport_bytes;

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
        rec.flights_a = a.flights;
        rec.flights_b = b.flights;
        rec.flights = a.flights + b.flights;
        rec.control = a.control_bytes + b.control_bytes;
        rec.data = a.data_bytes + b.data_bytes;
        rec.commit = a.commit + b.commit;

        if a.inserted != 0 {
            rec.failures.push(format!("dialer inserted {}", a.inserted));
        }
        if b.inserted != n_new {
            rec.failures.push(format!("acceptor inserted {} of {n_new}", b.inserted));
        }
        if a.duplicates + b.duplicates != 0 {
            rec.failures.push("duplicate inserts".into());
        }
        if a.learned != truth || b.learned != truth {
            rec.failures.push("a side did not learn the true difference".into());
        }
        if rec.flights != sim.flights || rec.control != sim.bytes_a + sim.bytes_b + 4 * sim.flights as usize {
            rec.failures.push(format!(
                "wire run differs from in-memory run: flights {} vs {}, control {} vs {}",
                rec.flights,
                sim.flights,
                rec.control,
                sim.bytes_a + sim.bytes_b + 4 * sim.flights as usize
            ));
        }
    }

    let pairs_a = store_a.pairs()?;
    let pairs_b = store_b.pairs()?;
    let expected = chains * EVENTS_PER_CHAIN as usize + n_new;
    if pairs_a != pairs_b || pairs_a.len() != expected {
        rec.failures.push(format!(
            "stores differ after sync: rows {} vs {}, expected {expected}",
            pairs_a.len(),
            pairs_b.len()
        ));
    }

    drop((party_a, party_b, store_a, store_b));
    let _ = std::fs::remove_file(&a_path);
    let _ = std::fs::remove_file(&b_path);
    Ok(rec)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn parse_list(s: &str) -> Result<Vec<usize>> {
    s.split(',').map(|x| x.trim().parse::<usize>().map_err(Into::into)).collect()
}

fn fmt_ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1e3)
}

async fn matrix(dir: &Path, chains: usize, repeats: usize, p: Params, ns: &[usize]) -> Result<()> {
    let base = dir.join(format!("base-{chains}.redb"));
    println!("## matrix: chains={chains} K={} T={} batch={} repeats={repeats}", p.k, p.t, p.batch);
    println!("| N | flights (A+B) | control B | data B | transport B (QUIC) | wall ms, median | of which commit ms, median | index ms, median | checks |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for &n in ns {
        let mut runs = Vec::new();
        for r in 0..repeats {
            let rec = sync_once(dir, &base, chains, n, p, &format!("m{n}-{r}")).await?;
            println!(
                "run N={n} r={} flights={} (A {} B {}) control={} data={} wall={} ms commit={} ms index={} ms transport={} ok={}{}",
                r + 1,
                rec.flights,
                rec.flights_a,
                rec.flights_b,
                rec.control,
                rec.data,
                fmt_ms(rec.wall),
                fmt_ms(rec.commit),
                fmt_ms(rec.index),
                rec.transport,
                rec.ok(),
                if rec.ok() { String::new() } else { format!(" FAILED: {}", rec.failures.join("; ")) }
            );
            runs.push(rec);
        }
        let first = &runs[0];
        let same = runs.iter().all(|r| r.flights == first.flights && r.control == first.control && r.data == first.data);
        let all_ok = runs.iter().all(Record::ok);
        let mut transport: Vec<u64> = runs.iter().map(|r| r.transport).collect();
        transport.sort();
        println!(
            "| {n} | {} | {} | {} | {} | {} | {} | {} | {} |",
            first.flights,
            first.control,
            first.data,
            transport[transport.len() / 2],
            fmt_ms(median(runs.iter().map(|r| r.wall).collect())),
            fmt_ms(median(runs.iter().map(|r| r.commit).collect())),
            fmt_ms(median(runs.iter().map(|r| r.index).collect())),
            if all_ok && same { "all ok, identical across runs" } else { "SEE RUN LINES" }
        );
    }
    if let Some(ls) = logsync_control(chains) {
        println!("LogSync control for this chain count (S1, CBOR, not re-run): {ls} B, 2 flights before data.");
    }
    Ok(())
}

async fn sweep(dir: &Path, chains: usize, ks: &[usize], ts: &[usize], ns: &[usize]) -> Result<()> {
    let base = dir.join(format!("base-{chains}.redb"));
    println!("## sweep: chains={chains} (one run per row, bytes and flights are deterministic)");
    println!("| K | T | N | flights | control B | data B | ok |");
    println!("|---|---|---|---|---|---|---|");
    for &k in ks {
        for &t in ts {
            for &n in ns {
                let p = Params { k, t, batch: 256 };
                let rec = sync_once(dir, &base, chains, n, p, &format!("s{k}-{t}-{n}")).await?;
                println!(
                    "| {k} | {t} | {n} | {} | {} | {} | {}{} |",
                    rec.flights,
                    rec.control,
                    rec.data,
                    rec.ok(),
                    if rec.ok() { String::new() } else { format!(" {}", rec.failures.join("; ")) }
                );
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| -> Result<&str> { args.get(i).map(String::as_str).context("missing argument") };
    match arg(0)? {
        "matrix" => {
            let dir = PathBuf::from(arg(1)?);
            let chains: usize = arg(2)?.parse()?;
            let repeats: usize = arg(3)?.parse()?;
            let p = Params { k: arg(4)?.parse()?, t: arg(5)?.parse()?, batch: 256 };
            matrix(&dir, chains, repeats, p, &parse_list(arg(6)?)?).await
        }
        "sweep" => {
            let dir = PathBuf::from(arg(1)?);
            let chains: usize = arg(2)?.parse()?;
            sweep(&dir, chains, &parse_list(arg(3)?)?, &parse_list(arg(4)?)?, &parse_list(arg(5)?)?).await
        }
        other => bail!("unknown command {other}"),
    }
}
