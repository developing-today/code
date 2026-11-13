# CEN-07 — Real-adapter conformance and CI adoption

<!-- pmo-9196:CEN-07 -->

**Issue title:** `design(pm-orchestration): Real-adapter conformance and CI adoption`  
**Design parent:** #9198 · **Scope:** #9198 crosswalk/census · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R160, C-R162, H-R046, P-R027. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

run shared golden command/receipt/selector/cursor/authority fixtures through existing adapters and add their owner-approved command to CI; no new service/backend. -

## Curated requirement trace

- **H-R046 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: adapter/CI validation method; not a literal request for a new CI workflow or service; occurrences `H-083`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-083:65; design: components/9198-requirement-crosswalk-current-state-census.md#3-v2--contract-and-compatibility-crosswalk; outcome: Include acceptance and migration/rollback tests in the design handoff.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced
- **C-R160 — unverified (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: adapter/CI validation method; not a literal request for a new CI workflow or service; occurrences `C-237`; citations: 02_charter_v1_1.md#C-237:186; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Acceptance must cover atomic claims, stale lease fencing, full/partial hydration, per-source change cursors, external comment handling, managed-content conflicts, triage feedback, weighted exploration statistics, boundary hysteresis, PM Stack projection fidelity, mobile operations, authenticated slash commands, direct Summit/worker session receipt, duplicate-webhook resistance, crash recovery after remote mutation, PR merge verification, and immediate direct queue completion; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R162 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: adapter/CI validation method; not a literal request for a new CI workflow or service; occurrences `C-240`; citations: 02_charter_v1_1.md#C-240:190; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: The design agent should provide: current architecture map, gaps, decisions/ADRs, integrated schema/API/CLI contracts, safe migration plan, phased implementation issues with acceptance tests, risks and go/no-go gates, actual created documents/issue links, dispatch receipts, and verified status; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **P-R027 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: adapter/CI validation method; not a literal request for a new CI workflow or service; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; citations: 03_full_orchestration_proposal.md#P-1662:2328, 03_full_orchestration_proposal.md#P-1663:2328, 03_full_orchestration_proposal.md#P-1664:2330, 03_full_orchestration_proposal.md#P-1665:2332, 03_full_orchestration_proposal.md#P-1666:2334, 03_full_orchestration_proposal.md#P-1667:2335, 03_full_orchestration_proposal.md#P-1668:2336, 03_full_orchestration_proposal.md#P-1669:2337, 03_full_orchestration_proposal.md#P-1670:2338, 03_full_orchestration_proposal.md#P-1671:2339, 03_full_orchestration_proposal.md#P-1672:2341, 03_full_orchestration_proposal.md#P-1673:2343, 03_full_orchestration_proposal.md#P-1674:2344, 03_full_orchestration_proposal.md#P-1675:2345, 03_full_orchestration_proposal.md#P-1676:2346, 03_full_orchestration_proposal.md#P-1677:2347, 03_full_orchestration_proposal.md#P-1678:2348, 03_full_orchestration_proposal.md#P-1679:2349, 03_full_orchestration_proposal.md#P-1680:2350, 03_full_orchestration_proposal.md#P-1681:2352, 03_full_orchestration_proposal.md#P-1682:2354, 03_full_orchestration_proposal.md#P-1683:2355, 03_full_orchestration_proposal.md#P-1684:2356, 03_full_orchestration_proposal.md#P-1685:2357, 03_full_orchestration_proposal.md#P-1686:2358, 03_full_orchestration_proposal.md#P-1687:2359, 03_full_orchestration_proposal.md#P-1688:2361, 03_full_orchestration_proposal.md#P-1689:2363, 03_full_orchestration_proposal.md#P-1690:2364, 03_full_orchestration_proposal.md#P-1691:2365, 03_full_orchestration_proposal.md#P-1692:2366, 03_full_orchestration_proposal.md#P-1693:2367, 03_full_orchestration_proposal.md#P-1694:2368, 03_full_orchestration_proposal.md#P-1695:2369, 03_full_orchestration_proposal.md#P-1696:2370, 03_full_orchestration_proposal.md#P-1697:2371, 03_full_orchestration_proposal.md#P-1698:2373, 03_full_orchestration_proposal.md#P-1699:2375, 03_full_orchestration_proposal.md#P-1700:2376, 03_full_orchestration_proposal.md#P-1701:2377, 03_full_orchestration_proposal.md#P-1702:2378, 03_full_orchestration_proposal.md#P-1703:2379, 03_full_orchestration_proposal.md#P-1704:2380, 03_full_orchestration_proposal.md#P-1705:2382, 03_full_orchestration_proposal.md#P-1706:2384, 03_full_orchestration_proposal.md#P-1707:2385, 03_full_orchestration_proposal.md#P-1708:2386, 03_full_orchestration_proposal.md#P-1709:2387, 03_full_orchestration_proposal.md#P-1710:2388, 03_full_orchestration_proposal.md#P-1711:2389, 03_full_orchestration_proposal.md#P-1712:2390, 03_full_orchestration_proposal.md#P-1713:2391, 03_full_orchestration_proposal.md#P-1714:2392, 03_full_orchestration_proposal.md#P-1715:2393, 03_full_orchestration_proposal.md#P-1716:2395, 03_full_orchestration_proposal.md#P-1717:2397, 03_full_orchestration_proposal.md#P-1718:2398, 03_full_orchestration_proposal.md#P-1719:2399, 03_full_orchestration_proposal.md#P-1720:2400, 03_full_orchestration_proposal.md#P-1721:2401, 03_full_orchestration_proposal.md#P-1722:2402, 03_full_orchestration_proposal.md#P-1723:2403, 03_full_orchestration_proposal.md#P-1724:2404, 03_full_orchestration_proposal.md#P-1725:2405, 03_full_orchestration_proposal.md#P-1726:2406, 03_full_orchestration_proposal.md#P-1727:2407; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts, components/9204-queue-selection-bounded-exploration.md#eligibility-and-deterministic-normal-selection, components/9206-work-item-lifecycle-fleet-subcommands.md#selection-and-peer-order-boundary-consumes-92049200, components/9207-context-hydration-incremental-activity.md#4-incremental-activity-and-cursor-correctness, components/9208-typed-dependencies-notes-journal-field-authority.md#4-field-authority-and-external-synchronization, components/9209-leases-fencing-outbox-direct-completion.md#5-direct-completion-protocol-and-receipt; outcome: Accept against realistic end-to-end selection, hydration, activity, authority, dependency, outbox, lifecycle, completion/recovery and worker scenarios.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `.github/workflows/pmo-contract-conformance.yml`
- `pkgs/fleet/crates/fleet-queue/tests/pmo_contract_conformance.rs`

Explicit read-only consumers (not writable ownership): pm-stack/projector/test/webhook-delivery-id.test.ts, pm-stack/projector/src/webhook.ts.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: PRI-01, REL-03, HYD-06, AUTH-01
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: #9028 (runtime gate: reconcile the current PM PostgREST producer bypass before canonical reservation/cutover; offline contracts and fixtures may proceed).
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9200→PRI-01, #9203→REL-03, #9207→HYD-06, #9208→AUTH-01, #9209→REL-03.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `CI runs existing package suite(s) and exact named filters; unexpected upstream/provider behavior is typed/degraded. Mocks prove contract only; if a live read/write/dispatch acceptance is required, it remains a separately authorized issue. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

CI runs existing package suite(s) and exact named filters; unexpected upstream/provider behavior is typed/degraded. Mocks prove contract only; if a live read/write/dispatch acceptance is required, it remains a separately authorized issue. -

Negative coverage: CI failure prevents claiming conformance; reverting CI hook leaves test scripts but does not roll back any live state. Never silently point projector writes at Fleet or treat projection ACK as command acceptance.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9198-requirement-crosswalk-current-state-census.md`, slice `CEN-07` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): H-R046, C-R160, C-R162, P-R027: adapter/CI validation method; not a literal request for a new CI workflow or service [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### CEN-07 — Real-adapter conformance and CI adoption

- **Tier:** recommended after offline model, before any mutation capability is advertised. **Goal:** run shared golden command/receipt/selector/cursor/authority fixtures through existing adapters and add their owner-approved command to CI; no new service/backend.
- **Owns:** adapter-owned tests in existing `pkgs/fleet/crates/fleet-queue` and `pm-stack/projector/test/webhook-delivery-id.test.ts`, plus proposed `.github/workflows/pmo-contract-conformance.yml`; workflow calls package-owner scripts already declared in Cargo/Bun manifests, not a new service.
- **Hard dependencies:** CEN-05 fixture/output contract, #9200/#9203/#9207/#9208/#9209 adapter seams, current test scripts; #9028 before canonical producer reservation tests.
- **First failing oracle:** `test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance` in the shared conformance suite proves same request/event delivered twice produces one projection event, while canonical operation owner separately emits one accepted operation only after authorization; comparator/manifest/digest result matches independent golden oracle. These are two authorities and two distinct assertions.
- **Done check:** CI runs existing package suite(s) and exact named filters; unexpected upstream/provider behavior is typed/degraded. Mocks prove contract only; if a live read/write/dispatch acceptance is required, it remains a separately authorized issue.
- **Negative / rollback:** CI failure prevents claiming conformance; reverting CI hook leaves test scripts but does not roll back any live state. Never silently point projector writes at Fleet or treat projection ACK as command acceptance.



## Merged source-key/facade acceptance

- `ADAPTER-01` → `CEN-07`: Real-adapter conformance/CI requirement is owned by CEN-07; no adapter backend duplicate. Exact command rows allocated here: no direct command rows.
