# MOB-09 — Desired/observed reconciliation, freshness and rollback

<!-- pmo-9196:MOB-09 -->

**Issue title:** `design(pm-orchestration): Desired/observed reconciliation, freshness and rollback`  
**Design parent:** #9201 · **Scope:** #9201 PM projection/mobile · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R082. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

readback/drift receipts, stale/partial read behavior, operational rollback controls. -

## Curated requirement trace

- **C-R082 — design-only (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `C-110`; citations: 02_charter_v1_1.md#C-110:78; design: components/9201-pm-projection-mobile-control.md#31-read-model-one-projection-provenance-per-fact; outcome: Show operation accepted, running, pending external sync, blocked, conflicted, and verified complete separately; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pm-stack/aggregator/src/work-operation-reconciliation.ts`
- `pm-stack/aggregator/test/work-operation-reconciliation.test.ts`

Explicit read-only consumers (not writable ownership): pkgs/fleet/crates/fleet-queue/src/work/operations.rs.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: MOB-05, #8580, #8581
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `fake Project writer accepts request but remote readback remains old; UI presently must not call it verified. Recovery test disables mutations yet leaves pending intent inspectable/replayable. -`
- **Oracle:** fault-injected desired→pending→conflict/verified states; second independent read confirms every “verified” fixture. Shadow mode compares live projection to legacy board with no writes; rollback config disables new mutation adapter and retains receipts/action intents for later review. -
- **Done command:** `fault-injected desired→pending→conflict/verified states; second independent read confirms every “verified” fixture. Shadow mode compares live projection to legacy board with no writes; rollback config disables new mutation adapter and retains receipts/action intents for later review. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

fault-injected desired→pending→conflict/verified states; second independent read confirms every “verified” fixture. Shadow mode compares live projection to legacy board with no writes; rollback config disables new mutation adapter and retains receipts/action intents for later review. -

Negative coverage: stale input, duplicate/replay, ambiguous resource, unauthorized actor, partial remote effect, unknown version, and no false verified/completed result where applicable.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9201-pm-projection-mobile-control.md`, slice `MOB-09` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### MOB-09 — Desired/observed reconciliation, freshness and rollback

- **Parent:** #9201. **Requirements:** H-DISTRIBUTED/H-SCENARIOS, C-6/C-16/C-17, O §18–20. **Goal:** readback/drift receipts, stale/partial read behavior, operational rollback controls.
- **Owned files:** `pm-stack/aggregator/src/work-projection.ts`, proposed `src/work-operation-reconciliation.ts`, `test/work-operation-reconciliation.test.ts`, freshness display in `src/render.ts`, and rollback/runbook docs. No new durable store.
- **Failing test:** fake Project writer accepts request but remote readback remains old; UI presently must not call it verified. Recovery test disables mutations yet leaves pending intent inspectable/replayable.
- **Named red test:** `work_verified_effect_requires_source_readback` in `pm-stack/aggregator/test/work-operation-reconciliation.test.ts`.
- **Done/oracle:** fault-injected desired→pending→conflict/verified states; second independent read confirms every “verified” fixture. Shadow mode compares live projection to legacy board with no writes; rollback config disables new mutation adapter and retains receipts/action intents for later review.
- **Edges:** hard MOB-05/06; Project writer and #8580/#8581 integration; live Project proof is separately recorded, mocks never equal deployment.
