//! Crash sweep. Every storage call of the 50-transaction workload is a crash point. Each crash
//! point runs under every survival policy, and writes also run under every tear mode.
//!
//! Usage: sweep [--lying-sync] [--threads N] [--out DIR]
//!   --lying-sync  control run: sync_data reports success without making data durable.
//!                 The oracle must fail these runs, or it is not testing durability.

use redb_crash::faultfs::{Kind, Plan, Survive, Tear, splitmix};
use redb_crash::harness::{Outcome, Verdict, run_one};
use redb_crash::workload;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

const SURVIVE: [Survive; 4] = [Survive::Drop, Survive::Keep, Survive::Random, Survive::Reorder];
const TEAR: [Tear; 3] = [Tear::Off, Tear::Byte, Tear::Sector];

fn survive_name(s: Survive) -> &'static str {
    match s {
        Survive::Drop => "Drop",
        Survive::Keep => "Keep",
        Survive::Random => "Random",
        Survive::Reorder => "Reorder",
    }
}

fn tear_name(t: Tear) -> &'static str {
    match t {
        Tear::Off => "Off",
        Tear::Byte => "Byte",
        Tear::Sector => "Sector",
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Write => "write",
        Kind::SetLen => "set_len",
        Kind::Sync => "sync",
    }
}

/// Every plan for one crash point. Writes get each survival policy with each tear mode (12).
/// set_len gets each survival policy (4). A sync gets each survival policy with the sync lost,
/// plus one run where the sync finished and power was lost right after it (5).
fn plans_for(event: u64, kind: Kind) -> Vec<Plan> {
    let mut out = Vec::new();
    let mut variant = 0u64;
    let mut push = |survive, tear, sync_completed| {
        variant += 1;
        let seed = splitmix(event * 64 + variant);
        out.push(Plan { event, survive, tear, sync_completed, seed });
    };
    match kind {
        Kind::Write => {
            for tear in TEAR {
                for survive in SURVIVE {
                    push(survive, tear, false);
                }
            }
        }
        Kind::SetLen => {
            for survive in SURVIVE {
                push(survive, Tear::Off, false);
            }
        }
        Kind::Sync => {
            for survive in SURVIVE {
                push(survive, Tear::Off, false);
            }
            push(Survive::Drop, Tear::Off, true);
        }
    }
    out
}

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    // A panic is a verdict, not a reason to stop the sweep. Keep stderr quiet.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let lying = args.iter().any(|a| a == "--lying-sync");
    let threads: usize = value(&args, "--threads").map_or(4, |v| v.parse().expect("--threads N"));
    let out_dir = PathBuf::from(value(&args, "--out").unwrap_or_else(|| "results".into()));
    let label = if lying { "lying-sync-control" } else { "faithful" };
    std::fs::create_dir_all(&out_dir).expect("create output dir");

    let models = workload::models();
    let (clean, trace) = run_one(None, lying, &models);
    println!(
        "clean run (no cut): verdict={} acked={} storage_calls={}",
        clean.verdict.name(),
        clean.acked,
        trace.len()
    );
    let mut per_kind: BTreeMap<&str, u64> = BTreeMap::new();
    for (k, _, _) in &trace {
        *per_kind.entry(kind_name(*k)).or_default() += 1;
    }
    println!("crash points (one per storage call): {} {:?}", trace.len(), per_kind);

    let jobs: Vec<Plan> = trace
        .iter()
        .enumerate()
        .flat_map(|(i, (k, _, _))| plans_for(i as u64 + 1, *k))
        .collect();
    let chunk = jobs.len().div_ceil(threads.max(1));
    let rows: Vec<(Plan, Outcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                let models = &models;
                s.spawn(move || {
                    part.iter()
                        .map(|p| (*p, run_one(Some(*p), lying, models).0))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread panicked"))
            .collect()
    });

    let kind_of = |event: u64| trace[event as usize - 1].0;
    let mut tsv = String::from(
        "event\tkind\tsurvive\ttear\tsync_completed\tseed\tacked\tverdict\trepairs\tdetail\n",
    );
    for (p, o) in &rows {
        writeln!(
            tsv,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            p.event,
            kind_name(kind_of(p.event)),
            survive_name(p.survive),
            tear_name(p.tear),
            u8::from(p.sync_completed),
            p.seed,
            o.acked,
            o.verdict.name(),
            o.repairs,
            o.detail.replace(['\t', '\n'], " ")
        )
        .unwrap();
    }
    std::fs::write(out_dir.join(format!("sweep-{label}.tsv")), tsv).unwrap();

    let mut by_verdict: BTreeMap<&str, u64> = BTreeMap::new();
    let mut by_kind_verdict: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    let mut repair_runs = 0u64;
    let mut repair_calls = 0u64;
    let mut failing_points: BTreeSet<u64> = BTreeSet::new();
    let mut failures: Vec<&(Plan, Outcome)> = Vec::new();
    for row in &rows {
        let (p, o) = row;
        *by_verdict.entry(o.verdict.name()).or_default() += 1;
        *by_kind_verdict
            .entry((kind_name(kind_of(p.event)), o.verdict.name()))
            .or_default() += 1;
        if o.repairs > 0 {
            repair_runs += 1;
            repair_calls += o.repairs;
        }
        if o.verdict.is_failure() {
            failing_points.insert(p.event);
            failures.push(row);
        }
    }
    println!("runs: {} (control: {})", rows.len(), lying);
    println!("verdicts: {by_verdict:?}");
    for ((k, v), n) in &by_kind_verdict {
        println!("  {k:<8} {v:<22} {n}");
    }
    println!("reopens that ran redb repair: {repair_runs} runs, {repair_calls} callbacks");
    println!(
        "failing runs: {} at {} distinct crash points",
        failures.len(),
        failing_points.len()
    );

    let mut corpus = String::from(
        "# name\tlying_sync\tevent\tsurvive\ttear\tsync_completed\tseed\texpect\n",
    );
    for (i, (p, o)) in failures.iter().map(|r| (&r.0, &r.1)).enumerate() {
        writeln!(
            corpus,
            "{label}-{i:03}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            u8::from(lying),
            p.event,
            survive_name(p.survive),
            tear_name(p.tear),
            u8::from(p.sync_completed),
            p.seed,
            o.verdict.name()
        )
        .unwrap();
    }
    std::fs::write(out_dir.join(format!("failures-{label}.tsv")), corpus).unwrap();
    for (p, o) in failures.iter().take(10) {
        println!(
            "  FAIL event={} ({}) survive={} tear={} sync_completed={} seed={} verdict={} acked={} detail={}",
            p.event,
            kind_name(kind_of(p.event)),
            survive_name(p.survive),
            tear_name(p.tear),
            p.sync_completed,
            p.seed,
            o.verdict.name(),
            o.acked,
            o.detail
        );
    }
    let _ = Verdict::Panic; // keep the public verdict list in scope for readers of this file
}
