# HYD-06 — Per-source incremental activity revisions and tombstones

<!-- pmo-9196:HYD-06 -->

**Issue title:** `design(pm-orchestration): Per-source incremental activity revisions and tombstones`  
**Design parent:** #9207 · **Scope:** #9207 hydration/activity · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R009, C-R107, C-R108, C-R111, C-R112, C-R118, C-R119, C-R120, C-R121, C-R122, C-R150, C-R151, H-R014, H-R016, H-R017, H-R024, H-R031, O-R022, O-R023, O-R028, O-R030, O-R049, O-R050, P-R009, P-R012, P-R015, P-R016, P-R018, P-R027, P-R039, P-R050, P-R051, P-R065, P-R066. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

detect new comments and edits to older comments, retain revision/tombstone/minimized observations, distinguish occurrence/observation/update times.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **C-R111 — open implementation (requirement)**; occurrences `C-164`; full trace: census/requirements-crosswalk.jsonl
- **C-R112 — design-only (requirement)**; occurrences `C-165`; full trace: census/requirements-crosswalk.jsonl
- **C-R118 — open implementation (requirement)**; occurrences `C-185`; full trace: census/requirements-crosswalk.jsonl
- **C-R119 — design-only (requirement)**; occurrences `C-186`; full trace: census/requirements-crosswalk.jsonl
- **C-R120 — design-only (requirement)**; occurrences `C-187`; full trace: census/requirements-crosswalk.jsonl
- **C-R121 — design-only (requirement)**; occurrences `C-188`; full trace: census/requirements-crosswalk.jsonl
- **C-R122 — design-only (requirement)**; occurrences `C-189`; full trace: census/requirements-crosswalk.jsonl
- **C-R150 — partially implemented (requirement)**; occurrences `C-226`; full trace: census/requirements-crosswalk.jsonl
- **C-R151 — partially implemented (requirement)**; occurrences `C-227`; full trace: census/requirements-crosswalk.jsonl
- **H-R014 — design-only (requirement)**; occurrences `H-071`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R022 — design-only (candidate)**; occurrences `O-782, O-783, O-784, O-785, O-786, O-787, O-788, O-789, O-790, O-791, O-792, O-793, O-794, O-795, O-796, O-797, O-798, O-799, O-800, O-801, O-802, O-803, O-804, O-805, O-806, O-807, O-808, O-809, O-810, O-811, O-812, O-813, O-814, O-815, O-816, O-817, O-818, O-819, O-820, O-821, O-822, O-823, O-824, O-825, O-826, O-827, O-828, O-829, O-830, O-831, O-832, O-833, O-834, O-835, O-836, O-837, O-838, O-839, O-840, O-841, O-842, O-843, O-844, O-845`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R028 — design-only (candidate)**; occurrences `O-867, O-868, O-869, O-870, O-871, O-872, O-873, O-874, O-875, O-876, O-877, O-878, O-879, O-880, O-881, O-882, O-883, O-884, O-885, O-886, O-887, O-888, O-889, O-890, O-891, O-892, O-893, O-894, O-895, O-896, O-897, O-898, O-899, O-900, O-901, O-902, O-903, O-904, O-905, O-906, O-907, O-908, O-909, O-910, O-911, O-912, O-913, O-914, O-915, O-916, O-917, O-918, O-919, O-920, O-921, O-922`; full trace: census/requirements-crosswalk.jsonl
- **O-R030 — partially implemented (candidate)**; occurrences `O-946, O-947, O-948, O-949, O-950, O-951, O-952, O-953, O-954, O-955`; full trace: census/requirements-crosswalk.jsonl
- **O-R049 — design-only (example)**; occurrences `O-1037, O-1038, O-1039, O-1040, O-1041, O-1042, O-1043, O-1044, O-1045, O-1046, O-1047, O-1048`; full trace: census/requirements-crosswalk.jsonl
- **O-R050 — design-only (example)**; occurrences `O-1050, O-1051, O-1052, O-1053, O-1054, O-1055, O-1056, O-1057, O-1058, O-1059, O-1060`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R012 — design-only (candidate)**; occurrences `P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R015 — design-only (candidate)**; occurrences `P-554, P-555, P-556, P-557, P-558, P-559, P-560, P-561, P-562, P-563, P-564, P-565, P-566, P-567, P-568, P-569, P-570, P-571, P-572, P-573, P-574, P-575, P-576, P-577, P-578, P-579, P-580, P-581, P-582, P-583, P-584, P-585, P-586, P-587, P-588, P-589, P-590, P-591, P-592, P-593, P-594, P-595, P-596, P-597, P-598, P-599, P-600, P-601, P-602, P-603, P-604, P-605, P-606, P-607, P-608, P-609, P-610, P-611, P-612, P-613, P-614, P-615, P-616, P-617, P-618, P-619, P-620, P-621, P-622, P-623, P-624, P-625, P-626, P-627, P-628, P-629, P-630, P-631, P-632, P-633, P-634, P-635, P-636, P-637, P-638, P-639, P-640, P-641, P-642, P-643, P-644, P-645, P-646`; full trace: census/requirements-crosswalk.jsonl
- **P-R016 — design-only (candidate)**; occurrences `P-647, P-648, P-649, P-650, P-651, P-652, P-653, P-654, P-655, P-656, P-657, P-658, P-659, P-660, P-661, P-662, P-663, P-664, P-665, P-666, P-667, P-668, P-669, P-670, P-671, P-672, P-673, P-674, P-675, P-676, P-677, P-678, P-679, P-680, P-681, P-682, P-683, P-684, P-685, P-686, P-687, P-688, P-689, P-690, P-691, P-692, P-693, P-694, P-695, P-696, P-697, P-698, P-699, P-700, P-701, P-702, P-703, P-704, P-705, P-706, P-707, P-708, P-709, P-710, P-711, P-712, P-713, P-714, P-715`; full trace: census/requirements-crosswalk.jsonl
- **P-R018 — design-only (candidate)**; occurrences `P-789, P-790, P-791, P-792, P-793, P-794, P-795, P-796, P-797, P-798, P-799, P-800, P-801, P-802, P-803, P-804, P-805, P-806, P-807, P-808, P-809, P-810, P-811, P-812, P-813, P-814, P-815, P-816, P-817, P-818, P-819, P-820, P-821, P-822, P-823, P-824, P-825, P-826, P-827, P-828, P-829, P-830, P-831, P-832, P-833, P-834, P-835, P-836, P-837, P-838, P-839, P-840, P-841, P-842, P-843, P-844, P-845, P-846, P-847`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R039 — design-only (candidate)**; occurrences `P-1411`; full trace: census/requirements-crosswalk.jsonl
- **P-R050 — design-only (candidate)**; occurrences `P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680`; full trace: census/requirements-crosswalk.jsonl
- **P-R051 — design-only (candidate)**; occurrences `P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687`; full trace: census/requirements-crosswalk.jsonl
- **P-R065 — design-only (candidate)**; occurrences `P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680`; full trace: census/requirements-crosswalk.jsonl
- **P-R066 — design-only (candidate)**; occurrences `P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687`; full trace: census/requirements-crosswalk.jsonl
Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.
## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/activity.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: HYD-05, JRN-01, CLI-05
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9208→JRN-01, #9206→CLI-05.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_hyd_refresh_observes_edit_behind_creation_cursor`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `confirm both listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`; then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_observes_edit_behind_creation_cursor` and the same command filtered by `pmo_hyd_replay_deduplicates_revision_not_creation`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

per source/resource/endpoint/query/API cursor; cursor post-durable; overlap not proof; deletion gap reported.

Negative coverage: equal `updated_at`, shifting pages, absent deletion, minimized records.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: unknown full snapshot; rollback disables incremental and retains generations.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9207-context-hydration-incremental-activity.md`, slice `HYD-06` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `activity.list, activity.reconcile, activity.since, activity.watch, flag.checkpoint, flag.since, flag.since-checkpoint, github.comment.list, github.issue.comments, github.issue.timeline, github.pr.reviews, work.changes, work.refresh`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### HYD-06 — Per-source incremental activity revisions and tombstones

**Tier:** crucial. **Goal:** detect new comments and edits to older comments, retain revision/tombstone/minimized observations, distinguish occurrence/observation/update times. **Owns:** `src/work/activity.rs` normalized records/revisions/change projection; append seam in `src/work/journal.rs`. **Hard blockers:** HYD-05; #9208 journal contract; granular #9206 session/checkpoint-ref slice. **First failing oracles:** `pmo_hyd_refresh_observes_edit_behind_creation_cursor` and `pmo_hyd_replay_deduplicates_revision_not_creation`; fake API edits old comment after ID watermark. **Done command:** confirm both listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`; then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_observes_edit_behind_creation_cursor` and the same command filtered by `pmo_hyd_replay_deduplicates_revision_not_creation`. **Acceptance:** per source/resource/endpoint/query/API cursor; cursor post-durable; overlap not proof; deletion gap reported. **Negative:** equal `updated_at`, shifting pages, absent deletion, minimized records. **Migration:** unknown full snapshot; rollback disables incremental and retains generations. **Performance:** O(new+overlap pages/bytes), bounded RAM.


## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):
- blocked by `HYD-05` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/activity.rs`
