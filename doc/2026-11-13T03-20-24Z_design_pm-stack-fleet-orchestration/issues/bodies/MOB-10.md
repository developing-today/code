# MOB-10 — Hosted PM/Fleet projection parity

<!-- pmo-9196:MOB-10 -->

**Issue title:** `design(pm-orchestration): Hosted PM/Fleet projection parity`  
**Design parent:** #9201 · **Scope:** #9201 PM projection/mobile · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** H-R005, H-R023. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

implement the same read/action schemas in those hosted editions with no semantic drift. This is an additive compatibility requirement; do not attribute it as a literal recovered-original clause unless the statement inventory finds that source evidence. -

## Curated requirement trace

- **H-R005 — partially implemented (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: hosted compatibility validation method; not a literal adoption of the recovered original's hosted deployment mechanism; occurrences `H-092, H-093, H-094, H-095, H-096`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-092:71, 01_handoff_pm_stack_fleet_orchestration.md#H-093:71, 01_handoff_pm_stack_fleet_orchestration.md#H-094:71, 01_handoff_pm_stack_fleet_orchestration.md#H-095:71, 01_handoff_pm_stack_fleet_orchestration.md#H-096:71; design: components/9200-priority-distribution.md#minimal-event-identity-revision-snapshot-and-receipt; outcome: Treat priority distribution as its own design: trace producers/consumers and specify authoritative policy, precedence, stable IDs, versioned payloads, provenance/reasons, replay/reconciliation, cross-target ordering, and hard eligibility separation.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R023 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: hosted compatibility validation method; not a literal adoption of the recovered original's hosted deployment mechanism; occurrences `H-083, H-084, H-085, H-086, H-087`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-083:65, 01_handoff_pm_stack_fleet_orchestration.md#H-084:65, 01_handoff_pm_stack_fleet_orchestration.md#H-085:65, 01_handoff_pm_stack_fleet_orchestration.md#H-086:65, 01_handoff_pm_stack_fleet_orchestration.md#H-087:65; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Deliver committed inventory, architecture/component designs, ADRs, traceability, issue index, staged plan, acceptance/migration/rollback tests, census evidence, gaps, and truthful status/receipts; keep implementation issues open until verified.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pm-stack/aggregator/test/hosted-mobile-contract.test.ts`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: MOB-01, #9079, #9081
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `hosted_mobile_contract_matches_shared_golden`
- **Oracle:** same cross-target golden corpus and conformance suite; explicit unsupported capability reports; migration/rollback notes per edition. Existing aggregator corpus command: `cd pm-stack/aggregator && bun run contract:check`; the analogous hosted-suite command must be recorded from #9079/#9081's owning package before implementation. Do not claim deployment until hosted smoke + readback evidence exists. -
- **Done command:** `same cross-target golden corpus and conformance suite; explicit unsupported capability reports; migration/rollback notes per edition. Existing aggregator corpus command: `cd pm-stack/aggregator && bun run contract:check`; the analogous hosted-suite command must be recorded from #9079/#9081's owning package before implementation. Do not claim deployment until hosted smoke + readback evidence exists. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

same cross-target golden corpus and conformance suite; explicit unsupported capability reports; migration/rollback notes per edition. Existing aggregator corpus command: `cd pm-stack/aggregator && bun run contract:check`; the analogous hosted-suite command must be recorded from #9079/#9081's owning package before implementation. Do not claim deployment until hosted smoke + readback evidence exists. -

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
- Design ref: `components/9201-pm-projection-mobile-control.md`, slice `MOB-10` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): H-R005, H-R023: hosted compatibility validation method; not a literal adoption of the recovered original's hosted deployment mechanism [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### MOB-10 — Hosted PM/Fleet projection parity

- **Parent:** #9201, after V2 contract accepted. **Trace:** program-level V4 hosted parity and adjacent hosted-edition work #9079/#9081. **Goal:** implement the same read/action schemas in those hosted editions with no semantic drift. This is an additive compatibility requirement; do not attribute it as a literal recovered-original clause unless the statement inventory finds that source evidence.
- **Owned files:** only the existing hosted service/TS tool modules assigned under #9079/#9081; contract corpus/version fixtures under existing shared test path.
- **Failing test / oracle:** `hosted_mobile_contract_matches_shared_golden` in the common conformance fixture suite returns different column, permissions, or operation state on laptop and hosted implementations.
- **Done/oracle:** same cross-target golden corpus and conformance suite; explicit unsupported capability reports; migration/rollback notes per edition. Existing aggregator corpus command: `cd pm-stack/aggregator && bun run contract:check`; the analogous hosted-suite command must be recorded from #9079/#9081's owning package before implementation. Do not claim deployment until hosted smoke + readback evidence exists.
- **Edges:** hard settled V2, MOB-01/05, #9079/#9081; no duplicate hosted service.
