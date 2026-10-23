//! Kill test. A child process commits 1,000-claim batches in a loop and acknowledges
//! each one on stdout. The parent sends SIGKILL at a random time, reopens the store,
//! and checks that every batch is wholly present or wholly absent, that the rows match
//! the content each seq must hold, and that the count fits the acknowledgements.

use crate::keys::{Claim, Id};
use crate::store::Store;
use crate::{Rng, redb_store::RedbStore, sqlite_store::SqliteStore, turso_store::{TursoRowidStore, TursoStore}};
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const BATCH: u64 = 1000;

fn author() -> Id {
    *blake3::hash(b"s6-durability-author").as_bytes()
}

/// The claim at `seq`. Each claim is a pure function of its seq, so the parent can
/// recompute what every surviving claim must look like.
pub fn expected_claim(seq: u64) -> Claim {
    Claim {
        author: author(),
        seq,
        subject: *blake3::hash(&seq.to_be_bytes()).as_bytes(),
        name: format!("durable-{:03}", seq % 17).into_bytes(),
        value: Some(format!("value-{seq:012}").into_bytes()),
    }
}

/// Batch `b` holds seqs `b*1000+1 ..= b*1000+1000`.
fn batch_claims(b: u64) -> Vec<Claim> {
    (b * BATCH + 1..=(b + 1) * BATCH).map(expected_claim).collect()
}

/// Child: commit batches until killed, printing `committed <b>` after each commit.
fn child<S: Store>(path: &Path, max_batches: u64) -> Result<()> {
    let mut store = S::open(path)?;
    for b in 0..max_batches {
        store.insert_batch(&batch_claims(b))?;
        println!("committed {b}");
        std::io::stdout().flush()?;
    }
    Ok(())
}

struct Verdict {
    acked: u64,
    present_batches: u64,
    present_claims: u64,
    partial_batches: u64,
    mismatched_claims: u64,
    gap: bool,
    index_rows_mismatch: bool,
}

impl Verdict {
    fn ok(&self) -> bool {
        self.partial_batches == 0
            && self.mismatched_claims == 0
            && !self.gap
            && !self.index_rows_mismatch
            && self.acked <= self.present_batches
            && self.present_batches <= self.acked + 1
    }
}

/// Reopens the store and checks it against the acknowledged batch count.
fn verify<S: Store>(path: &Path, acked: u64) -> Result<Verdict> {
    let store = S::open(path)?;
    let claims = store.all_claims()?;
    let counts = store.counts()?;
    let mut per_batch: BTreeMap<u64, u64> = BTreeMap::new();
    let mut mismatched = 0u64;
    for c in &claims {
        ensure!(c.seq >= 1, "seq 0 found in the store");
        *per_batch.entry((c.seq - 1) / BATCH).or_default() += 1;
        if *c != expected_claim(c.seq) {
            mismatched += 1;
        }
    }
    let total = claims.len() as u64;
    let present_batches = per_batch.len() as u64;
    Ok(Verdict {
        acked,
        present_batches,
        present_claims: total,
        partial_batches: per_batch.values().filter(|&&n| n != BATCH).count() as u64,
        mismatched_claims: mismatched,
        gap: !per_batch.keys().copied().eq(0..present_batches),
        index_rows_mismatch: counts.iter().any(|&n| n != total),
    })
}

fn test<S: Store>(trials: usize, dir: &Path) -> Result<()> {
    let mut rng = Rng::new(0xD0_0D_5EED);
    let mut totals = [0u64; 6]; // acked, present batches, partial, mismatched, ==acked, ==acked+1
    let mut failed = 0usize;
    for t in 0..trials {
        let tdir = dir.join(format!("trial-{t:02}"));
        let _ = std::fs::remove_dir_all(&tdir);
        std::fs::create_dir_all(&tdir)?;
        let path = tdir.join(format!("index-{}.db", S::NAME));
        let mut child = Command::new(std::env::current_exe()?)
            .args(["durable-child", S::NAME, path.to_str().context("non-UTF-8 path")?, "1000000"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().context("child has no stdout")?;
        let reader = std::thread::spawn(move || -> Vec<u64> {
            BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .filter_map(|l| l.strip_prefix("committed ")?.parse().ok())
                .collect()
        });
        let delay_ms = 200 + rng.below(2800);
        std::thread::sleep(Duration::from_millis(delay_ms));
        child.kill()?; // SIGKILL on Unix
        let status = child.wait()?;
        let committed = reader.join().map_err(|_| anyhow!("reader thread panicked"))?;
        let acked = committed.last().map_or(0, |b| b + 1);
        let v = verify::<S>(&path, acked)?;
        let ok = v.ok();
        println!(
            "trial {t:02}: delay_ms={delay_ms} exit=\"{status}\" acked_batches={} present_batches={} present_claims={} partial={} mismatched={} gap={} index_rows_mismatch={} ok={ok}",
            v.acked, v.present_batches, v.present_claims, v.partial_batches, v.mismatched_claims, v.gap, v.index_rows_mismatch
        );
        if !ok {
            failed += 1;
        }
        totals[0] += v.acked;
        totals[1] += v.present_batches;
        totals[2] += v.partial_batches;
        totals[3] += v.mismatched_claims;
        totals[4] += u64::from(v.present_batches == v.acked);
        totals[5] += u64::from(v.present_batches == v.acked + 1);
    }
    println!(
        "summary store={} trials={trials} failed={failed} acked_batches={} present_batches={} partial_batches={} mismatched_claims={} present_eq_acked={} present_eq_acked_plus_1={}",
        S::NAME, totals[0], totals[1], totals[2], totals[3], totals[4], totals[5]
    );
    ensure!(failed == 0, "{failed} trials failed");
    Ok(())
}

pub fn child_for(kind: &str, path: &Path, max_batches: u64) -> Result<()> {
    match kind {
        "redb" => child::<RedbStore>(path, max_batches),
        "sqlite" => child::<SqliteStore>(path, max_batches),
        "turso" => child::<TursoStore>(path, max_batches),
        "turso-rowid" => child::<TursoRowidStore>(path, max_batches),
        other => bail!("unknown store {other}"),
    }
}

pub fn test_for(kind: &str, trials: usize, dir: &Path) -> Result<()> {
    match kind {
        "redb" => test::<RedbStore>(trials, dir),
        "sqlite" => test::<SqliteStore>(trials, dir),
        "turso" => test::<TursoStore>(trials, dir),
        "turso-rowid" => test::<TursoRowidStore>(trials, dir),
        other => bail!("unknown store {other}"),
    }
}
