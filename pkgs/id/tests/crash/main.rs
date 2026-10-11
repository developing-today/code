//! The crash-safety gate for redb (plan step P1.4).
//!
//! `faultfs` is a `redb::StorageBackend` that loses unsynced writes at a simulated power cut,
//! applies one of four survival policies to them, and can tear the write in flight. `workload` is
//! 50 redb write transactions with a model for every prefix. `harness` cuts the power at one
//! storage call, reopens the image, and classifies the result. `sweep` runs the plans on all
//! threads, and `corpus` reads and writes the checked-in replay list.
//!
//! `cargo test` runs the sampled sweep (every `SAMPLE_STRIDE`th storage call), the lying-sync
//! control, and the corpus replay. `just crash-gate` runs the two full tests in release mode,
//! where every storage call is a crash point. A failure found by a sweep goes into `corpus.tsv`
//! with its fix (see `corpus.rs`).

// Offsets and lengths index in-memory disk images, which stay far below 4 GiB, so the u64 to usize
// casts cannot truncate on any target this runs on.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

mod corpus;
mod faultfs;
mod harness;
mod store;
mod sweep;
mod workload;

use corpus::{Expect, Header, Row};
use faultfs::{Kind, Trace};
use harness::{Outcome, Verdict, run_one};
use std::collections::BTreeMap;
use sweep::{Job, describe, jobs_for, report, run_jobs, sample_events};
use workload::{Model, TXNS};

/// The default run crashes at every `SAMPLE_STRIDE`th storage call, and at the last one. At 20 the
/// sampled sweep is about 380 runs, a few seconds in debug.
const SAMPLE_STRIDE: u64 = 20;

/// How many failing runs an assertion message lists.
const LISTED: usize = 20;

const CORPUS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/crash/corpus.tsv");
const CORPUS: &str = include_str!("corpus.tsv");

/// Sweeps the given crash points of one trace. Returns the jobs, their outcomes, and the indices of
/// the failing runs.
fn sweep_points(
    label: &str,
    trace: &Trace,
    events: &[u64],
    lying: bool,
    models: &[Model],
) -> (Vec<Job>, Vec<Outcome>, Vec<usize>) {
    let jobs = jobs_for(trace, events, lying);
    let outcomes = run_jobs(&jobs, models);
    let failing = report(label, &jobs, &outcomes);
    (jobs, outcomes, failing)
}

/// The first failing runs, one per line, for an assertion message.
fn listing(jobs: &[Job], outcomes: &[Outcome], failing: &[usize]) -> String {
    failing
        .iter()
        .take(LISTED)
        .map(|&i| describe(&jobs[i], &outcomes[i]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A lying `sync_data` must lose committed data somewhere, or the oracle cannot see durability.
fn assert_control_fails(outcomes: &[Outcome], failing: &[usize]) {
    assert!(
        !failing.is_empty(),
        "the lying-sync control produced no failures: the oracle cannot see lost durability"
    );
    assert!(
        failing.iter().any(|&i| matches!(
            outcomes[i].verdict,
            Verdict::LostCommitted | Verdict::OpenError
        )),
        "the control failed, but never with lost_committed or open_error"
    );
}

#[test]
fn clean_run_reopens_with_every_transaction() {
    let models = workload::models();
    let (outcome, trace) = run_one(None, false, &models);
    assert_eq!(outcome.verdict, Verdict::OkAcked, "{}", outcome.detail);
    assert_eq!(outcome.acked, TXNS);
    assert!(!trace.is_empty());
}

#[test]
fn sampled_sweep_has_no_failures() {
    let models = workload::models();
    let (_, trace) = run_one(None, false, &models);
    let events = sample_events(trace.len() as u64, SAMPLE_STRIDE);
    let (jobs, outcomes, failing) = sweep_points("sampled sweep", &trace, &events, false, &models);
    assert!(
        failing.is_empty(),
        "{} failing runs:\n{}",
        failing.len(),
        listing(&jobs, &outcomes, &failing)
    );
}

#[test]
fn lying_sync_control_produces_failures() {
    let models = workload::models();
    let (_, trace) = run_one(None, true, &models);
    let events = sample_events(trace.len() as u64, SAMPLE_STRIDE);
    let (_, outcomes, failing) =
        sweep_points("control (lying sync)", &trace, &events, true, &models);
    assert_control_fails(&outcomes, &failing);
}

#[test]
fn corpus_replays_with_recorded_verdicts() {
    let models = workload::models();
    let (_, trace) = run_one(None, false, &models);
    let (header, rows) = corpus::parse(CORPUS).expect("corpus.tsv parses");
    assert_eq!(
        header,
        Header {
            storage_calls: trace.len() as u64,
            trace_fnv1a: corpus::trace_fnv1a(&trace),
        },
        "the storage-call trace changed. Check the change, then regenerate corpus.tsv (see its header)"
    );
    let mut problems = Vec::new();
    for row in &rows {
        let actual = (row.job.plan.event as usize)
            .checked_sub(1)
            .and_then(|i| trace.get(i))
            .map(|call| call.0);
        if actual != Some(row.kind) {
            let actual = actual.map_or("no such call", Kind::name);
            problems.push(format!(
                "{}: event {} is {actual}, not {}",
                row.name,
                row.job.plan.event,
                row.kind.name()
            ));
        }
    }
    let jobs: Vec<Job> = rows.iter().map(|row| row.job).collect();
    let outcomes = run_jobs(&jobs, &models);
    for (row, outcome) in rows.iter().zip(&outcomes) {
        if !row.expect.accepts(outcome.verdict) {
            problems.push(format!(
                "{}: got {} ({}), expected {}",
                row.name,
                outcome.verdict.name(),
                outcome.detail,
                row.expect.name()
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} corpus rows changed:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
#[ignore = "the full gate: every storage call under every plan. `just crash-gate` runs it in release"]
fn full_sweep_has_no_failures() {
    let models = workload::models();
    let (_, trace) = run_one(None, false, &models);
    let events: Vec<u64> = (1..=trace.len() as u64).collect();
    let (jobs, outcomes, failing) = sweep_points("full sweep", &trace, &events, false, &models);
    assert!(
        failing.is_empty(),
        "{} failing runs:\n{}",
        failing.len(),
        listing(&jobs, &outcomes, &failing)
    );
}

#[test]
#[ignore = "the full control: every storage call under a lying sync. `just crash-gate` runs it in release"]
fn full_lying_sync_control_produces_failures() {
    let models = workload::models();
    let (_, trace) = run_one(None, true, &models);
    let events: Vec<u64> = (1..=trace.len() as u64).collect();
    let (_, outcomes, failing) =
        sweep_points("full control (lying sync)", &trace, &events, true, &models);
    assert_control_fails(&outcomes, &failing);
}

/// Rewrites `corpus.tsv`: every faithful point that ends in a pre-commit open error or an
/// acknowledged-plus-one state, two of each failing class from the control, and the `fix-` rows
/// already in the file. A faithful failure stops it, because such a failure needs a fix first.
#[test]
#[ignore = "rewrites corpus.tsv. Run by hand, in release, after a deliberate workload or redb change"]
fn write_corpus() {
    let models = workload::models();
    let (_, trace) = run_one(None, false, &models);
    let all: Vec<u64> = (1..=trace.len() as u64).collect();

    let (jobs, outcomes, failing) = sweep_points("faithful sweep", &trace, &all, false, &models);
    assert!(
        failing.is_empty(),
        "a faithful run failed. Fix it, then add its row by hand as fix-<commit>:\n{}",
        listing(&jobs, &outcomes, &failing)
    );
    let mut rows = Vec::new();
    for (job, outcome) in jobs.iter().zip(&outcomes) {
        let (prefix, expect) = match outcome.verdict {
            Verdict::OpenErrorPreCommit => ("faithful-pre-commit", Expect::Exact(outcome.verdict)),
            Verdict::OkAckedPlusOne => ("faithful-plus-one", Expect::Valid),
            _ => continue,
        };
        rows.push(corpus_row(plan_name(prefix, job), *job, &trace, expect));
    }

    let (cjobs, coutcomes, _) = sweep_points("control sweep", &trace, &all, true, &models);
    let mut taken: BTreeMap<&str, usize> = BTreeMap::new();
    for (job, outcome) in cjobs.iter().zip(&coutcomes) {
        let v = outcome.verdict;
        // The failing classes must match exactly. An acknowledged-plus-one control point may differ
        // between debug and release (crash-tests/RESULTS.md, E9 in round 2), so it accepts either
        // valid prefix.
        let expect = match v {
            Verdict::LostCommitted | Verdict::OpenError | Verdict::Panic => Expect::Exact(v),
            Verdict::OkAckedPlusOne => Expect::Valid,
            _ => continue,
        };
        let count = taken.entry(v.name()).or_default();
        if *count < 2 {
            *count += 1;
            let name = format!("control-{}-{count}", v.name());
            rows.push(corpus_row(name, *job, &trace, expect));
        }
    }

    let existing = std::fs::read_to_string(CORPUS_PATH).expect("read corpus.tsv");
    let (_, kept) = corpus::parse(&existing).expect("corpus.tsv parses");
    rows.extend(kept.into_iter().filter(|row| row.name.starts_with("fix-")));

    let header = Header {
        storage_calls: trace.len() as u64,
        trace_fnv1a: corpus::trace_fnv1a(&trace),
    };
    std::fs::write(CORPUS_PATH, corpus::format(&header, &rows)).expect("write corpus.tsv");
    println!("wrote {} rows to {CORPUS_PATH}", rows.len());
}

fn plan_name(prefix: &str, job: &Job) -> String {
    let p = job.plan;
    format!(
        "{prefix}-{:03}-{}-{}-{}",
        p.event,
        p.survive.name(),
        p.tear.name(),
        u8::from(p.sync_completed)
    )
}

fn corpus_row(name: String, job: Job, trace: &Trace, expect: Expect) -> Row {
    Row {
        name,
        job,
        kind: trace[job.plan.event as usize - 1].0,
        expect,
    }
}
