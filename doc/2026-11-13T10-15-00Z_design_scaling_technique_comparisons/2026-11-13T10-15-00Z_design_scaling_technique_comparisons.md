# Scaling-technique comparisons — design decision record

Issue #5895, decomposition item of #5685 (DoS suite & performance
benchmark programme). Programme: epic #2056. Status: **decided.**

This record settles how the two scaling techniques the parent names —
**workers** and **additional/sharded databases** — are compared against
the standard server baseline. It re-decides no parent decision and
neither prerequisite's: the baseline it compares against is #5894's
(`server-baseline/v1.0`: reference machine class AMD Ryzen 7 7840U,
16 threads, 123 GB RAM, NixOS; one server process, one database
instance, default settings; the five-workload suite at scales S/M/L;
artefact and versioning discipline), and the load it compares under is
#5891's (one generator core in `pkgs/id/tests/common/loadgen.rs`;
workloads are data, profiles are parameters; deterministic seeded
fixed-count runs; closed-loop pacing for benchmark mode; one output
contract). Where this record narrows those, it says so. It implements
no worker or sharding code; implementation timing is a separate
decision, as the issue itself notes.

## Decision summary

1. **A technique is a configuration variant, never a new baseline.**
   Every comparison run is a #5894 protocol run whose configuration
   axis is set to a technique arm (below) instead of the baseline, and
   whose artefact is labelled `technique=<arm>@<params>` against the
   baseline version it compares to. #5894 §1 already fixes this
   ("any deviation is a technique run measured against this baseline,
   never a new baseline"); this record names the arms and the reading
   rules.
2. **Two arms, measured singly before jointly.** Arm W (workers) and
   Arm D (additional/sharded databases) are each measured against the
   baseline alone. A combined W×D arm runs only after both singles
   have landed results, so an interaction effect is measured, never
   assumed or attributed by arithmetic.
3. **The full #5894 matrix, every arm.** Each arm runs the complete
   workload × scale × interface matrix. A result that reports only
   the scale a technique was built for is incomplete (#5894 §3).
4. **Adoption thresholds are stated in advance** (§4) and are part of
   the measurement, not a verdict applied to it afterwards.

## 1. The arms — DECIDED

### Arm W — workers

Additional server worker processes sharing the one baseline database:
W ∈ {2, 4, 8} workers behind the same interface surfaces, all other
configuration at baseline defaults. This isolates compute parallelism
from storage scaling: if throughput does not move under W, the
bottleneck is not server CPU, and Arm D's numbers are read in that
light.

- Worker means a process, not a thread pool: in-process thread counts
  are baseline tuning (the baseline server is run as shipped, per
  #5894 §1), not a technique.
- The load-balancing/affinity rule between workers is part of the
  arm's configuration and is recorded in the artefact's config hash;
  changing it is a new arm variant, not a tweak to a reported result.

### Arm D — additional / sharded databases

D ∈ {2, 4} database instances on the same reference machine, with
worlds partitioned across them by a deterministic placement:
`shard = hash(world_id) mod D`, recorded in the run header. One server
process serves all D instances in v1 of the comparison (process count
is Arm W's axis; holding it at the baseline's one process is what
keeps D a storage comparison rather than a second combined arm).

- Placement is a run parameter, not a workload change: the #5894
  suite is run unchanged, with worlds placed per the rule above. The
  world-sync workload at scale L (1,000 worlds resident) is where
  cross-shard placement costs appear; no new workload is added in v1,
  and adding one would be a suite change under #5894 §4(c), making
  old and new numbers incomparable — recorded, not smuggled.
- Cross-world operations follow the existing cross-world access
  design; the comparison measures their cost under sharding, it does
  not redesign them here.

### Arm W×D — combined

Runs only with both singles landed: W ∈ {4} × D ∈ {2, 4} (the
mid-points), full matrix. Its purpose is exactly one question: is the
combined effect the sum of the singles, or does one arm eat the
other? Any other combined cell is out of scope until that is answered.

## 2. Measurement protocol — DECIDED

The #5894 protocol, unchanged, plus the technique labelling:

- **Environment:** the #5894 reference machine class, pinned per run
  (OS/kernel, toolchain, server commit SHA, config hash — the config
  hash is what carries the arm's parameters). A run on any other
  machine class is a smoke/comparability run, never a comparison
  result (CI likewise, per #5894 §1).
- **Load:** the #5891 generator, honest profiles only, closed-loop
  pacing (benchmark mode). Open-loop (DoS-mode) numbers are never
  compared with closed-loop results (#5891 §1). Same seed set for
  baseline and arm runs of a cell: the op stream of a cell is a pure
  function of (seed, workload, scale), so a cell diff is a
  configuration diff, not a traffic diff.
- **Validity:** a run where the generator saturates is invalid, not a
  server result (#5894 §1). A run whose client-side integrity checks
  fail (e.g. blob verification) is an error, not a slow success
  (#5894 §2).
- **Reporting per cell:** throughput, p50/p95/p99 latency, and CPU
  per operation — each reported as the arm's value, the baseline's
  value, and the ratio. Raw throughput alone is not a comparison
  result (#5894 §2).

## 3. How results are interpreted — DECIDED

- **Per-cell reading.** A technique is said to _win_ a cell only when
  its throughput ratio is > 1 **and** its p95 latency ratio is ≤ 1:
  capacity bought by queueing is not capacity. A cell where
  throughput rises and p95 rises with it is recorded as a
  _trade-off cell_, named as such in the summary — never averaged
  away across the matrix.
- **No matrix-wide single number.** Results are reported as the full
  matrix of cells with wins / trade-offs / losses marked. Collapsing
  the matrix into one speedup figure is rejected: it hides exactly
  the S-scale and tail-latency regressions this programme exists to
  catch (§5, alternative 3).

## 4. Adoption thresholds — DECIDED, stated in advance

A technique arm earns an implementation follow-up (a separate design
on #5685, not this record) only when **all** of these hold:

1. Throughput ratio ≥ 2.0 at scale L on at least one workload, against
   `server-baseline/v1.0` (or its then-current successor, named).
2. No p99 latency regression > 10% on any workload at any scale.
3. CPU per operation does not worsen by > 25% on the winning
   workload(s): a technique that buys throughput purely by burning
   proportionally more machine is a capacity purchase, and is
   reported as one, not as a scaling result.

An arm that fails the thresholds is not a failed experiment: the
recorded result _is_ the deliverable, and it closes the question
until a re-baseline event (#5894 §4) or a stated change in the
workload suite reopens it. Thresholds are not revised after results
are seen; revising them requires a new record that says it is doing
so and why.

## 5. Alternatives rejected

1. **A new baseline per technique** — rejected: #5894 §1 fixes the
   baseline as one standard server; per-technique baselines destroy
   the comparison the programme is for.
2. **Cluster or cloud comparison first** — rejected for the same
   reason #5894 §7 rejected them as the baseline environment:
   comparability lives on the reference machine class; multi-machine
   effects are a later programme item, not this comparison.
3. **Comparing only at scale L** — rejected: the techniques are
   motivated by L, but their costs (coordination, placement,
   cross-shard traffic) land at every scale, and #5894 §3 makes the
   full matrix the unit of a complete result.
4. **Implement the sharding, then measure** — rejected: that inverts
   the programme. This record exists so the implementation question
   is decided by measured comparisons of the cheapest faithful
   configurations, not by sunk cost.
5. **Per-technique bespoke harnesses** — rejected: #5891's one
   generator is the comparison's fairness guarantee; a harness per
   arm makes every diff a harness diff first.
6. **Thread-count scaling as the workers arm** — rejected as an arm
   (see §1): it is inside the baseline server's own tuning surface.
   If a future record wants it measured, it enters as its own arm
   under this protocol, not as a redefinition of W.

## 6. Invariants implementation must hold

1. Same environment + same baseline version, or the comparison is
   labelled incomparable — never a silent percentage (#5894 §8.1,
   carried over).
2. A comparison run is replayable from its header: seed, profile,
   workload, scale, surface, arm, and arm parameters (#5891 §4's
   determinism rule, extended with the arm fields).
3. The baseline server's defaults are never changed by a technique
   arm; an arm is a run configuration, and the shipped defaults
   remain the baseline's.
4. Shard placement is a pure function of the world id and D, stated
   in the run header; no run may hand-place worlds to flatter a cell.
5. The generator never targets a shared or production server
   (#5894 §8.3, carried over).
6. Adoption thresholds (§4) are evaluated as written, against the
   baseline version named in the artefact; a waiver names that
   version (#5894 §8.4).

## 7. Assumptions and dependencies

- **#5891 (generator) — design landed, implementation pending.** The
  comparisons in this record run when `loadgen` exists; until then
  this record is the protocol those runs will follow. Nothing here
  forks #5891's decisions: workloads-as-data, three surfaces, and
  the output contract are consumed as-is.
- **#5894 (baseline) — design landed; `server-baseline/v1.0` first
  stored run pending.** Arm runs require a stored baseline to ratio
  against; if the baseline's first stored run and an arm's first run
  happen in the same session, the baseline is stored first and the
  arm's artefact names it.
- **#5896 (per-interface matrix)** consumes this record's cells per
  interface; **#5897 (gate budgets/waivers)** governs baseline
  regressions, not technique adoption — adoption is decided by §4
  and recorded on #5685 when results land.
- **Parent #5685:** no parent decision is changed by this record. If
  running these comparisons appears to require one, that change goes
  on #5685 explicitly, not here (#5894 §6's rule, adopted).

**Verification performed for this record (2026-11-13):** issues
#5895, #5891, #5894, and #5685 read fresh; #5891's record
(`doc/2026-11-08T11-27-00Z_design_shared_workload_generator/`) and
#5894's record (decision record on issue #5894, task 12854) read in
full; both prerequisites verified CLOSED with `status:done`.
