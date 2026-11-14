# #9204 — Queue selection and bounded exploration

**Status:** Proposed component contract for review; no implementation or live selection change authorized.
**Inventory target:** `components/9204-queue-selection-bounded-exploration.md` (provisional target in `inventory/statements.md`).
**Evidence baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`; census `census/track-a.md` at 2026-11-13T03:30Z.
**Related:** #9199 priority model; #9200 distribution contract; #9208 hard dependencies; #9209 lease/fencing; #9178 claim semantics; #9179 park semantics; #8097 lane filter policy; #6533 live dispatch proof.
**Source trace:** H-EXPLORE/H-SURVIVE-A/C-9/O-PART1 §§4, 7–9, 15–17/S-EXPLORATION/S-RATIO; inventory domains `queue-selection`, `priority`, `leases-outbox-completion`.

## Decision frame

Normal work selection and exploratory sampling are separate algorithms. Proposed baseline: preserve the one current Fleet queue; normal choice remains deterministic highest effective urgency among eligible work for the requested lane; fairness is explicit lane/work-share policy, not random score perturbation. Exploration samples without claiming; dispositions are durable before extended action; claim performs a fresh broker eligibility/revision check. Reuse the current broker/atomic claim path. Do not implement a parallel scheduler, queue or database. Parent retains final design decision.

## V1 — observed baseline (pinned source, not deployment proof)

- `turso_queue.rs:3110-3129` specifies one queue and two empty-by-default predicates: the task's requirements must be satisfied by the claimer **AND** the claimer's explicit filter expression must match the task. Claimer identity is recorded, not itself a filter; it only supplies values for explicit task requirements.
- `turso_queue.rs:4210-4272` atomically selects/updates the same row; ordering is priority non-null first, numeric ascending (lower sooner), then creation time. Claim order lacks a final stable ID tie; list adds `id ASC` (`:6454-6457`). Claim guards and active eligibility checks are not replaced by this design.
- No weighted exploration, draw dispositions, density correction, sampling configuration or simulation was found in the pinned claim path; census classifies these as design-only/open implementation.
- Exact filtered lanes and their scope are owned by #8097. Default claim has both predicates empty/true. Do not turn lane or worker identity into an implicit filter.
- Current queue priority uses live low-is-sooner profile; #7030 mapping `expedite=15`, `next-up=25`, live queue task value `10` remains dated ratified evidence, not a new exploration threshold. Do not remap current values using the historical 450/1000 profile.

## V2 — selection contract proposal

### Eligibility and deterministic normal selection

Define one immutable selection input snapshot. This is internal model notation; external requests/results use the shared `Command`/`Result` envelope from `../architecture.md`, snake_case keys and canonical decimal strings for unsafe integer-like values.

```text
SelectionInput = {
  authority_id, queue_snapshot_revision, item_revisions[], policy_version, taxonomy_version,
  graph_revision, as_of_utc, lane_id, lane_capabilities,
  declared_requirements, claim_time_filter, fairness_state_revision,
  candidate_set_hash
}
```

Let `TaskRequirements(task, claimer)` mean that all of the task's declared requirements (including its normal repository target and any explicit `requires` fields) are satisfied by this claimer; let `ClaimerFilter(task)` be the caller's explicit Boolean filter expression over task metadata. A candidate passes **iff** `TaskRequirements AND ClaimerFilter` are both true (with an empty predicate true). OR/NOT are permitted _inside_ an explicit filter expression, not between these two gates. Example: a task targets repo `code` and requires `platform=linux`; caller filter `repo=code` matches the task, but a `platform=windows` claimer still fails `TaskRequirements`. Identity is not a filter unless it satisfies an explicit task requirement.

`candidateSet` contains canonical unique `(authority_id, work_id)` string-pair identities after both predicates and all hard gates. Eligibility is not a low weight. Reject/skip: closed/completed/cancelled, persistent #9178 `manual_only` eligibility flag, unresolved hard dependency, unavailable required capability, incompatible ownership, active lease/concurrency limit, policy-disabled, or invalid graph evaluation. `hold` is separate operational/lifecycle state. Retain typed reason codes for every exclusion. `do_now`/direct-session is separately routed and does not bypass auth, hard dependencies, current lease, concurrency cap or claim guards. Authenticated principal/capabilities authorize; `actor_ref` is not proof.

Normal selection follows the shared comparator: both matching predicates, hard eligibility, effective urgency ascending, exact peer rank within equal effective urgency, creation time, normalized canonical identity. V1 claim behavior retains its no-final-ID tie defect and is not relabeled as deterministic total order. Claim atomically rechecks work version, both predicates, eligibility, graph/policy version, lease and lane constraints; if the snapshot changed, recompute or return stale/retry, never take stale head. Fairness allocates service opportunity among lanes/workers but does not reorder urgent items without explicit policy.

### Urgent-protection gate

Exploration is eligible only if no qualifying Emergency/Supercritical/Critical/Do Now work **that requires the requesting lane** would be delayed by the draw. Name the qualifying classes from the active ratified policy profile; the 450/1000 historical categories are not runtime thresholds. A claimable urgent item means hard-eligible for this exact lane under the same pinned input, not merely high score in a PM board. Recheck this gate immediately before the exploration event and before chosen item claim. Race with newly urgent work cancels/defer draw and returns to normal urgent selection.

### Candidate exploration configuration (all remain hypotheses)

Keep a named, versioned candidate profile with:

```text
explorationProbability = 0.04
candidateCount = 3
preferenceMultiplier r = 1.25
preferenceDistance d = 200
densityExponent alpha = 0.5
bucketWidth = 200
uniformMixture epsilon = 0.05 (optional)
```

These are historical source values for simulation and comparison—not approved production constants and not copied into the live 10–90 profile. `d=200` is meaningful against broad charter scores but compresses distinctions across the live profile; fit a separately named live profile only after #7030 mapping and observed distributions are established. Preserve original outcomes/intent, not arbitrary constants.

For eligible candidate `i`, candidate weight is `w_i = exp(-ln(r)*(u_i-u_ref)/d) * n_bucket(i)^(-alpha)`, and normalize over the eligible candidate set. Lower numeric urgency receives higher weight. Because `u_ref` is a shared additive exponent term, it cancels under normalization; set a stable reference for clarity, not as a hidden preference. Use log weights and subtract maximum log-weight before exponentiation to prevent overflow/underflow. Inputs require `r>1`, `d>0`, `alpha>=0`, bucket width>0, `epsilon in [0,1]`, finite scores; invalid config returns typed error, no fallback random behavior. Bucket population is count of distinct eligible canonical item IDs in the fixed sampling population, not completed/unrelated items. Specify bucket boundary convention (e.g. half-open intervals) and negative/offset score behavior in implementation contract.

Freeze one eligible population, compute bucket densities once, compute weighted probabilities, then apply the optional uniform mixture once to produce initial masses `q_i=(1-epsilon)*Pweighted(i)+epsilon/N` (when epsilon is zero, `q_i=Pweighted(i)`). Draw without replacement by removing each chosen candidate's initial mass and renormalizing the remaining `q_i`; do **not** recompute bucket density, weighted scores, or the epsilon mixture after each draw. This is the proposed unambiguous sampling semantics. Analytical oracle for the **first draw** is `q_i`; for draws 2/3, compare observed marginal inclusion against exact sequential conditional probabilities computed from the fixed `q_i` masses (or a reference simulation)—not the first-draw probability formula. Candidate count is `min(configuredCount, eligibleCount)`; 0 candidates emits no event, 1/2 returns available count explicitly. If weights underflow after stabilization, reject invalid numeric state; do not silently fall back to uniform.

Importance is null/unavailable until distinct data exists. Do not fabricate it from urgency. Any future axis mixture is a separate versioned policy with distinct filter and statistical tests.

### Durable draw/disposition lifecycle

One `ExplorationRun` contains run ID, lane, seeded RNG algorithm/seed commitment, exact input snapshot refs/hash, policy/config versions, start time, eligibility/urgent-gate result, unique candidate IDs, first-draw probabilities (or reproducible computation inputs), and durable dispositions. Draw does not claim/lease candidates. Persist all drawn candidates' dispositions **before** lengthy work: `select_for_execution`, `triage`, `defer_or_reprioritize`, `release_unchanged`, or explicit `no_action` with reason. A state machine allows `drawn → disposed → (selected → claim_attempt → claimed | stale | rejected)`. It is idempotent by run/event/candidate ID; repeated writes with identical disposition are no-op; conflicting disposition requires an authorized superseding event and provenance.

Only the selected-for-execution item enters ordinary atomic claim. Before claim, recalculate urgent gate and revalidate canonical item revision, graph, lane, lease/concurrency, directive and eligibility. If stale, retain run record, mark `stale`, return to normal selection; no speculative extra claims. Interruption after persisted dispositions leaves durable triage hints, not hidden score mutation. The historical -20/-10/+10 examples are optional hints only; applying them requires a separate authorized triage decision under #9199 with evidence/version.

### Fairness is separate

Urgency comparator selects what should go first; lane/worker fairness allocates finite service opportunities. Possible future mechanisms include bounded share/round-robin among eligible lanes or age of last lane service. #8097 is the designated source for allowed filtered lanes; claimer identity is recorded, not a filter. Define the fairness objective and urgent-work service bound before changing selection. Exploration is not a starvation fix for critical work and does not bypass a strict urgent-protection gate. Do not add noise to intrinsic scores or randomize normal highest-urgency choice.

### Performance and complexity

- Naive candidate construction/filter/sort: `O(N log N)` time and `O(N)` memory. A safe prefilter/top-k/index may reduce ordinary cost, but must yield identical canonical top order and be validated against full-scan oracle.
- Dependency/eligibility calculation: graph closure worst-case `O(V+E)`; cycle-safe budget exhaustion returns `evaluation_incomplete` and fails closed, never drops an unseen blocker.
- Weighted draws over `M` eligible items: `O(M)` weights and `O(kM)` naive sequential draw for `k` without-replacement items; a prefix/Fenwick approach can reduce draw cost to `O(M+k log M)` if update and numerical equivalence are proven. Initial historic `k=3` does not justify premature data structure complexity.
- Durable record cost is `O(k)` candidate/disposition rows or one run record with candidate entries, plus one actual claim; payloads should hold IDs, versions, evidence references, not full queue context. Define maximum candidate payload and explicit typed overflow; do not silently truncate dispositions.

## V3 — implementation slices / proposed issue specs

No production changes authorized. Consume granular contracts `PRI-CMP-01`, `PRI-PROFILE-01`, `DST-SNAPSHOT-01`, and `DST-INTENT-01` before claimant changes; coordinate lease behavior with #9209's specific lease/fence contract and existing #9178 claim semantics. #8097's lane filters are source-of-truth. Contract keys are prerequisites; whole domain starters are not blockers.

In this issue table, any `src/work/selection.rs` or `src/turso_queue.rs` shorthand is relative to `pkgs/fleet/crates/fleet-queue/`; the full proposed files are `pkgs/fleet/crates/fleet-queue/src/work/selection.rs` and `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` in the existing crate.

| Key / tier             | Deliverable / seam candidate                                                                                                                                                           | Hard dependencies                                                                                                                | Done check / oracle                                                                                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SEL-01` / crucial     | Pure reference selector in `pkgs/fleet/crates/fleet-queue/src/work/selection.rs`; both task-requirement and caller-filter predicates; hard gates and urgent predicate. No persistence. | `PRI-CMP-01`, `PRI-PROFILE-01`, `DST-SNAPSHOT-01`; #8097's concrete filter expressions as source, not whole #9199/#9200 starters | Golden/permutation fixtures; both predicates independently true; task-requires-linux + repo=code filter still excludes Windows claimer; hard-ineligible never selected.     |
| `SEL-02` / crucial     | Typed claim revalidation in `src/work/selection.rs` plus existing `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` atomic claim.                                                     | `SEL-01`; existing #9178 claim contract; granular lease/fence contract from #9209                                                | At most one concurrent claim; stale graph/item/policy rejected/recomputed; no bypass for directive or exploration; claim/list tie fixed by `PRI-CMP-01`.                    |
| `SEL-03` / recommended | Versioned exploration profile and stable fixed-mass sampler in `src/work/selection.rs`, initially feature-disabled.                                                                    | `PRI-PROFILE-01`; `DST-SNAPSHOT-01`                                                                                              | Exact tiny-population first/marginal probabilities; invalid input typed; bucket density frozen once; mixture masses frozen once; no per-draw density/epsilon recomputation. |
| `SEL-04` / recommended | Durable run/disposition state machine in `src/work/selection.rs` with integration in existing `src/turso_queue.rs`; no early leases or automatic score edits.                          | `SEL-02`; `DST-INTENT-01`; granular operation-intent contract from #9209                                                         | Crash/replay; duplicate no-op; conflicting revision explicit; only selected candidate calls claim once.                                                                     |
| `SEL-05` / recommended | Seeded offline simulator/reference-model tests in `src/work/selection.rs`, no production service.                                                                                      | `SEL-03`; read-only distribution evidence is a soft calibration input                                                            | Exact fixed-mass oracle and preregistered confidence intervals; urgent latency/starvation, cold-item review, fairness metrics; no production-benefit claim.                 |
| `SEL-06` / contingent  | Lane fairness policy interface in `src/work/selection.rs`; consume #8097's existing filter contract.                                                                                   | #8097 eligibility/filter semantics; fairness measure is a V2 decision, not a whole-starter blocker                               | Model-based fairness bound while exact urgent order remains unchanged; no eligibility bypass or intrinsic-score change.                                                     |
| `SEL-07` / contingent  | Read-only shadow/disable controls and rollback tests in `src/work/selection.rs`; no migration or rollout in this slice.                                                                | `SEL-01`–`SEL-06`; separate rollout authorization; #6533 only for distinct live dispatch proof                                   | Shadow-vs-reference diff; kill switch; read-only rollback; no priority changes; operational approval recorded separately.                                                   |

**Future red-first test commands / oracles per issue:** All Rust tests live in the existing `fleet-queue` crate; do not add a crate. Exact future commands: SEL-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_requirements_and_filter_both_apply`; SEL-02 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_stale_claim_revalidates`; SEL-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_fixed_mass_probabilities`; SEL-04 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_disposition_replay_no_claim`; SEL-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_simulation_matches_reference`; SEL-06 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_fairness_preserves_urgent_order`; SEL-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_rollback_keeps_live_values`. Each issue must first run its exact command with `-- --list` and assert the exact name is listed, then run the filter; zero-test success is not acceptance. SEL-01 oracle uses both-predicates counterexample above; SEL-03/05 use exact fixed-mass sequential probabilities. Hosted parity (#9079/#9081) and mobile display (#9201) are soft consumers, not blockers to the pure model suite.

## V4 — hosted conformance

Consume #9079 (PM hosted service) and #9081 (Fleet-tool hosted equivalent). Their adapters execute the same selector fixtures: exact candidate set, urgent gate, comparator version, eligible-lane context, seeded draw/probabilities, durable disposition results and stale claim response. Compare normalized canonical IDs plus policy/graph/snapshot hashes; database layout and scheduling backend may differ. If hosted target cannot produce complete inputs or preserve disposition revision, it must return unsupported/degraded and must not claim parity or claim an item from stale data. Do not interpret parent #9078 closure as these children being implemented.

## Offline simulation / acceptance specification

### Deterministic and model-based suite

Oracle is the pure reference evaluator specified above plus expected transition table. Cover: zero candidates; one/two candidates; duplicates; null and extreme scores; equal weights; same/different density buckets; bucket boundaries; `epsilon=0`/`1`; policy versions; missing/unknown profile; stale graph; no lane requirement; both task-requirement and caller-filter predicates independently true/false; Linux task requirement with repo=code filter but Windows claimer; task identity distinct from caller filter; all candidates hard-blocked; one urgent item requiring this lane; urgent item not requiring lane; `do_now`/direct-session and persistent `manual_only`/hold; active lease; concurrent two-worker claim; urgent arrival after draw; crash after draw and after each disposition; stale selected item; repeated/contradictory dispositions. Invariants: urgent gate is exact; no draw candidate has lease absent selected claim; no two leases for one item; hard-ineligible never claimed; same fixture/seed/version yields exact same draw; all decisions durable or typed failure; no silent random fallback.

### Statistical oracle

- Pin PRNG algorithm, seed values, workload generator version, input profiles and software build. Do not claim reproducibility from a seed if implementation PRNG is unspecified.
- For `N<=small fixture threshold`, enumerate all draw sequences and compute exact candidate inclusion probabilities for sequential weighted without replacement. For first draw, use analytical normalized weight. For larger populations, fixed seeded repeated simulation checks candidate frequency against preregistered confidence intervals; intervals and tolerances are declared **before** running, not fitted after.
- Test synthetic populations: uniform; high-density clump vs sparse band; heavy cold tail; mixed lane skills/ownership; scarce single-lane candidate; dependency chain; score ties; missing importance; urgent arrivals; stale snapshots; candidate count below three. Compute initial density and `q_i` once per snapshot; after each draw, exact oracle removes the chosen mass and renormalizes the remainder without recomputing densities or mixture. Compare candidate profiles against uniform/no-density controls.
- Report probability normalization, per-band/per-bucket inclusion, urgent-work displacement (required target is zero under gate), high-priority service latency, cold backlog time-to-review, lane-level fairness metric, and empty/invalid outcomes. Owner must set acceptable non-urgent thresholds; this document does not invent them.
- Historical `4%, 3, 1.25, 200, 0.5, width 200, epsilon 0.05` remain explicit candidates only. Since the live scale is narrow, compare calibrated profile options on the **same immutable input corpus** without altering current urgency values. Do not infer production benefit from simulation; later read-only production-distribution calibration and a separate go/no-go are required.

## Alternatives / consolidated FDR link

- Strict urgent deterministic selection only vs bounded exploration; preserve both and quantify starvation risk/benefit before activation.
- Draw 3 without replacement vs sequentially inspect until choice; historic source outcome is three durable candidates; alternative needs its own acceptance review.
- Parameter calibration for live taxonomy and whether epsilon exists; do not transplant broad-scale `d=200` or invent new constants.
- Fairness in lane service vs item-level order; owner objective required, not assumed.
- Consolidate urgent protection, deterministic selection, fairness objective and exploration parameter calibration under [FDR-PMO-008](../decisions/FDR-PMO-008-exploration-fairness-calibration.md), already present in [`../decisions/README.md`](../decisions/README.md). The historical profile/taxonomy compatibility subdecision links to [FDR-PMO-002](../decisions/FDR-PMO-002-priority-profiles-ordering-provenance.md). Preserve the alternatives above as sections of those records; do not allocate extra decision IDs or reuse #9180's reserved FDR-0001…0010. The indexed record status remains proposed until adoption is recorded there.

## Rollback / safety boundary

Start with offline contract, then read-only shadow selection and comparison; keep production feature disabled until approved. Existing urgent selection and claim behavior remain fallback. Rollback must disable exploration/disposition writes without rewriting priorities or breaking claims. The number of candidates sampled must never imply claim count. No live queue, schema, schedule, priority, dispatch, or mobile control change is authorized by this design.
