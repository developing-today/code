//! F11.8 slice 1: benchmark harness for the pkgs/id world stack (#539).
//!
//! Hand-rolled harness (no criterion): each benchmark prints one stable,
//! machine-parseable line —
//!
//! ```text
//! bench <name> unit=<unit> median=<value> min=<value> reps=<n>
//! ```
//!
//! so slice 3's CI job can diff medians against the committed baseline
//! (slice 2) without parsing prose. `--quick` shrinks every workload to a
//! smoke size; `--only <name>` runs a single benchmark.
//!
//! The workloads and the surface each one measures:
//!
//! - `guest_replay` — replaying N committed events: `WorldCore::restore`
//!   over a journal of N `Committed` chat events, then the restored
//!   `replay` events are pushed through a `WorldProgram` (an in-process
//!   counting guest). The wasm sandbox replay path is environment-gated
//!   (it needs the nix-built host), so this measures the host-side replay
//!   pipeline every guest replay pays for; sandbox trap costs are on top.
//! - `journal_append_fsync` — `FileJournal` append of one entry per call,
//!   each ending in `sync_data`: the durable cost of acknowledging one
//!   world mutation.
//! - `checkpoint_compaction` — `FileJournal::compact` replacing a journal
//!   of N entries with a checkpoint-sized set (header + 64 entries): one
//!   batched encode, one write, one file fsync, one directory fsync.
//! - `session_fanout` — one committed chat event delivered to K session
//!   subscribers through the actor's broadcast channel: spawn a world,
//!   subscribe K receivers, publish, drain all K.
//! - `many_worlds` — 1,000 durable idle worlds plus 100 leased worlds at
//!   20 counter inputs/s each; report command latency percentiles and verify
//!   recent checkpoints plus records/sequence across idle eviction and reopen.
//!   The full-size configuration is parameterizable for profiling runs via
//!   `ID_BENCH_MW_IDLE_WORLDS`, `ID_BENCH_MW_ACTIVE_WORLDS`,
//!   `ID_BENCH_MW_EVENTS_PER_SECOND`, and `ID_BENCH_MW_DURATION_SECS`
//!   (defaults are the committed full size). After the timed load it also
//!   prints one `profile many_worlds ...` line aggregating the world
//!   actors' own metrics (journal-append and command latency histograms,
//!   deepest actor queue depth) across the active worlds, so a run
//!   attributes its latency without external tooling.
//! - `fuel_spin_isolation` — F12.10's exit-criterion measurement (#1067): one
//!   victim world accepts inputs in a calm phase and again while hostile Wasm
//!   worlds spin until their fuel budget is exhausted. Prints each phase's
//!   victim command p99, their ratio, and `target_ratio=1.10`; it measures
//!   the scheduling gap only and implements no scheduler, quota, or
//!   back-pressure. It also prints one `profile fuel_spin_isolation ...`
//!   line attributing each phase's victim latency: the driver's wake
//!   lateness versus the input acknowledgement wait, plus the victim
//!   actor's own journal-append and command latency histograms, so a run
//!   shows where the spinning phase's added latency sits. Since the
//!   admission slice (#1067 slice 2) the profile line also carries the
//!   guest burn budget's counters (bound, in-flight high-water mark,
//!   and the victim key's admissions and waits); setting
//!   `ID_BENCH_GUEST_BURN_CONCURRENCY` runs the workload's guests on a
//!   dedicated budget of that size instead of the shared default, for
//!   sizing experiments.
//! - `stress_gen` — the #6304 scale & performance stress program's
//!   seeded world generator (#13083): materializes a world directory
//!   set for a scale tier (quick smoke, 1x, 10x, 100x — the design's
//!   tier table) plus a `manifest.json` recording the seed, tier,
//!   event mix, per-world event counts, and each measured world's
//!   expected final state hash, so the join/replay/memory slices
//!   (#13084–#13086) measure against a verifiable fixture instead of
//!   generating their own. Generation bulk-writes journal files in
//!   the store's on-disk format (it is a fixture producer, not the
//!   measured append path) and verifies every measured world by
//!   restoring it through `WorldCore::restore` before recording its
//!   hash. Tier, seed, output root, and per-axis sizes are
//!   overridable via `ID_BENCH_STRESS_TIER`, `ID_BENCH_STRESS_SEED`,
//!   `ID_BENCH_STRESS_OUT`, and the `ID_BENCH_STRESS_*` axis knobs.
//!   Generation materializes into a marker-verified staging sibling
//!   of the output root and publishes by rename, so an interrupted
//!   run is recoverable on retry and the output root only ever holds
//!   a complete generation (#13181).
//! - `frame_cost` — the scale program's frame-cost slice (#13085):
//!   view (and layout) fuel and wall time for the Territory-class
//!   guest at the stress tiers' hot-world event counts (10k / 100k /
//!   1M applied events), measured at full state through
//!   `WorldInstance::metrics`. Prints one `profile frame_cost ...`
//!   line per size; `ID_BENCH_FRAME_EVENTS` selects a single size.
//! - `stress_mem` — the #6304 program's memory profile (#13086): RSS
//!   per resident world at the selected tier (the tier's idle,
//!   active, and hot worlds restored through `WorldCore::restore`
//!   and held, with RSS sampled between phases), and the peak
//!   additional RSS while compacting a journal of 2,000 and of
//!   100,000 entries (bulk-written fixture, recovered through
//!   `FileJournal::open`, restored, then replaced with the
//!   compaction bench's checkpoint-sized set). Tier and seed follow
//!   the `stress_gen` knobs; `ID_BENCH_STRESS_MEM_PART` selects
//!   `all` (default), `compact`, or `resident`, so each measurement
//!   can run in a fresh process when attribution matters.
//! - `stress_join` — the #6304 program's join-time profile (#13084):
//!   cold join (open the world store, load the latest checkpoint,
//!   replay the journal tail, produce the first view frame) and
//!   warm join (a joiner's catch-up snapshot projected from the
//!   resident world, plus the live program's first frame), reported
//!   as p50/p95/p99 per tier. Each is measured twice over, per the
//!   design: with the journal at a checkpoint boundary and at the
//!   mid-interval worst case (the largest tail the session
//!   checkpoint cadence — 256 frames or 2,000 ms — allows behind
//!   the latest checkpoint). Worlds come from the `stress_gen`
//!   fixture, generated on demand for the selected tier/seed;
//!   every join verifies against the manifest-verified reference
//!   state, so a fast-but-wrong join fails the run instead of
//!   recording a number. Tier and seed follow the `stress_gen`
//!   knobs. The bench program is an in-process counting guest (see
//!   `guest_replay` for why), and its fixture checkpoints use the
//!   journal's legacy admin-history form — the current form's type
//!   is not constructible outside `id-worlds`.
//! - `bot_battle` (design only, not runnable yet) — a deterministic, mixed-
//!   strategy territory workload for N=2/4/8 users is specified in
//!   `../../../doc/2026-11-06T00-50-21Z_design_dynamic_world_tick_cost/`; its current-
//!   stack proxy must not be reported as a per-user DLL benchmark.
//!
//! Numbers are only comparable on the same machine class; baselines must
//! record where they were taken (slice 2).
//!
//! Benchmark matrix (#5896): the four loadgen-core workloads above are
//! also emitted as a versioned `results.jsonl` bundle — one run-manifest
//! record plus one cell record per (workload × interface × configuration)
//! tuple, per the decision record posted on #5896. The measured interface
//! is the loadgen `in_process` surface; the shell-contract transport IDs
//! (`iroh`, `websocket`, `ssh-exec`) have no adapter in this harness, so
//! their cells are recorded explicitly as `unsupported` with a reason —
//! never silently omitted and never represented as zero. The legacy
//! `bench` lines remain the compatibility view; the JSONL bundle is the
//! source of truth for matrix consumers (#5897's comparator keys on the
//! full matrix identity). The two async workloads (`many_worlds`,
//! `fuel_spin_isolation`) keep their dedicated drivers and legacy lines
//! only for now: their pacing is the loadgen record's later executor
//! steps, so they are not matrix cells yet.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use id::guest_burn::GuestBurnBudget;
use id::sandbox::{GuestCall, Sandbox, SandboxLimits};
use id::world::{
    CapabilityBounds, JoinCapability, JournalAckMode, JournalEntry, Participant, RestoredWorld,
    WorldCore, WorldEvent, WorldEventKind, WorldHandle, WorldJournal, WorldLimits, WorldProgram,
    WorldScopes,
};
use id::world_hub::{HubLimits, OpenFuture, WorldHub, WorldOpener};
use id::world_limits::RuntimeLimits;
use id::world_session::{WorldService, module_hash};
use id::world_store::{FileJournal, open_world};

// The shared workload-generator core (#5891): the synchronous
// workloads below are loadgen workloads (mix table, profile, seeded
// run spec) executed through its scheduler against the InProcess
// surface each one implements as direct core calls. The two async,
// wall-clock-paced workloads (many_worlds, fuel_spin_isolation)
// keep their dedicated drivers in this step — their pacing is the
// open-loop / rate-based executor work of the record's later steps —
// and their output lines are unchanged, as are all lines here.
#[path = "../tests/common/loadgen.rs"]
mod loadgen;

// The stress generator's output lifecycle (#13181): staging,
// identity verification, recovery, and atomic publication.
#[path = "../tests/common/stress_output.rs"]
mod stress_output;

// Scale-program stress workloads (split from this file, #13057).
#[path = "world_bench/stress.rs"]
mod stress;

/// The fixed seed of every bench run on the core: a bench run must
/// be replayable from its spec (#5891, determinism before realism).
const BENCH_SEED: u64 = 0x5891;

const MANY_WORLDS_ADMIN: &str = "f11-many-worlds-benchmark";
const COUNTER_WASM: &[u8] = include_bytes!("../examples/roc-counter/counter.wasm");
const TERRITORY_WASM: &[u8] = include_bytes!("../examples/territory/territory.wasm");

struct DurableBenchOpener {
    root: PathBuf,
    limits: WorldLimits,
    runtime: RuntimeLimits,
    blobs: iroh_blobs::api::Store,
    counter_hash: String,
}

impl WorldOpener for DurableBenchOpener {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a> {
        Box::pin(async move {
            let dir = self.root.join(name);
            if !create && !dir.is_dir() {
                return Ok(None);
            }

            let blobs = self.blobs.clone();
            let (service, _) = open_world(
                &dir,
                name,
                self.limits.clone(),
                self.runtime,
                move |handle| {
                    WorldService::new(handle, Some(MANY_WORLDS_ADMIN.to_owned()))
                        .with_blob_store(blobs)
                },
            )
            .await?;

            // Idle worlds need only durable headers. Active worlds run the
            // checked-in counter guest so inputs, records, checkpoints, and
            // restoration all traverse the real durable-world path.
            if create && name.starts_with("active-") {
                service
                    .install_wasm(
                        Some(MANY_WORLDS_ADMIN),
                        COUNTER_WASM.to_vec(),
                        1,
                        &self.counter_hash,
                    )
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("install many-worlds counter module: {error:?}")
                    })?;
            }

            Ok(Some(service))
        })
    }

    fn exists(&self, name: &str) -> bool {
        self.root.join(name).is_dir()
    }

    fn list(&self) -> Vec<String> {
        std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect()
    }

    fn durable(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct ManyWorldsConfig {
    idle_worlds: usize,
    active_worlds: usize,
    events_per_second: usize,
    duration_secs: u64,
    checkpoint_every: u64,
}

/// One `ID_BENCH_MW_*` profiling override, falling back to the committed
/// full-size value when the variable is unset or unparseable.
fn mw_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Benchmark-only journal ack mode selector (#557): `ID_BENCH_MW_JOURNAL_ACK_MODE`
/// with `sync` (default), `batched`, or `async`. Unset or unparseable falls
/// back to the production default [`JournalAckMode::Sync`] — the same
/// fallback rule as [`mw_env`]. Never changes the production default or
/// durability contracts; the bench alone opts into the relaxed modes.
fn mw_journal_ack_mode() -> JournalAckMode {
    parse_journal_ack_mode(&std::env::var("ID_BENCH_MW_JOURNAL_ACK_MODE").unwrap_or_default())
}

fn parse_journal_ack_mode(value: &str) -> JournalAckMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "batched" => JournalAckMode::Batched,
        "async" => JournalAckMode::Async,
        _ => JournalAckMode::Sync,
    }
}

/// Machine-readable name of a [`JournalAckMode`] for the `bench many_worlds`
/// result line (`journal_ack_mode=<name>`).
fn journal_ack_mode_name(mode: JournalAckMode) -> &'static str {
    match mode {
        JournalAckMode::Sync => "sync",
        JournalAckMode::Batched => "batched",
        JournalAckMode::Async => "async",
    }
}

impl ManyWorldsConfig {
    fn full() -> Self {
        let idle_worlds = mw_env("ID_BENCH_MW_IDLE_WORLDS", 1_000);
        let active_worlds = mw_env("ID_BENCH_MW_ACTIVE_WORLDS", 100);
        let events_per_second = mw_env("ID_BENCH_MW_EVENTS_PER_SECOND", 20);
        let duration_secs = mw_env("ID_BENCH_MW_DURATION_SECS", 10) as u64;
        Self {
            idle_worlds,
            active_worlds,
            events_per_second,
            duration_secs,
            // Keep checkpoint compaction out of the timed 20 ev/s window;
            // one extra input per world below triggers it before eviction.
            // The install event plus the timed inputs leave every world
            // exactly one event short of this cadence.
            checkpoint_every: events_per_second as u64 * duration_secs + 2,
        }
    }

    fn quick() -> Self {
        Self {
            idle_worlds: 10,
            active_worlds: 4,
            events_per_second: 20,
            duration_secs: 1,
            checkpoint_every: 22,
        }
    }
}

struct ActiveWorld {
    name: String,
    lease: id::world_hub::WorldLease,
    token: id::world::JoinCapability,
}

struct ExpectedWorld {
    name: String,
    token: id::world::JoinCapability,
    sequence: u64,
    records: id::world::Records,
}

fn percentile(samples: &[Duration], fraction: f64) -> f64 {
    let mut sorted: Vec<f64> = samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1_000.0)
        .collect();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted[index]
}

/// Merge per-world latency histograms into one. All snapshots share the
/// actor metrics' bucket bounds, so bucket counts, totals, and sums add
/// directly; percentiles read back off the merged buckets are bucket
/// upper bounds (the same approximation `HistogramSnapshot::p95_us`
/// documents), while means from `sum_us / count` are exact.
fn merge_histograms(snapshots: &[id::world::HistogramSnapshot]) -> id::world::HistogramSnapshot {
    let mut merged = id::world::HistogramSnapshot {
        bucket_bounds_us: snapshots
            .first()
            .map(|snapshot| snapshot.bucket_bounds_us.clone())
            .unwrap_or_default(),
        ..Default::default()
    };
    for snapshot in snapshots {
        merged.count += snapshot.count;
        merged.sum_us += snapshot.sum_us;
        if merged.buckets.len() < snapshot.buckets.len() {
            merged.buckets.resize(snapshot.buckets.len(), 0);
        }
        for (index, bucket) in snapshot.buckets.iter().enumerate() {
            merged.buckets[index] += bucket;
        }
    }
    merged
}

/// Per-phase difference of two cumulative latency histograms taken from
/// the same world's metrics: the observations recorded between the two
/// snapshots. Bucket counts, totals, and sums subtract directly because
/// both snapshots share the actor metrics' bucket bounds.
fn delta_histogram(
    after: &id::world::HistogramSnapshot,
    before: &id::world::HistogramSnapshot,
) -> id::world::HistogramSnapshot {
    let mut delta = id::world::HistogramSnapshot {
        bucket_bounds_us: after.bucket_bounds_us.clone(),
        ..Default::default()
    };
    delta.count = after.count.saturating_sub(before.count);
    delta.sum_us = after.sum_us.saturating_sub(before.sum_us);
    delta.buckets = after
        .buckets
        .iter()
        .enumerate()
        .map(|(index, bucket)| {
            bucket.saturating_sub(before.buckets.get(index).copied().unwrap_or(0))
        })
        .collect();
    delta
}

/// Approximate percentile in microseconds from a bucketed histogram:
/// the upper bound of the first bucket holding the percentile
/// observation; the overflow bucket reports `u64::MAX`.
fn histogram_percentile_us(histogram: &id::world::HistogramSnapshot, fraction: f64) -> u64 {
    if histogram.count == 0 {
        return 0;
    }
    let target = (histogram.count as f64 * fraction).ceil() as u64;
    let mut cumulative = 0_u64;
    for (index, bucket) in histogram.buckets.iter().enumerate() {
        cumulative += bucket;
        if cumulative >= target {
            return histogram
                .bucket_bounds_us
                .get(index)
                .copied()
                .unwrap_or(u64::MAX);
        }
    }
    u64::MAX
}

fn histogram_mean_us(histogram: &id::world::HistogramSnapshot) -> u64 {
    histogram.sum_us.checked_div(histogram.count).unwrap_or(0)
}

fn checkpoint_sequences(dir: &Path) -> Result<Vec<u64>> {
    let journal = std::fs::read_to_string(dir.join("journal.jsonl"))?;
    let entries: Vec<JournalEntry> = journal
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| match entry {
            JournalEntry::Checkpoint { sequence, .. } => Some(sequence),
            _ => None,
        })
        .collect())
}

async fn bench_many_worlds(root: &Path, quick: bool) -> Result<()> {
    let config = if quick {
        ManyWorldsConfig::quick()
    } else {
        ManyWorldsConfig::full()
    };
    let mut limits = WorldLimits::default();
    limits.checkpoint_every = config.checkpoint_every;
    limits.journal_ack_mode = mw_journal_ack_mode();
    let ack_mode_name = journal_ack_mode_name(limits.journal_ack_mode);
    let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
    let runtime = RuntimeLimits::default();
    let opener = Arc::new(DurableBenchOpener {
        root: root.to_owned(),
        limits,
        runtime,
        blobs,
        counter_hash: module_hash(COUNTER_WASM),
    });
    let hub = WorldHub::new(
        opener,
        Some(MANY_WORLDS_ADMIN.to_owned()),
        "lobby",
        HubLimits::default(),
    );

    // Materialize 1,000 idle durable worlds on disk while the hub lazily opens
    // and evicts them through its real 256-open-world ceiling.
    for index in 0..config.idle_worlds {
        let name = format!("idle-{index:04}");
        let lease = hub
            .lease(Some(&name), Some(MANY_WORLDS_ADMIN))
            .await
            .map_err(|error| anyhow::anyhow!(error.message()))?;
        drop(lease);
    }
    let initially_evicted = hub.evict_idle(Duration::ZERO).await;
    ensure!(
        hub.open_count() == 0,
        "idle worlds remained open after eviction"
    );

    // Hold 100 leases concurrently. Every active world runs the checked-in
    // counter guest; the idle worlds have durable journals but no resident
    // actor, matching F12.12's lazy-open target.
    let mut active = Vec::with_capacity(config.active_worlds);
    for index in 0..config.active_worlds {
        let name = format!("active-{index:03}");
        let lease = hub
            .lease(Some(&name), Some(MANY_WORLDS_ADMIN))
            .await
            .map_err(|error| anyhow::anyhow!(error.message()))?;
        let (_participant, token) = lease
            .service()
            .world()
            .issue(format!("bench-{index}"), WorldScopes::GUEST)
            .await?;
        active.push(ActiveWorld { name, lease, token });
    }
    ensure!(
        hub.open_count() == config.active_worlds,
        "expected only active worlds to remain open"
    );
    ensure!(
        hub.list().len() == config.idle_worlds + config.active_worlds,
        "durable world inventory does not match the workload"
    );

    // Give each world one input every 50 ms, with a phase offset within that
    // interval. A task per world avoids creating a task for every command and
    // avoids an artificial 100-world burst on each tick. Each sample separates
    // time from deadline to task wake-up from time awaiting input acknowledgement;
    // the end-to-end sample remains deadline through acknowledgement. The two
    // component percentiles are diagnostic and must not be added to estimate a
    // combined percentile.
    let ticks = config.events_per_second as u64 * config.duration_secs;
    let mut pending = tokio::task::JoinSet::new();
    let load_started = Instant::now();
    let interval_nanos = 1_000_000_000 / config.events_per_second as u64;
    for (index, world) in active.iter().enumerate() {
        let handle = world.lease.service().world().clone();
        let token = world.token.clone();
        let phase_nanos = index as u64 * interval_nanos / config.active_worlds as u64;
        pending.spawn(async move {
            let mut samples = Vec::with_capacity(ticks as usize);
            for tick in 0..ticks {
                let deadline =
                    load_started + Duration::from_nanos(tick * interval_nanos + phase_nanos);
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                let input_started = Instant::now();
                let wake_lateness = input_started.saturating_duration_since(deadline);
                handle.input(token.clone(), b"inc".to_vec()).await?;
                samples.push((deadline.elapsed(), wake_lateness, input_started.elapsed()));
            }
            Ok::<Vec<(Duration, Duration, Duration)>, anyhow::Error>(samples)
        });
    }

    let expected_commands = ticks as usize * config.active_worlds;
    let mut latencies = Vec::with_capacity(expected_commands);
    let mut wake_lateness = Vec::with_capacity(expected_commands);
    let mut input_ack = Vec::with_capacity(expected_commands);
    while let Some(result) = pending.join_next().await {
        for (latency, wake, acknowledgement) in result.context("join many-worlds load task")?? {
            latencies.push(latency);
            wake_lateness.push(wake);
            input_ack.push(acknowledgement);
        }
    }
    let load_elapsed_secs = load_started.elapsed().as_secs_f64();
    ensure!(
        latencies.len() == expected_commands,
        "only {} of {expected_commands} inputs were acknowledged",
        latencies.len()
    );

    // Profile attribution (#557): aggregate the active worlds' actor
    // metrics over the timed load — journal-append latency (the durable
    // cost of one command), total command-handling latency, and the
    // deepest actor queue depth (whether the #8239 group-commit drain
    // ever had a batch to coalesce). Snapshot before the checkpoint
    // triggers below add their own appends and checkpoints.
    let mut journal_histograms = Vec::with_capacity(active.len());
    let mut command_histograms = Vec::with_capacity(active.len());
    let mut queue_depth_max = 0_u64;
    let mut checkpoints_during_load = 0_u64;
    for world in &active {
        let metrics = world.lease.service().world().metrics().await?;
        journal_histograms.push(metrics.journal_append_latency);
        command_histograms.push(metrics.command_latency);
        queue_depth_max = queue_depth_max.max(metrics.actor_queue_depth_max);
        checkpoints_during_load += metrics.checkpoints_total;
    }
    let journal_histogram = merge_histograms(&journal_histograms);
    let command_histogram = merge_histograms(&command_histograms);
    println!(
        "profile many_worlds journal_appends={} journal_append_mean_us={} journal_append_p50_us={} journal_append_p95_us={} journal_append_p99_us={} commands={} command_mean_us={} command_p50_us={} command_p95_us={} command_p99_us={} queue_depth_max={} checkpoints_during_load={}",
        journal_histogram.count,
        histogram_mean_us(&journal_histogram),
        histogram_percentile_us(&journal_histogram, 0.50),
        histogram_percentile_us(&journal_histogram, 0.95),
        histogram_percentile_us(&journal_histogram, 0.99),
        command_histogram.count,
        histogram_mean_us(&command_histogram),
        histogram_percentile_us(&command_histogram, 0.50),
        histogram_percentile_us(&command_histogram, 0.95),
        histogram_percentile_us(&command_histogram, 0.99),
        queue_depth_max,
        checkpoints_during_load,
    );

    // Force one verified checkpoint per active world after the timed load.
    // The install event plus the measured inputs leave every world exactly
    // one event short of its configured checkpoint cadence.
    for world in &active {
        let event = world
            .lease
            .service()
            .world()
            .input(world.token.clone(), b"inc".to_vec())
            .await?;
        black_box(event.sequence);
    }
    let expected_count = ticks + 1;

    // Capture stable, acknowledged state before eviction. Reading the view is
    // an actor barrier after the final input, so records include every update.
    let mut expected = Vec::with_capacity(active.len());
    for world in &active {
        let view = world
            .lease
            .service()
            .world()
            .view(world.token.clone())
            .await?;
        ensure!(
            view.as_deref()
                .is_some_and(|view| view.contains(&format!("count={expected_count}"))),
            "{} did not apply all {expected_count} counter inputs: {view:?}",
            world.name
        );
        let snapshot = world
            .lease
            .service()
            .world()
            .snapshot(world.token.clone())
            .await?;
        let (sequence, records) = world.lease.service().world().records();
        ensure!(
            snapshot.current_sequence == sequence,
            "{} sequence changed while capturing records",
            world.name
        );
        expected.push(ExpectedWorld {
            name: world.name.clone(),
            token: world.token.clone(),
            sequence,
            records: records.as_ref().clone(),
        });
    }

    // Release every active lease, let the real hub evict the durable actors,
    // and require a recent checkpoint in each journal before reopening it.
    drop(active);
    let evicted = hub.evict_idle(Duration::ZERO).await;
    ensure!(
        evicted == config.active_worlds,
        "evicted {evicted} active worlds, expected {}",
        config.active_worlds
    );
    let mut total_checkpoints = 0;
    for world in &expected {
        let checkpoints = checkpoint_sequences(&root.join(&world.name))?;
        let latest = checkpoints
            .iter()
            .copied()
            .max()
            .context("no checkpoint written")?;
        ensure!(
            latest <= world.sequence && world.sequence - latest <= config.checkpoint_every,
            "{} checkpoint at {latest} is not recent enough for sequence {}",
            world.name,
            world.sequence
        );
        total_checkpoints += checkpoints.len();
    }

    for world in &expected {
        let lease = hub
            .lease(Some(&world.name), None)
            .await
            .map_err(|error| anyhow::anyhow!(error.message()))?;
        let view = lease.service().world().view(world.token.clone()).await?;
        ensure!(
            view.as_deref()
                .is_some_and(|view| view.contains(&format!("count={expected_count}"))),
            "{} lost counter state after checkpoint/reopen: {view:?}",
            world.name
        );
        let snapshot = lease
            .service()
            .world()
            .snapshot(world.token.clone())
            .await?;
        let (sequence, records) = lease.service().world().records();
        ensure!(
            sequence == world.sequence && snapshot.current_sequence == world.sequence,
            "{} reopened at sequence {sequence}, expected {}",
            world.name,
            world.sequence
        );
        ensure!(
            records.as_ref() == &world.records,
            "{} records changed after checkpoint/reopen",
            world.name
        );
        drop(lease);
    }
    let reopened_evicted = hub.evict_idle(Duration::ZERO).await;
    ensure!(
        reopened_evicted == config.active_worlds,
        "reopened worlds could not be evicted cleanly"
    );

    let p50 = percentile(&latencies, 0.50);
    let p95 = percentile(&latencies, 0.95);
    let p99 = percentile(&latencies, 0.99);
    let wake_p50 = percentile(&wake_lateness, 0.50);
    let wake_p95 = percentile(&wake_lateness, 0.95);
    let wake_p99 = percentile(&wake_lateness, 0.99);
    let input_ack_p50 = percentile(&input_ack, 0.50);
    let input_ack_p95 = percentile(&input_ack, 0.95);
    let input_ack_p99 = percentile(&input_ack, 0.99);
    let target_met = p99 < 50.0;
    let completed_events_per_second = latencies.len() as f64 / load_elapsed_secs;
    let min = latencies
        .iter()
        .map(|latency| latency.as_secs_f64() * 1_000.0)
        .fold(f64::INFINITY, f64::min);
    println!(
        "bench many_worlds unit=ms_per_command median={p50:.3} min={min:.3} reps=1 journal_ack_mode={ack_mode_name} p50_ms={p50:.3} p95_ms={p95:.3} p99_ms={p99:.3} wake_p50_ms={wake_p50:.3} wake_p95_ms={wake_p95:.3} wake_p99_ms={wake_p99:.3} input_ack_p50_ms={input_ack_p50:.3} input_ack_p95_ms={input_ack_p95:.3} input_ack_p99_ms={input_ack_p99:.3} target_p99_ms=50 target_met={target_met} idle_worlds={} active_worlds={} events_per_second={} duration_secs={} commands={} load_elapsed_secs={load_elapsed_secs:.3} completed_events_per_second={completed_events_per_second:.1} checkpoint_triggers={} checkpoints={} evicted={} reopened={}",
        config.idle_worlds,
        config.active_worlds,
        config.events_per_second,
        config.duration_secs,
        latencies.len(),
        expected.len(),
        total_checkpoints,
        evicted,
        expected.len(),
    );
    ensure!(
        hub.open_count() == 0,
        "many-worlds benchmark left resident worlds open"
    );
    black_box(initially_evicted);
    Ok(())
}

/// F12.10's exit criterion (#1067): a world that spins on fuel must not raise
/// another world's command p99 by more than ten percent.
const TARGET_P99_RATIO: f64 = 1.10;

/// Minimal Plaza guest whose `plaza_update` never returns: the call runs until
/// its fuel budget is spent, then traps with fuel exhaustion. The required
/// alloc/init/view/out_len exports stay trivial so only the update burns fuel.
const FUEL_SPIN_WAT: &str = r#"
    (module
      (memory (export "memory") 1 1)
      (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "plaza_free") (param i32 i32))
      (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
      (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
        (i32.const 0)
        (loop $again (br $again)))
      (func (export "plaza_view") (param i32) (result i32) (i32.const 0))
      (func (export "plaza_out_len") (result i32) (i32.const 0)))
"#;

#[derive(Clone, Copy)]
struct FuelSpinConfig {
    /// Duration of each phase (calm, then spinning).
    phase_millis: u64,
    /// Victim inputs scheduled per second within a phase.
    commands_per_second: u64,
    /// Hostile worlds spinning concurrently.
    spinners: usize,
    /// Wasm fuel one hostile update may spend before it traps.
    spin_fuel: u64,
}

impl FuelSpinConfig {
    fn full(worker_threads: usize) -> Self {
        Self {
            phase_millis: 3_000,
            commands_per_second: 100,
            // Leave one worker for the victim and its actor: every other
            // thread is occupied by a fuel-limited guest call.
            spinners: worker_threads.saturating_sub(1).max(1),
            spin_fuel: 20_000_000,
        }
    }

    fn quick(worker_threads: usize) -> Self {
        Self {
            phase_millis: 750,
            commands_per_second: 100,
            spinners: worker_threads.saturating_sub(1).clamp(1, 4),
            spin_fuel: 2_000_000,
        }
    }

    fn ticks(&self) -> u64 {
        self.phase_millis * self.commands_per_second / 1_000
    }

    fn interval_nanos(&self) -> u64 {
        1_000_000_000 / self.commands_per_second
    }
}

struct BenchWorld {
    handle: WorldHandle,
    token: id::world::JoinCapability,
}

fn spawn_guest_world(
    name: &str,
    participant: &str,
    program: Box<dyn WorldProgram>,
) -> Result<BenchWorld> {
    let mut core = WorldCore::new(name, WorldLimits::default())?;
    let (_participant, token) = core.issue_capability(
        participant,
        WorldScopes::GUEST,
        CapabilityBounds {
            expires_at: None,
            uses: None,
        },
    )?;
    Ok(BenchWorld {
        handle: WorldHandle::spawn_with_program(core, program),
        token,
    })
}

/// The burn budget the fuel-spin worlds draw from when the
/// operator sets `ID_BENCH_GUEST_BURN_CONCURRENCY` (a dedicated
/// budget of that size, for sizing experiments); `None` leaves the
/// guests on the process-wide shared default, the production
/// shape. Read once per process.
fn bench_burn_budget() -> Option<Arc<GuestBurnBudget>> {
    static BUDGET: std::sync::OnceLock<Option<Arc<GuestBurnBudget>>> = std::sync::OnceLock::new();
    BUDGET
        .get_or_init(|| {
            std::env::var("ID_BENCH_GUEST_BURN_CONCURRENCY")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|size| *size > 0)
                .map(|size| Arc::new(GuestBurnBudget::new(size)))
        })
        .clone()
}

/// The budget the fuel-spin profile line reports: the dedicated
/// bench budget when one is set, else the shared default the
/// guests actually used.
fn report_burn_budget() -> Arc<GuestBurnBudget> {
    bench_burn_budget().unwrap_or_else(GuestBurnBudget::shared)
}

fn counter_program() -> Result<Box<dyn WorldProgram>> {
    let mut guest = Sandbox::compile(COUNTER_WASM, SandboxLimits::default())?.instantiate(1)?;
    // The victim's burn charge accumulates under one key across the
    // whole run (#1067 admission): it is the light key the fairness
    // rule protects, so it must not re-key per call.
    guest.set_charge_key("fuel-spin-victim".to_owned());
    if let Some(budget) = bench_burn_budget() {
        guest.set_burn_budget(budget);
    }
    Ok(Box::new(guest))
}

fn fuel_spin_sandbox(fuel: u64) -> Result<Sandbox> {
    let wasm = wat::parse_str(FUEL_SPIN_WAT).context("parse fuel-spin WAT fixture")?;
    Sandbox::compile(
        &wasm,
        SandboxLimits {
            fuel,
            ..SandboxLimits::default()
        },
    )
    .context("compile fuel-spin guest")
}

/// Keep hostile Wasm worlds spinning until their fuel budget is exhausted.
///
/// The stack poisons a program instance after a trapping call, so each world
/// contributes exactly one spin: the task replaces it with a freshly
/// instantiated world and spins again. Every spin must fail with fuel
/// exhaustion — a spin that returns is a harness bug, not a measurement.
async fn spin_worlds(
    sandbox: Sandbox,
    stop: Arc<AtomicBool>,
    hard_deadline: Instant,
    worker: usize,
) -> Result<u64> {
    let mut spins = 0u64;
    while !stop.load(Ordering::Relaxed) && Instant::now() < hard_deadline {
        let mut guest = sandbox.instantiate(spins)?;
        // Charge every spin of this worker to the worker's key, not
        // to a fresh per-instance key: the spinner's attained burn
        // must persist across its re-instantiations, or the fairness
        // rule cannot see that this key just burned (#1067).
        guest.set_charge_key(format!("fuel-spin-{worker}"));
        if let Some(budget) = bench_burn_budget() {
            guest.set_burn_budget(budget);
        }
        let world = spawn_guest_world(
            &format!("fuel-spin-{worker}-{spins}"),
            "bench-spinner",
            Box::new(guest),
        )?;
        match world
            .handle
            .input(world.token.clone(), b"spin until fuel is gone".to_vec())
            .await
        {
            Ok(event) => anyhow::bail!(
                "fuel-spin guest acknowledged input at sequence {} instead of exhausting its fuel",
                event.sequence
            ),
            Err(error) => {
                let message = format!("{error:#}");
                ensure!(
                    message.contains("fuel exhausted"),
                    "fuel-spin guest failed without fuel exhaustion: {message}"
                );
            }
        }
        spins += 1;
        let _ = world.handle.shutdown().await;
    }
    Ok(spins)
}

/// Submit `ticks` victim inputs on absolute deadlines. Each sample is the
/// triple the `many_worlds` driver records — (deadline-to-acknowledgement,
/// wake lateness, input acknowledgement wait) — so a late acknowledgement
/// stays in the end-to-end sample rather than shrinking the phase, and the
/// profile can separate the driver's own scheduling delay from the wait
/// for the world to acknowledge. The two component percentiles are
/// diagnostic and must not be added to estimate a combined percentile.
async fn drive_victim(
    victim: &BenchWorld,
    start: Instant,
    ticks: u64,
    interval_nanos: u64,
) -> Result<Vec<(Duration, Duration, Duration)>> {
    let mut samples = Vec::with_capacity(usize::try_from(ticks)?);
    for tick in 0..ticks {
        let deadline = start + Duration::from_nanos(tick * interval_nanos);
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        let input_started = Instant::now();
        let wake_lateness = input_started.saturating_duration_since(deadline);
        victim
            .handle
            .input(victim.token.clone(), b"inc".to_vec())
            .await?;
        samples.push((deadline.elapsed(), wake_lateness, input_started.elapsed()));
    }
    Ok(samples)
}

async fn run_calm_phase(
    victim: &BenchWorld,
    config: FuelSpinConfig,
) -> Result<Vec<(Duration, Duration, Duration)>> {
    let start = Instant::now() + Duration::from_millis(50);
    tokio::time::sleep_until(tokio::time::Instant::from_std(start)).await;
    drive_victim(victim, start, config.ticks(), config.interval_nanos()).await
}

/// Run the victim load while hostile worlds spin, returning the victim's
/// samples and how many fuel-exhausted spins the phase observed.
async fn run_spinning_phase(
    victim: &BenchWorld,
    config: FuelSpinConfig,
) -> Result<(Vec<(Duration, Duration, Duration)>, u64)> {
    let sandbox = fuel_spin_sandbox(config.spin_fuel)?;
    let stop = Arc::new(AtomicBool::new(false));
    let phase = Duration::from_millis(config.phase_millis);
    // The stop flag ends a healthy run; the hard deadline only bounds a
    // pathological one (a spin wider than the whole phase).
    let hard_deadline = Instant::now() + phase + Duration::from_secs(10);
    let mut spinners = tokio::task::JoinSet::new();
    for worker in 0..config.spinners {
        let sandbox = sandbox.clone();
        let stop = Arc::clone(&stop);
        spinners.spawn(spin_worlds(sandbox, stop, hard_deadline, worker));
    }

    let start = Instant::now() + Duration::from_millis(100);
    tokio::time::sleep_until(tokio::time::Instant::from_std(start)).await;
    let samples = drive_victim(victim, start, config.ticks(), config.interval_nanos()).await?;

    stop.store(true, Ordering::Relaxed);
    let mut spins = 0u64;
    while let Some(result) = spinners.join_next().await {
        spins += result.context("join fuel-spin task")??;
    }
    Ok((samples, spins))
}

/// F12.10 exit measurement (#1067): the victim world's command p99 calm
/// versus while hostile Wasm worlds spin until their fuel budget is gone.
///
/// This slice measures the gap only. It implements no fair scheduler,
/// per-account quota, or slow-client back-pressure, so an unmet target is
/// the expected recorded result rather than a regression. Alongside the
/// gap it prints a `profile fuel_spin_isolation` line attributing each
/// phase: the driver's wake lateness versus the input acknowledgement
/// wait (the sample split the `many_worlds` driver records), and the
/// victim actor's own journal-append and command latency histograms
/// differenced from its metrics snapshots across the phase.
async fn bench_fuel_spin_isolation(worker_threads: usize, quick: bool) -> Result<()> {
    let config = if quick {
        FuelSpinConfig::quick(worker_threads)
    } else {
        FuelSpinConfig::full(worker_threads)
    };
    let victim = spawn_guest_world("fuel-spin-victim", "bench-victim", counter_program()?)?;
    let metrics_start = victim.handle.metrics().await?;

    let calm_started = Instant::now();
    let calm = run_calm_phase(&victim, config).await?;
    let calm_elapsed = calm_started.elapsed().as_secs_f64();
    let metrics_after_calm = victim.handle.metrics().await?;

    let spinning_started = Instant::now();
    let (spinning, spins) = run_spinning_phase(&victim, config).await?;
    let spinning_elapsed = spinning_started.elapsed().as_secs_f64();
    let metrics_after_spinning = victim.handle.metrics().await?;

    // An empty phase is a harness failure, never a result: CI must not pass
    // on a measurement that collected nothing.
    let expected = usize::try_from(config.ticks())?;
    ensure!(
        !calm.is_empty(),
        "fuel_spin_isolation calm phase collected no victim samples"
    );
    ensure!(
        !spinning.is_empty(),
        "fuel_spin_isolation spinning phase collected no victim samples"
    );
    ensure!(
        calm.len() == expected,
        "calm phase acknowledged {} of {expected} victim inputs",
        calm.len()
    );
    ensure!(
        spinning.len() == expected,
        "spinning phase acknowledged {} of {expected} victim inputs",
        spinning.len()
    );
    ensure!(
        spins > 0,
        "fuel_spin_isolation observed no fuel-exhausted spinning world"
    );

    let split = |samples: &[(Duration, Duration, Duration)], index: usize| -> Vec<Duration> {
        samples
            .iter()
            .map(|sample| match index {
                0 => sample.0,
                1 => sample.1,
                _ => sample.2,
            })
            .collect()
    };
    let calm_latencies = split(&calm, 0);
    let calm_wake = split(&calm, 1);
    let calm_ack = split(&calm, 2);
    let spinning_latencies = split(&spinning, 0);
    let spinning_wake = split(&spinning, 1);
    let spinning_ack = split(&spinning, 2);

    let calm_p99 = percentile(&calm_latencies, 0.99);
    let spinning_p50 = percentile(&spinning_latencies, 0.50);
    let spinning_p99 = percentile(&spinning_latencies, 0.99);
    let spinning_min = spinning_latencies
        .iter()
        .map(|sample| sample.as_secs_f64() * 1_000.0)
        .fold(f64::INFINITY, f64::min);
    let ratio = if calm_p99 > 0.0 {
        spinning_p99 / calm_p99
    } else {
        f64::INFINITY
    };
    let target_met = ratio <= TARGET_P99_RATIO;
    println!(
        "bench fuel_spin_isolation unit=ms_per_command median={spinning_p50:.3} min={spinning_min:.3} reps=1 victim_p99_ms_calm={calm_p99:.3} victim_p99_ms_spinning={spinning_p99:.3} ratio={ratio:.3} target_ratio=1.10 target_met={target_met} calm_samples={} spinning_samples={} spins={spins} spinners={} phase_ms={} commands_per_second={} spin_fuel={} calm_elapsed_secs={calm_elapsed:.3} spinning_elapsed_secs={spinning_elapsed:.3}",
        calm.len(),
        spinning.len(),
        config.spinners,
        config.phase_millis,
        config.commands_per_second,
        config.spin_fuel,
    );

    let calm_journal = delta_histogram(
        &metrics_after_calm.journal_append_latency,
        &metrics_start.journal_append_latency,
    );
    let spinning_journal = delta_histogram(
        &metrics_after_spinning.journal_append_latency,
        &metrics_after_calm.journal_append_latency,
    );
    let calm_command = delta_histogram(
        &metrics_after_calm.command_latency,
        &metrics_start.command_latency,
    );
    let spinning_command = delta_histogram(
        &metrics_after_spinning.command_latency,
        &metrics_after_calm.command_latency,
    );
    // The burn budget's counters over the whole run (#1067 slice 2):
    // whether admission engaged at all, and whether the victim's
    // calls ever had to wait for a burn permit.
    let burn_snapshot = report_burn_budget().snapshot();
    println!(
        "profile fuel_spin_isolation calm_wake_p99_ms={:.3} calm_input_ack_p99_ms={:.3} spinning_wake_p99_ms={:.3} spinning_input_ack_p99_ms={:.3} calm_journal_appends={} calm_journal_append_mean_us={} calm_journal_append_p99_us={} spinning_journal_appends={} spinning_journal_append_mean_us={} spinning_journal_append_p99_us={} calm_command_mean_us={} calm_command_p99_us={} spinning_command_mean_us={} spinning_command_p99_us={} victim_queue_depth_max={} victim_checkpoints={} guest_burn_max_concurrent={} guest_burn_in_flight_max={} victim_burn_admitted={} victim_burn_waits={}",
        percentile(&calm_wake, 0.99),
        percentile(&calm_ack, 0.99),
        percentile(&spinning_wake, 0.99),
        percentile(&spinning_ack, 0.99),
        calm_journal.count,
        histogram_mean_us(&calm_journal),
        histogram_percentile_us(&calm_journal, 0.99),
        spinning_journal.count,
        histogram_mean_us(&spinning_journal),
        histogram_percentile_us(&spinning_journal, 0.99),
        histogram_mean_us(&calm_command),
        histogram_percentile_us(&calm_command, 0.99),
        histogram_mean_us(&spinning_command),
        histogram_percentile_us(&spinning_command, 0.99),
        metrics_after_spinning.actor_queue_depth_max,
        metrics_after_spinning
            .checkpoints_total
            .saturating_sub(metrics_start.checkpoints_total),
        burn_snapshot.max_concurrent,
        burn_snapshot.in_flight_max,
        burn_snapshot
            .keys
            .get("fuel-spin-victim")
            .map_or(0, |charge| charge.admitted),
        burn_snapshot
            .keys
            .get("fuel-spin-victim")
            .map_or(0, |charge| charge.waits),
    );
    let _ = victim.handle.shutdown().await;
    Ok(())
}

/// A guest that counts the events replayed through it.
#[derive(Default)]
struct CountingProgram {
    seen: u64,
}

impl WorldProgram for CountingProgram {
    fn update(&mut self, _event: &WorldEvent) -> Result<()> {
        self.seen += 1;
        Ok(())
    }
}

fn chat_event(sequence: u64, text: &str) -> WorldEvent {
    WorldEvent {
        sequence,
        participant_id: 1,
        kind: WorldEventKind::Chat(text.to_owned()),
    }
}

fn input_event(sequence: u64, text: &str) -> WorldEvent {
    WorldEvent {
        sequence,
        participant_id: 1,
        kind: WorldEventKind::Input(text.as_bytes().to_vec()),
    }
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn report(name: &str, unit: &str, samples: Vec<f64>, cells: &mut Vec<MatrixCell>) {
    let med = median(samples.clone());
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    println!(
        "bench {name} unit={unit} median={med:.3} min={min:.3} reps={}",
        samples.len()
    );
    record_matrix_cells(name, unit, &samples, cells);
}

/// One planned cell of the #5896 benchmark matrix. `samples` is empty
/// unless `status` is `measured`.
struct MatrixCell {
    workload_id: String,
    interface_id: String,
    status: &'static str,
    reason: Option<String>,
    unit: Option<String>,
    samples: Vec<f64>,
}

/// The interface axis of the matrix (#5896 decision record): the
/// loadgen `in_process` surface this harness measures, plus the
/// shell-contract transport IDs (`pkgs/id/contract/shell-contract.v1.json`).
/// Presentation kinds (`WorldSurface::Screen` / `Text`) are not
/// transports and are never interface IDs.
const MATRIX_INTERFACES: [&str; 4] = ["in_process", "iroh", "websocket", "ssh-exec"];

/// The configuration axis (#5894): the standard server baseline's
/// default configuration. The scale (S0/S1/S2 per #5894) is recorded
/// on each cell as provenance; `--quick` smoke sizes are not a scale.
const MATRIX_CONFIGURATION: &str = "standard-default";

fn record_matrix_cells(name: &str, unit: &str, samples: &[f64], cells: &mut Vec<MatrixCell>) {
    for interface in MATRIX_INTERFACES {
        if interface == "in_process" {
            cells.push(MatrixCell {
                workload_id: name.to_owned(),
                interface_id: interface.to_owned(),
                status: "measured",
                reason: None,
                unit: Some(unit.to_owned()),
                samples: samples.to_vec(),
            });
        } else {
            cells.push(MatrixCell {
                workload_id: name.to_owned(),
                interface_id: interface.to_owned(),
                status: "unsupported",
                reason: Some(format!(
                    "no {interface} transport adapter in the world_bench harness: \
                     this workload is measured on the in_process surface only; \
                     transport cells require the server harness of the #5894 \
                     baseline programme"
                )),
                unit: None,
                samples: Vec::new(),
            });
        }
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

/// Days-from-civil algorithm (Howard Hinnant) — the inverse: convert
/// Unix epoch seconds to an ISO-8601 UTC timestamp without a date
/// crate, so the manifest's `started_at_utc` is human-readable.
fn iso8601_utc(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs = epoch_secs % 86_400;
    let (hour, minute, second) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Write the #5896 `results.jsonl` bundle: a run-manifest record
/// followed by one record per planned matrix cell. The comparison
/// identity of a measured metric is (machine_class, workload_id,
/// interface_id, configuration_id, metric, statistic) — consumers
/// must not compare across machine classes, configurations, units,
/// or workload/interface IDs.
fn write_results_jsonl(cells: &[MatrixCell], quick: bool) -> Result<()> {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let run_id = format!("{}-{}", started, std::process::id());
    // The canonical machine classes are #5894's: `fleet-laptop` (the
    // reference class of BASELINES.md) and `ci-runner` (relative-only
    // series). The workflow sets ci-runner explicitly; a bare local
    // run is on the reference machine by fleet convention.
    let machine_class =
        std::env::var("ID_BENCH_MACHINE_CLASS").unwrap_or_else(|_| "fleet-laptop".to_owned());
    let git_sha = std::env::var("ID_BENCH_GIT_SHA")
        .or_else(|_| std::env::var("GITHUB_SHA"))
        .unwrap_or_else(|_| "unknown".to_owned());
    let toolchain = std::env::var("ID_BENCH_TOOLCHAIN").unwrap_or_else(|_| "unknown".to_owned());
    let build_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let scale = if quick { "smoke" } else { "S1" };

    let mut out = String::new();
    out.push_str(&format!(
        "{{\"type\":\"manifest\",\"schema_version\":1,\"run_id\":\"{}\",\"git_sha\":\"{}\",\"started_at_utc\":\"{}\",\"machine_class\":\"{}\",\"os\":\"{}\",\"arch\":\"{}\",\"cpu_threads\":{},\"toolchain\":\"{}\",\"build_profile\":\"{}\",\"generator_version\":{},\"seed\":{}}}\n",
        json_escape(&run_id),
        json_escape(&git_sha),
        iso8601_utc(started),
        json_escape(&machine_class),
        std::env::consts::OS,
        std::env::consts::ARCH,
        threads,
        json_escape(&toolchain),
        build_profile,
        loadgen::GENERATOR_VERSION,
        BENCH_SEED,
    ));
    for cell in cells {
        let mut line = format!(
            "{{\"type\":\"cell\",\"run_id\":\"{}\",\"workload_id\":\"{}\",\"interface_id\":\"{}\",\"configuration_id\":\"{}\",\"scale\":\"{}\",\"status\":\"{}\"",
            json_escape(&run_id),
            json_escape(&cell.workload_id),
            json_escape(&cell.interface_id),
            MATRIX_CONFIGURATION,
            scale,
            cell.status,
        );
        if let Some(reason) = &cell.reason {
            line.push_str(&format!(",\"reason\":\"{}\"", json_escape(reason)));
        }
        line.push_str(",\"metrics\":[");
        if cell.status == "measured" {
            let unit = cell.unit.as_deref().unwrap_or("");
            let samples_json = cell
                .samples
                .iter()
                .map(|s| format!("{s:.3}"))
                .collect::<Vec<_>>()
                .join(",");
            for (i, (statistic, value)) in [
                ("median", median(cell.samples.clone())),
                (
                    "min",
                    cell.samples.iter().copied().fold(f64::INFINITY, f64::min),
                ),
            ]
            .iter()
            .enumerate()
            {
                if i > 0 {
                    line.push(',');
                }
                line.push_str(&format!(
                    "{{\"metric\":\"latency\",\"statistic\":\"{statistic}\",\"unit\":\"{}\",\"value\":{value:.3},\"sample_count\":{},\"samples\":[{samples_json}]}}",
                    json_escape(unit),
                    cell.samples.len(),
                ));
            }
        }
        line.push_str("]}\n");
        out.push_str(&line);
    }

    let path = std::env::var("ID_BENCH_RESULTS").unwrap_or_else(|_| "results.jsonl".to_owned());
    std::fs::write(&path, out)
        .with_context(|| format!("write benchmark matrix results to {path}"))?;
    println!("matrix results={path} cells={}", cells.len());
    Ok(())
}

/// The `journal_append_fsync` workload on the loadgen core (#5891
/// step 1): one actor, a single-family journal-append mix, the
/// `InProcess` surface as direct `FileJournal` calls.
struct AppendTarget {
    dir: PathBuf,
    rep: usize,
}

struct AppendSession {
    journal: FileJournal,
    seq: u64,
}

impl loadgen::Target<AppendSession> for AppendTarget {
    fn connect(
        &mut self,
        _seed: u64,
        _actor: &loadgen::Actor,
    ) -> Result<AppendSession, loadgen::OpError> {
        let path = self.dir.join(format!("append-{}.jsonl", self.rep));
        self.rep += 1;
        let (journal, _) = FileJournal::open(&path, "bench-append")
            .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
        Ok(AppendSession { journal, seq: 0 })
    }

    fn execute(
        &mut self,
        session: &mut AppendSession,
        op: loadgen::OpKind,
        _payload: &[u8],
    ) -> Result<Duration, loadgen::OpError> {
        match op {
            loadgen::OpKind::JournalAppend => {
                session.seq += 1;
                let start = Instant::now();
                session
                    .journal
                    .append(&JournalEntry::Committed {
                        event: chat_event(session.seq, "benchmark chat payload"),
                    })
                    .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
                Ok(start.elapsed())
            }
            other => Err(loadgen::OpError::Unsupported(other)),
        }
    }

    fn disconnect(&mut self, session: AppendSession) -> Result<(), loadgen::OpError> {
        black_box(session.journal);
        Ok(())
    }
}

fn bench_journal_append(dir: &PathBuf, entries: u64, reps: usize) -> Result<Vec<f64>> {
    let mut target = AppendTarget {
        dir: dir.clone(),
        rep: 0,
    };
    let mut samples = Vec::new();
    for _ in 0..reps {
        let summary = loadgen::run(
            &mut target,
            &loadgen::RunSpec {
                seed: BENCH_SEED,
                workload: loadgen::Workload {
                    name: "journal_append_fsync".to_owned(),
                    mix: loadgen::MixTable::single(loadgen::OpKind::JournalAppend),
                    ops_per_actor: entries,
                    warm_up_ops: 0,
                    payload_bytes: 0,
                },
                profile: loadgen::Profile::honest("bench"),
                scale: "S0".to_owned(),
                surface: loadgen::Surface::InProcess,
                actors: 1,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let stats = summary.stats(loadgen::OpKind::JournalAppend);
        samples.push(stats.total().as_secs_f64() * 1e6 / entries as f64);
    }
    Ok(samples)
}

/// The `checkpoint_compaction` workload on the loadgen core: the
/// journal fill is session setup (untimed, as before); the measured
/// operation is the single compaction.
struct CompactTarget {
    dir: PathBuf,
    entries: u64,
    rep: usize,
}

struct CompactSession {
    journal: FileJournal,
    compacted: Vec<JournalEntry>,
}

impl loadgen::Target<CompactSession> for CompactTarget {
    fn connect(
        &mut self,
        _seed: u64,
        _actor: &loadgen::Actor,
    ) -> Result<CompactSession, loadgen::OpError> {
        let path = self.dir.join(format!("compact-{}.jsonl", self.rep));
        self.rep += 1;
        let (mut journal, _) = FileJournal::open(&path, "bench-compact")
            .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
        for seq in 1..=self.entries {
            journal
                .append(&JournalEntry::Committed {
                    event: chat_event(seq, "benchmark chat payload"),
                })
                .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
        }
        // The set a real compaction writes: header + participants +
        // checkpoint, far smaller than the journal it replaces. 64
        // stand-in entries keep the payload honest without building
        // a full checkpoint state here.
        let compacted: Vec<JournalEntry> = (1..=64)
            .map(|seq| JournalEntry::Committed {
                event: chat_event(seq, "checkpoint stand-in payload"),
            })
            .collect();
        Ok(CompactSession { journal, compacted })
    }

    fn execute(
        &mut self,
        session: &mut CompactSession,
        op: loadgen::OpKind,
        _payload: &[u8],
    ) -> Result<Duration, loadgen::OpError> {
        match op {
            loadgen::OpKind::JournalCompact => {
                let start = Instant::now();
                session
                    .journal
                    .compact(&session.compacted)
                    .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
                Ok(start.elapsed())
            }
            other => Err(loadgen::OpError::Unsupported(other)),
        }
    }

    fn disconnect(&mut self, session: CompactSession) -> Result<(), loadgen::OpError> {
        black_box(session.journal);
        Ok(())
    }
}

fn bench_compaction(dir: &PathBuf, entries: u64, reps: usize) -> Result<Vec<f64>> {
    let mut target = CompactTarget {
        dir: dir.clone(),
        entries,
        rep: 0,
    };
    let mut samples = Vec::new();
    for _ in 0..reps {
        let summary = loadgen::run(
            &mut target,
            &loadgen::RunSpec {
                seed: BENCH_SEED,
                workload: loadgen::Workload {
                    name: "checkpoint_compaction".to_owned(),
                    mix: loadgen::MixTable::single(loadgen::OpKind::JournalCompact),
                    ops_per_actor: 1,
                    warm_up_ops: 0,
                    payload_bytes: 0,
                },
                profile: loadgen::Profile::honest("bench"),
                scale: "S0".to_owned(),
                surface: loadgen::Surface::InProcess,
                actors: 1,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let stats = summary.stats(loadgen::OpKind::JournalCompact);
        samples.push(stats.total().as_secs_f64() * 1e6);
    }
    Ok(samples)
}

/// The `guest_replay` workload on the loadgen core: building the
/// journal is session setup; the measured operation is one
/// restore + replay of the whole journal (a journal read).
struct ReplayTarget {
    events: u64,
}

struct ReplaySession {
    journal: Vec<JournalEntry>,
    events: u64,
}

impl loadgen::Target<ReplaySession> for ReplayTarget {
    fn connect(
        &mut self,
        _seed: u64,
        _actor: &loadgen::Actor,
    ) -> Result<ReplaySession, loadgen::OpError> {
        // Build the journal once: world created, one participant
        // issued, then N committed input events — the committed
        // kind restore() actually puts in the replay list after a
        // restart. (Committed chat is applied to core state
        // directly and never replayed, which is how the slice-2
        // baseline came to measure an empty guest pass; #1403.)
        let mut core = WorldCore::new("bench-replay", WorldLimits::default())
            .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
        let (participant, _token) = core
            .issue_capability(
                "bench",
                WorldScopes::GUEST,
                CapabilityBounds {
                    expires_at: None,
                    uses: None,
                },
            )
            .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
        let mut journal: Vec<JournalEntry> = vec![JournalEntry::Created {
            world_id: "bench-replay".to_owned(),
            version: 1,
        }];
        // Rebuild the Issued entry from the minted participant via
        // restore's own expectations: committed events only need a
        // participant id that exists, and restore validates the
        // header + ordering + each event kind against the
        // participant's scopes, so the participant is issued the
        // guest set (JOIN|CHAT|INPUT): an INPUT-less Issued entry
        // makes restore refuse the first committed input event
        // (#8316). Issue the capability in the restored core's
        // place by using participant 1 events after an Issued entry
        // we construct from the scratch participant's public fields.
        journal.push(JournalEntry::Issued {
            participant,
            digest: "0".repeat(64),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
            issued_at: None,
        });
        for seq in 1..=self.events {
            journal.push(JournalEntry::Committed {
                event: input_event(seq, "replay me"),
            });
        }
        Ok(ReplaySession {
            journal,
            events: self.events,
        })
    }

    fn execute(
        &mut self,
        session: &mut ReplaySession,
        op: loadgen::OpKind,
        _payload: &[u8],
    ) -> Result<Duration, loadgen::OpError> {
        match op {
            loadgen::OpKind::JournalRead => {
                let start = Instant::now();
                let restored = WorldCore::restore(
                    "bench-replay",
                    WorldLimits::default(),
                    session.journal.clone(),
                )
                .map_err(|e| loadgen::OpError::Failed(format!("restore: {e:#}")))?;
                let mut guest = CountingProgram::default();
                for event in &restored.replay {
                    guest
                        .update(event)
                        .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
                }
                // The guest must have seen every committed event: an
                // empty or short replay is a harness bug (#1403
                // shipped a baseline whose guest saw zero events),
                // never a timing to record.
                if guest.seen != session.events {
                    return Err(loadgen::OpError::Failed(format!(
                        "guest replay saw {} of {} committed events",
                        guest.seen, session.events
                    )));
                }
                let elapsed = start.elapsed();
                black_box(guest.seen);
                black_box(restored.replay.len());
                Ok(elapsed)
            }
            other => Err(loadgen::OpError::Unsupported(other)),
        }
    }

    fn disconnect(&mut self, _session: ReplaySession) -> Result<(), loadgen::OpError> {
        Ok(())
    }
}

fn bench_guest_replay(events: u64, reps: usize) -> Result<Vec<f64>> {
    let mut target = ReplayTarget { events };
    let mut samples = Vec::new();
    for _ in 0..reps {
        let summary = loadgen::run(
            &mut target,
            &loadgen::RunSpec {
                seed: BENCH_SEED,
                workload: loadgen::Workload {
                    name: "guest_replay".to_owned(),
                    mix: loadgen::MixTable::single(loadgen::OpKind::JournalRead),
                    ops_per_actor: 1,
                    warm_up_ops: 0,
                    payload_bytes: 0,
                },
                profile: loadgen::Profile::honest("bench"),
                scale: "S0".to_owned(),
                surface: loadgen::Surface::InProcess,
                actors: 1,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let stats = summary.stats(loadgen::OpKind::JournalRead);
        samples.push(stats.total().as_secs_f64() * 1e6 / events as f64);
    }
    Ok(samples)
}

/// The `session_fanout` workload on the loadgen core: session setup
/// spawns the world and subscribes every client; the measured
/// operation is one chat round delivered to all of them (a sync
/// round).
struct FanoutTarget {
    runtime: tokio::runtime::Runtime,
    clients: usize,
}

struct FanoutSession {
    handle: WorldHandle,
    tokens: Vec<JoinCapability>,
    receivers: Vec<tokio::sync::broadcast::Receiver<WorldEvent>>,
    round: u64,
}

impl loadgen::Target<FanoutSession> for FanoutTarget {
    fn connect(
        &mut self,
        _seed: u64,
        _actor: &loadgen::Actor,
    ) -> Result<FanoutSession, loadgen::OpError> {
        // WorldHandle::spawn starts the world's task on the runtime,
        // so session setup runs inside block_on, as the pre-core
        // bench did for the whole workload.
        let clients = self.clients;
        self.runtime.block_on(async move {
            let mut core = WorldCore::new("bench-fanout", WorldLimits::default())
                .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
            // Rotate across several senders: the per-participant chat
            // rate limit (50 messages / 10 s window) would otherwise
            // refuse the full-size workload (100 rounds from one sender)
            // partway through. Four senders x 25 rounds stay under the
            // burst cap while measuring the same delivery path.
            let mut tokens = Vec::new();
            for sender in ["bench-0", "bench-1", "bench-2", "bench-3"] {
                let (_participant, token) = core
                    .issue_capability(
                        sender,
                        WorldScopes::JOIN.union(WorldScopes::CHAT),
                        CapabilityBounds {
                            expires_at: None,
                            uses: None,
                        },
                    )
                    .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
                tokens.push(token);
            }
            let handle = WorldHandle::spawn(core);
            let receivers: Vec<_> = (0..clients).map(|_| handle.subscribe()).collect();
            Ok(FanoutSession {
                handle,
                tokens,
                receivers,
                round: 0,
            })
        })
    }

    fn execute(
        &mut self,
        session: &mut FanoutSession,
        op: loadgen::OpKind,
        _payload: &[u8],
    ) -> Result<Duration, loadgen::OpError> {
        match op {
            loadgen::OpKind::WorldSync => {
                let round = session.round;
                session.round += 1;
                let start = Instant::now();
                self.runtime.block_on(async {
                    let token = session.tokens[(round as usize) % session.tokens.len()].clone();
                    session
                        .handle
                        .chat(token, format!("fanout {round}"))
                        .await
                        .map_err(|e| loadgen::OpError::Failed(format!("{e:#}")))?;
                    for receiver in &mut session.receivers {
                        let event = receiver
                            .recv()
                            .await
                            .map_err(|e| loadgen::OpError::Failed(format!("fanout recv: {e}")))?;
                        black_box(event.sequence);
                    }
                    Ok::<(), loadgen::OpError>(())
                })?;
                Ok(start.elapsed())
            }
            other => Err(loadgen::OpError::Unsupported(other)),
        }
    }

    fn disconnect(&mut self, _session: FanoutSession) -> Result<(), loadgen::OpError> {
        Ok(())
    }
}

fn bench_fanout(clients: usize, rounds: u64, reps: usize) -> Result<Vec<f64>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut target = FanoutTarget { runtime, clients };
    let mut samples = Vec::new();
    for _ in 0..reps {
        let summary = loadgen::run(
            &mut target,
            &loadgen::RunSpec {
                seed: BENCH_SEED,
                workload: loadgen::Workload {
                    name: "session_fanout".to_owned(),
                    mix: loadgen::MixTable::single(loadgen::OpKind::WorldSync),
                    ops_per_actor: rounds,
                    warm_up_ops: 0,
                    payload_bytes: 0,
                },
                profile: loadgen::Profile::honest("bench"),
                scale: "S0".to_owned(),
                surface: loadgen::Surface::InProcess,
                actors: 1,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let stats = summary.stats(loadgen::OpKind::WorldSync);
        samples.push(stats.total().as_secs_f64() * 1e6 / (rounds * clients as u64) as f64);
    }
    Ok(samples)
}


fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|arg| arg == "--quick");
    let only = args
        .iter()
        .position(|arg| arg == "--only")
        .and_then(|idx| args.get(idx + 1))
        .map(String::as_str);

    let (events, clients, rounds, reps): (u64, usize, u64, usize) = if quick {
        (200, 4, 20, 3)
    } else {
        (10_000, 32, 100, 5)
    };

    let dir = std::env::temp_dir().join(format!("id-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;

    const BENCHMARKS: [&str; 10] = [
        "journal_append_fsync",
        "checkpoint_compaction",
        "guest_replay",
        "session_fanout",
        "many_worlds",
        "fuel_spin_isolation",
        "stress_gen",
        "stress_mem",
        "stress_join",
        "frame_cost",
    ];
    if let Some(name) = only
        && !BENCHMARKS.contains(&name)
    {
        eprintln!("error: unknown benchmark '{name}' in --only");
        eprintln!("valid benchmark names: {}", BENCHMARKS.join(", "));
        std::process::exit(2);
    }

    let run = |name: &str| only.is_none() || only == Some(name);

    // The #5896 matrix cells for the loadgen-core workloads; the
    // async workloads below are not matrix cells yet (header note).
    let mut cells: Vec<MatrixCell> = Vec::new();

    if run("journal_append_fsync") {
        report(
            "journal_append_fsync",
            "us_per_append",
            bench_journal_append(&dir, events.min(2_000), reps)?,
            &mut cells,
        );
    }
    if run("checkpoint_compaction") {
        report(
            "checkpoint_compaction",
            "us_per_compact",
            bench_compaction(&dir, events.min(2_000), reps)?,
            &mut cells,
        );
    }
    if run("guest_replay") {
        report(
            "guest_replay",
            "us_per_event",
            bench_guest_replay(events, reps)?,
            &mut cells,
        );
    }
    if run("session_fanout") {
        report(
            "session_fanout",
            "us_per_delivery",
            bench_fanout(clients, rounds, reps)?,
            &mut cells,
        );
    }
    if run("many_worlds") {
        let worker_threads = std::thread::available_parallelism().map_or(1, usize::from);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()?;
        runtime.block_on(bench_many_worlds(&dir.join("many-worlds"), quick))?;
    }
    if run("fuel_spin_isolation") {
        // A single-threaded runtime cannot show contention between worlds:
        // the hostile guests must occupy real worker threads.
        let worker_threads = std::thread::available_parallelism().map_or(1, usize::from);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()?;
        runtime.block_on(bench_fuel_spin_isolation(worker_threads, quick))?;
    }

    if run("stress_gen") {
        stress::bench_stress_gen(quick)?;
    }

    if run("stress_mem") {
        stress::bench_stress_mem(quick)?;
    }

    if run("stress_join") {
        stress::bench_stress_join(quick)?;
    }

    if run("frame_cost") {
        stress::bench_frame_cost(quick)?;
    }

    if !cells.is_empty() {
        write_results_jsonl(&cells, quick)?;
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod ack_mode_selector_tests {
    use super::{journal_ack_mode_name, parse_journal_ack_mode};
    use id::world::JournalAckMode;

    #[test]
    fn accepted_values_map_to_modes() {
        assert_eq!(parse_journal_ack_mode("sync"), JournalAckMode::Sync);
        assert_eq!(parse_journal_ack_mode("batched"), JournalAckMode::Batched);
        assert_eq!(parse_journal_ack_mode("async"), JournalAckMode::Async);
    }

    #[test]
    fn matching_is_case_and_whitespace_tolerant() {
        assert_eq!(parse_journal_ack_mode("  Batched "), JournalAckMode::Batched);
        assert_eq!(parse_journal_ack_mode("ASYNC"), JournalAckMode::Async);
    }

    #[test]
    fn unset_or_garbage_falls_back_to_sync_default() {
        assert_eq!(parse_journal_ack_mode(""), JournalAckMode::Sync);
        assert_eq!(
            parse_journal_ack_mode("fsync-everything"),
            JournalAckMode::Sync
        );
        assert_eq!(
            parse_journal_ack_mode("sync "),
            JournalAckMode::Sync,
            "trailing input still resolves before the fallback"
        );
    }

    #[test]
    fn machine_readable_names_cover_every_mode() {
        assert_eq!(journal_ack_mode_name(JournalAckMode::Sync), "sync");
        assert_eq!(journal_ack_mode_name(JournalAckMode::Batched), "batched");
        assert_eq!(journal_ack_mode_name(JournalAckMode::Async), "async");
    }
}
