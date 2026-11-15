# REL-04 — General lifecycle mutation outbox in existing authority store

<!-- pmo-9196:REL-04 -->

**Issue title:** `design(pm-orchestration): General lifecycle mutation outbox in existing authority store`  
**Design parent:** #9209 · **Scope:** #9209 lease/outbox/completion · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R007, C-R009, C-R107, C-R108, C-R110, C-R133, C-R138, C-R139, C-R140, C-R145, H-R001, H-R015, H-R016, H-R017, H-R024, H-R031, O-R022, O-R023, O-R032, O-R038, P-R006, P-R009, P-R021, P-R022, P-R025, P-R027, P-R028, P-R029, P-R033, P-R038. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

atomic local state+journal+intent transaction, immutable intent payload, attempt log/status/event order.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R007 — design-only (requirement)**; occurrences `C-214, C-215, C-216, C-217, C-218`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **C-R110 — design-only (example)**; occurrences `C-162`; full trace: census/requirements-crosswalk.jsonl
- **C-R133 — partially implemented (requirement)**; occurrences `C-202`; full trace: census/requirements-crosswalk.jsonl
- **C-R138 — partially implemented (requirement)**; occurrences `C-207`; full trace: census/requirements-crosswalk.jsonl
- **C-R139 — design-only (requirement)**; occurrences `C-208`; full trace: census/requirements-crosswalk.jsonl
- **C-R140 — design-only (requirement)**; occurrences `C-209`; full trace: census/requirements-crosswalk.jsonl
- **C-R145 — partially implemented (requirement)**; occurrences `C-220`; full trace: census/requirements-crosswalk.jsonl
- **H-R001 — design-only (requirement)**; occurrences `H-029`; full trace: census/requirements-crosswalk.jsonl
- **H-R015 — partially implemented (requirement)**; occurrences `H-072`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R022 — design-only (candidate)**; occurrences `O-782, O-783, O-784, O-785, O-786, O-787, O-788, O-789, O-790, O-791, O-792, O-793, O-794, O-795, O-796, O-797, O-798, O-799, O-800, O-801, O-802, O-803, O-804, O-805, O-806, O-807, O-808, O-809, O-810, O-811, O-812, O-813, O-814, O-815, O-816, O-817, O-818, O-819, O-820, O-821, O-822, O-823, O-824, O-825, O-826, O-827, O-828, O-829, O-830, O-831, O-832, O-833, O-834, O-835, O-836, O-837, O-838, O-839, O-840, O-841, O-842, O-843, O-844, O-845`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R032 — design-only (candidate)**; occurrences `O-970, O-971, O-972, O-973, O-974, O-975, O-976, O-977, O-978, O-979, O-980, O-981, O-982, O-983, O-984, O-985, O-986, O-987, O-988`; full trace: census/requirements-crosswalk.jsonl
- **O-R038 — design-only (candidate)**; occurrences `O-1178, O-1179, O-1180, O-1181, O-1182, O-1183, O-1184, O-1185, O-1186, O-1187, O-1188, O-1189, O-1190, O-1191, O-1192, O-1193, O-1194, O-1195, O-1196, O-1197, O-1198, O-1199, O-1200, O-1201, O-1202, O-1203, O-1204, O-1205, O-1206, O-1207, O-1208, O-1209, O-1210, O-1211, O-1212, O-1213, O-1214, O-1215, O-1216, O-1217, O-1218, O-1219, O-1220, O-1221, O-1222, O-1223, O-1224, O-1225, O-1226, O-1227, O-1228, O-1229, O-1230, O-1231, O-1232, O-1233, O-1234, O-1235, O-1236, O-1237, O-1238, O-1239, O-1240, O-1241`; full trace: census/requirements-crosswalk.jsonl
- **P-R006 — design-only (candidate)**; occurrences `P-110, P-111, P-112, P-113, P-114, P-115, P-116, P-117, P-118, P-119, P-120, P-121, P-122`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R021 — design-only (candidate)**; occurrences `P-959, P-960, P-961, P-962, P-963, P-964, P-965, P-966, P-967, P-968, P-969, P-970, P-971, P-972, P-973, P-974, P-975, P-976, P-977, P-978, P-979, P-980, P-981, P-982, P-983, P-984, P-985, P-986, P-987, P-988, P-989, P-990, P-991, P-992, P-993, P-994, P-995, P-996, P-997, P-998, P-999, P-1000, P-1001, P-1002, P-1003, P-1004, P-1005, P-1006, P-1007, P-1008, P-1009, P-1010, P-1011, P-1012, P-1013, P-1014, P-1015, P-1016, P-1017, P-1018, P-1019, P-1020`; full trace: census/requirements-crosswalk.jsonl
- **P-R022 — design-only (candidate)**; occurrences `P-1434, P-1435, P-1436, P-1437, P-1438, P-1439, P-1440, P-1441, P-1442, P-1443, P-1444, P-1445, P-1446, P-1447, P-1448, P-1449, P-1450, P-1451, P-1452, P-1453, P-1454, P-1455, P-1456, P-1457`; full trace: census/requirements-crosswalk.jsonl
- **P-R025 — partially implemented (candidate)**; occurrences `P-1540, P-1541, P-1542, P-1543, P-1544, P-1545, P-1546, P-1547, P-1548, P-1549, P-1550, P-1551, P-1552, P-1553, P-1554, P-1555, P-1556, P-1557, P-1558, P-1559, P-1560, P-1561, P-1562, P-1563, P-1564, P-1565, P-1566, P-1567, P-1568, P-1569, P-1570, P-1571, P-1572, P-1573, P-1574, P-1575, P-1576, P-1577, P-1578, P-1579, P-1580, P-1581, P-1582, P-1583, P-1584, P-1585, P-1586, P-1587, P-1588, P-1589, P-1590, P-1591, P-1592, P-1593, P-1594, P-1595, P-1596, P-1597, P-1598, P-1599, P-1600, P-1601, P-1602, P-1603, P-1604, P-1605, P-1606, P-1607, P-1608`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R028 — design-only (candidate)**; occurrences `P-1729, P-1730, P-1731, P-1732, P-1733, P-1734, P-1735, P-1736, P-1737, P-1738, P-1739, P-1740, P-1741, P-1742, P-1743, P-1744, P-1745, P-1746, P-1747, P-1748, P-1749, P-1750, P-1751, P-1752, P-1753, P-1754, P-1755, P-1756, P-1757, P-1758, P-1759`; full trace: census/requirements-crosswalk.jsonl
- **P-R029 — design-only (candidate)**; occurrences `P-1761, P-1762, P-1763, P-1764, P-1765, P-1766, P-1767, P-1768, P-1769, P-1770, P-1771, P-1772, P-1773, P-1774, P-1775, P-1776`; full trace: census/requirements-crosswalk.jsonl
- **P-R033 — design-only (candidate)**; occurrences `P-456, P-457, P-458, P-459, P-460, P-461, P-462, P-463, P-464, P-465, P-466, P-467, P-468, P-469, P-470, P-471, P-472, P-473, P-474, P-475, P-476, P-477`; full trace: census/requirements-crosswalk.jsonl
- **P-R038 — design-only (candidate)**; occurrences `P-1383, P-1384`; full trace: census/requirements-crosswalk.jsonl
  Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`
- `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: REL-03
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_rel_state_and_intent_commit_or_rollback_together`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `verify exact test listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_state_and_intent_commit_or_rollback_together`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

verify exact test listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_state_and_intent_commit_or_rollback_together`.

Negative coverage: intent absent when state commits, duplicate key, reordered per-target op, DB busy.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: disable producer but preserve pending intents/remote effects. No new DB/relay.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9209-leases-fencing-outbox-direct-completion.md`, slice `REL-04` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `flag.mirror, github.comment.add, outbox.generalization, sync.outbox.inspect, sync.outbox.list, sync.run, sync.status`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### REL-04 — General lifecycle mutation outbox in existing authority store

**Tier:** crucial. **Goal:** atomic local state+journal+intent transaction, immutable intent payload, attempt log/status/event order. **Owns:** `src/work/operations.rs` plus existing `src/turso_queue.rs` migration/storage seam after owner approval; creation pilot remains isolated. **Hard blockers:** REL-03; Queue Table Improvements; proven DB transaction contract. **First failing oracle:** `pmo_rel_state_and_intent_commit_or_rollback_together` injects failure after each local write. **Done command:** verify exact test listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_state_and_intent_commit_or_rollback_together`. **Negative:** intent absent when state commits, duplicate key, reordered per-target op, DB busy. **Rollback:** disable producer but preserve pending intents/remote effects. No new DB/relay.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `HYD-05` — `shared-file:pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`
- blocked by `DSP-09` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/operations.rs`
