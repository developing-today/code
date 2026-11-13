# MOB-03 — Responsive mobile queue, item detail, and saved views

<!-- pmo-9196:MOB-03 -->

**Issue title:** `design(pm-orchestration): Responsive mobile queue, item detail, and saved views`  
**Design parent:** #9201 · **Scope:** #9201 PM projection/mobile · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R083, H-R001, H-R013, O-R013. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

expose all/triage/ready/high/critical-path/blocking/my-work/active-agent/review/blocked-parked/needs-triage/recent/cold backlog/by repo/project/owner views from one read model. -

## Curated requirement trace

- **C-R083 — design-only (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `C-111`; citations: 02_charter_v1_1.md#C-111:78; design: components/9201-pm-projection-mobile-control.md#34-mobile-and-accessibility-envelope; outcome: Use accessible touch-sized controls, a compact responsive layout, keyboard navigation, and confirmations for consequential actions; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R001 — design-only (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `H-029`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-029:28; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts, components/9199-distributed-priority.md#triage-mechanics, components/9200-priority-distribution.md#cross-target-order-and-reconciliation, components/9201-pm-projection-mobile-control.md#34-mobile-and-accessibility-envelope, components/9203-direct-dispatch-slash-commands-event-routing.md#34-direct-session-routing-and-launch-reservation, components/9204-queue-selection-bounded-exploration.md#eligibility-and-deterministic-normal-selection, components/9206-work-item-lifecycle-fleet-subcommands.md#selection-and-peer-order-boundary-consumes-92049200, components/9207-context-hydration-incremental-activity.md#22-versioned-manifest-normative-shape-candidate, components/9208-typed-dependencies-notes-journal-field-authority.md#3-notes-checkpoints-and-append-only-journal, components/9209-leases-fencing-outbox-direct-completion.md#5-direct-completion-protocol-and-receipt; outcome: Preserve the two coordinated tracks: priority/triage/PM visibility/direct dispatch/mobile control and lifecycle/hydration/dependencies/journal/outbox/direct verified completion.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R013 — design-only (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `H-070`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-070:55; design: components/9201-pm-projection-mobile-control.md#34-mobile-and-accessibility-envelope; outcome: Provide mobile-first PM status/search/notes/reprioritization/triage/dispatch/live-session controls via shared validated Fleet APIs, not independent frontend policy.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **O-R013 — design-only (candidate)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `O-535, O-536, O-537, O-538, O-539, O-540, O-541, O-542, O-543, O-544, O-545, O-546, O-547, O-548, O-549, O-550, O-551, O-552, O-553, O-554, O-555, O-556, O-557, O-558, O-559, O-560, O-561, O-562, O-563, O-564, O-565, O-566, O-567, O-568, O-569, O-570, O-571, O-572, O-573, O-574, O-575, O-576, O-577, O-578, O-579, O-580, O-581`; citations: 04_recovered_original_combined_design.md#O-535:13, 04_recovered_original_combined_design.md#O-536:13, 04_recovered_original_combined_design.md#O-537:13, 04_recovered_original_combined_design.md#O-538:13, 04_recovered_original_combined_design.md#O-539:13, 04_recovered_original_combined_design.md#O-540:13, 04_recovered_original_combined_design.md#O-541:13, 04_recovered_original_combined_design.md#O-542:13, 04_recovered_original_combined_design.md#O-543:13, 04_recovered_original_combined_design.md#O-544:13, 04_recovered_original_combined_design.md#O-545:13, 04_recovered_original_combined_design.md#O-546:13, 04_recovered_original_combined_design.md#O-547:13, 04_recovered_original_combined_design.md#O-548:13, 04_recovered_original_combined_design.md#O-549:13, 04_recovered_original_combined_design.md#O-550:13, 04_recovered_original_combined_design.md#O-551:13, 04_recovered_original_combined_design.md#O-552:13, 04_recovered_original_combined_design.md#O-553:13, 04_recovered_original_combined_design.md#O-554:13, 04_recovered_original_combined_design.md#O-555:13, 04_recovered_original_combined_design.md#O-556:13, 04_recovered_original_combined_design.md#O-557:13, 04_recovered_original_combined_design.md#O-558:13, 04_recovered_original_combined_design.md#O-559:13, 04_recovered_original_combined_design.md#O-560:13, 04_recovered_original_combined_design.md#O-561:13, 04_recovered_original_combined_design.md#O-562:13, 04_recovered_original_combined_design.md#O-563:13, 04_recovered_original_combined_design.md#O-564:13, 04_recovered_original_combined_design.md#O-565:13, 04_recovered_original_combined_design.md#O-566:13, 04_recovered_original_combined_design.md#O-567:13, 04_recovered_original_combined_design.md#O-568:13, 04_recovered_original_combined_design.md#O-569:13, 04_recovered_original_combined_design.md#O-570:13, 04_recovered_original_combined_design.md#O-571:13, 04_recovered_original_combined_design.md#O-572:13, 04_recovered_original_combined_design.md#O-573:13, 04_recovered_original_combined_design.md#O-574:13, 04_recovered_original_combined_design.md#O-575:13, 04_recovered_original_combined_design.md#O-576:13, 04_recovered_original_combined_design.md#O-577:13, 04_recovered_original_combined_design.md#O-578:13, 04_recovered_original_combined_design.md#O-579:13, 04_recovered_original_combined_design.md#O-580:13, 04_recovered_original_combined_design.md#O-581:13; design: components/9201-pm-projection-mobile-control.md#34-mobile-and-accessibility-envelope, components/9203-direct-dispatch-slash-commands-event-routing.md#34-direct-session-routing-and-launch-reservation; outcome: Provide interactive mobile control and capability-aware agent panel through same authorized Fleet operation layer; report pending/rejected/conflicted outcomes and actual provider capabilities.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pm-stack/aggregator/src/render.ts`
- `pm-stack/aggregator/src/ui.css`
- `pm-stack/aggregator/src/work-mobile-views.ts`
- `pm-stack/aggregator/test/work-mobile-views.test.ts`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: MOB-00, MOB-01, #9070, #1276
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `page at 360px and 200% zoom must have no horizontal overflow for queue and detail flows; keyboard can reach filter, item, back, and capability panel. -`
- **Oracle:** Bun reference-filter unit test `cd pm-stack/aggregator && bun test test/work-mobile-views.test.ts` plus browser assertions in `test/browser/mobile-work-view.spec.ts`, exact command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. Browser oracle checks rendered refs vs pure view filter at 360/768/1280 CSS px and 200% zoom; no screenshot-only pass. -
- **Done command:** `Bun reference-filter unit test `cd pm-stack/aggregator && bun test test/work-mobile-views.test.ts` plus browser assertions in `test/browser/mobile-work-view.spec.ts`, exact command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. Browser oracle checks rendered refs vs pure view filter at 360/768/1280 CSS px and 200% zoom; no screenshot-only pass. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

Bun reference-filter unit test `cd pm-stack/aggregator && bun test test/work-mobile-views.test.ts` plus browser assertions in `test/browser/mobile-work-view.spec.ts`, exact command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. Browser oracle checks rendered refs vs pure view filter at 360/768/1280 CSS px and 200% zoom; no screenshot-only pass. -

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
- Design ref: `components/9201-pm-projection-mobile-control.md`, slice `MOB-03` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### MOB-03 — Responsive mobile queue, item detail, and saved views

- **Parent:** #9201. **Requirements:** H-SURVIVE-A, C-7, O §§12–13, all 15 mobile requirements read-only subset. **Goal:** expose all/triage/ready/high/critical-path/blocking/my-work/active-agent/review/blocked-parked/needs-triage/recent/cold backlog/by repo/project/owner views from one read model.
- **Owned files:** `pm-stack/aggregator/src/render.ts`, `src/ui.css`, proposed `src/work-views.ts`, `test/work-mobile-views.test.ts`, browser fixture and view documentation; avoid action-store semantics.
- **Failing test:** page at 360px and 200% zoom must have no horizontal overflow for queue and detail flows; keyboard can reach filter, item, back, and capability panel.
- **Named red test:** `work_mobile_views_match_reference_filter` in `pm-stack/aggregator/test/work-mobile-views.test.ts`.
- **Done/oracle:** Bun reference-filter unit test `cd pm-stack/aggregator && bun test test/work-mobile-views.test.ts` plus browser assertions in `test/browser/mobile-work-view.spec.ts`, exact command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. Browser oracle checks rendered refs vs pure view filter at 360/768/1280 CSS px and 200% zoom; no screenshot-only pass.
- **Edges:** hard MOB-00 for browser acceptance, MOB-01/02 for projection behavior; soft #9070 genealogy and existing views store #1276; no new view database.
