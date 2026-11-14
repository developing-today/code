# DST-07 — Typed degraded/unsupported/stale behavior and payload limits. Owns proposed src/work/distribution.rs, src/work/contracts.rs, and the same crate's fixture tests

<!-- pmo-9196:DST-07 -->

**Issue title:** `design(pm-orchestration): Typed degraded/unsupported/stale behavior and payload limits. Owns proposed src/work/distribution.rs, src/work/contracts.rs, and the same crate's fixture tests`  
**Design parent:** #9200 · **Scope:** #9200 priority distribution · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** H-R005, P-R026. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

Typed degraded/unsupported/stale behavior and payload limits. Owns proposed `src/work/distribution.rs`, `src/work/contracts.rs`, and the same crate's fixture tests.

## Curated requirement trace

- **H-R005 — partially implemented (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: versioned/degraded/partial output method; no invented runtime capability; occurrences `H-092, H-093, H-094, H-095, H-096`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-092:71, 01_handoff_pm_stack_fleet_orchestration.md#H-093:71, 01_handoff_pm_stack_fleet_orchestration.md#H-094:71, 01_handoff_pm_stack_fleet_orchestration.md#H-095:71, 01_handoff_pm_stack_fleet_orchestration.md#H-096:71; design: components/9200-priority-distribution.md#minimal-event-identity-revision-snapshot-and-receipt; outcome: Treat priority distribution as its own design: trace producers/consumers and specify authoritative policy, precedence, stable IDs, versioned payloads, provenance/reasons, replay/reconciliation, cross-target ordering, and hard eligibility separation.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **P-R026 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: versioned/degraded/partial output method; no invented runtime capability; occurrences `P-1610, P-1611, P-1612, P-1613, P-1614, P-1615, P-1616, P-1617, P-1618, P-1619, P-1620, P-1621, P-1622, P-1623, P-1624, P-1625, P-1626, P-1627, P-1628, P-1629, P-1630, P-1631, P-1632, P-1633, P-1634, P-1635, P-1636, P-1637, P-1638, P-1639, P-1640, P-1641, P-1642, P-1643, P-1644, P-1645, P-1646, P-1647, P-1648, P-1649, P-1650, P-1651, P-1652, P-1653, P-1654, P-1655, P-1656, P-1657, P-1658, P-1659, P-1660`; citations: 03_full_orchestration_proposal.md#P-1610:2247, 03_full_orchestration_proposal.md#P-1611:2247, 03_full_orchestration_proposal.md#P-1612:2249, 03_full_orchestration_proposal.md#P-1613:2251, 03_full_orchestration_proposal.md#P-1614:2253, 03_full_orchestration_proposal.md#P-1615:2255, 03_full_orchestration_proposal.md#P-1616:2257, 03_full_orchestration_proposal.md#P-1617:2259, 03_full_orchestration_proposal.md#P-1618:2261, 03_full_orchestration_proposal.md#P-1619:2262, 03_full_orchestration_proposal.md#P-1620:2263, 03_full_orchestration_proposal.md#P-1621:2264, 03_full_orchestration_proposal.md#P-1622:2265, 03_full_orchestration_proposal.md#P-1623:2266, 03_full_orchestration_proposal.md#P-1624:2268, 03_full_orchestration_proposal.md#P-1625:2270, 03_full_orchestration_proposal.md#P-1626:2272, 03_full_orchestration_proposal.md#P-1627:2274, 03_full_orchestration_proposal.md#P-1628:2275, 03_full_orchestration_proposal.md#P-1629:2276, 03_full_orchestration_proposal.md#P-1630:2277, 03_full_orchestration_proposal.md#P-1631:2278, 03_full_orchestration_proposal.md#P-1632:2279, 03_full_orchestration_proposal.md#P-1633:2280, 03_full_orchestration_proposal.md#P-1634:2282, 03_full_orchestration_proposal.md#P-1635:2284, 03_full_orchestration_proposal.md#P-1636:2286, 03_full_orchestration_proposal.md#P-1637:2288, 03_full_orchestration_proposal.md#P-1638:2290, 03_full_orchestration_proposal.md#P-1639:2292, 03_full_orchestration_proposal.md#P-1640:2294, 03_full_orchestration_proposal.md#P-1641:2296, 03_full_orchestration_proposal.md#P-1642:2297, 03_full_orchestration_proposal.md#P-1643:2298, 03_full_orchestration_proposal.md#P-1644:2299, 03_full_orchestration_proposal.md#P-1645:2301, 03_full_orchestration_proposal.md#P-1646:2303, 03_full_orchestration_proposal.md#P-1647:2305, 03_full_orchestration_proposal.md#P-1648:2307, 03_full_orchestration_proposal.md#P-1649:2309, 03_full_orchestration_proposal.md#P-1650:2311, 03_full_orchestration_proposal.md#P-1651:2312, 03_full_orchestration_proposal.md#P-1652:2313, 03_full_orchestration_proposal.md#P-1653:2314, 03_full_orchestration_proposal.md#P-1654:2315, 03_full_orchestration_proposal.md#P-1655:2316, 03_full_orchestration_proposal.md#P-1656:2317, 03_full_orchestration_proposal.md#P-1657:2318, 03_full_orchestration_proposal.md#P-1658:2320, 03_full_orchestration_proposal.md#P-1659:2322, 03_full_orchestration_proposal.md#P-1660:2324; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Use dependency-aware implementation phases from census/contracts through hydration, lifecycle, sync/activity, direct completion and advanced scheduling; adapt actual order to existing dependencies.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/distribution.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: REL-03
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9209→REL-03.

Source-local candidate aliases normalized for this packet: DST-CMD-01 → DST-01, CLI-02; DST-SNAPSHOT-01 → DST-05.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_distribution_unknown_version_degrades`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_unknown_version_degrades`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

Typed degraded/unsupported/stale behavior and payload limits. Owns proposed `src/work/distribution.rs`, `src/work/contracts.rs`, and the same crate's fixture tests.

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
- Design ref: `components/9200-priority-distribution.md`, slice `DST-07` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): H-R005, P-R026: versioned/degraded/partial output method; no invented runtime capability [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

| `DST-07` / contingent | Typed degraded/unsupported/stale behavior and payload limits. Owns proposed `src/work/distribution.rs`, `src/work/contracts.rs`, and the same crate's fixture tests. | `DST-CMD-01`, `DST-SNAPSHOT-01`; #9209 granular receipt contract | Stale/unsupported never reports fresh/applied; oversize/unknown version is typed and retains authorized recovery intent. |
**Future red-first test commands / oracles per issue:** Exact future Rust test commands are: DST-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_command_identity_reason_and_version`; DST-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_atomic_intent_revision`; DST-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_projection_replay`; DST-06 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_host_order_hash`; DST-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_unknown_version_degrades`. Before each, run that exact command with `-- --list` and assert the named test exists; zero-test success is not acceptance. DST-01 oracle enumerates each reason code and checks field-effect class/no mutation on rejection or unsupported code. DST-02: `cd pm-stack/dispatch && bun test test/validate.test.ts` proves 15/25 round-trip exactly, not current snapping. DST-04 first asserts `test -f pm-stack/projector/test/priority-webhook.test.ts`, then runs `cd pm-stack/projector && bun test test/priority-webhook.test.ts`; DST-05 likewise asserts `test -f pm-stack/projector/test/priority-reconcile.test.ts` before `cd pm-stack/projector && bun test test/priority-reconcile.test.ts`. Both projector oracles cover replay, stale revision, partial effects and readback repair. These are future red-first specs, not current tests. No test may trust `actor_ref` or client-supplied authorization. Hard blockers are granular keys above; #9079/#9081 hosted and #9201 mobile are soft consumers.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `DST-04` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/distribution.rs`
