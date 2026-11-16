# FDR-PMO-008 — Bounded exploration, fairness, and calibration

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. Historical settings stay hypotheses; activation thresholds are not adopted.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** deterministic normal selection, lane fairness boundary, optional weighted exploration and enablement evidence
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -002, -003

## Context and problem

The charter requests a low-probability exploration path to review cold/low-confidence work without displacing urgent work or taking multiple exclusive claims. Suggested historical values include 4% chance, three candidates, `r=1.25`, distance 200, density exponent .5, bucket width 200, and optional uniform mixture .05. These are hypotheses, not approved constants; distance 200 does not map blindly to live 10–90 values. Normal urgency selection and fairness are separate concerns. The pinned broker has no exploration sampler/disposition lifecycle.

Sources: [architecture, “Ordering and distributed priority”](../architecture.md); [source 01 lines 53–59, 71](../sources/01_handoff_pm_stack_fleet_orchestration.md); [source 02 §9](../sources/02_charter_v1_1.md); [#9204, “Decision frame,” “Eligibility and deterministic normal selection,” “Urgent-protection gate,” and “Offline simulation / acceptance specification”](../components/9204-queue-selection-bounded-exploration.md); [#9199, “Dependency pressure, cycle safety and resource bounds” and “V3 — implementation slices after approval”](../components/9199-distributed-priority.md); [prior art A, “Four distinct architecture shapes,” “Deterministic urgency versus fairness and density-corrected exploration,” and “Statistical exploration oracle and evidence”](../research/track-a-state-of-art.md).

## Options considered

1. **Keep normal deterministic urgent-first selection; add feature-disabled exploration model with durable dispositions and offline calibration (recommended).** Preserves safety and permits measured evaluation before rollout.
2. **Randomize ordinary selection or perturb priority scores for fairness.** Simple but obscures canonical urgent order, provenance, and reproducibility.
3. **Never explore; rely only on ordinary queue selection.** Safest short-term and valid default, but cold backlog may not receive deliberate review.
4. **Enable historical constants directly in production.** Satisfies a numeric example superficially but assumes score distribution, lane eligibility and statistical benefit not measured.
5. **Claim all sampled candidates up front.** Reserves work unnecessarily and creates stale/stranded exclusive leases.

**Tradeoff comparison:** A disabled sampler/reference model adds small test complexity and future extensibility without production cost; enabled sampling adds O(M) weighting and O(kM) straightforward draw work plus durable disposition writes. Randomizing normal selection is cheaper to describe but sacrifices urgency reliability and reproducibility. No exploration is safest/cheapest and remains the fallback, at the cost of cold-work discovery. Fixed historical constants are simple but have no measured fit to live score/lane distributions. Fairness mechanisms may improve service distribution but need explicit objectives and can add state/coordination cost.

## Proposed operational decision

Normal selection remains deterministic highest effective urgency among eligible work in the lane, with hard gates before ordering and fresh atomic claim validation. Task requirements and caller filter are both required. Fairness concerns allocation of finite lane/worker service, not random alteration of per-item urgency; #8097 owns filtered-lane semantics. Do not invent fairness thresholds without an owner-defined objective and measurement.

Exploration remains optional and disabled until calibrated. It may run only when no eligible Emergency/Supercritical/Critical/Do Now work requiring that lane would be delayed; gate uses the active profile and same pinned eligibility snapshot, then rechecks immediately before actual claim. Draw unique candidates without exclusive claims, persist all candidate dispositions before extended action, and revalidate selected item before one ordinary claim. If interrupted, retain durable hints, never hidden score edits.

Keep historical formula/settings as named simulation profile only. Freeze eligible population and density once, normalize weighted masses with documented stable numerics, sample without replacement with masses renormalized after each pick, and persist seed/algorithm/input snapshot/config/dispositions for replay. Any changed live profile requires separate version, statistical evaluation, and owner-approved thresholds. Importance remains null unless independently sourced.

## Rationale and evidence boundary

**Facts:** pinned census finds no exploration implementation. Charter and recovered source preserve candidate mechanism/settings; research identifies starvation risk in deterministic urgent-first and distinguishes fairness from priority. No representative production distribution or performance benchmark is supplied.

**Inference:** a disabled model and offline simulation preserve the user outcome without imposing unvalidated random policy. Sampling is O(M) initial weights and O(kM) straightforward draws for M eligible items and small k; optimize only if measured need, preserving reference-oracle parity.

## Consequences, compatibility, and migration

- No new scheduler/queue/database. Selection consumes the existing broker and #9178 claim semantics; atomic boundary also validates graph/policy/item/lane inputs (FDR-PMO-003).
- Exploration profile is not copied into current 10–90 taxonomy; no automatic score modification or claim of statistical benefit from simulated-only evidence.
- Rollout requires read-only distribution evidence, seeded model/statistical acceptance criteria, kill switch, and separate operational approval. The #6533 live dispatch test remains distinct.
- Persisted draw history/storage is schema-coupled and gated by #3424/#8575/#8579 and QTI/#9241 disposition; pure model/simulation is not gated.

## Negative cases and test oracles

- **Urgent-gate oracle:** any eligible urgent work requiring this lane suppresses draw; unrelated urgent work for a different lane does not; late urgent arrival cancels/defer draw before claim.
- **Sampler oracle:** first draw matches analytical mass; subsequent draws match fixed-mass sequential without-replacement reference probabilities; zero/one/two candidates report available population explicitly.
- **Determinism oracle:** seed + candidate set + policy snapshot reproduce choices/dispositions; invalid numeric/config state yields typed error, never silent uniform fallback.
- **Persistence oracle:** crash after disposition write recovers same run; no sampled-but-unselected candidate acquires a lease; duplicate identical disposition is no-op, conflict requires authorized superseding record.
- **Fairness oracle:** bounded service policy, if later adopted, preserves urgent ordering and does not make an ineligible lane eligible.
- **Statistics oracle:** preregistered confidence intervals, cold-item review latency, starvation, urgent service latency, lane fairness, and comparison controls; synthetic simulation is not evidence of production benefit.

## Revisit triggers

Reopen after representative queue/lane distributions and owner-defined fairness/service thresholds exist; if measured top-k performance requires data structures; if urgent policy changes; or if calibrated simulation fails cold-work discovery without unacceptable urgent delay. Do not enable solely because the proposal is mathematically reproducible.

## Implementation, research, and source links

- Existing: [#9204](https://github.com/developing-today/code/issues/9204), [#9199](https://github.com/developing-today/code/issues/9199), [#8097](https://github.com/developing-today/code/issues/8097), [#9178](https://github.com/developing-today/code/issues/9178), [#6533](https://github.com/developing-today/code/issues/6533).
- Existing granular component keys: #9204 `SEL-01`–`SEL-07`; #9199 `PRI-01` (profile) and `PRI-03` (comparator/rank); #9200 `DST-01` (shared contract) and `DST-05` (snapshot/reconciliation); #9209 `REL-03` (operation/idempotency identity); #8097 is existing lane-filter authority. Component-source aliases such as `PRI-CMP-01`, `DST-SNAPSHOT-01`, and `DST-INTENT-01` resolve to these canonical slice keys in the current issue-packet draft; they are not new backlog IDs. Actual GitHub issue URLs await filing.
- Pinned source `a038f084dbeb525494d7bb928ad91f597db2ac51`; see census Track A, #9204, and research Track A. Historical constants and distribution formula come from source 02 §9 and source 01 §58.
