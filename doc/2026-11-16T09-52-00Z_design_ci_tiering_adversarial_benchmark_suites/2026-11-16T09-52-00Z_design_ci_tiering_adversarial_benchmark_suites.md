# CI tiering for the adversarial and benchmark suites — design decision record

Issue #5893, Decomposition Item for CI tiering of #5685 (DoS suite &
performance benchmark programme). Programme: epic #2056. This record
settles only what #5893 owns: the tier structure both suites run in,
each tier's time budget, what gates and what merely reports, and the
environment guards that keep every tier off shared or production
targets. It does not re-decide the sibling tickets: the shared
generator is #5891 (record landed on main, PR #7662), the standard
server baseline is #5894 (record on issue #5894; first stored baseline
`server-baseline/v1.0`), regression gates, waivers and retention are
#5897 (record landed on main), the per-interface DoS scenario
catalogue is #5892 (record written, in PR #7738 at the time of this
record), the per-interface matrix is #5896, scaling comparisons
#5895. Where a choice here constrains those tickets, the constraint
is stated as an invariant for them to consume, not as their design.

Inputs and current state, verified against `main` at `25a77703e1`
(2026-11-16): `.github/workflows/id-bench.yml` runs the F11.8 bench
suite on push-to-main and manual dispatch only, `timeout-minutes: 60`,
explicitly non-blocking (the bench step is `continue-on-error` and the
compare script always exits 0); `.github/ci-policy.md` classifies id
bench as no-PR / full-main. `pkgs/id/tests/world_dos.rs` is a
consolidated in-process refusal suite — adversarial coverage exists,
but as a unit-tier test file, not as a tiered load workflow. The
#5891 generator record fixes the three target surfaces (`InProcess`,
`LocalSocket`, `Iroh`) and one measurement contract for both kinds of
run. The #5892 catalogue fixes bounded fixture profiles: fixed seed,
30-second attack phase, 30-second recovery window, 128 offered
operations/second open-loop, a 1 valid operation/second canary, and
cap / cap+1 boundary cases where an interface declares a cap — and it
already assigns deadline-bound scenarios (whose pass condition is a
configured deadline longer than the profile, e.g. the SSH ten-minute
inactivity timeout) to an extended profile "in the scheduled/soak
tier" or to a test-only deadline override. The #5894 record fixes
that CI runners are a smoke/comparability environment, never the
reference environment: `--quick`/smoke numbers are never committed as
baselines, and baseline-comparable numbers come only from the
reference machine class.

## Decision summary

1. **One tiering scheme, both suites.** The adversarial suite and the
   benchmark programme share three tiers — **smoke**, **scheduled**,
   **soak** — exactly the shape the parent sketches. A scenario or
   workload is assigned to a tier by the rules in §2, never ad hoc
   per run.
2. **Smoke gates on behaviour, scheduled gates on budgets, soak
   reports and files findings.** Smoke verdicts are server-behaviour
   assertions only (health probe answers, excess rejected, recovery
   inside the window) — never raw throughput, per the parent and
   #5894. Scheduled benchmark verdicts use the #5897 budgets and
   waiver discipline against `server-baseline` versions. Soak
   failures enter the parent's findings loop as hardening follow-ups;
   soak is not a merge gate.
3. **Budgets are caps with teeth.** Each tier has a wall-clock budget
   enforced by the job timeout (§3). A tier that outgrows its budget
   has a contents defect: content moves down a tier or is split — the
   cap is raised only by amending this record.
4. **Environment guards are structural, not conventional.** Every
   tier targets only fixtures the run itself created, through the
   #5891 target surfaces, and the generator refuses anything else
   (§4). No tier configuration can name a shared or production
   target.

## 1. The tiers

| Tier      | Trigger                                                            | Adversarial contents                                                                                                                                                                                  | Benchmark contents                                                                                                                                                                   | Verdict                                                                                                                          |
| --------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| Smoke     | Every PR / push that touches `pkgs/id/**` or the suites' own paths | The in-process refusal suite (`world_dos.rs`) plus, per changed surface, the #5892 canary profile (1 valid op/s canary under a light adversarial modifier) and cap / cap+1 boundary spot checks       | `--quick` micro-benchmarks for changed hot paths (the existing F11.8 `--quick` shape)                                                                                                | Gate (behaviour assertions; bench numbers report-only and never committed as baselines, per #5894)                               |
| Scheduled | Nightly schedule + merge queue                                     | The full #5892 catalogue at its fixed fixture profiles (30 s attack + 30 s recovery per scenario), deadline-bound scenarios via test-only deadline override where one exists                          | The full honest matrix (workload × scale × interface) at #5894 scales; on the reference machine class when the run is baseline-comparable, otherwise labelled a CI comparability run | Gate (DoS behaviour assertions; bench budgets/waivers per #5897)                                                                 |
| Soak      | Weekly schedule + release gate                                     | Extended profiles at production deadlines (deadline-bound scenarios without an override, e.g. SSH inactivity at ten minutes), long-duration fairness / noisy-neighbour and state-exhaustion scenarios | Long-duration honest runs and the release-gate run of the standard server against the current `server-baseline` version                                                              | Report + findings loop (failures file hardening follow-ups per the parent); release-gate bench run gates the release, not merges |

The existing `id-bench` workflow is the seed of the scheduled bench
run, not a fourth tier: its main-tier, non-blocking reporting shape
stays for the F11.8 in-process suite exactly as #5894 records, and
the scheduled gate for server-baseline metrics is a separate verdict
built on #5897's budgets — this record does not retroactively make
the F11.8 reporting blocking.

## 2. Tier assignment rules

A catalogue scenario or benchmark workload is placed by the first
rule that matches:

1. **Deadline-bound** (pass condition needs a configured deadline
   longer than the fixed fixture profile): soak, or scheduled with a
   test-only deadline override where the fixture supports one — the
   choice #5892's record already makes per scenario; the fixture
   manifest records which.
2. **Boundary / canary shape** (cap and cap+1 cases, canary survival,
   refusal correctness): smoke, for the surfaces the change touches;
   the same cases also run in scheduled as part of the full
   catalogue.
3. **Everything else in the catalogue**: scheduled.
4. **Duration outliers** (state-exhaustion, fairness, and any
   scenario whose honest duration exceeds the scheduled per-scenario
   profile by design): soak.
5. **Benchmarks**: `--quick` shapes smoke; full-scale matrix shapes
   scheduled; long-duration and release shapes soak.

## 3. Time budgets

Budgets are wall-clock caps per tier invocation, enforced by job
timeouts; per-scenario cost inside scheduled is already bounded by
the #5892 fixture profiles.

| Tier      | Budget                                      | Basis                                                                                                                                                     |
| --------- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Smoke     | ≤ 10 minutes per PR (job timeout 15)        | Must stay cheaper than the existing required checks it rides with; contents are canary/boundary/`--quick` shapes only                                     |
| Scheduled | ≤ 120 minutes per nightly / merge-queue run | Full catalogue at 30 s + 30 s per scenario plus the full bench matrix; the bench portion keeps the existing 60-minute `id-bench` cap inside this envelope |
| Soak      | ≤ 6 hours per weekly run                    | Deadline-bound scenarios run at production deadlines (ten-minute SSH inactivity is the floor case) plus long-duration fairness/exhaustion profiles        |

A run that hits its job timeout is a failed run, not a slow pass:
the tier's contents are re-assigned per §2 or split, and the failure
is visible in the tier's own reporting. Raising a budget in this
table is an amendment to this record, with the reason stated —
budgets never drift by editing a workflow timeout alone.

## 4. Environment guards

1. **Fixture-only targets.** Every tier runs against targets the run
   itself created: `InProcess`, `LocalSocket` on an ephemeral local
   socket, or `Iroh` endpoints on loopback / a run-scoped test
   network — the #5891 surfaces, nothing else.
2. **Refusal, not convention.** The generator refuses a target that
   is not recognisably a run-created fixture (non-loopback address,
   a target named in configuration rather than created by the
   fixture, a production-shaped identifier). This is #5894's
   invariant — the generator can never target a shared or production
   server — enforced at target construction, so a misconfigured tier
   fails at setup instead of loading a shared system.
3. **Run-scoped state.** Dedicated fixture identities, data
   directories, and ports per run, torn down at run end; no tier
   reads or writes another run's artefacts except through the #5897
   results store.
4. **Soak isolation.** Soak's extended profiles run in the dedicated
   environment (reference machine class or a dedicated host), never
   in the shared CI pool: their durations and deadlines are exactly
   what shared tenancy corrupts, and their failure mode must stay
   attributable.
5. **No secrets, no egress.** Tier jobs hold no production
   credentials and need no network egress beyond the run's own
   fixtures; a scenario that appears to need either is a design
   defect in the scenario, escalated to #5892, not worked around
   here.

## 5. Deferred — explicit dependencies

Two confirmations are deferred, by the acceptance terms of #5893
itself, and are the only parts of this design not final:

1. **The exhaustive per-scenario tier table.** §2's rules assign
   every catalogue scenario, and the deadline-bound placement is
   already decided inside #5892's record; the scenario-by-scenario
   table is confirmed when #5892's record lands (PR #7738, open at
   the time of this record, its checks red on failures this record
   does not own). Until then the rules govern and the table is
   derived, not enumerated.
2. **Scheduled bench numeric budgets per metric.** The budget
   discipline (thresholds, noise model, waivers, re-baselining) is
   #5897's landed record; the per-metric numbers for the server
   matrix are confirmed against the first stored
   `server-baseline/v1.0` run, so they are set from measured noise,
   not guessed here.

Issue #5893 stays open on these two confirmations, with this task's
queue item dep-blocked on issue #5892; when the catalogue lands, the
confirmation pass amends this record with the table and the numbers
and closes #5893. Everything else in this record is final and
implementation packets may be cut against §§1–4 immediately.

## 6. Parent #5685 decisions — none changed

No parent decision is changed by this record. The three tiers are
the parent's own sketch ("a fast adversarial smoke tier on every
relevant change, a heavier suite on a schedule / merge queue, and a
soak tier run less frequently — with explicit budgets"); the bench
cadence is the parent's (per-change micro-benchmarks, scheduled
full-suite runs, release-gate runs); behaviour-not-throughput
gating, recovery-as-part-of-the-test, blast-radius guards, and the
findings loop are all the parent's requirements, given concrete
budgets and enforcement points here. If implementing this record
ever appears to require changing a parent decision, that change goes
on #5685 explicitly, not here.

## 7. Alternatives rejected

- **One tier (everything nightly).** Rejected: the parent requires a
  per-change adversarial smoke tier; a nightly-only suite finds
  regressions a day late and couples every failure to a day of
  changes.
- **Full catalogue on every PR.** Rejected: the catalogue at fixed
  fixture profiles costs orders of magnitude more than the smoke
  budget, and per-PR throughput verdicts on shared runners are
  noise, not signal (#5894).
- **Throughput-gated smoke.** Rejected: runner variance makes raw
  numbers flake between runs; the parent and #5894 both fix
  behaviour assertions as the gate and CI numbers as comparability
  reporting.
- **Soak in the shared CI pool.** Rejected: deadline-bound and
  long-duration profiles are precisely the runs shared tenancy
  corrupts; §4.4 isolates them.
- **One combined job for both suites per tier.** Rejected: the
  suites have different verdict kinds (behaviour assertions vs
  baseline budgets); one job would blur which discipline a failure
  belongs to. The tiers are shared; the jobs and verdicts are
  separate.

## 8. Invariants implementation must hold

1. A tier's contents are exactly what §2's rules assign; adding a
   scenario to a tier without a rule change is a defect.
2. No tier job can construct a non-fixture target (§4.2); a guard
   failure is a setup failure, never a skipped run.
3. Smoke never asserts on raw throughput and never writes baseline
   artefacts; scheduled bench verdicts name the `server-baseline`
   version they judge against (#5894, #5897).
4. A budget change is a record amendment (§3); workflow timeouts
   cite this record.
5. Every soak failure leaves a hardening follow-up linked from the
   run (the parent's findings loop); a soak run with failures and no
   follow-ups is an incomplete run.
6. Recovery is asserted in every adversarial tier, not only soak:
   no restart required, no corrupted state, honest clients succeed
   again (the parent's rule, applied per tier).

**Verification performed for this record (2026-11-16):**
`.github/workflows/id-bench.yml` (main-tier, non-blocking,
`timeout-minutes: 60`) and the `world_dos.rs` in-process shape read
at `main` `25a77703e1`; the #5891 record read on main
(`doc/2026-11-08T11-27-00Z_design_shared_workload_generator/`); the
#5897 record read on main
(`doc/2026-11-08T23-45-30Z_design_id5897_regression_gates/`); the
#5894 record read on issue #5894 (environment pins,
`server-baseline/v1.0`, CI as smoke/comparability tier); the #5892
record read in PR #7738 (`doc/2026-11-08_decision_id5892_dos_scenario_catalogue.md`:
fixed fixture profiles, deadline-bound exemption assigning extended
profiles to the scheduled/soak tier); parent issue #5685 read this
shift — §6 reflects its text.
