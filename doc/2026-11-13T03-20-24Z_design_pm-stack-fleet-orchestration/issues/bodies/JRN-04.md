# JRN-04 — Full-history replay and bounded retention feasibility research

<!-- pmo-9196:JRN-04 -->

**Issue title:** `design(pm-orchestration): Full-history replay and bounded retention feasibility research`  
**Design parent:** #9208 · **Scope:** #9208 relations/journal/authority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** H-R023, P-R006. **Slice status:** design-expansion support, intentionally separate from intake-source requirement IDs. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

investigate the source requirement for full event/history replay against current queue `events`, `result`, `blocked_deps`, #3424 six-piece durable task model and #8579/#8602 schema audit; define what can be reconstructed, what is merely telemetry/observation, and bounded retention/redaction/compaction alternatives. Do not create a second event database or claim full event sourcing.

## Curated requirement trace

- **P-R006 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: design-expansion feasibility research only; also consumes #3424's existing full-history requirement; not a literal full-event-store mandate or parallel DB; occurrences `P-110, P-111, P-112, P-113, P-114, P-115, P-116, P-117, P-118, P-119, P-120, P-121, P-122`; citations: 03_full_orchestration_proposal.md#P-110:165, 03_full_orchestration_proposal.md#P-111:167, 03_full_orchestration_proposal.md#P-112:169, 03_full_orchestration_proposal.md#P-113:171, 03_full_orchestration_proposal.md#P-114:172, 03_full_orchestration_proposal.md#P-115:173, 03_full_orchestration_proposal.md#P-116:174, 03_full_orchestration_proposal.md#P-117:175, 03_full_orchestration_proposal.md#P-118:176, 03_full_orchestration_proposal.md#P-119:177, 03_full_orchestration_proposal.md#P-120:178, 03_full_orchestration_proposal.md#P-121:179, 03_full_orchestration_proposal.md#P-122:181; design: components/9209-leases-fencing-outbox-direct-completion.md#3-leasefencing-and-existing-one-queue-contract; outcome: Make work recoverable across interruption using durable progress/ownership, idempotency, outbox, optimistic concurrency, leases, checkpoints, incremental activity, and repair.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R023 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: design-expansion feasibility research only; also consumes #3424's existing full-history requirement; not a literal full-event-store mandate or parallel DB; occurrences `H-083, H-084, H-085, H-086, H-087`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-083:65, 01_handoff_pm_stack_fleet_orchestration.md#H-084:65, 01_handoff_pm_stack_fleet_orchestration.md#H-085:65, 01_handoff_pm_stack_fleet_orchestration.md#H-086:65, 01_handoff_pm_stack_fleet_orchestration.md#H-087:65; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Deliver committed inventory, architecture/component designs, ADRs, traceability, issue index, staged plan, acceptance/migration/rollback tests, census evidence, gaps, and truthful status/receipts; keep implementation issues open until verified.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: CEN-04
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: #3424 (existing durable-task-model compatibility evidence; do not duplicate its research/intake issue), #8579 (existing schema-audit evidence; consume, do not duplicate).
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9198→runtime gate.

Source-local candidate aliases normalized for this packet: DEP-01 → CEN-04.


Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_history_replay_evidence_matrix_distinguishes_observation_from_authority`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** ``python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md").read_text(); assert all(x in s for x in ("retention", "replay", "telemetry", "unknown", "alternatives", "prior art"))'`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

use Track A/B prior-art receipts and historical issue/doc sources; record source pointer, accessible surfaces, limits, retention/redaction and rollback implications, migration evidence requirements and alternatives including no full replay. Never infer immutable domain-journal semantics from an `events` table name.

Negative coverage: reopen/alter #3424/#8579 research ownership, equate matching schema shapes, invent unbounded retention, or claim deployment.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: research only.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9208-typed-dependencies-notes-journal-field-authority.md`, slice `JRN-04` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: https://github.com/developing-today/code/issues/3424, doc/2026-11-10_design_queue_task_data_model_3424.md:43-46.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): P-R006, H-R023: design-expansion feasibility research only; also consumes #3424's existing full-history requirement; not a literal full-event-store mandate or parallel DB [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: developing-today/code#3424 doc/2026-11-10_design_queue_task_data_model_3424.md:43-46: Full event history — an append-only events table records every task change with before/after values and actor; past state is reconstructable from the stream. (existing #3424 requirement retained for feasibility research only; not assigned a fabricated crosswalk ID and not a mandate to create a parallel event store).
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### JRN-04 — Full-history replay and bounded retention feasibility research

**Tier:** research only. **Goal:** investigate the source requirement for full event/history replay against current queue `events`, `result`, `blocked_deps`, #3424 six-piece durable task model and #8579/#8602 schema audit; define what can be reconstructed, what is merely telemetry/observation, and bounded retention/redaction/compaction alternatives. Do not create a second event database or claim full event sourcing.
**Owned files:** `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md` (future research deliverable only).
**Hard blockers:** none for read-only research; coordinate with DEP-01 and #9198 crosswalk, but do not duplicate the worker's #3424/#8579 candidate investigation.
**Recommended after:** DEP-01 crosswalk review where available.
**First failing oracle:** `pmo_history_replay_evidence_matrix_distinguishes_observation_from_authority` checks every record source is classified as authoritative mutation, observation, telemetry, projection or unknown, and every replay claim has primary-source support.
**Done command:** `python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md").read_text(); assert all(x in s for x in ("retention", "replay", "telemetry", "unknown", "alternatives", "prior art"))'`.
**Acceptance:** use Track A/B prior-art receipts and historical issue/doc sources; record source pointer, accessible surfaces, limits, retention/redaction and rollback implications, migration evidence requirements and alternatives including no full replay. Never infer immutable domain-journal semantics from an `events` table name. **Negative:** reopen/alter #3424/#8579 research ownership, equate matching schema shapes, invent unbounded retention, or claim deployment. **Rollback:** research only.



## Merged source-key/facade acceptance

- `OPT-REPLAY-01` → `JRN-04`: Full-history replay/retention research maps to JRN-04; no event-sourcing rewrite. Exact command rows allocated here: no direct command rows.
