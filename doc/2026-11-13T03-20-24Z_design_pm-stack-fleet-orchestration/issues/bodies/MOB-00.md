# MOB-00 — Deterministic browser acceptance harness for mobile views

<!-- pmo-9196:MOB-00 -->

**Issue title:** `design(pm-orchestration): Deterministic browser acceptance harness for mobile views`  
**Design parent:** #9201 · **Scope:** #9201 PM projection/mobile · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R081, C-R160, O-R013. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

add browser-level evidence for responsive/mobile and accessible interactions without changing production semantics. -

## Curated requirement trace

- **C-R081 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: browser test harness as an explicit validation method, not a new app/service; occurrences `C-109`; citations: 02_charter_v1_1.md#C-109:78; design: components/9201-pm-projection-mobile-control.md#31-read-model-one-projection-provenance-per-fact; outcome: From a phone or browser, the authorized user should be able to search/filter the Fleet queue, read summaries and linked issues/PRs, add notes and comments, reprioritize, change permitted state, request triage or direct agent dispatch, inspect dependencies, request a status report, find an active session and send a live instruction; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R160 — unverified (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: browser test harness as an explicit validation method, not a new app/service; occurrences `C-237`; citations: 02_charter_v1_1.md#C-237:186; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Acceptance must cover atomic claims, stale lease fencing, full/partial hydration, per-source change cursors, external comment handling, managed-content conflicts, triage feedback, weighted exploration statistics, boundary hysteresis, PM Stack projection fidelity, mobile operations, authenticated slash commands, direct Summit/worker session receipt, duplicate-webhook resistance, crash recovery after remote mutation, PR merge verification, and immediate direct queue completion; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **O-R013 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: browser test harness as an explicit validation method, not a new app/service; occurrences `O-535, O-536, O-537, O-538, O-539, O-540, O-541, O-542, O-543, O-544, O-545, O-546, O-547, O-548, O-549, O-550, O-551, O-552, O-553, O-554, O-555, O-556, O-557, O-558, O-559, O-560, O-561, O-562, O-563, O-564, O-565, O-566, O-567, O-568, O-569, O-570, O-571, O-572, O-573, O-574, O-575, O-576, O-577, O-578, O-579, O-580, O-581`; citations: 04_recovered_original_combined_design.md#O-535:13, 04_recovered_original_combined_design.md#O-536:13, 04_recovered_original_combined_design.md#O-537:13, 04_recovered_original_combined_design.md#O-538:13, 04_recovered_original_combined_design.md#O-539:13, 04_recovered_original_combined_design.md#O-540:13, 04_recovered_original_combined_design.md#O-541:13, 04_recovered_original_combined_design.md#O-542:13, 04_recovered_original_combined_design.md#O-543:13, 04_recovered_original_combined_design.md#O-544:13, 04_recovered_original_combined_design.md#O-545:13, 04_recovered_original_combined_design.md#O-546:13, 04_recovered_original_combined_design.md#O-547:13, 04_recovered_original_combined_design.md#O-548:13, 04_recovered_original_combined_design.md#O-549:13, 04_recovered_original_combined_design.md#O-550:13, 04_recovered_original_combined_design.md#O-551:13, 04_recovered_original_combined_design.md#O-552:13, 04_recovered_original_combined_design.md#O-553:13, 04_recovered_original_combined_design.md#O-554:13, 04_recovered_original_combined_design.md#O-555:13, 04_recovered_original_combined_design.md#O-556:13, 04_recovered_original_combined_design.md#O-557:13, 04_recovered_original_combined_design.md#O-558:13, 04_recovered_original_combined_design.md#O-559:13, 04_recovered_original_combined_design.md#O-560:13, 04_recovered_original_combined_design.md#O-561:13, 04_recovered_original_combined_design.md#O-562:13, 04_recovered_original_combined_design.md#O-563:13, 04_recovered_original_combined_design.md#O-564:13, 04_recovered_original_combined_design.md#O-565:13, 04_recovered_original_combined_design.md#O-566:13, 04_recovered_original_combined_design.md#O-567:13, 04_recovered_original_combined_design.md#O-568:13, 04_recovered_original_combined_design.md#O-569:13, 04_recovered_original_combined_design.md#O-570:13, 04_recovered_original_combined_design.md#O-571:13, 04_recovered_original_combined_design.md#O-572:13, 04_recovered_original_combined_design.md#O-573:13, 04_recovered_original_combined_design.md#O-574:13, 04_recovered_original_combined_design.md#O-575:13, 04_recovered_original_combined_design.md#O-576:13, 04_recovered_original_combined_design.md#O-577:13, 04_recovered_original_combined_design.md#O-578:13, 04_recovered_original_combined_design.md#O-579:13, 04_recovered_original_combined_design.md#O-580:13, 04_recovered_original_combined_design.md#O-581:13; design: components/9201-pm-projection-mobile-control.md#34-mobile-and-accessibility-envelope, components/9203-direct-dispatch-slash-commands-event-routing.md#34-direct-session-routing-and-launch-reservation; outcome: Provide interactive mobile control and capability-aware agent panel through same authorized Fleet operation layer; report pending/rejected/conflicted outcomes and actual provider capabilities.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pm-stack/aggregator/package.json`
- `pm-stack/aggregator/bun.lock`
- `pm-stack/aggregator/playwright.config.ts`
- `pm-stack/aggregator/test/browser/mobile-work-view.spec.ts`
- `.github/workflows/pm-aggregator-tests.yml`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract`
- **Oracle:** isolated deterministic fixture data only (network blocked except local test server; API fixtures or existing fixture-source mode), no GitHub/Project/session credentials. Exact command: `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. `bun test` contract/read-model fixtures remain separate and fast. CI runs this job in the existing aggregator workflow with pinned dependency/browser cache; report trace/video on failure, not screenshot-only pass. Verify the named test is listed and actually executes; zero-test success is not acceptance. -
- **Done command:** `isolated deterministic fixture data only (network blocked except local test server; API fixtures or existing fixture-source mode), no GitHub/Project/session credentials. Exact command: `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. `bun test` contract/read-model fixtures remain separate and fast. CI runs this job in the existing aggregator workflow with pinned dependency/browser cache; report trace/video on failure, not screenshot-only pass. Verify the named test is listed and actually executes; zero-test success is not acceptance. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

isolated deterministic fixture data only (network blocked except local test server; API fixtures or existing fixture-source mode), no GitHub/Project/session credentials. Exact command: `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. `bun test` contract/read-model fixtures remain separate and fast. CI runs this job in the existing aggregator workflow with pinned dependency/browser cache; report trace/video on failure, not screenshot-only pass. Verify the named test is listed and actually executes; zero-test success is not acceptance. -

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
- Design ref: `components/9201-pm-projection-mobile-control.md`, slice `MOB-00` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): C-R081, C-R160, O-R013: browser test harness as an explicit validation method, not a new app/service [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### MOB-00 — Deterministic browser acceptance harness for mobile views

- **Parent:** #9201. **Requirements:** C-7, O §13. **Goal:** add browser-level evidence for responsive/mobile and accessible interactions without changing production semantics.
- **Existing substrate:** `pm-stack/aggregator` has Bun unit tests and `.github/workflows/pm-aggregator-tests.yml`; package currently has no Playwright config/dependency/browser-test CI target. This slice extends those, not a new service/workflow/binary.
- **Owned files:** `pm-stack/aggregator/package.json`, `bun.lock`, new `playwright.config.ts`, new `test/browser/mobile-work-view.spec.ts`, and the existing `.github/workflows/pm-aggregator-tests.yml` test job. Pin the exact `@playwright/test` package version in package manifest and lockfile, pin/browser-cache matching browser revision in CI, and make browser install deterministic. No dependency installation or browser run is part of this design pass.
- **Failing test to start:** `pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract` in `test/browser/mobile-work-view.spec.ts`; it begins red on the current view by asserting primary 360px queue/detail flow, keyboard reachability, and accessible action/status announcement.
- **Done/oracle:** isolated deterministic fixture data only (network blocked except local test server; API fixtures or existing fixture-source mode), no GitHub/Project/session credentials. Exact command: `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. `bun test` contract/read-model fixtures remain separate and fast. CI runs this job in the existing aggregator workflow with pinned dependency/browser cache; report trace/video on failure, not screenshot-only pass. Verify the named test is listed and actually executes; zero-test success is not acceptance.
- **Edges:** MOB-03/MOB-04 browser acceptance hard-blocked on MOB-00; their pure model/render unit tests and design may proceed. No new project-wide browser service or binary.
