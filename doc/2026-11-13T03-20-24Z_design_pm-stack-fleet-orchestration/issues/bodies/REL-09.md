# REL-09 — Last-activity gates, duplicate webhook, post-completion policy

<!-- pmo-9196:REL-09 -->

**Issue title:** `design(pm-orchestration): Last-activity gates, duplicate webhook, post-completion policy`  
**Design parent:** #9209 · **Scope:** #9209 lease/outbox/completion · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R007, C-R009, C-R107, C-R108, C-R110, C-R140, C-R141, C-R143, C-R144, C-R145, C-R146, C-R147, C-R148, C-R150, H-R001, H-R015, H-R016, H-R017, H-R024, H-R031, O-R023, O-R025, O-R027, O-R032, O-R034, O-R035, O-R050, O-R052, P-R001, P-R007, P-R009, P-R011, P-R021, P-R022, P-R023, P-R025, P-R027, P-R029, P-R033, P-R036, P-R038, P-R040. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

define invalidating activity classes, watermark, refresh→merge residual race, duplicate delivery and post-completion policy.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R007 — design-only (requirement)**; occurrences `C-214, C-215, C-216, C-217, C-218`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **C-R110 — design-only (example)**; occurrences `C-162`; full trace: census/requirements-crosswalk.jsonl
- **C-R140 — design-only (requirement)**; occurrences `C-209`; full trace: census/requirements-crosswalk.jsonl
- **C-R141 — design-only (requirement)**; occurrences `C-210`; full trace: census/requirements-crosswalk.jsonl
- **C-R143 — design-only (requirement)**; occurrences `C-213`; full trace: census/requirements-crosswalk.jsonl
- **C-R144 — partially implemented (requirement)**; occurrences `C-219`; full trace: census/requirements-crosswalk.jsonl
- **C-R145 — partially implemented (requirement)**; occurrences `C-220`; full trace: census/requirements-crosswalk.jsonl
- **C-R146 — design-only (requirement)**; occurrences `C-221`; full trace: census/requirements-crosswalk.jsonl
- **C-R147 — design-only (requirement)**; occurrences `C-222`; full trace: census/requirements-crosswalk.jsonl
- **C-R148 — design-only (requirement)**; occurrences `C-223`; full trace: census/requirements-crosswalk.jsonl
- **C-R150 — partially implemented (requirement)**; occurrences `C-226`; full trace: census/requirements-crosswalk.jsonl
- **H-R001 — design-only (requirement)**; occurrences `H-029`; full trace: census/requirements-crosswalk.jsonl
- **H-R015 — partially implemented (requirement)**; occurrences `H-072`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R025 — design-only (candidate)**; occurrences `O-853, O-854, O-855, O-856`; full trace: census/requirements-crosswalk.jsonl
- **O-R027 — design-only (candidate)**; occurrences `O-860, O-861, O-862, O-863, O-864, O-865, O-866`; full trace: census/requirements-crosswalk.jsonl
- **O-R032 — design-only (candidate)**; occurrences `O-970, O-971, O-972, O-973, O-974, O-975, O-976, O-977, O-978, O-979, O-980, O-981, O-982, O-983, O-984, O-985, O-986, O-987, O-988`; full trace: census/requirements-crosswalk.jsonl
- **O-R034 — design-only (requirement)**; occurrences `O-995, O-996, O-997, O-998, O-999, O-1000, O-1001, O-1002, O-1003, O-1004, O-1005, O-1006, O-1007, O-1008, O-1009, O-1010, O-1011, O-1012, O-1013, O-1014, O-1015, O-1016, O-1017, O-1018, O-1019, O-1020, O-1021, O-1022, O-1023, O-1024, O-1025, O-1026, O-1027, O-1028, O-1029, O-1030, O-1031, O-1032, O-1033, O-1034, O-1035, O-1036`; full trace: census/requirements-crosswalk.jsonl
- **O-R035 — design-only (example)**; occurrences `O-1092, O-1093, O-1094, O-1095, O-1096, O-1097, O-1098, O-1099, O-1100, O-1101`; full trace: census/requirements-crosswalk.jsonl
- **O-R050 — design-only (example)**; occurrences `O-1050, O-1051, O-1052, O-1053, O-1054, O-1055, O-1056, O-1057, O-1058, O-1059, O-1060`; full trace: census/requirements-crosswalk.jsonl
- **O-R052 — design-only (example)**; occurrences `O-1089, O-1090`; full trace: census/requirements-crosswalk.jsonl
- **P-R001 — design-only (candidate)**; occurrences `P-016, P-017, P-018, P-019, P-020, P-021, P-022, P-023, P-024, P-025, P-026, P-027, P-028, P-029, P-030, P-031, P-032, P-033, P-034, P-035, P-036, P-037, P-038, P-039, P-040, P-041, P-042, P-043, P-044, P-045, P-046, P-047, P-048, P-049, P-050, P-051, P-052, P-053, P-054, P-055, P-056, P-057`; full trace: census/requirements-crosswalk.jsonl
- **P-R007 — design-only (candidate)**; occurrences `P-123, P-124, P-125, P-126, P-127, P-128`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R011 — design-only (candidate)**; occurrences `P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310`; full trace: census/requirements-crosswalk.jsonl
- **P-R021 — design-only (candidate)**; occurrences `P-959, P-960, P-961, P-962, P-963, P-964, P-965, P-966, P-967, P-968, P-969, P-970, P-971, P-972, P-973, P-974, P-975, P-976, P-977, P-978, P-979, P-980, P-981, P-982, P-983, P-984, P-985, P-986, P-987, P-988, P-989, P-990, P-991, P-992, P-993, P-994, P-995, P-996, P-997, P-998, P-999, P-1000, P-1001, P-1002, P-1003, P-1004, P-1005, P-1006, P-1007, P-1008, P-1009, P-1010, P-1011, P-1012, P-1013, P-1014, P-1015, P-1016, P-1017, P-1018, P-1019, P-1020`; full trace: census/requirements-crosswalk.jsonl
- **P-R022 — design-only (candidate)**; occurrences `P-1434, P-1435, P-1436, P-1437, P-1438, P-1439, P-1440, P-1441, P-1442, P-1443, P-1444, P-1445, P-1446, P-1447, P-1448, P-1449, P-1450, P-1451, P-1452, P-1453, P-1454, P-1455, P-1456, P-1457`; full trace: census/requirements-crosswalk.jsonl
- **P-R023 — design-only (candidate)**; occurrences `P-1459, P-1460, P-1461, P-1462, P-1463, P-1464, P-1465, P-1466, P-1467, P-1468, P-1469, P-1470, P-1471, P-1472, P-1473, P-1474, P-1475, P-1476, P-1477, P-1478, P-1479, P-1480, P-1481, P-1482, P-1483, P-1484, P-1485, P-1486, P-1487, P-1488, P-1489, P-1490, P-1491, P-1492`; full trace: census/requirements-crosswalk.jsonl
- **P-R025 — partially implemented (candidate)**; occurrences `P-1540, P-1541, P-1542, P-1543, P-1544, P-1545, P-1546, P-1547, P-1548, P-1549, P-1550, P-1551, P-1552, P-1553, P-1554, P-1555, P-1556, P-1557, P-1558, P-1559, P-1560, P-1561, P-1562, P-1563, P-1564, P-1565, P-1566, P-1567, P-1568, P-1569, P-1570, P-1571, P-1572, P-1573, P-1574, P-1575, P-1576, P-1577, P-1578, P-1579, P-1580, P-1581, P-1582, P-1583, P-1584, P-1585, P-1586, P-1587, P-1588, P-1589, P-1590, P-1591, P-1592, P-1593, P-1594, P-1595, P-1596, P-1597, P-1598, P-1599, P-1600, P-1601, P-1602, P-1603, P-1604, P-1605, P-1606, P-1607, P-1608`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R029 — design-only (candidate)**; occurrences `P-1761, P-1762, P-1763, P-1764, P-1765, P-1766, P-1767, P-1768, P-1769, P-1770, P-1771, P-1772, P-1773, P-1774, P-1775, P-1776`; full trace: census/requirements-crosswalk.jsonl
- **P-R033 — design-only (candidate)**; occurrences `P-456, P-457, P-458, P-459, P-460, P-461, P-462, P-463, P-464, P-465, P-466, P-467, P-468, P-469, P-470, P-471, P-472, P-473, P-474, P-475, P-476, P-477`; full trace: census/requirements-crosswalk.jsonl
- **P-R036 — design-only (example)**; occurrences `P-1214, P-1215, P-1216, P-1217, P-1218, P-1219, P-1220, P-1221, P-1222, P-1223, P-1224, P-1225, P-1226, P-1227, P-1228, P-1229, P-1230, P-1231, P-1232, P-1233, P-1234, P-1235, P-1236, P-1237, P-1238, P-1239, P-1240, P-1241, P-1242, P-1243, P-1244, P-1245, P-1246, P-1247, P-1248, P-1249, P-1250, P-1251, P-1252, P-1253, P-1254, P-1255, P-1256, P-1257, P-1258, P-1259, P-1260, P-1261, P-1262, P-1263, P-1264, P-1265, P-1266, P-1267, P-1268, P-1269, P-1270, P-1271, P-1272, P-1273, P-1274, P-1275, P-1276, P-1277, P-1278, P-1279, P-1280, P-1281, P-1282, P-1283, P-1284, P-1285, P-1286, P-1287, P-1288, P-1289, P-1290, P-1291, P-1292, P-1293, P-1294, P-1295, P-1296, P-1297, P-1298, P-1299, P-1300, P-1301, P-1302, P-1303, P-1304, P-1305, P-1306, P-1307, P-1308, P-1309, P-1310, P-1311, P-1312, P-1313, P-1314, P-1315, P-1316, P-1317, P-1318, P-1319, P-1320, P-1321, P-1322, P-1323, P-1324, P-1325, P-1326, P-1327, P-1328, P-1329, P-1330, P-1331, P-1332, P-1333, P-1334, P-1335, P-1336, P-1337, P-1338, P-1339, P-1340, P-1341, P-1342, P-1343, P-1344, P-1345, P-1346, P-1347, P-1348, P-1349, P-1350, P-1351, P-1352, P-1353, P-1354, P-1355, P-1356, P-1357, P-1358, P-1359, P-1360, P-1361, P-1362, P-1363, P-1364, P-1365, P-1366, P-1367, P-1368, P-1369, P-1370, P-1371, P-1372, P-1373, P-1374, P-1375, P-1376, P-1377, P-1378, P-1379, P-1380, P-1381, P-1382`; full trace: census/requirements-crosswalk.jsonl
- **P-R038 — design-only (candidate)**; occurrences `P-1383, P-1384`; full trace: census/requirements-crosswalk.jsonl
- **P-R040 — design-only (candidate)**; occurrences `P-1414`; full trace: census/requirements-crosswalk.jsonl
  Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/completion.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: HYD-06, REL-03, AUTH-01
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9207→HYD-06, #9203→REL-03, #9208→AUTH-01.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `verify listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary`.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

verify listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary`.

Negative coverage: tiny comment race, delayed/duplicate webhook, post-terminal event; no global lock claim.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: conservative hold only for policy-required classes, owner approval before production. No dispatch/#6533 proof.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9209-leases-fencing-outbox-direct-completion.md`, slice `REL-09` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `flag.require-clean-reconciliation, work.complete`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### REL-09 — Last-activity gates, duplicate webhook, post-completion policy

**Tier:** recommended; hard before live completion claims clean reconciliation. **Goal:** define invalidating activity classes, watermark, refresh→merge residual race, duplicate delivery and post-completion policy. **Owns:** `src/work/completion.rs` fixtures/policy; #9207 owns reads, #9203 event routing/dedupe. **Hard blockers:** #9207 cursor contract; #9203 authenticated event/delivery identity; #9208 authority. **First failing oracle:** `pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary` injects late review between read/merge and asserts receipt exposes freshness limit. **Done command:** verify listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary`. **Negative:** tiny comment race, delayed/duplicate webhook, post-terminal event; no global lock claim. **Migration:** conservative hold only for policy-required classes, owner approval before production. No dispatch/#6533 proof.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `REL-08` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/completion.rs`
