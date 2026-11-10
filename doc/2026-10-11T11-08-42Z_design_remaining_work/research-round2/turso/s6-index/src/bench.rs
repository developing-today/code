//! Load test. N claims go in as 1,000-claim transactions. The store is then closed and
//! reopened, and 1,000 random lookups of each query shape are timed. Every lookup is
//! checked against answers computed in memory from the same dataset.

use crate::keys::{Claim, Id};
use crate::store::Store;
use crate::{Rng, status_kb};
use anyhow::{Result, ensure};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

const BATCH: usize = 1000;
const LOOKUPS: usize = 1000;
const SUBJECTS: usize = 50_000;
const AUTHORS: usize = 100;
const NAMES: usize = 200;
const VALUES: usize = 20_000;
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// `n` claims: 32-byte subjects, 8 to 12 byte names, 16 to 24 byte values, 5% null values.
pub fn dataset(n: usize, seed: u64) -> Vec<Claim> {
    let mut rng = Rng::new(seed);
    let authors: Vec<Id> = (0..AUTHORS).map(|_| rng.id()).collect();
    let subjects: Vec<Id> = (0..SUBJECTS).map(|_| rng.id()).collect();
    let names: Vec<Vec<u8>> = (0..NAMES)
        .map(|_| {
            let len = 8 + rng.below(5) as usize;
            rng.bytes(len, LOWER)
        })
        .collect();
    let values: Vec<Vec<u8>> = (0..VALUES)
        .map(|_| {
            let len = 16 + rng.below(9) as usize;
            rng.bytes(len, ALNUM)
        })
        .collect();
    let mut next_seq = [1u64; AUTHORS];
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let a = rng.below(AUTHORS as u64) as usize;
        let seq = next_seq[a];
        next_seq[a] += 1;
        let subject = subjects[rng.below(SUBJECTS as u64) as usize];
        let name = names[rng.below(NAMES as u64) as usize].clone();
        let value = if rng.below(100) < 5 {
            None
        } else {
            Some(values[rng.below(VALUES as u64) as usize].clone())
        };
        out.push(Claim { author: authors[a], seq, subject, name, value });
    }
    out
}

/// Nearest-rank percentile of an ascending list.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn wal_len(path: &Path) -> u64 {
    std::fs::metadata(format!("{}-wal", path.display())).map(|m| m.len()).unwrap_or(0)
}

pub fn run<S: Store>(dir: &Path, n: usize) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("index-{}.db", S::NAME));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }

    let claims = dataset(n, 0x5_0006);
    let peak_after_gen_kb = status_kb("VmHWM:");

    // Load: one transaction per 1,000 claims.
    let mut store = S::open(&path)?;
    let mut batch_times = Vec::with_capacity(n / BATCH);
    let mut inserted = 0usize;
    let t0 = Instant::now();
    for batch in claims.chunks(BATCH) {
        let t = Instant::now();
        inserted += store.insert_batch(batch)?.inserted;
        batch_times.push(t.elapsed());
    }
    let insert_total = t0.elapsed();
    ensure!(inserted == n, "inserted {inserted} of {n}");
    let counts_loaded = store.counts()?;
    let wal_before_close = wal_len(&path);
    drop(store);
    let db_bytes = std::fs::metadata(&path)?.len();
    let peak_after_load_kb = status_kb("VmHWM:");

    // Reopen, then query. Each lookup is timed on its own.
    let store = S::open(&path)?;
    let mut expect_subject: HashMap<Id, usize> = HashMap::new();
    let mut expect_author: HashMap<Id, usize> = HashMap::new();
    let mut expect_pair: HashMap<(Vec<u8>, Vec<u8>), Vec<Id>> = HashMap::new();
    for c in &claims {
        *expect_subject.entry(c.subject).or_default() += 1;
        *expect_author.entry(c.author).or_default() += 1;
        if let Some(v) = &c.value {
            expect_pair.entry((c.name.clone(), v.clone())).or_default().push(c.subject);
        }
    }
    for subjects in expect_pair.values_mut() {
        subjects.sort();
    }
    let with_value: Vec<&Claim> = claims.iter().filter(|c| c.value.is_some()).collect();
    let mut authors: Vec<Id> = expect_author.keys().copied().collect();
    authors.sort();

    let mut rng = Rng::new(0x1_0000);
    let mut lat_subject = Vec::with_capacity(LOOKUPS);
    let mut lat_pair = Vec::with_capacity(LOOKUPS);
    let mut lat_author = Vec::with_capacity(LOOKUPS);
    let (mut hits_subject, mut hits_pair, mut hits_author) = (0usize, 0usize, 0usize);
    for _ in 0..LOOKUPS {
        let subject = claims[rng.below(n as u64) as usize].subject;
        let t = Instant::now();
        let got = store.by_subject(&subject)?;
        lat_subject.push(t.elapsed());
        ensure!(got.len() == expect_subject[&subject], "by_subject: {} rows", got.len());
        hits_subject += got.len();
    }
    for _ in 0..LOOKUPS {
        let c = with_value[rng.below(with_value.len() as u64) as usize];
        let name = c.name.clone();
        let value = c.value.clone().unwrap_or_default();
        let t = Instant::now();
        let mut got = store.subjects_with(&name, &value)?;
        lat_pair.push(t.elapsed());
        got.sort();
        let want = &expect_pair[&(name, value)];
        ensure!(&got == want, "subjects_with: {} rows, expected {}", got.len(), want.len());
        hits_pair += got.len();
    }
    for _ in 0..LOOKUPS {
        let author = authors[rng.below(authors.len() as u64) as usize];
        let t = Instant::now();
        let got = store.by_author(&author)?;
        lat_author.push(t.elapsed());
        ensure!(got.len() == expect_author[&author], "by_author: {} rows", got.len());
        ensure!(got.windows(2).all(|w| w[0].seq < w[1].seq), "by_author out of seq order");
        hits_author += got.len();
    }
    let peak_end_kb = status_kb("VmHWM:");

    let report = |label: &str, mut v: Vec<Duration>, hits: usize| {
        v.sort();
        println!(
            "lookup={label} n={LOOKUPS} p50_us={:.1} p99_us={:.1} max_us={:.1} mean_results={:.1}",
            us(percentile(&v, 50.0)),
            us(percentile(&v, 99.0)),
            us(*v.last().expect("lookups ran")),
            hits as f64 / LOOKUPS as f64
        );
    };
    println!("store={} n={n} batch={BATCH} txns={}", S::NAME, n / BATCH);
    batch_times.sort();
    println!(
        "insert_total_s={:.3} per_claim_us={:.2} txn_p50_ms={:.2} txn_p99_ms={:.2}",
        insert_total.as_secs_f64(),
        us(insert_total) / n as f64,
        batch_times[batch_times.len() / 2].as_secs_f64() * 1e3,
        batch_times[(batch_times.len() * 99).div_ceil(100) - 1].as_secs_f64() * 1e3,
    );
    println!("counts_after_load={counts_loaded:?}");
    println!("db_bytes_after_close={db_bytes} wal_bytes_before_close={wal_before_close}");
    println!(
        "peak_rss_kb: after_dataset={peak_after_gen_kb} after_load={peak_after_load_kb} end={peak_end_kb}"
    );
    report("subject(alpha prefix)", lat_subject, hits_subject);
    report("name=value(omega prefix)", lat_pair, hits_pair);
    report("author(author_seq prefix)", lat_author, hits_author);
    Ok(())
}
