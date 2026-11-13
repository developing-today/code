# PRI-07 — Read-only shadow report/test in pkgs/fleet/crates/fleet-queue/src/work/priority.rs

<!-- pmo-9196:PRI-07 -->

**Issue title:** `design(pm-orchestration): Read-only shadow report/test in pkgs/fleet/crates/fleet-queue/src/work/priority.rs`  
**Design parent:** #9199 · **Scope:** #9199 distributed priority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R047, C-R162, H-R046. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

Read-only shadow report/test in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; only separately approved cutover later.

## Curated requirement trace

- **C-R047 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: shadow/migration/rollback validation only; no priority cutover authorization; occurrences `C-070`; citations: 02_charter_v1_1.md#C-070:56; design: components/9199-distributed-priority.md#independent-dimensions; outcome: Validate migration against existing queue values; do not reinterpret them silently; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **H-R046 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: shadow/migration/rollback validation only; no priority cutover authorization; occurrences `H-083`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-083:65; design: components/9198-requirement-crosswalk-current-state-census.md#3-v2--contract-and-compatibility-crosswalk; outcome: Include acceptance and migration/rollback tests in the design handoff.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced
- **C-R162 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: shadow/migration/rollback validation only; no priority cutover authorization; occurrences `C-240`; citations: 02_charter_v1_1.md#C-240:190; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: The design agent should provide: current architecture map, gaps, decisions/ADRs, integrated schema/API/CLI contracts, safe migration plan, phased implementation issues with acceptance tests, risks and go/no-go gates, actual created documents/issue links, dispatch receipts, and verified status; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Source-local candidate aliases normalized for this packet: PRI-PROFILE-01 → PRI-01; PRI-CMP-01 → PRI-03; PRI-PRESSURE-01 → PRI-04; PRI-TRIAGE-01 → PRI-05a; PRI-05 → PRI-05a, PRI-05b, PRI-05c, PRI-05d.


Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_priority_shadow_preserves_live_values`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_shadow_preserves_live_values`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

Read-only shadow report/test in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; only separately approved cutover later.

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
- Design ref: `components/9199-distributed-priority.md`, slice `PRI-07` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): C-R047, H-R046, C-R162: shadow/migration/rollback validation only; no priority cutover authorization [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

| `PRI-07` / contingent | Read-only shadow report/test in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; only separately approved cutover later. | `PRI-PROFILE-01`, `PRI-CMP-01`, `PRI-PRESSURE-01`, `PRI-TRIAGE-01`; Queue Table Improvements only before schema changes; explicit rollout approval | Shadow diff uses pinned snapshot/version; live values unchanged; rollback tests pass; no production rollout in this issue. |
**Future red-first test commands / oracles per issue:** Exact future Rust commands: PRI-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_profile_preserves_ratified_values`; PRI-02 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_dimensions_pin_explanation`; PRI-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_peer_rank_rebalances_local_range`; PRI-04 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dependency_cycle_opposite_edge_pair`; PRI-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_triage_is_bounded_and_bidirectional`; PRI-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_shadow_preserves_live_values`. Before running each, issue verification must run its exact command with `-- --list` and assert the test is present; zero-test success is not acceptance. PRI-01 TS regression: `cd pm-stack/dispatch && bun test test/validate.test.ts` (existing test path). PRI-06 first asserts `test -f pm-stack/aggregator/src/priority-actions.test.ts`, then runs `cd pm-stack/aggregator && bun test src/priority-actions.test.ts`; oracle is shared command result plus visible stale/conflict. These are proposed future tests in the listed existing modules/services, not current tests or a new crate.


## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):
- blocked by `PRI-04` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/priority.rs`
