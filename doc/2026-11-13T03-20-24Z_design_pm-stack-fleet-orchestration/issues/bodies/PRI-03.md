# PRI-03 — Required before/after peer ordering within equal effective urgency

<!-- pmo-9196:PRI-03 -->

**Issue title:** `design(pm-orchestration): Required before/after peer ordering within equal effective urgency`  
**Design parent:** #9199 · **Scope:** #9199 distributed priority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R002, C-R005, C-R009, C-R041, C-R046, C-R049, C-R053, C-R107, C-R108, H-R004, H-R007, H-R017, H-R024, H-R031, O-R003, O-R004, O-R005, O-R006, O-R007, O-R009, O-R017, O-R023, O-R038, O-R054, O-R055, O-R056, O-R057, O-R059, O-R066, O-R067, O-R068, O-R069, P-R009, P-R014, S-R005, S-R007. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

Required before/after peer ordering within equal effective urgency; exact integer rank and local deterministic rebalance in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; comparator parity in `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R002 — design-only (requirement)**; occurrences `C-071, C-072`; full trace: census/requirements-crosswalk.jsonl
- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R041 — design-only (requirement)**; occurrences `C-053`; full trace: census/requirements-crosswalk.jsonl
- **C-R046 — design-only (requirement)**; occurrences `C-059`; full trace: census/requirements-crosswalk.jsonl
- **C-R049 — partially implemented (requirement)**; occurrences `C-074`; full trace: census/requirements-crosswalk.jsonl
- **C-R053 — design-only (example)**; occurrences `C-079`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **H-R004 — design-only (requirement)**; occurrences `H-047, H-048, H-049`; full trace: census/requirements-crosswalk.jsonl
- **H-R007 — design-only (requirement)**; occurrences `H-118, H-119, H-120, H-121, H-122, H-123, H-124, H-125`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R003 — design-only (candidate)**; occurrences `O-237, O-238, O-239, O-240, O-241, O-242, O-243, O-244, O-245, O-246, O-247, O-248`; full trace: census/requirements-crosswalk.jsonl
- **O-R004 — design-only (candidate)**; occurrences `O-249, O-250, O-251, O-252, O-253, O-254, O-255, O-256, O-257, O-258, O-259, O-260`; full trace: census/requirements-crosswalk.jsonl
- **O-R005 — design-only (candidate)**; occurrences `O-262, O-263, O-264, O-265, O-266, O-267, O-268, O-269, O-270, O-271, O-272, O-273, O-274, O-275, O-276, O-277, O-278, O-279, O-280, O-281, O-282, O-283, O-284, O-285, O-286, O-287, O-288, O-289, O-290, O-291, O-292, O-293, O-294, O-295`; full trace: census/requirements-crosswalk.jsonl
- **O-R006 — design-only (candidate)**; occurrences `O-297, O-298, O-299, O-300, O-301, O-302, O-303, O-304, O-305, O-306, O-307, O-308, O-309, O-310, O-311, O-312, O-313, O-314, O-315, O-316, O-317, O-318, O-319, O-320, O-321, O-322, O-323, O-324, O-325, O-326, O-327, O-328, O-329, O-330, O-331`; full trace: census/requirements-crosswalk.jsonl
- **O-R007 — design-only (candidate)**; occurrences `O-333, O-334, O-335, O-336, O-337, O-338, O-339, O-340, O-341, O-342, O-343, O-344, O-345, O-346, O-347, O-348, O-349, O-350, O-351, O-352, O-353, O-354, O-355`; full trace: census/requirements-crosswalk.jsonl
- **O-R009 — design-only (candidate)**; occurrences `O-379, O-380, O-381, O-382, O-383, O-384, O-385, O-386, O-387, O-388, O-389, O-390, O-391, O-392, O-393, O-394, O-395, O-396, O-397, O-398, O-399, O-400, O-401, O-402, O-403, O-404, O-405`; full trace: census/requirements-crosswalk.jsonl
- **O-R017 — design-only (candidate)**; occurrences `O-688, O-689, O-690, O-691, O-692, O-693, O-694, O-695, O-696, O-697, O-698`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R038 — design-only (candidate)**; occurrences `O-1178, O-1179, O-1180, O-1181, O-1182, O-1183, O-1184, O-1185, O-1186, O-1187, O-1188, O-1189, O-1190, O-1191, O-1192, O-1193, O-1194, O-1195, O-1196, O-1197, O-1198, O-1199, O-1200, O-1201, O-1202, O-1203, O-1204, O-1205, O-1206, O-1207, O-1208, O-1209, O-1210, O-1211, O-1212, O-1213, O-1214, O-1215, O-1216, O-1217, O-1218, O-1219, O-1220, O-1221, O-1222, O-1223, O-1224, O-1225, O-1226, O-1227, O-1228, O-1229, O-1230, O-1231, O-1232, O-1233, O-1234, O-1235, O-1236, O-1237, O-1238, O-1239, O-1240, O-1241`; full trace: census/requirements-crosswalk.jsonl
- **O-R054 — design-only (candidate)**; occurrences `O-262, O-263, O-264, O-265, O-266, O-267, O-268, O-269, O-270, O-271, O-272, O-273, O-274, O-275, O-276, O-277, O-278, O-279, O-280, O-281`; full trace: census/requirements-crosswalk.jsonl
- **O-R055 — design-only (candidate)**; occurrences `O-282, O-283, O-284, O-285`; full trace: census/requirements-crosswalk.jsonl
- **O-R056 — design-only (candidate)**; occurrences `O-286, O-287, O-288, O-289, O-290, O-291, O-292`; full trace: census/requirements-crosswalk.jsonl
- **O-R057 — design-only (candidate)**; occurrences `O-293, O-294, O-295`; full trace: census/requirements-crosswalk.jsonl
- **O-R059 — design-only (candidate)**; occurrences `O-384, O-385, O-386, O-387, O-388, O-389, O-390`; full trace: census/requirements-crosswalk.jsonl
- **O-R066 — design-only (candidate)**; occurrences `O-262, O-263, O-264, O-265, O-266, O-267, O-268, O-269, O-270, O-271, O-272, O-273, O-274, O-275, O-276, O-277, O-278, O-279, O-280, O-281`; full trace: census/requirements-crosswalk.jsonl
- **O-R067 — design-only (candidate)**; occurrences `O-282, O-283, O-284, O-285`; full trace: census/requirements-crosswalk.jsonl
- **O-R068 — design-only (candidate)**; occurrences `O-286, O-287, O-288, O-289, O-290, O-291, O-292`; full trace: census/requirements-crosswalk.jsonl
- **O-R069 — design-only (candidate)**; occurrences `O-293, O-294, O-295`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R014 — design-only (candidate)**; occurrences `P-522, P-523, P-524, P-525, P-526, P-527, P-528, P-529, P-530, P-531, P-532, P-533, P-534, P-535, P-536, P-537, P-538, P-539, P-540, P-541, P-542, P-543, P-544, P-545, P-546, P-547, P-548, P-549, P-550, P-551, P-552`; full trace: census/requirements-crosswalk.jsonl
- **S-R005 — design-only (candidate)**; occurrences `S-029, S-030, S-031, S-032, S-033, S-034`; full trace: census/requirements-crosswalk.jsonl
- **S-R007 — design-only (candidate)**; occurrences `S-040, S-041, S-042, S-043, S-044, S-045, S-046`; full trace: census/requirements-crosswalk.jsonl
  Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`
- `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: #9074
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Source-local candidate aliases normalized for this packet: DST-ORDER-01 → PRI-03; PRI-CMP-01 → PRI-03; PRI-05 → PRI-05a, PRI-05b, PRI-05c, PRI-05d.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_peer_rank_rebalances_local_range; pmo_work_id_decimal_strings_order_numeric_not_lexical`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `Verify `pmo_peer_rank_rebalances_local_range`and`pmo_work_id_decimal_strings_order_numeric_not_lexical` are listed; then run both exact fleet-queue filters. Comparator oracle asserts ID 9 sorts before 10 numerically for equal prior keys.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

Required before/after peer ordering within equal effective urgency; exact integer rank and local deterministic rebalance in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; comparator parity in `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`.

Negative coverage: stale input, duplicate/replay, ambiguous resource, unauthorized actor, partial remote effect, unknown version, and no false verified/completed result where applicable.

### Canonical work-ID tie order and selection-order digest

Wire work IDs are canonical unsigned decimal strings, but after authority IDs compare bytewise, work IDs compare by arbitrary-precision numeric value (digit-count then digits), not lexical string order or floating point. Golden tie fixture must place work ID `9` before `10`; canonical IDs are not coerced through JavaScript numbers. Opaque provider/session IDs remain opaque strings. The optional full `order_hash` uses `fleet.selection-order/v1` and the shared `ASCII(domain) || 0x00 || UTF8(JCS(order_object))` + SHA-256 `sha256:` lower-hex framing. Hash the complete eligible population in canonical comparator order—not a UI top-page—and include `owner_policy_ref` and all selection-policy/revision inputs. This fingerprint neither grants a claim nor proves a complete snapshot.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9199-distributed-priority.md`, slice `PRI-03` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: architecture.md#identity-and-authority, architecture.md#digest-and-serialization-contract, research/canonical-json-evidence.md, contracts/digest-vectors.json.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `flag.order, flag.priority-after, flag.priority-before, priority.move`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

| `PRI-03` / recommended | Required before/after peer ordering within equal effective urgency; exact integer rank and local deterministic rebalance in `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`; comparator parity in `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`. | `PRI-CMP-01`; `DST-ORDER-01`; persistent rank writes gated on Queue Table Improvements/#9074 schema disposition | Same snapshot permutations yield same order; moves work within group; cross-group requires confirmation; stale neighbors/epoch conflict; rebalance bounded/atomic/audited. |
**Future red-first test commands / oracles per issue:** Exact future Rust commands: PRI-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_profile_preserves_ratified_values`; PRI-02 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_dimensions_pin_explanation`; PRI-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_peer_rank_rebalances_local_range`; PRI-04 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dependency_cycle_opposite_edge_pair`; PRI-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_triage_is_bounded_and_bidirectional`; PRI-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_priority_shadow_preserves_live_values`. Before running each, issue verification must run its exact command with `-- --list` and assert the test is present; zero-test success is not acceptance. PRI-01 TS regression: `cd pm-stack/dispatch && bun test test/validate.test.ts` (existing test path). PRI-06 first asserts `test -f pm-stack/aggregator/src/priority-actions.test.ts`, then runs `cd pm-stack/aggregator && bun test src/priority-actions.test.ts`; oracle is shared command result plus visible stale/conflict. These are proposed future tests in the listed existing modules/services, not current tests or a new crate.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `HYD-03` — `shared-file:pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`
- blocked by `PRI-02` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/priority.rs`
