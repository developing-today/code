# SEL-05 — Seeded offline simulator and reference-model tests for selection

Issue: #9495 (slice SEL-05 of #9204, program #9196). Status: design
record. It decides what the offline simulator checks, which reference
model it checks against, how its tolerances are fixed before any run,
what it measures, and the shape of `--explain` output so a live
decision can be replayed offline. It writes no code; implementation
follows the gates in §9.

Two rules hold throughout:

- **Offline only.** The simulator is a pure function in
  `work/selection.rs`, run by tests. It reads no queue, writes no
  queue, starts no service or schedule, and makes no network call.
- **No benefit claim.** A simulation report is evidence about the
  model under synthetic workloads. It never says exploration helps
  production, and nothing may cite it as approval to activate.

## 0. Governing decisions consumed

| Source                                      | What this record takes from it                                                                                                                                                         |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9495 slice text                            | Owned file `work/selection.rs`; first red test `pmo_selection_simulation_matches_reference`; exact fixed-mass oracle; preregistered intervals; urgent, cold-item and fairness metrics. |
| #9204 "Offline simulation / acceptance"     | The deterministic suite's fixture list, the statistical oracle, the synthetic populations, the report measures, and "owner must set acceptable non-urgent thresholds".                 |
| #9204 "Candidate exploration configuration" | First draw is `q_i`; later draws remove the chosen mass and renormalize the fixed `q_i`; no per-draw density or mixture recomputation; historical values are candidates.               |
| SEL-04 record (`2026-11-26T03-15-10Z`)      | The run, its stored seed and masses, the dispositions, the D7 crash points, and D3's reproducibility requirement.                                                                      |
| SEL-07 record (`2026-11-25T21-38-00Z`)      | `shadow_draw` uses a seed derived from pinned inputs; parts not yet built report `not_present`, never a stand-in.                                                                      |
| SEL-06 (landed, `work/selection.rs`)        | `LaneFairnessPolicy`, `NoFairness`, `BoundedShareFairness`, `FairnessState`: the fairness side of the scenario simulator.                                                              |
| Census rows                                 | `flag.explain` (primary SEL-05): "complete reasons and reproducible inputs exposed". `flag.strategy` and `queue.explore` (SEL-03 primary, SEL-05 supporting).                          |
| Drew's ruling, 2026-11-24 (on #9196)        | Design-only gates without a hard blocker are void. SEL-05 keeps its real edges (§9).                                                                                                   |

## 1. Verified state on main `4053b267`

- `work/selection.rs` holds SEL-06 only: the fairness interface, two
  policies and their tests. No selector (SEL-01), sampler (SEL-03),
  run model (SEL-04) or simulator exists.
- `fleet-queue` depends on no random-number crate. The only generator
  in the crate is `backoff.rs`'s xorshift for retry jitter, which is
  not seeded by its caller and is not a simulation source.
- No `--explain`, `--strategy` or `queue explore` is parsed by the
  queue client; the census marks all three absent.
- The SEL-04 and SEL-07 records are merged. The SEL-03 record does not
  exist; SEL-03 has a queued build task under Drew's 2026-11-24 ruling.
- `fleet-rust-tests.yml` runs the workspace tests nightly and on
  dispatch. Tests run there, not on a local compile.

## 2. Intent

Two independent things must agree before anyone trusts the sampler:
the production sampler SEL-03 writes, and a reference model written
here from the formula alone. On tiny populations they must agree
exactly, by enumeration. On larger ones, many seeded draws of the
production sampler must land inside intervals written down before the
first run. Then, with the selector, the run model and fairness in
place, a deterministic queue simulation reports what bounded
exploration does to urgent work, to cold items and to lanes, under
named synthetic workloads, so the owner can decide on thresholds with
numbers in front of them. Every report can be rerun to the byte, and
every live `--explain` can be replayed through the same reference
model.

## 3. Ownership

- **Owned:** in `pkgs/fleet/crates/fleet-queue/src/work/selection.rs`,
  the reference model, the workload generator, the scenario simulator,
  the report and explanation types, and their tests.
- **Called, not owned:** SEL-03's profile and sampler, SEL-01's
  eligibility and comparator, SEL-04's run model, SEL-06's policies.
  The simulator calls the production functions; it never reimplements
  them, except the reference model, whose whole purpose is to be a
  second implementation.
- **Not owned:** the thresholds for non-urgent metrics (Drew sets
  them), any live calibration (a later, read-only, separately approved
  step), and CLI registration (CLI-13).
- **Added nowhere:** no crate, dependency, command, service, schedule,
  table or workflow.

## 4. Decisions

### D1. Two layers, gated separately

| Layer                        | Needs                                                          | Tests                                       |
| ---------------------------- | -------------------------------------------------------------- | ------------------------------------------- |
| A, sampler against reference | SEL-03's profile and sampler only                              | exact enumeration, statistical, determinism |
| B, queue scenario simulation | SEL-01 selector and urgent predicate, SEL-04 run model, SEL-06 | metrics, invariants, crash scenarios        |

The slice's hard blocker is SEL-03, and layer A needs nothing else, so
the first red test is a layer A test. Layer B tests are written with
layer A but report `not_present` naming the missing part until SEL-01
and SEL-04 land, as SEL-07 §3.5 does. A missing part is never replaced
by a stand-in that would let a scenario pass.

### D2. The reference model

The reference model computes, from a profile and a frozen eligible
population, the exact distribution the sampler is supposed to have. It
is written from #9204's formula and shares no code with SEL-03:

1. Bucket each candidate by `floor((u_i - offset) / bucket_width)`,
   half-open, where `offset` and the boundary rule are SEL-03's
   declared convention, read from the profile, not assumed here.
2. `n_b` = distinct eligible ids in bucket `b`.
3. `w_i = r^(-(u_i - u_ref) / d) * n_b(i)^(-alpha)`, computed directly
   (no log-sum-exp) for fixtures small enough not to overflow;
   `P_i = w_i / sum w`.
4. `q_i = (1 - epsilon) * P_i + epsilon / N`.
5. For `k` draws without replacement, the probability of an ordered
   sequence `(i_1, ..., i_k)` is
   `prod_j q_{i_j} / (1 - sum_{m<j} q_{i_m})`. Inclusion probability of
   `i` is the sum over every sequence containing `i`.

Step 5 enumerates every ordered sequence, `N!/(N-k)!` of them, so it is
used only for `N <= 12` with `k <= 3` (at most 1,320 sequences).
Sums use Kahan compensation.

Exact agreement means, for every candidate: the sampler's reported
`q_i` and the reference `q_i` differ by at most `1e-12` absolute;
`sum q_i` is within `1e-12` of 1; and the sampler's own conditional
masses at each step match step 5's denominators within `1e-12`. These
tolerances cover `f64` rounding between two correct implementations
and nothing else.

### D3. Seeded statistical checks

For populations past the enumeration limit, and to check the actual
draws rather than the reported masses:

- **Generator for workloads:** SplitMix64, written inline (about ten
  lines, std only), checked in its own test against the published
  first outputs for seed 0. It generates workloads and interleavings,
  never exploration draws.
- **Generator for draws:** whatever SEL-03 pins, named in the report.
  The simulator passes it seeds; it does not pick the algorithm.
- **Seeds:** a fixed list in the test source, derived as SplitMix64
  outputs from the base seed `0x5E105`. The list is part
  of the preregistration below.
- **Draw counts:** `R = 100_000` runs per population.
- **Expected inclusion:** the reference inclusion from D2 when
  `N <= 12`; otherwise a reference Monte Carlo of `10 * R` draws using
  the reference model's own sequential procedure and the workload
  generator, with its own fixed seed.
- **Acceptance per candidate:**
  `|observed - R*p| <= 6 * sqrt(R*p*(1-p)) + 1`. Six standard
  deviations per candidate keeps a family of a few hundred checks
  below a one-in-a-million false failure for a correct sampler. When
  the expectation comes from the reference Monte Carlo, its own
  variance is added under the root.
- **Normalization:** every reported `q` vector sums to 1 within `1e-12`.

**Preregistration rule.** The seeds, `R`, the `6`-sigma bound, the
`+1` continuity term and the population list are committed in the
test source in the same PR that adds the tests, before the tests are
run against a sampler. After that, a failure is a defect to fix in the
sampler or the reference, never a reason to change a seed, `R` or the
bound. Changing any of them is a separate PR that says why and reruns
everything. Because the draws are seeded, a passing suite passes on
every rerun, and a failing one fails on every rerun.

### D4. Fixtures and populations

**Deterministic fixtures** (layer A unless marked B), from #9204's
list: zero candidates; one; two; three; duplicates in the input (must
collapse to one canonical id or be refused, per SEL-03); null score
(refused as non-finite, typed); extreme scores at both ends; all equal
weights; all in one bucket; one per bucket; scores exactly on bucket
boundaries; `epsilon` 0 and 1; `alpha` 0; invalid `r <= 1`, `d <= 0`,
negative `alpha`, `epsilon` outside 0 to 1 (each a typed error, no
draw); an unknown profile version (typed error); weights that underflow
after stabilization (typed error, never a silent uniform draw).

B-only fixtures: one urgent item requiring the lane (no draw); one
urgent item not requiring the lane (draw allowed); all candidates
hard-blocked; `do_now` and `manual_only` items present; an active
lease; two lanes claiming one item; urgent arrival between draw and
claim; crash after the draw and after each disposition (SEL-04 D7);
stale selected item; repeated and contradictory dispositions; task
requirement `platform=linux` with caller filter `repo=code` and a
Windows claimer.

**Synthetic populations** (named, versioned under
`fleet.selection-workload/v1`, parameters in the test source):
`uniform`; `clump_vs_sparse` (a dense band beside a thin band);
`cold_tail` (a long tail of low-urgency items); `mixed_lanes` (lane
skills and requirements differ); `scarce_lane` (one candidate only one
lane can take); `dep_chain` (a hard dependency chain); `ties` (many
equal scores); `no_importance` (importance absent, which must stay
absent, never derived from urgency); `urgent_arrivals` (urgent items
arrive during the run); `stale_snapshots` (items change between draw
and claim); `under_three` (fewer eligible than the count).

Each population is generated twice: on the historical broad scale the
charter values (`d = 200`, `bucket_width = 200`) were written for, and
on the live 10 to 90 scale. The report says which scale each result is
on. No result on one scale is reported as if it held on the other.

### D5. Profiles compared

Every scenario runs the same immutable workload under these, in one
report:

| Profile id             | What it is                                                                  |
| ---------------------- | --------------------------------------------------------------------------- |
| `none`                 | no exploration; today's behaviour                                           |
| `uniform_control`      | exploration with equal masses                                               |
| `no_density_control`   | SEL-03's weighting with `alpha = 0`                                         |
| `historical_candidate` | `0.04, 3, 1.25, 200, 0.5, width 200, epsilon 0.05`, labelled as a candidate |
| SEL-03 live profiles   | each named, versioned profile SEL-03 registers for the live scale           |

The two controls exist only inside the simulator. They are never
registered as `--strategy` values and never reach a live command.

### D6. The scenario simulator (layer B)

- **Time** is a tick counter. There is no wall clock, thread, sleep or
  I/O. Arrivals, lane requests and interleavings come from the
  workload generator.
- **Each tick**, in an order the generator sets: new items arrive;
  each lane that is free asks for work; the exploration Bernoulli with
  the profile's probability decides normal pick or exploration;
  exploration runs SEL-04's model end to end (gate, draw,
  dispositions by a scripted policy, at most one claim); normal pick
  uses SEL-01's comparator through SEL-06's fairness policy; a claimed
  item completes after a generated service time.
- **Disposition script:** the simulated lane selects the first drawn
  candidate it is eligible for, sends the rest to `triage`, and
  scripted variants exercise `release_unchanged`, `no_action` and
  supersede. Hints are recorded and never applied, as SEL-04 D5
  requires; the simulator asserts that.
- **Budgets:** at most 10,000 ticks and 2,000 items per scenario in the
  default suite, so the whole suite runs in seconds. Larger runs are a
  calibration step outside this slice.

### D7. What a report measures

| Measure                 | Definition                                                                                                            |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `urgent_displacement`   | ticks in which a claimable urgent item for a lane waited while that lane explored. Must be 0.                         |
| `urgent_latency`        | p50, p95 and max ticks from an urgent item becoming claimable to its claim                                            |
| `high_priority_latency` | the same for the top band of the active profile                                                                       |
| `cold_time_to_review`   | ticks from arrival to the first draw, disposition or claim of an item in the cold tail, with the share never reviewed |
| `lane_service_share`    | claims per lane, its max/min ratio and Jain's index                                                                   |
| `per_bucket_inclusion`  | draw inclusion per bucket and per band, observed against reference                                                    |
| `empty_and_invalid`     | counts of `nothing_to_explore`, `preempted`, typed profile errors and `unsupported`                                   |
| `claims_per_run`        | must be 0 or 1 for every run                                                                                          |
| `priority_writes`       | must be 0                                                                                                             |

Three measures are hard invariants and fail the test when broken:
`urgent_displacement = 0`, `claims_per_run <= 1` and
`priority_writes = 0`. Every other measure is reported, not judged.
#9204 leaves its thresholds to the owner, and this record does not
invent them.

### D8. The report

`fleet.selection-simulation-report/v1`, JSON, snake_case, integers and
versions as canonical decimal strings:

- `pinned`: build commit, sampler id and version, `rng_algorithm`,
  profile ids and values, workload generator version, population name
  and scale, seed list, tick and item budgets, part states (`applied`
  or `not_present` for SEL-01, SEL-03, SEL-04, SEL-06);
- `oracle`: D2 and D3 results per population, with the bound used;
- `measures`: D7 per profile;
- `invariants`: each hard invariant with `held` or the first violating
  tick;
- `evidence_class`: always `offline_simulation`.

The report has no field that states a benefit, a recommendation or an
approval. Rerunning the same pinned inputs produces byte-identical
JSON (keys sorted by JCS), which is the replay check.

### D9. `--explain` is a replayable input

The census gives `flag.explain` to this slice: "complete reasons and
reproducible inputs exposed". So `--explain` on `queue explore`
(SEL-03, SEL-04) and on `queue rank` (SEL-01) emits one document,
`fleet.selection-explanation/v1`:

- the pinned snapshot refs, policy, profile and comparator versions,
  lane context and `as_of`;
- every considered item with its eligibility verdict and SEL-01 reason
  codes for each exclusion;
- the urgent gate's result and, if it blocked, the urgent item's ref;
- for exploration: buckets, densities, `w_i`, `P_i`, `q_i`, the seed,
  `rng_algorithm` and the drawn sequence with its conditional masses;
- for a normal pick or rank: the comparator keys in order.

The decision is that this document is a valid input to the reference
model: feeding it back reproduces the same eligible set, masses and
draw, or reports which input it lacks. A live explanation can
therefore be checked offline without reading the queue again. The
explanation carries ids, versions and numbers only, never task bodies
or comments. `--strategy` accepts only the ids SEL-03 registers; the
simulator's controls (D5) are not among them.

### D10. Typed results

| Code                     | When                                                               |
| ------------------------ | ------------------------------------------------------------------ |
| `not_present`            | a layer B part is not built; the scenario compares nothing         |
| `reference_mismatch`     | D2 exact agreement failed; names the candidate and both values     |
| `interval_exceeded`      | D3 bound failed; names the candidate, observed, expected and bound |
| `invariant_violated`     | a D7 hard invariant broke; names it and the first tick             |
| `explanation_incomplete` | D9 replay lacks an input; names it                                 |
| `unknown_version`        | unknown report, workload, explanation or profile version           |
| `budget_exceeded`        | a scenario hit its tick or item budget before finishing            |

No code reports a pass for a comparison that did not run.

## 5. Negative cases

| Negative from the slice body          | Where it is closed                                                                                                                 |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| stale input                           | `stale_snapshots` population and B fixtures: a stale selection closes its run and counts in `empty_and_invalid`                    |
| duplicate or replay                   | byte-identical reports on rerun (D8); D9 replay; SEL-04 replays in layer B never draw or claim twice                               |
| ambiguous resource                    | canonical `(authority_id, work_id)` ids; duplicate input ids collapse or are refused per SEL-03, never counted twice               |
| unauthorized actor                    | the simulator has no write path and no actor that can change anything outside its own memory                                       |
| partial remote effect                 | none: no remote calls; a simulated claim is a model state                                                                          |
| unknown version                       | `unknown_version` for report, workload, explanation and profile                                                                    |
| no false verified or completed result | `not_present`, `budget_exceeded` and failed oracles keep a missing comparison from reading as agreement; `evidence_class` is fixed |
| silent random fallback                | invalid or underflowing configurations are typed errors in D4's fixtures                                                           |

## 6. Oracles and tests

All are future tests in `work/selection.rs`'s test module, written
failing first, under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.
Each name must first appear in that command's `-- --list` output; a
filter that matches zero tests is not acceptance. Tests run through
`fleet-rust-tests.yml` dispatch, not a local compile.

| Test                                                      | Oracle                                                                                                                                           |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pmo_selection_simulation_matches_reference` (first, red) | Layer A: on every `N <= 12` fixture, SEL-03's masses and conditional masses equal D2's within `1e-12`, and seeded draws fall inside D3's bounds. |
| `pmo_selection_reference_inclusion_by_enumeration`        | The reference model alone: hand-computed inclusion for a 3-item, `k = 2` fixture with unequal masses.                                            |
| `pmo_selection_splitmix64_reference_vectors`              | The inline generator matches the published outputs for seed 0.                                                                                   |
| `pmo_selection_invalid_profiles_are_typed`                | Each D4 invalid input returns its typed error and draws nothing.                                                                                 |
| `pmo_selection_no_per_draw_recompute`                     | A fixture where recomputing density after a draw would change the answer: the sampler matches the fixed-mass reference, not the recomputed one.  |
| `pmo_selection_report_is_reproducible`                    | Two runs of the same pinned inputs produce byte-identical reports.                                                                               |
| `pmo_selection_explanation_replays`                       | A D9 explanation fed back to the reference model reproduces the masses and the draw; removing one input gives `explanation_incomplete`.          |
| `pmo_selection_scenario_urgent_never_displaced`           | Layer B, every population and profile: `urgent_displacement = 0`. `not_present` until SEL-01 and SEL-04 land.                                    |
| `pmo_selection_scenario_one_claim_per_run`                | Layer B: `claims_per_run <= 1` and `priority_writes = 0` across all scenarios and crash points.                                                  |
| `pmo_selection_scenario_parts_not_present`                | With SEL-01 or SEL-04 absent, layer B reports `not_present` naming the part and compares nothing.                                                |
| `pmo_selection_scales_are_not_mixed`                      | Each result carries its scale; a report never pools broad-scale and live-scale results.                                                          |

## 7. Compatibility, migration and reversal

- Test-only code and pure types in an existing file. No live verb,
  table, schedule or spelling changes. Nothing runs outside tests.
- The `--explain` document is new output on commands that do not yet
  exist; nothing that parses current output changes.
- Reversal is deleting or skipping tests; there is no data or state to
  undo.

## 8. Requirement trace

| Requirement                                    | How this record serves it                                                                                                         |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| C-R004, C-R101, S-R010, H-R010, H-R036, O-R016 | The formula is checked exactly against an independent reference; the historical values are a labelled candidate profile (D2, D5). |
| C-R104                                         | Without replacement on a fixed population, per-lane profiles compared, seeded simulation and replayable `--explain` (D2, D3, D9). |
| C-R095, C-R097, H-R009, O-R015, S-R008         | `urgent_displacement = 0` is a hard invariant across every scenario (D7).                                                         |
| C-R098, C-R099, C-R003, C-R100                 | Layer B runs SEL-04's dispositions and asserts one claim per run and no priority writes (D6, D7).                                 |
| P-R014                                         | Fairness, starvation and weighted randomness are measured side by side under one workload (D5, D7).                               |
| P-R027, P-R049, P-R064                         | Realistic synthetic selection scenarios: eligibility, ranking, claim and stale cases, with explicit oracles (D4, D6).             |
| C-R009                                         | The claim-safely part of the agent flow is exercised by layer B's claim and stale scenarios (D6).                                 |
| C-R005, C-R107, C-R108, P-R009, O-R023, H-R017 | `--explain` and `--strategy` semantics for the explore and rank commands, registered by CLI-13, with no new command (D9).         |
| O-R038                                         | Ties stay valid (`ties` population), do-now stays separate (B fixtures), and recommendations stay recommendations (D7, D8).       |
| H-R024, H-R031                                 | Every fixture, error and failure case from #9204's suite is listed in D4 and §6 so it stays in acceptance coverage.               |

## 9. Gates

1. This record is merged.
2. SEL-03 lands its profile and sampler; this is the hard blocker and
   the only gate for layer A and the first red test.
3. SEL-04 is the shared-file predecessor on `work/selection.rs`; the
   later of the two rebases.
4. Layer B tests stay `not_present` until SEL-01 and SEL-04's model
   land, then become required.
5. Read-only production distributions are a soft calibration input
   for a later step, not a gate, and need their own approval to read.

Thresholds for non-urgent measures, and any go or no-go on activation,
are Drew's and SEL-07 §8's, not this slice's.

**Schedules:** this record creates, changes and retires no schedule on
any surface.

## 10. Non-goals

- The sampler, profile values or bucket convention (SEL-03), the
  selector (SEL-01), the run model (SEL-04), fairness (SEL-06), shadow
  and controls (SEL-07).
- Any production service, command, schedule or live read.
- Choosing thresholds, or claiming that any profile helps production.
- A simulation command for operators; exposing one would be a CLI-13
  registration decision outside this slice.
