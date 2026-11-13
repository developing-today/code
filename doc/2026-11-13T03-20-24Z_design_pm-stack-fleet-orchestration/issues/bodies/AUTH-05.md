# AUTH-05 — External attribution and uncertain origin

<!-- pmo-9196:AUTH-05 -->

**Issue title:** `design(pm-orchestration): External attribution and uncertain origin`  
**Design parent:** #9208 · **Scope:** #9208 relations/journal/authority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R121, C-R122, P-R018. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

#9208 `authority.rs` defines trusted principal/operation provenance fields and visibility; #9207 consumes them to classify events. Do not create a second activity classifier or duplicate writer.

## Curated requirement trace

- **C-R121 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: provenance/trust support for HYD-07's canonical activity classifier; not a second classifier; occurrences `C-188`; citations: 02_charter_v1_1.md#C-188:147; design: components/9207-context-hydration-incremental-activity.md#4-incremental-activity-and-cursor-correctness; outcome: Session attribution requires Fleet operation/session IDs, not only a GitHub username; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R122 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: provenance/trust support for HYD-07's canonical activity classifier; not a second classifier; occurrences `C-189`; citations: 02_charter_v1_1.md#C-189:147; design: components/9207-context-hydration-incremental-activity.md#4-incremental-activity-and-cursor-correctness; outcome: Unattributable events are uncertain, not silently excluded; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **P-R018 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: provenance/trust support for HYD-07's canonical activity classifier; not a second classifier; occurrences `P-789, P-790, P-791, P-792, P-793, P-794, P-795, P-796, P-797, P-798, P-799, P-800, P-801, P-802, P-803, P-804, P-805, P-806, P-807, P-808, P-809, P-810, P-811, P-812, P-813, P-814, P-815, P-816, P-817, P-818, P-819, P-820, P-821, P-822, P-823, P-824, P-825, P-826, P-827, P-828, P-829, P-830, P-831, P-832, P-833, P-834, P-835, P-836, P-837, P-838, P-839, P-840, P-841, P-842, P-843, P-844, P-845, P-846, P-847`; citations: 03_full_orchestration_proposal.md#P-789:1079, 03_full_orchestration_proposal.md#P-790:1079, 03_full_orchestration_proposal.md#P-791:1081, 03_full_orchestration_proposal.md#P-792:1083, 03_full_orchestration_proposal.md#P-793:1085, 03_full_orchestration_proposal.md#P-794:1087, 03_full_orchestration_proposal.md#P-795:1089, 03_full_orchestration_proposal.md#P-796:1090, 03_full_orchestration_proposal.md#P-797:1092, 03_full_orchestration_proposal.md#P-798:1093, 03_full_orchestration_proposal.md#P-799:1094, 03_full_orchestration_proposal.md#P-800:1096, 03_full_orchestration_proposal.md#P-801:1098, 03_full_orchestration_proposal.md#P-802:1100, 03_full_orchestration_proposal.md#P-803:1101, 03_full_orchestration_proposal.md#P-804:1102, 03_full_orchestration_proposal.md#P-805:1103, 03_full_orchestration_proposal.md#P-806:1104, 03_full_orchestration_proposal.md#P-807:1106, 03_full_orchestration_proposal.md#P-808:1108, 03_full_orchestration_proposal.md#P-809:1110, 03_full_orchestration_proposal.md#P-810:1112, 03_full_orchestration_proposal.md#P-811:1114, 03_full_orchestration_proposal.md#P-812:1116, 03_full_orchestration_proposal.md#P-813:1118, 03_full_orchestration_proposal.md#P-814:1120, 03_full_orchestration_proposal.md#P-815:1121, 03_full_orchestration_proposal.md#P-816:1122, 03_full_orchestration_proposal.md#P-817:1123, 03_full_orchestration_proposal.md#P-818:1124, 03_full_orchestration_proposal.md#P-819:1125, 03_full_orchestration_proposal.md#P-820:1126, 03_full_orchestration_proposal.md#P-821:1127, 03_full_orchestration_proposal.md#P-822:1128, 03_full_orchestration_proposal.md#P-823:1130, 03_full_orchestration_proposal.md#P-824:1132, 03_full_orchestration_proposal.md#P-825:1134, 03_full_orchestration_proposal.md#P-826:1136, 03_full_orchestration_proposal.md#P-827:1138, 03_full_orchestration_proposal.md#P-828:1139, 03_full_orchestration_proposal.md#P-829:1140, 03_full_orchestration_proposal.md#P-830:1141, 03_full_orchestration_proposal.md#P-831:1142, 03_full_orchestration_proposal.md#P-832:1144, 03_full_orchestration_proposal.md#P-833:1146, 03_full_orchestration_proposal.md#P-834:1148, 03_full_orchestration_proposal.md#P-835:1150, 03_full_orchestration_proposal.md#P-836:1152, 03_full_orchestration_proposal.md#P-837:1153, 03_full_orchestration_proposal.md#P-838:1154, 03_full_orchestration_proposal.md#P-839:1156, 03_full_orchestration_proposal.md#P-840:1158, 03_full_orchestration_proposal.md#P-841:1159, 03_full_orchestration_proposal.md#P-842:1160, 03_full_orchestration_proposal.md#P-843:1161, 03_full_orchestration_proposal.md#P-844:1162, 03_full_orchestration_proposal.md#P-845:1163, 03_full_orchestration_proposal.md#P-846:1165, 03_full_orchestration_proposal.md#P-847:1167; design: components/9207-context-hydration-incremental-activity.md#4-incremental-activity-and-cursor-correctness; outcome: Track source-specific activity/cursors and session attribution; support since-start/checkpoint/timestamp and external-only; overlap/deduplicate retrieval, distinguish timestamp meanings, advance only after durable processing, and refresh again before completion.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/authority.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: CLI-02, REL-03, HYD-06
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9206→CLI-02, #9207→HYD-06.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `provenance fixture rejects actor-username-only attribution and requires opaque provider session/ref strings.`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** ``cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_auth_provenance_requires_operation_session_refs` after list check.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_auth_provenance_requires_operation_session_refs` after list check.

Negative coverage: shared App token, coincident same body, changed remote ID, missing op map.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: old attribution uncertain.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9208-typed-dependencies-notes-journal-field-authority.md`, slice `AUTH-05` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): C-R121, C-R122, P-R018: provenance/trust support for HYD-07's canonical activity classifier; not a second classifier [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### AUTH-05 — External attribution and uncertain origin

**Tier:** recommended cross-link; #9207 HYD-07 is the canonical owner of external-only activity classification. **Goal/delta:** #9208 `authority.rs` defines trusted principal/operation provenance fields and visibility; #9207 consumes them to classify events. Do not create a second activity classifier or duplicate writer. **Owns:** `src/work/authority.rs` provenance schema/fixture only; cross-component acceptance `pmo_hyd_external_only_includes_uncertain_shared_actor` stays in #9207 HYD-07 and is referenced here, not separately reimplemented. **Hard blockers:** shared #9206 identity/auth contract and REL-03 operation identity; #9207 event envelope is a consumer integration edge. **First failing oracle:** provenance fixture rejects actor-username-only attribution and requires opaque provider session/ref strings. **Done command:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_auth_provenance_requires_operation_session_refs` after list check. **Negative:** shared App token, coincident same body, changed remote ID, missing op map. **Migration:** old attribution uncertain. **Docs:** one canonical external-only behavior, shared with HYD-07.


## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):
- blocked by `AUTH-01` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/authority.rs`
