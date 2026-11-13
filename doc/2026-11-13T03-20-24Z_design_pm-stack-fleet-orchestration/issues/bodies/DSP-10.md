# DSP-10 — Deployable source integration, observability and rollback

<!-- pmo-9196:DSP-10 -->

**Issue title:** `design(pm-orchestration): Deployable source integration, observability and rollback`  
**Design parent:** #9203 · **Scope:** #9203 direct dispatch/events · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R160, C-R162, H-R046, H-R048, O-R065. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

wire supported entrypoints and dashboards for operator inspection with kill switch and retention/retry policy. -

## Curated requirement trace

- **H-R046 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: rollout, recovery and evidence distinctions; no deployment or schedule authorization; occurrences `H-083`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-083:65; design: components/9198-requirement-crosswalk-current-state-census.md#3-v2--contract-and-compatibility-crosswalk; outcome: Include acceptance and migration/rollback tests in the design handoff.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced
- **H-R048 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: rollout, recovery and evidence distinctions; no deployment or schedule authorization; occurrences `H-085`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-085:65; design: components/9198-requirement-crosswalk-current-state-census.md#3-v2--contract-and-compatibility-crosswalk; outcome: Report designed, implemented, merged, deployed, live-verified, blocked, and pending states separately.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced
- **C-R160 — unverified (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: rollout, recovery and evidence distinctions; no deployment or schedule authorization; occurrences `C-237`; citations: 02_charter_v1_1.md#C-237:186; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Acceptance must cover atomic claims, stale lease fencing, full/partial hydration, per-source change cursors, external comment handling, managed-content conflicts, triage feedback, weighted exploration statistics, boundary hysteresis, PM Stack projection fidelity, mobile operations, authenticated slash commands, direct Summit/worker session receipt, duplicate-webhook resistance, crash recovery after remote mutation, PR merge verification, and immediate direct queue completion; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R162 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: rollout, recovery and evidence distinctions; no deployment or schedule authorization; occurrences `C-240`; citations: 02_charter_v1_1.md#C-240:190; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: The design agent should provide: current architecture map, gaps, decisions/ADRs, integrated schema/API/CLI contracts, safe migration plan, phased implementation issues with acceptance tests, risks and go/no-go gates, actual created documents/issue links, dispatch receipts, and verified status; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **O-R065 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: rollout, recovery and evidence distinctions; no deployment or schedule authorization; occurrences `O-629, O-630, O-631, O-632, O-633, O-634, O-635`; citations: 04_recovered_original_combined_design.md#O-629:13, 04_recovered_original_combined_design.md#O-630:13, 04_recovered_original_combined_design.md#O-631:13, 04_recovered_original_combined_design.md#O-632:13, 04_recovered_original_combined_design.md#O-633:13, 04_recovered_original_combined_design.md#O-634:13, 04_recovered_original_combined_design.md#O-635:13; design: components/9203-direct-dispatch-slash-commands-event-routing.md#3-v2--versioned-routing-contract-and-state-machines; outcome: Deduplicate event flow, bind and report dispatch receipts; on failure preserve retry intent and never silently downgrade immediate requests to ordinary queue work.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pm-stack/dispatch/test/event-route-rollback.test.ts`
- `pm-stack/dispatch/README.md`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: DSP-01, CLI-02, REL-03, DSP-04, DSP-05, DSP-06, DSP-07
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9196→runtime gate.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_dsp_10_disabled_route_has_zero_effects`
- **Oracle:** route disabled emits zero effects; stage smoke with signed test event and fake provider reports shared Result outcomes and receipt with secret/prompt canaries absent; rollback disables apply and preserves receipts. Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`. No production launch in this slice; the existing #6533 live proof is separately authorized/evidenced, and deploy state separately verified from source merge. -
- **Done command:** `route disabled emits zero effects; stage smoke with signed test event and fake provider reports shared Result outcomes and receipt with secret/prompt canaries absent; rollback disables apply and preserves receipts. Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`. No production launch in this slice; the existing #6533 live proof is separately authorized/evidenced, and deploy state separately verified from source merge. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

route disabled emits zero effects; stage smoke with signed test event and fake provider reports shared Result outcomes and receipt with secret/prompt canaries absent; rollback disables apply and preserves receipts. Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`. No production launch in this slice; the existing #6533 live proof is separately authorized/evidenced, and deploy state separately verified from source merge. -

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
- Design ref: `components/9203-direct-dispatch-slash-commands-event-routing.md`, slice `DSP-10` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): H-R046, H-R048, C-R160, C-R162, O-R065: rollout, recovery and evidence distinctions; no deployment or schedule authorization [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### DSP-10 — Deployable source integration, observability and rollback

- **Requirements:** H-SURVIVE-A/H-SCENARIOS, C-16/C-17, O §§17/20. **Goal:** wire supported entrypoints and dashboards for operator inspection with kill switch and retention/retry policy.
- **Owned files:** existing dispatcher/webhook/orchestration configuration under exact service owner, operator docs/runbook, metrics and deploy smoke tests. Fleet schedule registry updated in same change if a schedule is introduced/changed.
- **Failing test to start:** `pmo_dsp_10_disabled_route_has_zero_effects` in `pm-stack/dispatch/test/event-route-rollback.test.ts`.
- **Done/oracle:** route disabled emits zero effects; stage smoke with signed test event and fake provider reports shared Result outcomes and receipt with secret/prompt canaries absent; rollback disables apply and preserves receipts. Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`. No production launch in this slice; the existing #6533 live proof is separately authorized/evidenced, and deploy state separately verified from source merge.
- **Dependencies:** DSP-01 through DSP-07; #9196 safety gates; change/deploy approval from service owner.
