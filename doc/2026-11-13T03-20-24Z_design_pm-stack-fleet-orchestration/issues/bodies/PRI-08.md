# PRI-08 — Importance-axis mixing and calibration options research

<!-- pmo-9196:PRI-08 -->

**Issue title:** `design(pm-orchestration): Importance-axis mixing and calibration options research`  
**Design parent:** #9199 · **Scope:** #9199 distributed priority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R040, C-R105, H-R010. **Slice status:** design-expansion support, intentionally separate from intake-source requirement IDs. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

compare independent importance/impact signals and future profile-mixing options against the existing low-is-sooner urgency taxonomy; specify evidence, data sufficiency and a null/unavailable behavior. Importance must never be inferred from urgency. No numeric weights, live taxonomy change, schema, or production implementation is selected by this research.

## Curated requirement trace

- **C-R040 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: design-expansion support for distinct conditional importance-axis mixing; importance remains null/unfabricated until evidence and no urgency derivation; occurrences `C-052`; citations: 02_charter_v1_1.md#C-052:37; design: components/9199-distributed-priority.md#independent-dimensions; outcome: - Importance/impact: expected value, strategy, risk and scope; separate field or documented provisional dimension; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R105 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: design-expansion support for distinct conditional importance-axis mixing; importance remains null/unfabricated until evidence and no urgency derivation; occurrences `C-144`; citations: 02_charter_v1_1.md#C-144:103; design: components/9204-queue-selection-bounded-exploration.md#eligibility-and-deterministic-normal-selection; outcome: When separate importance scores exist, evaluate a tunable urgency-versus-importance axis mixture; do not fabricate importance from urgency; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R010 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: design-expansion support for distinct conditional importance-axis mixing; importance remains null/unfabricated until evidence and no urgency derivation; occurrences `H-135, H-136, H-137`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-135:102, 01_handoff_pm_stack_fleet_orchestration.md#H-136:102, 01_handoff_pm_stack_fleet_orchestration.md#H-137:102; design: components/9204-queue-selection-bounded-exploration.md#candidate-exploration-configuration-all-remain-hypotheses; outcome: Retain weighted-selection formula and candidate defaults for evaluation; normalize over eligible work, sample without replacement, expose seeded simulation/explanation, and never fabricate importance.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: #9073, #9074
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9198→runtime gate.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_importance_axis_prior_art_comparison_is_reproducible`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** ``python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md").read_text(); assert all(x in s for x in ("alternatives", "prior art", "null", "not inferred from urgency", "limitations"))'`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

use prior-art A/B receipts, current `#7030` taxonomy and source profile examples; distinguish fact/inference/anecdote, define falsifiable measurement needs and no-data behavior; mark all proposed weights DRAFT.

Negative coverage: copy 450/1000 into live priority, derive impact from urgency, assert simulated production benefit, or claim unlocated QTI equivalence.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: research artifact only; no schema or runtime change.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9199-distributed-priority.md`, slice `PRI-08` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): C-R040, C-R105, H-R010: design-expansion support for distinct conditional importance-axis mixing; importance remains null/unfabricated until evidence and no urgency derivation [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### PRI-08 — Importance-axis mixing and calibration options research

**Tier:** research only. **Goal:** compare independent importance/impact signals and future profile-mixing options against the existing low-is-sooner urgency taxonomy; specify evidence, data sufficiency and a null/unavailable behavior. Importance must never be inferred from urgency. No numeric weights, live taxonomy change, schema, or production implementation is selected by this research.
**Owned files:** `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md` (future research deliverable only).
**Hard blockers:** none for offline research; persisted fields remain gated by #9198/QTI and #9073/#9074 owners.
**First failing oracle:** `pmo_importance_axis_prior_art_comparison_is_reproducible` checks the research artifact contains a source-traceable alternatives matrix, evidence/limitations, a null-data path and explicit decision not to infer importance from urgency.
**Done command:** `python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md").read_text(); assert all(x in s for x in ("alternatives", "prior art", "null", "not inferred from urgency", "limitations"))'`.
**Acceptance:** use prior-art A/B receipts, current `#7030` taxonomy and source profile examples; distinguish fact/inference/anecdote, define falsifiable measurement needs and no-data behavior; mark all proposed weights DRAFT. **Negative:** copy 450/1000 into live priority, derive impact from urgency, assert simulated production benefit, or claim unlocated QTI equivalence. **Rollback:** research artifact only; no schema or runtime change.



## Merged source-key/facade acceptance

- `OPT-IMPORTANCE-01` → `PRI-08`: Importance-axis research maps to PRI-08; no inferred urgency/importance weights. Exact command rows allocated here: no direct command rows.
