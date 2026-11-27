# Scale & performance hardening — v1: large-world stress program design

First-iteration design for [#6304](https://github.com/developing-today/code/issues/6304),
under the Phase 14 core topic [#6281](https://github.com/developing-today/code/issues/6281)
(Scale & performance hardening), filed from the next-horizons discussion
[#1240](https://github.com/developing-today/code/issues/1240). Plan of record:
epic [#6312](https://github.com/developing-today/code/issues/6312). The v2 step
(growth policy and the degradation ladder) is
[#6305](https://github.com/developing-today/code/issues/6305), sequenced behind
this design.

Not to be confused with `.opencode/plans/phase-14-close-out.md`: that plan is
the close-out phase that shares the Phase 14 number. This document is the
scale-and-performance topic's v1 design, and it covers only the stress
program: how large worlds are generated, what is measured, and what the first
measured limits are.

## Entry condition — verified

The issue's entry condition is that the core topic's research (#6281) has
produced its first-pass findings and the design-queue items ahead of this
step are moving. Verified met on 2026-11-23 and recorded on #6304:

- #6281 first-pass findings exist (comment 7090445407 on #6281): a limits
  inventory from current main, journal/fan-out economics from the committed
  `pkgs/id/benches` baselines, the two measured target misses, and the
  degradation-ladder gap assigned to v2.
- The other sequencing gate, #1067, is actively landing (per-account quotas
  slice merged 2026-11-23).

This design is grounded in those findings and in the current source; every
number below names its source, and rows are marked **measured** (a committed
baseline or a recorded run), **codified** (a constant in source), or
**derived** (arithmetic on measured rows).

## Principles conformance (#1240)

Drew's Phase 14 principles govern this step. The stress program conforms by
construction:

- **Local-first always.** The generator and harness run fully offline on one
  host against local world stores. No measurement in this program requires a
  hosted instance, an account, or a network.
- **Hostable by anyone at any scale.** The program extends the existing
  `world_bench` harness — one cargo bench target, documented re-baseline
  flow (`benches/BASELINES.md`) — so anyone with the repo can reproduce a
  tier run. Tier definitions are parameters, not infrastructure.
- **P2P behavior identical to hosted behavior.** The program stresses the
  shared world-stack paths both topologies run on the same code: world store
  and journal, checkpoint/replay, guest frame execution, session fan-out.
  It introduces no hosted-only measurement path and no shortcut a peer node
  could not also take.
- **The hosted instance is a public backstop, never a requirement.** The
  server-class host class below is any sufficiently large host a volunteer
  has; it is not our hosted instance, and no tier is defined as "runs on the
  hosted instance."

## What "current size" means: the 1x anchors

Scale claims need a fixed 1x. These are the platform's current operating
points, from the committed harness configuration and baselines:

| Axis                 | 1x anchor                                     | Source                                                                  |
| -------------------- | --------------------------------------------- | ----------------------------------------------------------------------- |
| Worlds per host      | 1,000 idle / 100 active, 20 events/s for 10 s | `benches/world_bench.rs`, `ManyWorldsConfig::full`                      |
| Single-world history | 10,000 events                                 | `guest_replay` workload, `benches/BASELINES.md`                         |
| Compaction journal   | 2,000 entries                                 | `checkpoint_compaction` workload                                        |
| Members on one world | 32 clients × 100 rounds                       | `session_fanout` workload                                               |
| Guest frame budget   | 10M fuel per call (default)                   | `SandboxLimits::fuel`; perf-budgets doc in this directory               |
| Journal size cap     | 64 MiB per world journal                      | codified: `MAX_JOURNAL_BYTES`, `crates/id-worlds/src/world_store.rs:51` |

## Scale tiers

| Axis                          | 1x     | 10x     | 100x      |
| ----------------------------- | ------ | ------- | --------- |
| Idle worlds                   | 1,000  | 10,000  | 100,000   |
| Active worlds                 | 100    | 1,000   | 10,000    |
| Single-world history (events) | 10,000 | 100,000 | 1,000,000 |
| Members on the hot world      | 32     | 320     | 3,200     |

A tier is a measurement target, not a promise. The 100x fleet tier in
particular may exceed one host's memory; the program's job at that point is
to find the wall and record it as the measured limit (see Run protocol).
Eviction under per-world memory budgets (#557, criteria complete) means idle
worlds need not be resident, so "idle worlds" measures store, open, and
wake behaviour — not resident memory for the full set.

## Generator design

Extend the loadgen that already lives in `benches/world_bench.rs` rather
than building a second harness:

- **Deterministic.** The harness already runs on a fixed seed
  (`BENCH_SEED`) and emits a JSONL run manifest (run id, git SHA, machine
  class, toolchain, seed). The stress generator follows the same pattern:
  same seed + same tier ⇒ same worlds, byte for byte in event content.
- **Manifest-verified.** Generation emits a manifest alongside the world
  set: seed, tier, event mix, expected event counts per world, and the
  expected final state hash per measured world. Join and replay
  measurements verify against that hash, so a fast-but-wrong run cannot
  masquerade as a result.
- **Event mix.** Three shapes that cover the codified per-object limits:
  chat-class small events (the fan-out shape), document edits at the
  codified document shapes (64 KiB per document, 512 list items), and
  program installs (compile inputs capped at 8 files / 256 KiB total).
  The mix ratio is a generator parameter recorded in the manifest.
- **Profiles, not a new binary.** Tier runs are profiles of `world_bench`
  alongside `full` and `quick`, keeping the stable `bench <name> unit=…`
  output lines and the JSONL bundle, so `id-bench-compare.sh` regression
  reporting keeps working unchanged.

Implementation: [#13083](https://github.com/developing-today/code/issues/13083).

## Measurements

The issue names four measurements. This design adds a fifth — the
durable-append ceiling — because the findings show it is the constraint
the other four run into.

### 1. Join time

Cold join: open the world store, load the latest checkpoint, replay the
journal tail, produce the first view frame. Warm join: the same against a
resident world. Report p50/p95/p99 per tier, each measured twice over:
once with the journal at a checkpoint boundary and once at the
mid-interval worst case (just before the next checkpoint; the session
checkpoint interval defaults to 2,000 ms, codified in
`crates/id-store/src/session_checkpoint.rs`).

Implementation: [#13084](https://github.com/developing-today/code/issues/13084).

### 2. Replay time

Full replay from genesis and replay-from-checkpoint, at 10k / 100k / 1M
events. The committed baseline (0.170 µs/event) predicts 1.7 ms / 17 ms /
170 ms if replay stays linear. The design's job is to verify linearity and
find where it breaks — journal read, decode, or guest state rebuild — not
to assume the prediction.

Covered by the join bench's replay component
([#13084](https://github.com/developing-today/code/issues/13084)) and the
frame-cost slice's state-size sweep
([#13085](https://github.com/developing-today/code/issues/13085)).

### 3. Frame cost

View (and layout, where the guest exports it) fuel and wall time as a
function of applied state size: 10k / 100k / 1M events. Instrumentation
already exists — `WorldInstance::metrics` and `id world measure` (see the
perf-budgets doc in this directory) — so this slice adds workloads, not
instrumentation. The earlier ~7.1–7.8M Territory-class estimate is stale:
the corrected #13085 measurement is 958,616 view fuel and 958,683 layout
fuel (about 9.6% of the 10M default budget), flat from 10k through 1M
applied events. See the measured rows below; event history size did not
bring this guest frame workload near the ceiling.

Implementation: [#13085](https://github.com/developing-today/code/issues/13085).

### 4. Memory

RSS per resident world at 1x and 10x; resident-set growth under the
existing memory-budget/eviction behaviour; and the compaction memory spike
at a 100k-entry journal (the committed compaction baseline is time-only,
at 2,000 entries). A tier whose resident set cannot be held is a measured
limit, recorded as such.

Implementation: [#13086](https://github.com/developing-today/code/issues/13086).

### 5. Durable-append ceiling (the constraint behind the rest)

The committed baseline puts journal append+fsync at 5,180 µs median, which
derives a per-world durable-write ceiling of ≈193 appends/s before
batching. Demand already exceeds that shape at 1x fleet scale (100 active
worlds × 20 events/s = 2,000 appends/s host-wide, bounded per world by
journal contention), and both measured target misses — many-worlds p99
(#557) and the `fuel_spin_isolation` ratio (#1067 line) — trace to
durable-append / scheduling contention under concurrent load, not raw
compute. The stress program measures the aggregate ceiling per tier; it
does not re-design the fix. Group-commit batching is already scoped as
[#8239](https://github.com/developing-today/code/issues/8239), and the
growth policy around the ceiling is v2 (#6305) work.

## First measured limits table (v1)

Assembled from the committed baselines (2026-10-28, reference laptop:
Ryzen 7 7840U, 16 threads, 123 GB RAM, NixOS — `benches/BASELINES.md`) and
the source inventory from the #6281 findings. The 10x run
([#13087](https://github.com/developing-today/code/issues/13087)) replaces
derived rows with measured-at-tier rows; the 100x run
([#13088](https://github.com/developing-today/code/issues/13088)) adds the
100x column or an explicit wall per row.

First 10x run status (2026-11-25, ci-runner class — 4-core
GitHub-hosted runner, not the reference laptop): the fleet workload at
the full tier configuration reached a recorded wall before its load
phase — the hub's 256 concurrent-lease ceiling (rows below; follow-up
[#15056](https://github.com/developing-today/code/issues/15056)) — and
completed at the cap the default hub admits. The reference-laptop 10x
fleet run is still outstanding; the rows from this run name their
machine class and are never compared across classes.

A separate reference-laptop run (#13087) has since landed the
measured stress-profile rows below (memory, join, frame,
generation) and recorded its own many-worlds admission wall;
those rows name the reference laptop as their machine class.

| Limit                                                     | Value                                                                                                                                                                                                                                                                                                                                                                    | Kind     | Source                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Journal append + fsync                                    | 5,180 µs median per append                                                                                                                                                                                                                                                                                                                                               | measured | `benches/baseline.txt`                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Durable append ceiling, per world                         | ≈193 appends/s unbatched                                                                                                                                                                                                                                                                                                                                                 | derived  | arithmetic on the row above                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Checkpoint compaction                                     | 10,972 µs median per compact, 2,000-entry journal                                                                                                                                                                                                                                                                                                                        | measured | `benches/baseline.txt`                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Checkpoint compaction peak memory                         | 5.17 MiB over pre-op RSS at 2,000 entries; 89.36 MiB at 100,000 entries                                                                                                                                                                                                                                                                                                  | measured | #13087 `stress_mem`, reference laptop, median of 5 fresh processes; recover via `FileJournal::open`, restore, hold resident, compact to 64 entries; spike = peak RSS during operation − RSS just before it                                                                                                                                                                                                                                                 |
| Resident world RSS, 1x tier                               | 21,834 B per world blended (1,101 worlds held, 28.56 MiB total RSS); idle 11,350 B/world; active (100 events) 93,593 B/world; hot world (10k events, 32 members) +3,280,896 B                                                                                                                                                                                            | measured | #13086 `stress_mem` profile, reference laptop; full tier resident set held                                                                                                                                                                                                                                                                                                                                                                                 |
| Resident world RSS, 10x tier                              | +17,151 B/world blended over process start (11,001 worlds held); idle +8,039 B/world; active (100 events) +87,474 B/world; hot world (100k events, 256 members) +20,815,872 B; peak RSS 271,589,376 B                                                                                                                                                                    | measured | #13087 `stress_mem`, reference laptop, medians of 5 fresh processes; no wall, full 11,001-world set held                                                                                                                                                                                                                                                                                                                                                   |
| Guest replay                                              | 0.170 µs/event; 10,000 events ≈ 1.7 ms                                                                                                                                                                                                                                                                                                                                   | measured | `benches/baseline.txt`                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Session fan-out                                           | 1.437 µs per delivery, 32 clients × 100 rounds                                                                                                                                                                                                                                                                                                                           | measured | `benches/baseline.txt`                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Many-worlds command p99                                   | 4,549 ms vs 50 ms target — **missed**                                                                                                                                                                                                                                                                                                                                    | measured | #557 (later run 5,650 ms)                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Concurrent active worlds per hub, default limits          | 256 concurrent leases; the 257th is refused (`TooManyWorlds`) — eviction frees only unleased durable worlds, so the 10x tier's 1,000 active worlds cannot assemble under default limits                                                                                                                                                                                  | measured | `many_worlds` 10x attempt, ci-runner class: 5 of 5 reps at 1,000 active failed at assembly with "server has too many worlds open" (fleet-command run 45598189927, 2026-11-25); boundary run at 257 active failed the same way (run 45604414422). Codified side: `HubLimits::default().max_open_worlds = 256`, `crates/id-worlds/src/world_hub.rs`; the bench builds its hub with the default, `id serve` exposes it as `world_max_open`. Follow-up: #15056 |
| Many-worlds fleet run, 10x tier (1,000 active)            | **wall at assembly** — refused at the row above before the load phase; no load numbers exist at the tier configuration yet                                                                                                                                                                                                                                               | measured | same runs as the row above; per the run protocol the wall is the tier's measured limit until #15056 lands and the tier is re-run                                                                                                                                                                                                                                                                                                                           |
| Many-worlds at-cap run, 10x idle population (256 active)  | p50 211,217 ms / p99 1,007,153 ms vs 50 ms target — **missed**; completed 49.7/s of 5,120 offered; the 10 s load phase took 1,030 s to drain, wake-dominated (wake p99 999,145 ms) on the venue's 4 cores; checkpoints / evicted / reopened 256 / 256 / 256                                                                                                              | measured | `many_worlds` profile, 10,000 idle / 256 active, ci-runner class (4-core GitHub-hosted runner), fleet-command run 45604414422 rep 1, 2026-11-25, 1 completed rep (further reps cut off by the dispatch time limit). Venue-bound: 256 actors on 4 cores starve the runtime; decides nothing about the laptop class. Record: `benches/BASELINES.md`, "First 10x stress run" section                                                                          |
| Journal append under at-cap 10x-idle load                 | 459 µs mean over 51,712 appends — the append path is not the at-cap bottleneck on this venue; the scheduling tail above is                                                                                                                                                                                                                                               | measured | `profile many_worlds` line of the at-cap run above (its command-latency percentiles saturated the actor histogram's overflow bucket and printed `u64::MAX` unmarked — #15094)                                                                                                                                                                                                                                                                              |
| `fuel_spin_isolation` ratio                               | ~3.8–3.9 vs 1.10 target — **missed**                                                                                                                                                                                                                                                                                                                                     | measured | #1067 line, 2026-11-22 post-admission pairs                                                                                                                                                                                                                                                                                                                                                                                                                |
| Guest frame, Territory-class 2 seats, 10k applied events  | view 958,616 fuel / 75.7 µs median wall; layout 958,683 fuel / 76.3 µs median wall                                                                                                                                                                                                                                                                                       | measured | #13087 `frame_cost`, reference laptop, median of 5 fresh processes; fuel unchanged from corrected #13085 measurement                                                                                                                                                                                                                                                                                                                                       |
| Guest frame, Territory-class 2 seats, 100k applied events | view 958,616 fuel / 79.3 µs median wall; layout 958,683 fuel / 79.1 µs median wall                                                                                                                                                                                                                                                                                       | measured | #13087 `frame_cost`, reference laptop, median of 5 fresh processes                                                                                                                                                                                                                                                                                                                                                                                         |
| Guest frame, Territory-class 2 seats, 1M applied events   | view 958,616 fuel / 76.7 µs median wall; layout 958,683 fuel / 76.7 µs median wall — flat across all three sizes, each uses about 9.6% of the 10M default fuel budget                                                                                                                                                                                                    | measured | #13087 `frame_cost`, reference laptop, median of 5 fresh processes                                                                                                                                                                                                                                                                                                                                                                                         |
| Cold join, 1x tier                                        | hot world (10,000 events, 32 members): p50 4.608 ms / p95 5.057 ms / p99 5.057 ms at a checkpoint boundary; p50 7.312 ms / p95 8.475 ms / p99 8.475 ms mid-interval (255-event tail). Active worlds (100 events, 100-world population): p50 0.192 ms / p95 0.222 ms / p99 0.300 ms at boundary; p50 0.189 ms / p95 0.233 ms / p99 0.250 ms mid-interval (100-event tail) | measured | `stress_join` profile (#13084) in `benches/world_bench.rs`, reference laptop 2026-11-23; fixture seed 22673, manifest `hot_state_hash=cf92e013f46e3d2c`; hot world 5 reps, active worlds one join per world; every join verified against the manifest's expected state hash                                                                                                                                                                                |
| Warm join, 1x tier                                        | hot world: p50 0.868 ms / p95 0.971 ms / p99 0.971 ms at boundary; p50 0.846 ms / p95 0.927 ms / p99 0.927 ms mid-interval. Active worlds: p50 0.033 ms / p95 0.039 ms / p99 0.045 ms at boundary; p50 0.020 ms / p95 0.032 ms / p99 0.037 ms mid-interval                                                                                                               | measured | `stress_join` profile (#13084), same run as the cold-join 1x row above                                                                                                                                                                                                                                                                                                                                                                                     |
| Cold join, 10x tier                                       | hot world (100,000 events, 256 members): p50 3.245 ms at a checkpoint boundary / 5.199 ms mid-interval (255-event tail); active worlds (100 events, 1,000-world population): 106.0 µs boundary / 124.3 µs mid-interval                                                                                                                                                   | measured | #13087 `stress_join`, reference laptop; median of five process-level medians, 5 hot-world joins and 1,000 active-world joins per process; every join validated against manifest state hash                                                                                                                                                                                                                                                                 |
| Warm join, 10x tier                                       | hot world: 464.6 µs boundary / 464.0 µs mid-interval; active worlds: 14.7 µs boundary / 14.5 µs mid-interval                                                                                                                                                                                                                                                             | measured | #13087 `stress_join`, same five-process run; hot world remained at the 256-participant cap and the warm setup admitted joiners by revoking one roster member                                                                                                                                                                                                                                                                                               |
| Journal size cap                                          | 64 MiB per world journal                                                                                                                                                                                                                                                                                                                                                 | codified | `MAX_JOURNAL_BYTES`, `crates/id-worlds/src/world_store.rs:51`                                                                                                                                                                                                                                                                                                                                                                                              |
| Session checkpoint interval                               | 2,000 ms default                                                                                                                                                                                                                                                                                                                                                         | codified | `crates/id-store/src/session_checkpoint.rs`                                                                                                                                                                                                                                                                                                                                                                                                                |
| Documents                                                 | 64 KiB each / 512 list items                                                                                                                                                                                                                                                                                                                                             | codified | #6281 findings, limits inventory                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Compile inputs                                            | 8 files / 256 KiB total                                                                                                                                                                                                                                                                                                                                                  | codified | #6281 findings, limits inventory                                                                                                                                                                                                                                                                                                                                                                                                                           |
| SSH surface                                               | 64 connections / 16 tunnel streams                                                                                                                                                                                                                                                                                                                                       | codified | #6281 findings, limits inventory                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Participants per world                                    | 256                                                                                                                                                                                                                                                                                                                                                                      | codified | `WorldLimits::default().participants`, `crates/id-worlds/src/world/mod.rs` — found by the #13083 generator's restore verification (the 10x tier's 320-member hot world is refused); the generator caps the hot world at 256 and records the wall in its manifest                                                                                                                                                                                           |
| Hot-world participant target, 10x                         | 320 requested; generation stopped at the codified 256-participant limit (one recorded wall; cap unchanged)                                                                                                                                                                                                                                                               | measured | #13087 `stress_gen`; fixture manifest seed 22673, 11,001 worlds, 200,000 events, hot-state hash `c7c4fb2805ee72ed`                                                                                                                                                                                                                                                                                                                                         |
| Many-worlds command, 10x                                  | Wall after 89.76 s: `server has too many worlds open; try again shortly`                                                                                                                                                                                                                                                                                                 | measured | #13087 reference-laptop attempt (10,000 idle / 1,000 active worlds, 20 events/s per active world, 10 s duration); admission wall, no latency result; exact command/status/duration in `pkgs/id/benches/results/2026-11-25T10x-reference-laptop/jobs.json`                                                                                                                                                                                                  |
| Per-world caps on entities / programs                     | **none codified**                                                                                                                                                                                                                                                                                                                                                        | gap      | #6281 findings, Finding 1 (the members part of that finding is superseded by the row above)                                                                                                                                                                                                                                                                                                                                                                |

## Host classes

- **Laptop-class:** the fleet reference laptop (spec above). All committed
  baselines are laptop-class; tier runs report against it first.
- **Server-class:** any single host with at least the laptop's thread
  count and NVMe-class storage, recorded through the run manifest's
  `machine_class` field. The 100x run is server-class. Numbers are never
  compared across classes; each class keeps its own table column.

## Run protocol

- Fixed seed per tier; the manifest is committed with the results.
- `--quick` smoke first; a full tier run is 5 reps, medians reported, in
  the established baseline file format with the machine header
  (`benches/BASELINES.md` documents the re-baseline flow).
- A run that hits a wall — OOM, eviction storm, p99 blowup, fuel
  exhaustion — is a result: the wall is recorded as that tier's measured
  limit with the measurement that proves it. It is not a failed run.
- Results land under `pkgs/id/benches/`; this table is updated in the same
  PR that lands a tier run.

## Breakdown

Filed with this design and linked from #6304:

1. [#13083](https://github.com/developing-today/code/issues/13083) — stress
   generator: seeded tier worlds + manifest in `world_bench`.
2. [#13084](https://github.com/developing-today/code/issues/13084) —
   join-time benches: cold and warm join at tier sizes.
3. [#13085](https://github.com/developing-today/code/issues/13085) — frame
   cost vs state size: view fuel and wall time at 10k / 100k / 1M events.
4. [#13086](https://github.com/developing-today/code/issues/13086) — memory
   profile: RSS per resident world, compaction spike at 100k entries.
5. [#13087](https://github.com/developing-today/code/issues/13087) — first
   10x stress run; updates this table from derived to measured-at-tier.
6. [#13088](https://github.com/developing-today/code/issues/13088) — 100x
   feasibility run; gated on #13087 and on
   [#8239](https://github.com/developing-today/code/issues/8239)
   (group-commit batching).

## Out of scope — handed to v2 (#6305)

- The degradation ladder (what sheds first: presence detail, history
  depth, program fuel — and who decides). The findings establish it is not
  codified anywhere today; naming it is v2 design work.
- Growth policy around the ≈193 appends/s durable ceiling.
- Any production-code performance fix. This program measures and records;
  fixes get their own issues (#8239 is the standing example).
