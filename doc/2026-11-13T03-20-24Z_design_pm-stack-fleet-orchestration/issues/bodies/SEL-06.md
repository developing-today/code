# SEL-06 — Lane fairness policy interface in src/work/selection.rs

<!-- pmo-9196:SEL-06 -->

**Issue title:** `design(pm-orchestration): Lane fairness policy interface in src/work/selection.rs`  
**Design parent:** #9204 · **Scope:** #9204 queue selection/exploration · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R009, C-R096, P-R014. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

Lane fairness policy interface in `src/work/selection.rs`; consume #8097's existing filter contract.

## Curated requirement trace

- **C-R009 — partially implemented (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `C-012`; citations: 02_charter_v1_1.md#C-012:17; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Agents should be able to discover work, understand its priority and dependencies, claim it safely, hydrate GitHub context into files, execute work, report notes and comments, handle new activity, park or resume, and complete linked PR/issue/queue states directly and verifiably; residual: Full integrated command, hydration, event routing, typed dependencies, journal/outbox and remote verified completion remain separate component implementation slices.; missing evidence: end-to-end reference-model scenario A-D passing against composed interfaces, implementation and deployed/runtime proof for full workflow
- **C-R096 — partially implemented (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `C-126`; citations: 02_charter_v1_1.md#C-126:90; design: components/9204-queue-selection-bounded-exploration.md#fairness-is-separate; outcome: Normal lanes should select the highest eligible effective scheduling urgency, subject to capability, ownership, dependency and fairness rules; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **P-R014 — design-only (candidate)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `P-522, P-523, P-524, P-525, P-526, P-527, P-528, P-529, P-530, P-531, P-532, P-533, P-534, P-535, P-536, P-537, P-538, P-539, P-540, P-541, P-542, P-543, P-544, P-545, P-546, P-547, P-548, P-549, P-550, P-551, P-552`; citations: 03_full_orchestration_proposal.md#P-522:729, 03_full_orchestration_proposal.md#P-523:729, 03_full_orchestration_proposal.md#P-524:731, 03_full_orchestration_proposal.md#P-525:733, 03_full_orchestration_proposal.md#P-526:735, 03_full_orchestration_proposal.md#P-527:737, 03_full_orchestration_proposal.md#P-528:739, 03_full_orchestration_proposal.md#P-529:741, 03_full_orchestration_proposal.md#P-530:743, 03_full_orchestration_proposal.md#P-531:745, 03_full_orchestration_proposal.md#P-532:747, 03_full_orchestration_proposal.md#P-533:749, 03_full_orchestration_proposal.md#P-534:750, 03_full_orchestration_proposal.md#P-535:751, 03_full_orchestration_proposal.md#P-536:752, 03_full_orchestration_proposal.md#P-537:753, 03_full_orchestration_proposal.md#P-538:754, 03_full_orchestration_proposal.md#P-539:755, 03_full_orchestration_proposal.md#P-540:756, 03_full_orchestration_proposal.md#P-541:757, 03_full_orchestration_proposal.md#P-542:758, 03_full_orchestration_proposal.md#P-543:760, 03_full_orchestration_proposal.md#P-544:762, 03_full_orchestration_proposal.md#P-545:764, 03_full_orchestration_proposal.md#P-546:766, 03_full_orchestration_proposal.md#P-547:768, 03_full_orchestration_proposal.md#P-548:770, 03_full_orchestration_proposal.md#P-549:771, 03_full_orchestration_proposal.md#P-550:772, 03_full_orchestration_proposal.md#P-551:773, 03_full_orchestration_proposal.md#P-552:775; design: components/9199-distributed-priority.md#candidate-order-and-rank-semantics, components/9204-queue-selection-bounded-exploration.md#candidate-exploration-configuration-all-remain-hypotheses; outcome: Keep numeric priority configurable/canonical with derived labels; ranking considers age, dependencies, capability, complexity, repository affinity, fairness, starvation, weighted randomness and optional lower-number preference; prevent cluster domination and explain policy/eligibility/result.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/selection.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: #8097
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_selection_fairness_preserves_urgent_order`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_fairness_preserves_urgent_order`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

Lane fairness policy interface in `src/work/selection.rs`; consume #8097's existing filter contract.

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
- Design ref: `components/9204-queue-selection-bounded-exploration.md`, slice `SEL-06` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

| `SEL-06` / contingent | Lane fairness policy interface in `src/work/selection.rs`; consume #8097's existing filter contract. | #8097 eligibility/filter semantics; fairness measure is a V2 decision, not a whole-starter blocker | Model-based fairness bound while exact urgent order remains unchanged; no eligibility bypass or intrinsic-score change. |
**Future red-first test commands / oracles per issue:** All Rust tests live in the existing `fleet-queue` crate; do not add a crate. Exact future commands: SEL-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_requirements_and_filter_both_apply`; SEL-02 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_stale_claim_revalidates`; SEL-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_fixed_mass_probabilities`; SEL-04 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_disposition_replay_no_claim`; SEL-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_simulation_matches_reference`; SEL-06 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_fairness_preserves_urgent_order`; SEL-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_selection_rollback_keeps_live_values`. Each issue must first run its exact command with `-- --list` and assert the exact name is listed, then run the filter; zero-test success is not acceptance. SEL-01 oracle uses both-predicates counterexample above; SEL-03/05 use exact fixed-mass sequential probabilities. Hosted parity (#9079/#9081) and mobile display (#9201) are soft consumers, not blockers to the pure model suite.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `SEL-05` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/selection.rs`
