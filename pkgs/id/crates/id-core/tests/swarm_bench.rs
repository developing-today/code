//! Benchmark-fixture driver for the #2261 swarm scheduler PoC.
//!
//! Drives [`id_core::swarm::Scheduler`] with fixture holders (no
//! network): each holder has a per-request latency and a set of
//! groups it actually holds; the driver issues the scheduler's
//! assignments in request waves of at most [`REQUEST_GROUPS`]
//! groups, holders in parallel, and feeds outcomes back as
//! [`SchedEvent`]s. Simulated wall time for a wave is the slowest
//! holder's request count times its latency. The matrix is the
//! one issue #2261 requires — 1 vs 2 vs 4 complete holders, a
//! partial-holder mix, one holder leaving mid-transfer, and
//! optimistic (observe-off) mode — reported as simulated wall
//! time plus duplicated bytes against the single-source baseline.
//!
//! These are fixture measurements of the scheduling policy, not
//! live-network benchmarks: they show the policy's parallelism
//! and overhead shape. Live multi-holder timings remain a
//! follow-up recorded on #2261.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id_core::holders::HeldRanges;
use id_core::swarm::{ByteRange, CHUNK_GROUP_BYTES, SchedEvent, Scheduler};

/// Groups per fetch request in the fixture (the fetch loop chunks
/// a holder's assignment into requests of this size).
const REQUEST_GROUPS: u64 = 8;
/// Blob used across the matrix: 64 groups = 1 MiB.
const GROUPS: u64 = 64;
/// Per-request latency of a fixture holder, milliseconds.
const LATENCY_MS: u64 = 25;

struct Fixture {
    /// Groups the holder actually has (its claim may be optimistic).
    actual: Vec<bool>,
    /// The holder leaves after delivering this many groups in
    /// total; its remaining in-flight groups are stranded and must
    /// be requeued. `None` = stays for the whole fetch.
    dies_after_deliveries: Option<u64>,
    /// Groups delivered so far.
    delivered_total: u64,
}

struct Run {
    waves: u64,
    simulated_ms: u64,
    duplicated_bytes: u64,
    delivered: Vec<u64>,
    complete: bool,
    provenance_holders: usize,
}

fn group_range(blob_size: u64, g: u64) -> ByteRange {
    let start = g * CHUNK_GROUP_BYTES;
    ByteRange::new(start, (start + CHUNK_GROUP_BYTES).min(blob_size))
}

fn groups_in(range: ByteRange) -> Vec<u64> {
    (range.start / CHUNK_GROUP_BYTES..range.end.div_ceil(CHUNK_GROUP_BYTES)).collect()
}

fn provenance_holders(sched: &Scheduler) -> usize {
    sched
        .provenance
        .iter()
        .filter_map(|line| line.rsplit(" <- holder ").next())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn drive(holders: Vec<(String, HeldRanges)>, mut fixtures: Vec<Fixture>) -> Run {
    let blob_size = GROUPS * CHUNK_GROUP_BYTES;
    let mut sched = Scheduler::new(blob_size, holders);
    let mut delivered_counts: BTreeMap<u64, u64> = BTreeMap::new();
    let mut duplicated_bytes = 0u64;
    let mut simulated_ms = 0u64;
    let mut waves = 0u64;

    loop {
        if sched.is_complete() {
            break;
        }
        let assignments = sched.assignments();
        if assignments.is_empty() {
            break; // stalled: uncovered ranges remain (nobody-home)
        }
        waves += 1;
        // Wave time: each holder runs its requests sequentially,
        // holders run in parallel — the wave costs the slowest
        // holder's request count times its latency.
        let mut wave_ms = 0u64;
        let mut events: Vec<SchedEvent> = Vec::new();
        for a in &assignments {
            let groups: Vec<u64> = a.ranges.iter().flat_map(|r| groups_in(*r)).collect();
            let requests = (groups.len() as u64).div_ceil(REQUEST_GROUPS);
            wave_ms = wave_ms.max(requests.max(1) * LATENCY_MS);
            for &g in &groups {
                let range = group_range(blob_size, g);
                let fx = &mut fixtures[a.holder];
                let dead = fx
                    .dies_after_deliveries
                    .is_some_and(|cap| fx.delivered_total >= cap);
                if dead || !fx.actual[g as usize] {
                    // A holder that left, and a holder reporting a
                    // range it lacks, both surface to the scheduler
                    // as a range it cannot serve: the group requeues
                    // elsewhere.
                    events.push(SchedEvent::Missing {
                        holder: a.holder,
                        range,
                    });
                } else {
                    fx.delivered_total += 1;
                    let count = delivered_counts.entry(g).or_insert(0);
                    *count += 1;
                    if *count > 1 {
                        duplicated_bytes += range.end - range.start;
                    }
                    events.push(SchedEvent::Verified {
                        holder: a.holder,
                        range,
                    });
                }
            }
        }
        simulated_ms += wave_ms;
        for ev in events {
            sched.apply(ev);
        }
        assert!(waves < 10_000, "driver failed to converge");
    }

    Run {
        waves,
        simulated_ms,
        duplicated_bytes,
        delivered: delivered_counts.keys().copied().collect(),
        complete: sched.is_complete(),
        provenance_holders: provenance_holders(&sched),
    }
}

/// Endgame scenario, driven by hand because the wave driver
/// applies each wave's outcomes atomically: with every group in
/// flight, the scheduler duplicate-requests the trailing ranges
/// from a second holder; both copies are delivered and the second
/// verified copy is counted as duplicated bytes (first copy wins).
fn drive_endgame() -> Run {
    let blob_size = 4 * CHUNK_GROUP_BYTES;
    let mut sched = Scheduler::new(
        blob_size,
        vec![
            ("h1".to_string(), HeldRanges::Complete),
            ("h2".to_string(), HeldRanges::Complete),
        ],
    );
    let first = sched.assignments();
    assert!(!first.is_empty());
    let dup = sched.assignments(); // endgame: duplicates of in-flight
    let mut duplicated_bytes = 0u64;
    let mut delivered: Vec<u64> = Vec::new();
    for a in first.iter().chain(dup.iter()) {
        for r in &a.ranges {
            for g in groups_in(*r) {
                if delivered.contains(&g) {
                    let gr = group_range(blob_size, g);
                    duplicated_bytes += gr.end - gr.start;
                } else {
                    delivered.push(g);
                }
                sched.apply(SchedEvent::Verified {
                    holder: a.holder,
                    range: *r,
                });
            }
        }
    }
    Run {
        waves: 2,
        simulated_ms: 2 * LATENCY_MS,
        duplicated_bytes,
        delivered,
        complete: sched.is_complete(),
        provenance_holders: provenance_holders(&sched),
    }
}

fn complete_fixture() -> Fixture {
    Fixture {
        actual: vec![true; GROUPS as usize],
        dies_after_deliveries: None,
        delivered_total: 0,
    }
}

fn partial_fixture(first_half: bool) -> Fixture {
    let mut actual = vec![false; GROUPS as usize];
    let (lo, hi) = if first_half {
        (0, GROUPS as usize / 2)
    } else {
        (GROUPS as usize / 2, GROUPS as usize)
    };
    actual[lo..hi].fill(true);
    Fixture {
        actual,
        dies_after_deliveries: None,
        delivered_total: 0,
    }
}

fn run_matrix() -> Vec<(&'static str, Run)> {
    let blob = GROUPS * CHUNK_GROUP_BYTES;
    let half = GROUPS * CHUNK_GROUP_BYTES / 2;
    vec![
        (
            "1 holder, complete",
            drive(
                vec![("h1".to_string(), HeldRanges::Complete)],
                vec![complete_fixture()],
            ),
        ),
        (
            "2 holders, complete",
            drive(
                vec![
                    ("h1".to_string(), HeldRanges::Complete),
                    ("h2".to_string(), HeldRanges::Complete),
                ],
                vec![complete_fixture(), complete_fixture()],
            ),
        ),
        (
            "4 holders, complete",
            drive(
                vec![
                    ("h1".to_string(), HeldRanges::Complete),
                    ("h2".to_string(), HeldRanges::Complete),
                    ("h3".to_string(), HeldRanges::Complete),
                    ("h4".to_string(), HeldRanges::Complete),
                ],
                vec![
                    complete_fixture(),
                    complete_fixture(),
                    complete_fixture(),
                    complete_fixture(),
                ],
            ),
        ),
        (
            "2 holders, disjoint 50% each",
            drive(
                vec![
                    ("h1".to_string(), HeldRanges::Ranges(vec![(0, half)])),
                    ("h2".to_string(), HeldRanges::Ranges(vec![(half, blob)])),
                ],
                vec![partial_fixture(true), partial_fixture(false)],
            ),
        ),
        (
            "2 holders, one leaves mid-transfer",
            drive(
                vec![
                    ("h1".to_string(), HeldRanges::Complete),
                    ("h2".to_string(), HeldRanges::Complete),
                ],
                vec![
                    complete_fixture(),
                    Fixture {
                        actual: vec![true; GROUPS as usize],
                        dies_after_deliveries: Some(8),
                        delivered_total: 0,
                    },
                ],
            ),
        ),
        (
            "2 holders, optimistic (observe off)",
            drive(
                vec![
                    ("h1".to_string(), HeldRanges::Unknown),
                    ("h2".to_string(), HeldRanges::Unknown),
                ],
                vec![partial_fixture(true), complete_fixture()],
            ),
        ),
        ("endgame duplication (4 groups, 2 holders)", drive_endgame()),
    ]
}

/// Oracle for the benchmark matrix: every scenario completes, the
/// multi-holder scenarios draw from more than one holder
/// (provenance), and four holders beat the single-source baseline
/// on simulated wall time. The printed table is the #2261
/// benchmark report.
#[test]
fn swarm_bench_matrix() {
    let results = run_matrix();
    let baseline = results[0].1.simulated_ms;
    println!(
        "\n| scenario | waves | simulated ms | vs baseline | duplicated bytes | holders used |"
    );
    println!("|---|---|---|---|---|---|");
    for (name, run) in &results {
        assert!(run.complete, "{name} did not complete");
        let vs = if baseline > 0 && run.simulated_ms > 0 {
            format!("{:.2}x", baseline as f64 / run.simulated_ms as f64)
        } else {
            "-".to_string()
        };
        println!(
            "| {name} | {} | {} | {vs} | {} | {} |",
            run.waves, run.simulated_ms, run.duplicated_bytes, run.provenance_holders
        );
    }
    assert_eq!(results[0].1.delivered.len() as u64, GROUPS);
    let four = &results[2].1;
    assert!(
        four.simulated_ms < baseline,
        "4 holders ({}) must beat 1 holder ({baseline})",
        four.simulated_ms
    );
    assert!(
        results[1].1.provenance_holders >= 2,
        "2-holder fetch must draw from both holders"
    );
    assert!(
        results[3].1.provenance_holders >= 2,
        "partial mix must draw from both holders"
    );
    assert!(
        results[6].1.duplicated_bytes > 0,
        "endgame must duplicate trailing ranges"
    );
}
