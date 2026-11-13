# HYD-08 — Bounded refresh/watch/reconcile CLI and capability contract

<!-- pmo-9196:HYD-08 -->

**Issue title:** `design(pm-orchestration): Bounded refresh/watch/reconcile CLI and capability contract`  
**Design parent:** #9207 · **Scope:** #9207 hydration/activity · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R009, C-R107, C-R108, C-R110, C-R118, C-R150, C-R151, C-R153, H-R001, H-R014, H-R016, H-R017, H-R024, H-R031, O-R021, O-R022, O-R023, O-R043, O-R050, P-R009, P-R013, P-R018, P-R027, P-R029, P-R031, P-R035, P-R036, P-R037, P-R039. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

implement bounded `hydrate`, `refresh`, `changes`, `activity reconcile`, `watch` service behavior mapped to shared contracts.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R108 — partially implemented (requirement)**; occurrences `C-160`; full trace: census/requirements-crosswalk.jsonl
- **C-R110 — design-only (example)**; occurrences `C-162`; full trace: census/requirements-crosswalk.jsonl
- **C-R118 — open implementation (requirement)**; occurrences `C-185`; full trace: census/requirements-crosswalk.jsonl
- **C-R150 — partially implemented (requirement)**; occurrences `C-226`; full trace: census/requirements-crosswalk.jsonl
- **C-R151 — partially implemented (requirement)**; occurrences `C-227`; full trace: census/requirements-crosswalk.jsonl
- **C-R153 — open implementation (requirement)**; occurrences `C-229`; full trace: census/requirements-crosswalk.jsonl
- **H-R001 — design-only (requirement)**; occurrences `H-029`; full trace: census/requirements-crosswalk.jsonl
- **H-R014 — design-only (requirement)**; occurrences `H-071`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **O-R021 — design-only (candidate)**; occurrences `O-779, O-780, O-781`; full trace: census/requirements-crosswalk.jsonl
- **O-R022 — design-only (candidate)**; occurrences `O-782, O-783, O-784, O-785, O-786, O-787, O-788, O-789, O-790, O-791, O-792, O-793, O-794, O-795, O-796, O-797, O-798, O-799, O-800, O-801, O-802, O-803, O-804, O-805, O-806, O-807, O-808, O-809, O-810, O-811, O-812, O-813, O-814, O-815, O-816, O-817, O-818, O-819, O-820, O-821, O-822, O-823, O-824, O-825, O-826, O-827, O-828, O-829, O-830, O-831, O-832, O-833, O-834, O-835, O-836, O-837, O-838, O-839, O-840, O-841, O-842, O-843, O-844, O-845`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R043 — design-only (history)**; occurrences `O-058, O-059, O-060, O-061, O-062, O-063, O-064, O-065, O-066, O-067, O-068, O-069, O-070, O-071, O-072, O-073, O-074, O-075, O-076, O-077, O-078, O-079, O-080`; full trace: census/requirements-crosswalk.jsonl
- **O-R050 — design-only (example)**; occurrences `O-1050, O-1051, O-1052, O-1053, O-1054, O-1055, O-1056, O-1057, O-1058, O-1059, O-1060`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **P-R013 — design-only (candidate)**; occurrences `P-479, P-480, P-481, P-482, P-483, P-484, P-485, P-486, P-487, P-488, P-489, P-490, P-491, P-492, P-493, P-494, P-495, P-496, P-497, P-498, P-499, P-500, P-501, P-502, P-503, P-504, P-505, P-506, P-507, P-508, P-509, P-510, P-511, P-512, P-513, P-514, P-515, P-516, P-517, P-518, P-519, P-520, P-521`; full trace: census/requirements-crosswalk.jsonl
- **P-R018 — design-only (candidate)**; occurrences `P-789, P-790, P-791, P-792, P-793, P-794, P-795, P-796, P-797, P-798, P-799, P-800, P-801, P-802, P-803, P-804, P-805, P-806, P-807, P-808, P-809, P-810, P-811, P-812, P-813, P-814, P-815, P-816, P-817, P-818, P-819, P-820, P-821, P-822, P-823, P-824, P-825, P-826, P-827, P-828, P-829, P-830, P-831, P-832, P-833, P-834, P-835, P-836, P-837, P-838, P-839, P-840, P-841, P-842, P-843, P-844, P-845, P-846, P-847`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R029 — design-only (candidate)**; occurrences `P-1761, P-1762, P-1763, P-1764, P-1765, P-1766, P-1767, P-1768, P-1769, P-1770, P-1771, P-1772, P-1773, P-1774, P-1775, P-1776`; full trace: census/requirements-crosswalk.jsonl
- **P-R031 — design-only (candidate)**; occurrences `P-432, P-433, P-434, P-435, P-436, P-437, P-438, P-439, P-440, P-441, P-442, P-443`; full trace: census/requirements-crosswalk.jsonl
- **P-R035 — design-only (example)**; occurrences `P-1080, P-1081, P-1082, P-1083, P-1084, P-1085, P-1086, P-1087, P-1088, P-1089, P-1090, P-1091, P-1092, P-1093, P-1094, P-1095, P-1096, P-1097, P-1098, P-1099, P-1100, P-1101, P-1102, P-1103, P-1104, P-1105, P-1106, P-1107, P-1108, P-1109, P-1110, P-1111, P-1112, P-1113, P-1114, P-1115, P-1116, P-1117, P-1118, P-1119, P-1120, P-1121, P-1122, P-1123, P-1124, P-1125, P-1126, P-1127, P-1128, P-1129, P-1130, P-1131, P-1132, P-1133, P-1134, P-1135, P-1136, P-1137, P-1138, P-1139, P-1140, P-1141, P-1142, P-1143, P-1144, P-1145, P-1146, P-1147, P-1148, P-1149, P-1150, P-1151, P-1152, P-1153, P-1154, P-1155, P-1156, P-1157, P-1158, P-1159, P-1160, P-1161, P-1162, P-1163, P-1164, P-1165, P-1166, P-1167, P-1168, P-1169, P-1170, P-1171, P-1172, P-1173, P-1174, P-1175, P-1176, P-1177, P-1178, P-1179, P-1180, P-1181, P-1182, P-1183, P-1184, P-1185, P-1186, P-1187, P-1188, P-1189, P-1190, P-1191, P-1192, P-1193, P-1194, P-1195, P-1196, P-1197, P-1198, P-1199, P-1200, P-1201, P-1202, P-1203, P-1204, P-1205, P-1206, P-1207, P-1208, P-1209, P-1210, P-1211, P-1212, P-1213`; full trace: census/requirements-crosswalk.jsonl
- **P-R036 — design-only (example)**; occurrences `P-1214, P-1215, P-1216, P-1217, P-1218, P-1219, P-1220, P-1221, P-1222, P-1223, P-1224, P-1225, P-1226, P-1227, P-1228, P-1229, P-1230, P-1231, P-1232, P-1233, P-1234, P-1235, P-1236, P-1237, P-1238, P-1239, P-1240, P-1241, P-1242, P-1243, P-1244, P-1245, P-1246, P-1247, P-1248, P-1249, P-1250, P-1251, P-1252, P-1253, P-1254, P-1255, P-1256, P-1257, P-1258, P-1259, P-1260, P-1261, P-1262, P-1263, P-1264, P-1265, P-1266, P-1267, P-1268, P-1269, P-1270, P-1271, P-1272, P-1273, P-1274, P-1275, P-1276, P-1277, P-1278, P-1279, P-1280, P-1281, P-1282, P-1283, P-1284, P-1285, P-1286, P-1287, P-1288, P-1289, P-1290, P-1291, P-1292, P-1293, P-1294, P-1295, P-1296, P-1297, P-1298, P-1299, P-1300, P-1301, P-1302, P-1303, P-1304, P-1305, P-1306, P-1307, P-1308, P-1309, P-1310, P-1311, P-1312, P-1313, P-1314, P-1315, P-1316, P-1317, P-1318, P-1319, P-1320, P-1321, P-1322, P-1323, P-1324, P-1325, P-1326, P-1327, P-1328, P-1329, P-1330, P-1331, P-1332, P-1333, P-1334, P-1335, P-1336, P-1337, P-1338, P-1339, P-1340, P-1341, P-1342, P-1343, P-1344, P-1345, P-1346, P-1347, P-1348, P-1349, P-1350, P-1351, P-1352, P-1353, P-1354, P-1355, P-1356, P-1357, P-1358, P-1359, P-1360, P-1361, P-1362, P-1363, P-1364, P-1365, P-1366, P-1367, P-1368, P-1369, P-1370, P-1371, P-1372, P-1373, P-1374, P-1375, P-1376, P-1377, P-1378, P-1379, P-1380, P-1381, P-1382`; full trace: census/requirements-crosswalk.jsonl
- **P-R037 — design-only (example)**; occurrences `P-1386, P-1387, P-1388, P-1389, P-1390, P-1391, P-1392, P-1393, P-1394, P-1395, P-1396, P-1397, P-1398, P-1399, P-1400, P-1401, P-1402, P-1403, P-1404, P-1405, P-1406, P-1407, P-1408, P-1409, P-1410, P-1412, P-1413, P-1415, P-1416, P-1417, P-1418, P-1419, P-1420, P-1421, P-1422, P-1423, P-1424, P-1425, P-1426, P-1427, P-1428, P-1429, P-1430, P-1431, P-1432`; full trace: census/requirements-crosswalk.jsonl
- **P-R039 — design-only (candidate)**; occurrences `P-1411`; full trace: census/requirements-crosswalk.jsonl
Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.
## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/context.rs`
- `pkgs/fleet/crates/fleet-queue/src/work/activity.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: CLI-13, HYD-04, HYD-05, HYD-06, REL-03
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9206→CLI-13, #9203→REL-03.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `verify test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`. Shared CLI acceptance belongs its owner and has a separate named golden help/JSON test.`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

verify test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`. Shared CLI acceptance belongs its owner and has a separate named golden help/JSON test.

Negative coverage: ambiguous resource, unsupported include advertised, timeout returns success or fictional cancellation, watch spawns hidden poller.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: additive only; no queue rename. Deployment/runtime separate.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9207-context-hydration-incremental-activity.md`, slice `HYD-08` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `activity.list, activity.reconcile, activity.since, activity.watch, flag.external-only, flag.full, flag.include, flag.output, flag.since, flag.since-checkpoint, flag.since-start, flag.start, work.refresh, work.resume`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### HYD-08 — Bounded refresh/watch/reconcile CLI and capability contract

**Tier:** recommended. **Goal:** implement bounded `hydrate`, `refresh`, `changes`, `activity reconcile`, `watch` service behavior mapped to shared contracts. **Owns:** `pkgs/fleet/crates/fleet-queue/src/work/context.rs` and `activity.rs` unit tests; does not own `fleet/src/main.rs`, registry or shared CLI integration files (one #9206 integration slice owns those). **Hard blockers:** granular #9206 Command/Result contract and shared CLI-registration slice; HYD-04–06. #9203 live event routing excluded. **First failing oracle:** `pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt` tests deadline/cancel without worker spawn. **Done command:** verify test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`. Shared CLI acceptance belongs its owner and has a separate named golden help/JSON test. **Negative:** ambiguous resource, unsupported include advertised, timeout returns success or fictional cancellation, watch spawns hidden poller. **Migration:** additive only; no queue rename. Deployment/runtime separate.


## Merged source-key/facade acceptance

- `CLI-09` → `HYD-08`: Hydration/activity command façade maps to HYD-08; page/cursor/activity behavior remains HYD-05/06. Exact command rows allocated here: activity.list, activity.reconcile, activity.since, activity.watch, flag.external-only, flag.full, flag.include, flag.output, flag.since, flag.since-checkpoint, flag.since-start, flag.start.


## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):
- blocked by `HYD-07` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/activity.rs`
- blocked by `HYD-04` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/context.rs`
