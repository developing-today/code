# REL-01 — Claim and lease contract compatibility against #9178

<!-- pmo-9196:REL-01 -->

**Issue title:** `design(pm-orchestration): Claim and lease contract compatibility against #9178`  
**Design parent:** #9209 · **Scope:** #9209 lease/outbox/completion · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R009, C-R107, C-R108, C-R110, C-R134, C-R135, C-R144, C-R145, C-R151, C-R153, H-R006, H-R014, H-R016, H-R017, H-R024, H-R031, O-R023, O-R027, O-R032, O-R035, O-R052, P-R001, P-R006, P-R007, P-R009, P-R011, P-R020, P-R023, P-R025, P-R027, P-R033, P-R041, P-R042, P-R043, P-R044, P-R045, P-R046, P-R047, P-R048, P-R053, P-R056, P-R057, P-R058, P-R059, P-R060, P-R061, P-R062, P-R063, P-R068. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

pin #9178 claim/manual/takeover/resume and one-queue semantics, produce transition compatibility table and state-model tests.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **C-R110 — design-only (example)**; occurrences `C-162`; full trace: census/requirements-crosswalk.jsonl
- **C-R134 — partially implemented (requirement)**; occurrences `C-203`; full trace: census/requirements-crosswalk.jsonl
- **C-R135 — partially implemented (requirement)**; occurrences `C-204`; full trace: census/requirements-crosswalk.jsonl
- **C-R144 — partially implemented (requirement)**; occurrences `C-219`; full trace: census/requirements-crosswalk.jsonl
- **C-R145 — partially implemented (requirement)**; occurrences `C-220`; full trace: census/requirements-crosswalk.jsonl
- **C-R151 — partially implemented (requirement)**; occurrences `C-227`; full trace: census/requirements-crosswalk.jsonl
- **C-R153 — open implementation (requirement)**; occurrences `C-229`; full trace: census/requirements-crosswalk.jsonl
- **H-R006 — design-only (requirement)**; occurrences `H-115, H-116, H-117`; full trace: census/requirements-crosswalk.jsonl
- **H-R014 — design-only (requirement)**; occurrences `H-071`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R027 — design-only (candidate)**; occurrences `O-860, O-861, O-862, O-863, O-864, O-865, O-866`; full trace: census/requirements-crosswalk.jsonl
- **O-R032 — design-only (candidate)**; occurrences `O-970, O-971, O-972, O-973, O-974, O-975, O-976, O-977, O-978, O-979, O-980, O-981, O-982, O-983, O-984, O-985, O-986, O-987, O-988`; full trace: census/requirements-crosswalk.jsonl
- **O-R035 — design-only (example)**; occurrences `O-1092, O-1093, O-1094, O-1095, O-1096, O-1097, O-1098, O-1099, O-1100, O-1101`; full trace: census/requirements-crosswalk.jsonl
- **O-R052 — design-only (example)**; occurrences `O-1089, O-1090`; full trace: census/requirements-crosswalk.jsonl
- **P-R001 — design-only (candidate)**; occurrences `P-016, P-017, P-018, P-019, P-020, P-021, P-022, P-023, P-024, P-025, P-026, P-027, P-028, P-029, P-030, P-031, P-032, P-033, P-034, P-035, P-036, P-037, P-038, P-039, P-040, P-041, P-042, P-043, P-044, P-045, P-046, P-047, P-048, P-049, P-050, P-051, P-052, P-053, P-054, P-055, P-056, P-057`; full trace: census/requirements-crosswalk.jsonl
- **P-R006 — design-only (candidate)**; occurrences `P-110, P-111, P-112, P-113, P-114, P-115, P-116, P-117, P-118, P-119, P-120, P-121, P-122`; full trace: census/requirements-crosswalk.jsonl
- **P-R007 — design-only (candidate)**; occurrences `P-123, P-124, P-125, P-126, P-127, P-128`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R011 — design-only (candidate)**; occurrences `P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310`; full trace: census/requirements-crosswalk.jsonl
- **P-R020 — design-only (candidate)**; occurrences `P-885, P-886, P-887, P-888, P-889, P-890, P-891, P-892, P-893, P-894, P-895, P-896, P-897, P-898, P-899, P-900, P-901, P-902, P-903, P-904, P-905, P-906, P-907, P-908, P-909, P-910, P-911, P-912, P-913, P-914, P-915, P-916, P-917, P-918, P-919, P-920, P-921, P-922, P-923, P-924, P-925, P-926, P-927, P-928, P-929, P-930, P-931, P-932, P-933, P-934, P-935, P-936, P-937, P-938, P-939, P-940, P-941, P-942, P-943, P-944, P-945, P-946, P-947, P-948, P-949, P-950, P-951, P-952, P-953, P-954, P-955, P-956, P-957`; full trace: census/requirements-crosswalk.jsonl
- **P-R023 — design-only (candidate)**; occurrences `P-1459, P-1460, P-1461, P-1462, P-1463, P-1464, P-1465, P-1466, P-1467, P-1468, P-1469, P-1470, P-1471, P-1472, P-1473, P-1474, P-1475, P-1476, P-1477, P-1478, P-1479, P-1480, P-1481, P-1482, P-1483, P-1484, P-1485, P-1486, P-1487, P-1488, P-1489, P-1490, P-1491, P-1492`; full trace: census/requirements-crosswalk.jsonl
- **P-R025 — partially implemented (candidate)**; occurrences `P-1540, P-1541, P-1542, P-1543, P-1544, P-1545, P-1546, P-1547, P-1548, P-1549, P-1550, P-1551, P-1552, P-1553, P-1554, P-1555, P-1556, P-1557, P-1558, P-1559, P-1560, P-1561, P-1562, P-1563, P-1564, P-1565, P-1566, P-1567, P-1568, P-1569, P-1570, P-1571, P-1572, P-1573, P-1574, P-1575, P-1576, P-1577, P-1578, P-1579, P-1580, P-1581, P-1582, P-1583, P-1584, P-1585, P-1586, P-1587, P-1588, P-1589, P-1590, P-1591, P-1592, P-1593, P-1594, P-1595, P-1596, P-1597, P-1598, P-1599, P-1600, P-1601, P-1602, P-1603, P-1604, P-1605, P-1606, P-1607, P-1608`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R033 — design-only (candidate)**; occurrences `P-456, P-457, P-458, P-459, P-460, P-461, P-462, P-463, P-464, P-465, P-466, P-467, P-468, P-469, P-470, P-471, P-472, P-473, P-474, P-475, P-476, P-477`; full trace: census/requirements-crosswalk.jsonl
- **P-R041 — design-only (candidate)**; occurrences `P-1462, P-1463, P-1464, P-1465`; full trace: census/requirements-crosswalk.jsonl
- **P-R042 — design-only (candidate)**; occurrences `P-1466, P-1467, P-1468, P-1469, P-1470`; full trace: census/requirements-crosswalk.jsonl
- **P-R043 — design-only (candidate)**; occurrences `P-1471, P-1472, P-1473, P-1474`; full trace: census/requirements-crosswalk.jsonl
- **P-R044 — design-only (candidate)**; occurrences `P-1475, P-1476, P-1477`; full trace: census/requirements-crosswalk.jsonl
- **P-R045 — design-only (candidate)**; occurrences `P-1478, P-1479, P-1480, P-1481, P-1482`; full trace: census/requirements-crosswalk.jsonl
- **P-R046 — design-only (candidate)**; occurrences `P-1483, P-1484, P-1485`; full trace: census/requirements-crosswalk.jsonl
- **P-R047 — design-only (candidate)**; occurrences `P-1486, P-1487, P-1488, P-1489`; full trace: census/requirements-crosswalk.jsonl
- **P-R048 — design-only (candidate)**; occurrences `P-1490, P-1491, P-1492`; full trace: census/requirements-crosswalk.jsonl
- **P-R053 — design-only (candidate)**; occurrences `P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704`; full trace: census/requirements-crosswalk.jsonl
- **P-R056 — design-only (candidate)**; occurrences `P-1462, P-1463, P-1464, P-1465`; full trace: census/requirements-crosswalk.jsonl
- **P-R057 — design-only (candidate)**; occurrences `P-1466, P-1467, P-1468, P-1469, P-1470`; full trace: census/requirements-crosswalk.jsonl
- **P-R058 — design-only (candidate)**; occurrences `P-1471, P-1472, P-1473, P-1474`; full trace: census/requirements-crosswalk.jsonl
- **P-R059 — design-only (candidate)**; occurrences `P-1475, P-1476, P-1477`; full trace: census/requirements-crosswalk.jsonl
- **P-R060 — design-only (candidate)**; occurrences `P-1478, P-1479, P-1480, P-1481, P-1482`; full trace: census/requirements-crosswalk.jsonl
- **P-R061 — design-only (candidate)**; occurrences `P-1483, P-1484, P-1485`; full trace: census/requirements-crosswalk.jsonl
- **P-R062 — design-only (candidate)**; occurrences `P-1486, P-1487, P-1488, P-1489`; full trace: census/requirements-crosswalk.jsonl
- **P-R063 — design-only (candidate)**; occurrences `P-1490, P-1491, P-1492`; full trace: census/requirements-crosswalk.jsonl
- **P-R068 — design-only (candidate)**; occurrences `P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704`; full trace: census/requirements-crosswalk.jsonl
  Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: CEN-04, CLI-01
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: #9178 (upstream claim/manual/takeover/resume semantics must be verified before mutation/cutover; offline compatibility models may proceed).
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9198→CEN-04, #9206→CLI-01.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_rel_claim_requires_requirements_and_filter`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `verify both tests listed, then run package test filtered `pmo_rel_claim_`and`pmo_rel_manual_only_is_eligibility_not_directive`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

verify both tests listed, then run package test filtered `pmo_rel_claim_` and `pmo_rel_manual_only_is_eligibility_not_directive`.

Negative coverage: expired but unreclaimed, manual-only/parked/blocked, empty filters, preserve unresolved `BATCH_CLAIM_MAX`.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: no schema change; contract keeps existing semantics. Source, deployment and live claim evidence distinct.
- Rollback: no schema change; contract keeps existing semantics. Source, deployment and live claim evidence distinct.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9209-leases-fencing-outbox-direct-completion.md`, slice `REL-01` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `current.claim.bind, current.claim.filter, current.claim.from-session, current.claim.lease-seconds, current.claim.platform, current.claim.worker, flag.claim, flag.lease, queue.claim, queue.fallback.claim, queue.fallback.status, work.resume`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### REL-01 — Claim and lease contract compatibility against #9178

**Tier:** crucial. **Goal:** pin #9178 claim/manual/takeover/resume and one-queue semantics, produce transition compatibility table and state-model tests. **Owns:** this design's requirement crosswalk plus `pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs` pure eligibility fixtures; no CLI glue. **Hard blockers:** #9178 semantics/FDR verification; #9198 pinned source/store ownership; shared #9206 `contracts.rs` identity envelope. **First failing oracle:** `pmo_rel_claim_requires_requirements_and_filter` proves BOTH task requirements and claim-time filter predicates (AND), with defaults true; `pmo_rel_manual_only_is_eligibility_not_directive` separates manual-only from Do Now. **Done command:** verify both tests listed, then run package test filtered `pmo_rel_claim_` and `pmo_rel_manual_only_is_eligibility_not_directive`. **Negative:** expired but unreclaimed, manual-only/parked/blocked, empty filters, preserve unresolved `BATCH_CLAIM_MAX`. **Migration/rollback:** no schema change; contract keeps existing semantics. Source, deployment and live claim evidence distinct.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `CLI-03` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs`
