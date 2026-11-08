# Decision: scaling-technique comparisons — workers and additional/sharded databases vs the standard baseline

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5895](https://github.com/developing-today/code/issues/5895),
  "id: design: scaling-technique comparisons (from #5685)".
- **Parent design:** [#5685](https://github.com/developing-today/code/issues/5685)
  ("design(id): DoS suite & performance benchmark programme"). Its
  Part 2 decides the standard baseline, the advanced-scaling
  measurement discipline, the per-interface requirement, and the
  gate rules. **This record fills in one cell of that plan** — the
  protocol for comparing scaling techniques against the baseline —
  and re-decides none of the parent's choices. Where it would touch
  one, §1 and §10 say so explicitly.
- **Inputs cited:**
  - [#5891](https://github.com/developing-today/code/issues/5891) —
    the shared load & workload generator. **Prerequisite; not
    landed** (OPEN, `task/triage`). §8 records the assumption this
    record makes and what changes when it lands.
  - [#5894](https://github.com/developing-today/code/issues/5894) —
    the standard server baseline (reference environment, workload
    suite, scales, versioning). **Prerequisite; not landed** (OPEN,
    `queue/queued`, task 13054). §8 likewise.
  - [#5896](https://github.com/developing-today/code/issues/5896) —
    the per-interface benchmark matrix. Sibling consumer; §7 states
    the boundary between its matrix and this record's protocol.
  - [#5897](https://github.com/developing-today/code/issues/5897) —
    benchmark regression gates (budgets, noise model, waivers).
    Sibling consumer; §9/I8 keep thresholds out of this record.
  - In-tree evidence: `pkgs/id/benches/BASELINES.md` (the F11.8
    reference machine, regeneration procedure, non-blocking >20%
    reporting), `pkgs/id/benches/baseline.txt` (the five committed
    lines), `pkgs/id/benches/world_bench.rs` (the hand-rolled
    harness), `doc/2026-11-01_design_id557.md` §"F11.8 currently
    reports four microbenchmarks" and its step 5 (many-world
    benchmark and recorded result), and
    `doc/2026-10-30T19-35-00Z_investigation_hot_world_sharding/`
    (the Durable Object ceiling and the sharding escalation ladder).
  - The parent carries no landed standard-server or generator code:
    `git grep -il "standard server\|load generator\|reference environment"`
    over `doc/` and `pkgs/id/` at this baseline returns nothing
    beyond the parent issue text itself.
- **Evidence baseline:** `origin/main` at `0a27a7f19f`, worktree
  `~/git/fleet/12866`, 2026-11-08. Verified at that baseline:
  `pkgs/id/benches/` contains only `BASELINES.md`, `baseline.txt`,
  `world_bench.rs`; there is no load-generator or scaling-comparison
  harness; no PR links from #5895. This record is the first design
  for the comparison protocol.

## Decision summary

| Question | Decision |
| --- | --- |
| What a comparison *is* | A **paired** run: the same workload against the **standard baseline** [#5894] and against **one technique configuration**, in the same environment, with exactly one scaling dimension changed (§3) |
| Reporting unit | A **distribution plus cost**, never a mean: p50/p95/p99, throughput, and resource cost (CPU/op, bytes, connections), plus an operational-complexity note (§4.4, §7) |
| "Workers" means two different things | **Process/thread/async capacity** of one server *and* the **Cloudflare Worker / Durable Object isolate** of the hosted data plane. The record names both and keeps their curves separate (§5) |
| Workers measurement | The **scaling curve**: sweep worker/concurrency counts, report the curve and its **knee** (the count after which marginal gain < cost) (§5) |
| Database measurement | **Read replicas** (read-split gain, replication-lag visibility) and **additional / sharded storage** (which workloads actually partition, the cross-shard cost, the consistency-visible effects) (§6) |
| Honest outcomes | "No gain" and "regressed" are **recorded results**, not failures; every cell reports **gain, cost and complexity** (§4.4, §10) |
| Where the protocol runs | On the **standard baseline environment** [#5894]; CI stages reuse the same generator [#5891] with the same config, never a bespoke load tool (§4.1, §10) |
| Thresholds | **Not set here.** This record fixes *measurement*; budgets, noise model and waivers are [#5897] (§9/I8) |
| The matrix | This record fixes the **protocol**; the interface × workload × configuration matrix and its publication are [#5896] (§7) |

## 1. What the parent already fixed (not re-decided here)

#5685 Part 2 fixes, and this record takes as given:

1. **One standard server is THE baseline.** A single fixed,
   documented reference configuration (hardware class or CI runner
   class, server build, single database, default settings) is what
   every result is compared against, and changing it is a versioned
   event, never a silent drift. Owned by [#5894]; this record
   consumes the baseline's identity (§7).
2. **Advanced techniques are measured as baseline-vs-technique on
   identical workloads**, reporting **gain, cost and complexity
   honestly — including "no gain" results**. The parent names
   workers, additional / sharded databases, and (admitted only
   under the same discipline) caching, pooling and batching. This
   record covers the two named techniques (§5, §6); the others
   inherit its protocol (§9/I3).
3. **Per-interface**: every network-touching benchmark runs per
   transport / protocol surface on identical workloads, so interface
   choice becomes a measured fact. The matrix is [#5896]; the
   protocol is here.
4. **Gates, thresholds, noise model, waivers and re-baselining**
   are the parent's, and are designed by [#5897]. This record
   produces the numbers gates are later set from; it sets none
   (§9/I8).
5. **Shared infrastructure**: one load/workload generator framework
   serving honest and adversarial profiles (owned by [#5891]), one
   results store and reporting format, and an environment
   specification strict enough that "works on my runner" is
   diagnosable. This record writes the protocol in terms of that
   generator and that environment, and defines neither.
6. **Non-goals stand**: no production load testing; finding DoS
   weaknesses is not this programme's job; no competitor
   benchmarking; not a general observability platform.

The parent's **open question** "is the sharded configuration a real
roadmap item or a benchmark-only curiosity?" is answered by
*measurement* under §6, not by assertion here — this record's job is
to make that question answerable.

## 2. Scope: what counts as a "scaling technique"

A **scaling technique** is a change to the *deployment or resource
shape* of the running `id` server — not a change to its logic. The
comparison asks: **for a fixed honest workload, does changing the
shape buy throughput / tail latency, and at what cost?**

In scope:

- **Worker capacity** — more worker processes / threads / async
  concurrency serving one logical server (§5).
- **Additional / sharded databases** — extra database instances,
  read replicas, and sharded storage behind the server (§6).
- Later techniques the parent admits under the same discipline
  (caching layers, connection pooling, batching) — they adopt this
  protocol unchanged (§9/I3).

Out of scope:

- Implementing workers, sharding, or any production scaling code
  (the packet forbids it; this is a decision record).
- Re-deciding the standard baseline [#5894] or the generator
  [#5891]; this record states explicit **assumptions** instead
  (§8).
- The full interface matrix [#5896], the DoS catalogue [#5892],
  CI tiering [#5893], and gate thresholds [#5897].
- Adversarial load: that is [#5892]'s programme, sharing the
  generator but not this protocol's honest-workload intent.

## 3. The comparison shape

### 3.1 Paired, same-session, one variable

A valid comparison is a **pair** of runs:

- **B** — the standard baseline configuration [#5894];
- **T** — the same server build with **exactly one** scaling
  dimension changed (e.g. worker count 1→4, or one database→one
  primary + one read replica).

B and T run:

1. the **same workload** (same generator config [#5891], same
   profile, same scale), and
2. in the **same environment session** (same machine class, same
   build SHA, same DB engine build, same warm-up), and
3. **interleaved** (B, T, B, T …) across repetitions, so slow drift
   in the host cancels rather than loading onto whichever ran last.

The measured quantity is the **difference** T − B on the workload's
metrics, not either run's absolute value (§4.4).

### 3.2 Why paired, not absolute

The existing F11.8 numbers are explicitly **machine-class-specific**:
`BASELINES.md:18-23` says the fsync benchmarks "measure this
machine's storage as much as the code", and `BASELINES.md:90-95`
warns that CI runners are a different class from the laptop
reference. An absolute number from one run cannot answer "does
adding workers help?" — only a paired, same-environment difference
can. This is the same reason the parent forbids "raw throughput
numbers that flake between runners".

### 3.3 One variable per cell

Each comparison cell changes one dimension. Bundling two (e.g.
workers **and** a shard in one cell) makes the result unattributable
and is rejected (§10). If a technique genuinely requires a
companion change (a shard needs a routing decision), that companion
is held identical between B and T and named in the result artefact
(§7.2).

## 4. The measurement protocol

The protocol is stated generically so it binds whichever concrete
environment, workload suite and generator [#5894]/[#5891] land;
where those are still open, §8 names the assumption.

### 4.1 Environment

- **One reference environment**, the standard baseline [#5894],
  is the environment of record for a comparison. CI-stage repeats
  reuse the *same* generator and config [#5891] but are **not** the
  reference and are compared only within their own class
  (§9/I11).
- **Warm-up is mandatory and excluded from samples**: the server is
  brought to steady state (connections, caches, JIT-warm code
  paths) before measurement begins, matching the harness discipline
  already used by `many_worlds` (`BASELINES.md:136-137`, per-world
  phases staggered to avoid artificial bursts).
- **Co-tenancy is recorded**: the reference laptop was under load
  average ~8 during the F11.8 runs yet agreed to ~5%
  (`BASELINES.md:19-23`); the protocol records host load with every
  artefact so a noisy run is diagnosable, not silently averaged in.
- **The target is dedicated and isolated**: never a shared or
  production deployment, matching the parent's blast-radius
  non-goal (§9/I10).

### 4.2 Workload

- **One workload suite**, defined by [#5891] (honest workloads:
  identity operations, capability checks, journal append/read,
  world sync, blob transfer) at defined scales (single user, small
  community, large world) — the parent's list.
- **Identical between B and T**, byte-for-byte the same generator
  configuration, including scale, think-time distribution and
  concurrency model.
- The existing `many_worlds` workload is the **in-process
  precedent** for a load-shaped honest workload, and its committed
  result is a load-bearing baseline observation (§8.3).

### 4.3 Statistics

- **Repetitions**: a fixed N (consistent across the pair) with the
  full-size workload; `--quick` smoke sizes are never recorded as
  results — the same prohibition the F11.8 protocol already
  enforces (`BASELINES.md:64-66`, `doc/2026-11-01_design_id557.md`
  step 5).
- **Per-metric reporting**: report the **distribution**, not a
  scalar — at minimum p50, p95, p99 and throughput, plus resource
  cost (§4.4). The parent gates on p95 latency and ops/sec, so a
  mean-only report cannot feed a gate.
- **Variance is published**: the spread across repetitions is part
  of the result artefact, because the noise model [#5897] needs it.
- **Refusals and errors are outcomes**: if the technique changes
  the refusal/error rate, that is recorded and counts against it;
  a throughput gain bought with silent failures is not a gain
  (§9/I5).

### 4.4 Interpreting a cell

Every cell reports three things, per the parent's "gain, cost, and
complexity honestly":

1. **Gain** — the delta on each workload metric (throughput,
   p50/p95/p99), direction and magnitude, with the repetition
   spread.
2. **Cost** — resource cost of obtaining it: CPU per operation,
   peak/steady memory, bytes on the wire, connection count,
   storage/objects added, and any new failure mode.
3. **Complexity** — operational cost: new components to run,
   new consistency or routing concerns, new configuration
   surface.

The **knee**, where a sweep is involved, is the point past which
additional units raise cost faster than gain — reported as a point
on the curve, not a single "best" number (§5).

## 5. Workers — the scaling curve

### 5.1 Two things called "worker"

The repo uses "worker" for two unrelated objects; conflating them
would make the comparison meaningless, so the record names both:

- **(a) Server worker capacity** — processes / threads / async
  concurrency serving one logical `id` server. The native host runs
  in-process (tokio; `world_bench.rs` drives it in one process,
  `BASELINES.md:124-137`), and the sharding investigation says the
  native ceiling is "fsync latency × command cost, not core count"
  (`doc/2026-10-30T19-35-00Z_investigation_hot_world_sharding/…:81-82`).
  This is the dimension the parent's "additional worker processes /
  threads / async capacity" names.
- **(b) Cloudflare Worker / Durable Object isolate** — the hosted
  data-plane execution unit (`pkgs/id/hosted-worker/README.md`,
  `pkgs/id/worker-poc/README.md`). Its scaling story is
  **per-object**, bounded by the Durable Object soft ceiling
  (~1,000 req/s) and 10 GB storage cap
  (`doc/2026-10-30T19-35-00Z_investigation_hot_world_sharding/…:59-75`),
  and its remedy is sharding/partitioning, not "more workers".

### 5.2 What to measure

- **Dimension (a): the concurrency curve.** Sweep the server's
  worker/concurrency count (1, 2, 4, 8, … up to the reference
  machine's capacity, recorded per point) at a fixed honest
  workload and fixed database shape. Report throughput and
  p50/p95/p99 **per point**, and the **knee** where marginal gain
  drops below cost. This answers the parent's "scaling curve as
  workers are added, and the point of diminishing returns"
  directly.
- **Dimension (b): the isolate-capacity fact.** Measure how a
  single Durable Object behaves as offered load approaches its
  ceiling, and how a *partitioned* set of objects (the sharding
  ladder below) compares to one object — this is the DO half of
  §6, not a "worker count" sweep.

### 5.3 The result is a curve, not a number

A worker sweep produces a **curve plus a knee**, and the record
rejects "the best number wins": a configuration that maximises
throughput while multiplying cost (CPU, memory, fsync contention)
is reported with that cost beside it (§4.4).

## 6. Additional / sharded databases

### 6.1 The three database shapes to compare

1. **One database** (the baseline) — the standard server's single DB
   [#5894].
2. **Read replicas** — read traffic split off the primary. Measure:
   read-throughput gain, primary offload, and **replication lag**
   as a consistency-visible effect (a stale read is a correctness
   observation, not a latency one).
3. **Additional / sharded storage** — multiple primaries or sharded
   storage. Measure: which workloads actually partition, the
   **cross-shard cost** (fan-out, coordination, cross-shard
   transactions), and the consistency-visible effects.

### 6.2 The sharding precedent the record builds on

The hot-world sharding investigation already defines an escalation
ladder and trigger signals for the hosted data plane
(`doc/2026-10-30T19-35-00Z_investigation_hot_world_sharding/…`):

- the **~1,000 req/s soft ceiling and 10 GB cap** per Durable
  Object (:59-75);
- the ladder **shed load → partition by topic/channel → partition
  by region (sibling shard-worlds)** (:34-52, :121-164);
- **trigger signals already in `WorldMetrics`** — sustained command
  rate vs ceiling, journal-append p95, fan-out growth, write
  locality (:99-105), with the explicit warning that a workload
  whose writes are all one topic/region **cannot** be sharded
  (:105, :159).

This record **measures on that ladder** rather than inventing a
second one: a sharding comparison is a technique configuration that
implements one rung, measured against the un-runged baseline on the
same workload. It does **not** re-decide the ladder's signals — that
investigation owns them (§11).

### 6.3 The honest first fact

`many_worlds` (100 active worlds, 20 cmd/s each) records
**p99 = 5,650.792 ms against a 50 ms target**, and 1,523.1
completed events/s against 2,000 scheduled
(`BASELINES.md:139-153`, `baseline.txt`). The baseline is therefore
**already failing its own tail-latency target** in the multi-world
regime. A scaling comparison must not obscure that: the protocol
reports the baseline's absolute position on the metric as well as
the technique delta, so "technique T improves p99 by 30%" is never
read as "the system meets 50 ms" when the baseline is 100× off it.

## 7. Result records

### 7.1 One machine-readable artefact per cell

Every comparison cell emits one record carrying: the baseline
identity [#5894] (machine class, build SHA, DB engine build, config
version), the generator config [#5891], the **one** changed
dimension and its value, the workload and scale, N and the
per-repetition samples, the derived p50/p95/p99 and throughput, the
cost metrics, the complexity note, and the verdict-free **gain /
cost** pair. This is the parent's "every run stores machine-readable
results (environment, build, config, workload, raw numbers) so
results are comparable across months".

### 7.2 Boundary with the interface matrix [#5896]

This record defines *how a cell is measured*. [#5896] defines
*which cells exist* (the interface × workload × configuration matrix)
and how the matrix is published. A cell produced by this protocol
is an input to that matrix; neither record re-does the other.

## 8. Dependencies and assumptions (prerequisites not landed)

At drafting (2026-11-08), both prerequisites are OPEN and unlanded:

- **#5891 (shared load & workload generator)** — `task/triage`,
  no generator on main. This record assumes **one** generator
  producing honest workloads as configurations, per the parent's
  "one load/workload generator framework serving both programmes".
  If #5891 instead splits into per-protocol generators, this
  record's "identical workload" requirement (§4.2) becomes
  cross-generator equivalence and needs a conformance check.
- **#5894 (standard server baseline)** — `queue/queued` (task
  13054), no reference-environment record on main. This record
  assumes the baseline defines the reference environment, the
  standard server build, and the versioning scheme (§4.1, §7.1).
  Until it lands, the *protocol* is fully specified but
  **unmeasurable** — there is no baseline identity to pair against
  and no standard workload to run.

**Decision: define the protocol parametrically now; do not fork the
prerequisites.** Reasons, in order of weight:

1. **The protocol is derivable from the parent and the existing
   harness; it does not depend on the exact baseline chosen.** The
   paired/interleaved shape, the one-variable rule, the
   distribution-plus-cost reporting, and the worker curve are
   format- and method-driven. Whoever says "quarterly scaling
   comparison of this build against the standard baseline"
   needs the switching to introduce it, not an argument about each
   run.
2. **The prerequisites are design tickets with their own owners**;
   duplicating their decisions here is exactly the "silently
   fork" the parent forbids.
3. **The cost of being wrong is bounded and additive.** If #5894
   picks a container-of-fixed-limits reference instead of a
   cloud-instance class, or #5891 picks a different workload mix,
   the change lands as an **amendment section** to this record
   (§12) and touches §4.1/§4.2 only — not the shape.

**What would change if a prerequisite lands** is recorded where it
bites: a generator split changes §4.2's equivalence requirement; a
baseline that is *not* one fixed configuration changes §3.1's
"same environment session" and §7.1's baseline-identity field.
Both are single-section amendments.

### 8.1 The existing in-process baseline is not the standard baseline

`pkgs/id/benches/BASELINES.md` + `world_bench.rs` are the **F11.8**
programme (issue #539 / #1512): a single reference laptop, four
in-process microbenchmarks plus `many_worlds`, with >20%
non-blocking reporting and an explicit machine-class caveat. They are
**not** the versioned standard-server baseline the parent requires,
and this record does not treat them as one. They are cited as: the
protocol's machine-class caution (§3.2), the `--quick` prohibition
(§4.3), and the multi-world failure of §6.3.

## 9. Invariants implementation must hold

These are the contract the eventual comparison harness is tested
against; each maps to a section above.

1. **I1 — Paired comparability.** A technique result is valid only
   when produced against a baseline run in the same environment
   session (same machine class, build, DB engine, config). Cross-
   session or cross-machine-class pairs are **refused**, mirroring
   `id-bench-compare.sh`'s unit-change refusal
   (`BASELINES.md:59-63`).
2. **I2 — Identical workload.** B and T run byte-identical generator
   configuration; only the scaling dimension differs.
3. **I3 — One variable per cell.** Exactly one scaling dimension
   changes between B and T; any required companion change is held
   identical and named in the artefact. Later techniques (caching,
   pooling, batching) adopt this unchanged.
4. **I4 — Distribution, not mean.** Every cell reports p50/p95/p99,
   throughput and per-repetition spread; a scalar mean is never the
   whole result.
5. **I5 — Honest cost.** Every cell reports gain, cost and
   complexity together; resources, storage, wire bytes and new
   failure modes are part of the cost, not footnotes. A gain bought
   with silent refusals is not a gain.
6. **I6 — Negative results recorded.** "No gain" and "regressed" are
   first-class recorded outcomes, never dropped or hidden in an
   average.
7. **I7 — Environment metadata mandatory.** Every artefact carries
   the baseline identity [#5894], generator version [#5891], host
   load, and the exact changed-dimension value. An artefact without
   them is invalid.
8. **I8 — Measurement, not gates.** No threshold, budget, noise
   model, waiver or re-baselining rule is set here; those are
   [#5897]'s, applied on top of these numbers.
9. **I9 — Interface-parameterised.** Any network-touching
   measurement is run per interface; the protocol binds, the matrix
   is [#5896].
10. **I10 — Blast-radius safety.** No comparison is runnable against
    a shared or production target; guard rails and dedicated
    fixtures are required, matching the parent's non-goal.
11. **I11 — No cross-class comparison.** CI-stage repeats are
    compared only within their own runner class; the reference
    environment is the standard baseline [#5894].
12. **I12 — Reproducibility.** Every artefact is sufficient to
    re-run the same B/T pair and expect the same verdict within the
    recorded spread.

## 10. Alternatives rejected, and why

| Alternative | Why rejected |
| --- | --- |
| **Absolute single-run numbers** (measure the technique, compare to a remembered figure) | Not comparable: the existing baselines are explicitly machine-class-specific (`BASELINES.md:18-23`), so an unpaired number answers no question. The parent requires baseline-vs-technique on identical workloads. |
| **Mean-only reporting** | The parent gates on p95 latency and ops/sec; a mean hides the tail that is the whole point of the scaling question. Distribution-plus-cost is required (§4.4). |
| **Running each technique in its own bespoke harness** | Duplicates [#5891]'s shared generator and guarantees the numbers drift into incomparability — the exact failure the parent designs against ("the load-generation … machinery is largely the same"). |
| **Using CI runners as the primary reference** | Runner classes differ and drift; the parent makes the standard baseline [#5894] the environment of record and this record's I1/I11 refuse cross-class pairs. |
| **Bundling workers + sharding in one comparison** | Makes the delta unattributable (I3). One variable per cell. |
| **Choosing the highest-throughput configuration** | Ignores cost and complexity; the parent demands gain **and** cost **and** complexity. The worker sweep reports a *curve and its knee*, not a winner (§5.3). |
| **Dropping "no gain" results** | The parent explicitly requires including "no gain" results; suppressing them is the dashboard-decoration failure it names. I6 forbids it. |
| **Setting pass/fail thresholds here** | The parent owns gate rules and assigns them to [#5897]; setting them before the baseline and noise model exist recreates the flaky raw-throughput gate the parent forbids. I8 keeps them out. |
| **Inventing a second sharding ladder** | The hot-world sharding investigation already defines the ceiling, the ladder and the trigger signals (`doc/2026-10-30…hot_world_sharding/…`); this record measures on that ladder rather than forking it. |
| **Waiting for #5891/#5894 before drafting** | Both are unlanded design tickets. The protocol is method-driven and independent of their exact choices; §8 records the assumptions and the bounded amendment path. Waiting means the protocol does not exist. |

## 11. What this record leaves open (and to whom)

| Question | Owner | Why not here |
| --- | --- | --- |
| The concrete environment, standard server build, workload suite, scales, and versioning scheme | [#5894] | This record assumes them (§8); defining them here would fork the baseline |
| The concrete generator, workload profiles and adversarial profiles | [#5891] | Consumed as "the generator"; the conformance question if it splits is noted (§8) |
| The interface × workload × configuration matrix and its publication | [#5896] | This record fixes the cell protocol; the matrix is its own design |
| Thresholds, noise model, waiver and re-baselining procedure | [#5897] | Gates sit on top of measurement (I8) |
| The specific worker counts / concurrency points to standardise | [#5894] + [#5896] | Depends on the reference machine and the matrix |
| Sharding trigger thresholds and the ladder's exact signals | `doc/2026-10-30…hot_world_sharding/…` follow-ups | This record measures on the ladder; the ladder's own design owns its triggers |
| Whether sharding is a roadmap item or a benchmark-only curiosity | the measurement itself (parent §Open questions) | Answered by running §6, not by assertion |
| Admission of caching / pooling / batching as techniques | [#5685] follow-on | They inherit this protocol unchanged (I3) when admitted |

## 12. Change log

- 2026-11-08 — first recorded decision (task 12866, lane
  `335a5d1f-bfc1-4831-99ea-850c8939a727`, queue
  `repo-health-code-pilot`). Drafted from parent #5685 Part 2, the
  prerequisite issues #5891/#5894, `pkgs/id/benches/BASELINES.md`
  and `baseline.txt`, `doc/2026-11-01_design_id557.md`, and the
  hot-world sharding investigation. Prerequisites recorded as
  unlanded-and-proceeded (§8).
