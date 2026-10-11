//! Sweeps: which crash points a run covers, how the runs are spread over threads, and how a sweep
//! is reported.
//!
//! Every storage call of the workload is a crash point. Each crash point runs under every survival
//! policy, and writes also run under every tear mode.

use crate::faultfs::{Kind, Plan, Survive, Tear, Trace, splitmix};
use crate::harness::{Outcome, run_one};
use crate::workload::Model;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

const SURVIVE: [Survive; 4] = [
    Survive::Drop,
    Survive::Keep,
    Survive::Random,
    Survive::Reorder,
];
const TEAR: [Tear; 3] = [Tear::Off, Tear::Byte, Tear::Sector];

/// One run of the workload: a plan, and whether `sync_data` lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    pub plan: Plan,
    pub lying: bool,
}

/// Every plan for one crash point. A write gets each survival policy with each tear mode (12).
/// A `set_len` gets each survival policy (4). A sync gets each survival policy with the sync
/// lost, plus one run where the sync completed and power was lost right after it (5).
pub fn plans_for(event: u64, kind: Kind) -> Vec<Plan> {
    let mut out = Vec::new();
    let mut variant = 0u64;
    let mut push = |survive, tear, sync_completed| {
        variant += 1;
        let seed = splitmix(event * 64 + variant);
        out.push(Plan {
            event,
            survive,
            tear,
            sync_completed,
            seed,
        });
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

/// The jobs for the given crash points (1-based storage-call numbers), in order.
pub fn jobs_for(trace: &Trace, events: &[u64], lying: bool) -> Vec<Job> {
    events
        .iter()
        .flat_map(|&event| {
            let kind = trace[event as usize - 1].0;
            plans_for(event, kind)
                .into_iter()
                .map(move |plan| Job { plan, lying })
        })
        .collect()
}

/// Every `stride`th crash point, starting with the first, plus the last one (the final close).
pub fn sample_events(total: u64, stride: u64) -> Vec<u64> {
    (1..=total)
        .filter(|e| (e - 1) % stride == 0 || *e == total)
        .collect()
}

/// Runs the jobs of the event workload on all available threads.
pub fn run_jobs(jobs: &[Job], models: &[Model]) -> Vec<Outcome> {
    run_jobs_with(jobs, |job| run_one(Some(job.plan), job.lying, models).0)
}

/// Runs the jobs on all available threads, each one with `run`. The result is in the same order as
/// `jobs`.
pub fn run_jobs_with<F>(jobs: &[Job], run: F) -> Vec<Outcome>
where
    F: Fn(&Job) -> Outcome + Sync,
{
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map_or(4, usize::from)
        .min(jobs.len().max(1));
    let mut slots: Vec<Option<Outcome>> = (0..jobs.len()).map(|_| None).collect();
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(i) else { break };
                        done.push((i, run(job)));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            for (i, outcome) in worker.join().expect("sweep worker panicked") {
                slots[i] = Some(outcome);
            }
        }
    });
    slots
        .into_iter()
        .map(|o| o.expect("every job ran"))
        .collect()
}

/// Prints the verdict counts for one sweep and returns the indices of its failing runs.
pub fn report(label: &str, jobs: &[Job], outcomes: &[Outcome]) -> Vec<usize> {
    let mut verdicts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut points = BTreeSet::new();
    let mut failing = Vec::new();
    let mut repaired = 0;
    for (i, (job, outcome)) in jobs.iter().zip(outcomes).enumerate() {
        *verdicts.entry(outcome.verdict.name()).or_default() += 1;
        points.insert(job.plan.event);
        if outcome.verdict.is_failure() {
            failing.push(i);
        }
        if outcome.repairs > 0 {
            repaired += 1;
        }
    }
    println!(
        "{label}: {} crash points, {} runs",
        points.len(),
        jobs.len()
    );
    println!("{label}: verdicts {verdicts:?}");
    println!("{label}: reopens that ran redb repair: {repaired} runs");
    println!("{label}: failing runs {}", failing.len());
    failing
}

/// One line per run, for a failure message: where power was lost, and what the run found.
pub fn describe(job: &Job, outcome: &Outcome) -> String {
    let p = job.plan;
    format!(
        "event={} lying={} survive={} tear={} sync_completed={} seed={} verdict={} acked={} {}",
        p.event,
        u8::from(job.lying),
        p.survive.name(),
        p.tear.name(),
        u8::from(p.sync_completed),
        p.seed,
        outcome.verdict.name(),
        outcome.acked,
        outcome.detail
    )
}
