# CEN-01 — Requirement/source-occurrence crosswalk integrity

<!-- pmo-9196:CEN-01 -->

**Issue title:** `design(pm-orchestration): Requirement/source-occurrence crosswalk integrity`  
**Design parent:** #9198 · **Scope:** #9198 crosswalk/census · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R008, C-R010, C-R014, C-R015, C-R016, C-R018, C-R020, C-R021, C-R024, C-R026, C-R029, C-R030, C-R031, C-R032, C-R033, C-R034, C-R035, C-R036, C-R037, C-R038, C-R158, C-R160, C-R161, C-R162, C-R163, C-R164, C-R166, C-R167, H-R001, H-R002, H-R016, H-R018, H-R019, H-R022, H-R023, H-R024, H-R025, H-R026, H-R027, H-R028, H-R029, H-R032, H-R034, H-R037, H-R046, H-R047, H-R048, H-R050, H-R059, H-R061, H-R063, O-R001, O-R036, O-R037, O-R039, O-R040, O-R041, O-R042, O-R045, O-R048, O-R053, P-R025, P-R026, P-R027, P-R055, P-R070, S-R009. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

prove each of the 389 semantic record IDs occurs exactly once in the crosswalk, every occurrence citation resolves to the 3,571-row disposition ledger, duplicates preserve `duplicate_of`, groups preserve children, and no source/modality is silently lost. -

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R008 — design-only (requirement)**; occurrences `C-011`; full trace: census/requirements-crosswalk.jsonl
- **C-R010 — design-only (requirement)**; occurrences `C-013`; full trace: census/requirements-crosswalk.jsonl
- **C-R014 — design-only (requirement)**; occurrences `C-018`; full trace: census/requirements-crosswalk.jsonl
- **C-R015 — research (requirement)**; occurrences `C-019`; full trace: census/requirements-crosswalk.jsonl
- **C-R016 — design-only (requirement)**; occurrences `C-020`; full trace: census/requirements-crosswalk.jsonl
- **C-R018 — design-only (requirement)**; occurrences `C-023`; full trace: census/requirements-crosswalk.jsonl
- **C-R020 — design-only (requirement)**; occurrences `C-026`; full trace: census/requirements-crosswalk.jsonl
- **C-R021 — design-only (requirement)**; occurrences `C-027`; full trace: census/requirements-crosswalk.jsonl
- **C-R024 — design-only (requirement)**; occurrences `C-032`; full trace: census/requirements-crosswalk.jsonl
- **C-R026 — design-only (requirement)**; occurrences `C-034`; full trace: census/requirements-crosswalk.jsonl
- **C-R029 — unverified (requirement)**; occurrences `C-038`; full trace: census/requirements-crosswalk.jsonl
- **C-R030 — unverified (requirement)**; occurrences `C-039`; full trace: census/requirements-crosswalk.jsonl
- **C-R031 — unverified (requirement)**; occurrences `C-041`; full trace: census/requirements-crosswalk.jsonl
- **C-R032 — unverified (requirement)**; occurrences `C-042`; full trace: census/requirements-crosswalk.jsonl
- **C-R033 — unverified (requirement)**; occurrences `C-044`; full trace: census/requirements-crosswalk.jsonl
- **C-R034 — unverified (requirement)**; occurrences `C-045`; full trace: census/requirements-crosswalk.jsonl
- **C-R035 — verified existing (requirement)**; occurrences `C-046`; full trace: census/requirements-crosswalk.jsonl
- **C-R036 — verified existing (requirement)**; occurrences `C-047`; full trace: census/requirements-crosswalk.jsonl
- **C-R037 — unverified (requirement)**; occurrences `C-048`; full trace: census/requirements-crosswalk.jsonl
- **C-R038 — design-only (requirement)**; occurrences `C-050`; full trace: census/requirements-crosswalk.jsonl
- **C-R158 — research (requirement)**; occurrences `C-235`; full trace: census/requirements-crosswalk.jsonl
- **C-R160 — unverified (requirement)**; occurrences `C-237`; full trace: census/requirements-crosswalk.jsonl
- **C-R161 — design-only (requirement)**; occurrences `C-239`; full trace: census/requirements-crosswalk.jsonl
- **C-R162 — design-only (requirement)**; occurrences `C-240`; full trace: census/requirements-crosswalk.jsonl
- **C-R163 — design-only (requirement)**; occurrences `C-241`; full trace: census/requirements-crosswalk.jsonl
- **C-R164 — unverified (requirement)**; occurrences `C-242`; full trace: census/requirements-crosswalk.jsonl
- **C-R166 — design-only (requirement)**; occurrences `C-250`; full trace: census/requirements-crosswalk.jsonl
- **C-R167 — design-only (requirement)**; occurrences `C-251`; full trace: census/requirements-crosswalk.jsonl
- **H-R001 — design-only (requirement)**; occurrences `H-029`; full trace: census/requirements-crosswalk.jsonl
- **H-R002 — design-only (requirement)**; occurrences `H-008, H-009, H-010`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R018 — research (requirement)**; occurrences `H-156`; full trace: census/requirements-crosswalk.jsonl
- **H-R019 — design-only (requirement)**; occurrences `H-033, H-034, H-035, H-036, H-037, H-038, H-039, H-040, H-041, H-042, H-043, H-044, H-045, H-046`; full trace: census/requirements-crosswalk.jsonl
- **H-R022 — unverified (requirement)**; occurrences `H-077, H-078, H-079, H-080, H-081, H-082`; full trace: census/requirements-crosswalk.jsonl
- **H-R023 — design-only (requirement)**; occurrences `H-083, H-084, H-085, H-086, H-087`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R025 — design-only (requirement)**; occurrences `H-011, H-012`; full trace: census/requirements-crosswalk.jsonl
- **H-R026 — unverified (requirement)**; occurrences `H-013, H-014, H-015, H-016`; full trace: census/requirements-crosswalk.jsonl
- **H-R027 — design-only (candidate)**; occurrences `H-022, H-023, H-024`; full trace: census/requirements-crosswalk.jsonl
- **H-R028 — design-only (requirement)**; occurrences `H-030, H-031, H-032`; full trace: census/requirements-crosswalk.jsonl
- **H-R029 — design-only (requirement)**; occurrences `H-050, H-051, H-052, H-053, H-054, H-055, H-056`; full trace: census/requirements-crosswalk.jsonl
- **H-R032 — design-only (requirement)**; occurrences `H-097, H-098, H-099, H-100, H-101, H-102, H-103, H-104, H-105`; full trace: census/requirements-crosswalk.jsonl
- **H-R034 — design-only (history)**; occurrences `H-155, H-157, H-158, H-159, H-160, H-161, H-162, H-163`; full trace: census/requirements-crosswalk.jsonl
- **H-R037 — research (requirement)**; occurrences `H-156`; full trace: census/requirements-crosswalk.jsonl
- **H-R046 — design-only (requirement)**; occurrences `H-083`; full trace: census/requirements-crosswalk.jsonl
- **H-R047 — design-only (requirement)**; occurrences `H-084`; full trace: census/requirements-crosswalk.jsonl
- **H-R048 — design-only (requirement)**; occurrences `H-085`; full trace: census/requirements-crosswalk.jsonl
- **H-R050 — design-only (requirement)**; occurrences `H-087`; full trace: census/requirements-crosswalk.jsonl
- **H-R059 — design-only (requirement)**; occurrences `H-083`; full trace: census/requirements-crosswalk.jsonl
- **H-R061 — design-only (requirement)**; occurrences `H-085`; full trace: census/requirements-crosswalk.jsonl
- **H-R063 — design-only (requirement)**; occurrences `H-087`; full trace: census/requirements-crosswalk.jsonl
- **O-R001 — design-only (requirement)**; occurrences `O-174, O-175, O-176, O-177, O-178, O-179, O-180, O-181, O-182, O-183, O-184, O-185, O-186, O-187, O-188, O-189, O-190, O-191, O-192, O-193, O-194, O-195, O-196, O-197`; full trace: census/requirements-crosswalk.jsonl
- **O-R036 — design-only (candidate)**; occurrences `O-1103, O-1104, O-1105, O-1106, O-1107, O-1108, O-1109, O-1110, O-1111, O-1112, O-1113, O-1114, O-1115, O-1116, O-1117, O-1118, O-1119, O-1120, O-1121, O-1122, O-1123, O-1124, O-1125, O-1126, O-1127, O-1128, O-1129, O-1130, O-1131, O-1132, O-1133, O-1134, O-1135, O-1136, O-1137, O-1138, O-1139, O-1140, O-1141`; full trace: census/requirements-crosswalk.jsonl
- **O-R037 — design-only (example)**; occurrences `O-1143, O-1144, O-1145, O-1146, O-1147, O-1148, O-1149, O-1150, O-1151, O-1152, O-1153, O-1154, O-1155, O-1156, O-1157, O-1158, O-1159, O-1160, O-1161, O-1162, O-1163, O-1164, O-1165, O-1166, O-1167, O-1168, O-1169, O-1170, O-1171, O-1172, O-1173, O-1174, O-1175, O-1176, O-1177`; full trace: census/requirements-crosswalk.jsonl
- **O-R039 — design-only (candidate)**; occurrences `O-1242, O-1243, O-1244, O-1245, O-1246, O-1247, O-1248, O-1249, O-1250, O-1251, O-1252, O-1253, O-1254, O-1255, O-1256, O-1257, O-1258, O-1259, O-1260, O-1261, O-1262, O-1263, O-1264, O-1265, O-1266, O-1267, O-1268, O-1269, O-1270, O-1271, O-1272, O-1273, O-1274, O-1275, O-1276, O-1277, O-1278, O-1279, O-1280, O-1281, O-1282, O-1283, O-1284, O-1285, O-1286, O-1287, O-1288, O-1289, O-1290, O-1291, O-1292, O-1293, O-1294, O-1295, O-1296, O-1297, O-1298, O-1299, O-1300, O-1301, O-1302, O-1303, O-1304, O-1305, O-1306, O-1307, O-1308, O-1309, O-1310`; full trace: census/requirements-crosswalk.jsonl
- **O-R040 — design-only (history)**; occurrences `O-016, O-017, O-018, O-019, O-020, O-021, O-022, O-023, O-024`; full trace: census/requirements-crosswalk.jsonl
- **O-R041 — design-only (history)**; occurrences `O-025, O-026, O-027, O-028, O-029, O-030, O-031, O-032, O-033, O-034, O-035, O-036, O-037, O-038, O-039, O-040, O-041, O-042, O-043, O-044, O-045, O-046, O-047, O-048, O-049, O-050, O-051`; full trace: census/requirements-crosswalk.jsonl
- **O-R042 — design-only (candidate)**; occurrences `O-053, O-054, O-055, O-056, O-057`; full trace: census/requirements-crosswalk.jsonl
- **O-R045 — design-only (history)**; occurrences `O-107, O-108, O-109, O-110, O-111, O-112, O-113, O-114, O-115, O-116, O-117, O-118, O-119, O-120, O-121, O-122, O-123, O-124, O-125, O-126, O-127, O-128, O-129, O-130, O-131, O-132, O-133, O-134`; full trace: census/requirements-crosswalk.jsonl
- **O-R048 — design-only (history)**; occurrences `O-159, O-160, O-161, O-162, O-163, O-164, O-165, O-166, O-167, O-168, O-169, O-170`; full trace: census/requirements-crosswalk.jsonl
- **O-R053 — design-only (history)**; occurrences `O-006, O-011`; full trace: census/requirements-crosswalk.jsonl
- **P-R025 — partially implemented (candidate)**; occurrences `P-1540, P-1541, P-1542, P-1543, P-1544, P-1545, P-1546, P-1547, P-1548, P-1549, P-1550, P-1551, P-1552, P-1553, P-1554, P-1555, P-1556, P-1557, P-1558, P-1559, P-1560, P-1561, P-1562, P-1563, P-1564, P-1565, P-1566, P-1567, P-1568, P-1569, P-1570, P-1571, P-1572, P-1573, P-1574, P-1575, P-1576, P-1577, P-1578, P-1579, P-1580, P-1581, P-1582, P-1583, P-1584, P-1585, P-1586, P-1587, P-1588, P-1589, P-1590, P-1591, P-1592, P-1593, P-1594, P-1595, P-1596, P-1597, P-1598, P-1599, P-1600, P-1601, P-1602, P-1603, P-1604, P-1605, P-1606, P-1607, P-1608`; full trace: census/requirements-crosswalk.jsonl
- **P-R026 — design-only (candidate)**; occurrences `P-1610, P-1611, P-1612, P-1613, P-1614, P-1615, P-1616, P-1617, P-1618, P-1619, P-1620, P-1621, P-1622, P-1623, P-1624, P-1625, P-1626, P-1627, P-1628, P-1629, P-1630, P-1631, P-1632, P-1633, P-1634, P-1635, P-1636, P-1637, P-1638, P-1639, P-1640, P-1641, P-1642, P-1643, P-1644, P-1645, P-1646, P-1647, P-1648, P-1649, P-1650, P-1651, P-1652, P-1653, P-1654, P-1655, P-1656, P-1657, P-1658, P-1659, P-1660`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R055 — design-only (candidate)**; occurrences `P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **P-R070 — design-only (candidate)**; occurrences `P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **S-R009 — design-only (candidate)**; occurrences `S-056, S-057, S-058, S-059`; full trace: census/requirements-crosswalk.jsonl
Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.
## Owned files and boundaries

- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`
- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.md`
- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/check_requirements_crosswalk.py`
- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_requirements_crosswalk.py`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `test_requirements_crosswalk_rejects_missing_duplicate_occurrence_and_unresolved_design_anchor`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** ``python3 -I inventory/check_inventory.py --check` and `python3 -I inventory/check_requirements.py --check` remain passing; `python3 -I census/check_requirements_crosswalk.py --check` validates 389 IDs, group/alias edges, source occurrences, exact status vocabulary, evidence object on verified/partial, real anchors, 215 command-map keys and no wildcard. Check emits explicit per-class unmapped rows; it never treats absence of a join as zero gaps. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

`python3 -I inventory/check_inventory.py --check` and `python3 -I inventory/check_requirements.py --check` remain passing; `python3 -I census/check_requirements_crosswalk.py --check` validates 389 IDs, group/alias edges, source occurrences, exact status vocabulary, evidence object on verified/partial, real anchors, 215 command-map keys and no wildcard. Check emits explicit per-class unmapped rows; it never treats absence of a join as zero gaps. -

Negative coverage: duplicate ID, missing occurrence, wrong quote/span, dead file/heading, status typo, partial row with no source evidence, group child not found and wildcard command mapping each fail. Research artifacts only; rollback is restore the prior generated crosswalk, not to edit source ledgers. -

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9198-requirement-crosswalk-current-state-census.md`, slice `CEN-01` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### CEN-01 — Requirement/source-occurrence crosswalk integrity

- **Tier:** crucial. **Goal:** prove each of the 389 semantic record IDs occurs exactly once in the crosswalk, every occurrence citation resolves to the 3,571-row disposition ledger, duplicates preserve `duplicate_of`, groups preserve children, and no source/modality is silently lost.
- **Owns:** `census/requirements-crosswalk.jsonl`, `census/requirements-crosswalk.md`, and new strict validator/test files `census/check_requirements_crosswalk.py` and `census/test_requirements_crosswalk.py`; it may call but not change `inventory/check_inventory.py` or `inventory/check_requirements.py`. No edits to semantic inventory/dispositions. Exact design refs and local keys must parse against actual files/headings and `census/command-slice-map.jsonl`.
- **Hard dependencies:** frozen `inventory/requirements.jsonl` and `inventory/dispositions.jsonl`; crosswalk document in this package.
- **First failing oracle:** `test_requirements_crosswalk_rejects_missing_duplicate_occurrence_and_unresolved_design_anchor` in `census/test_requirements_crosswalk.py` mutates each relation in a temporary fixture and expects failure; oracle is exact ID/span set equality plus heading existence, not production implementation.
- **Done check:** `python3 -I inventory/check_inventory.py --check` and `python3 -I inventory/check_requirements.py --check` remain passing; `python3 -I census/check_requirements_crosswalk.py --check` validates 389 IDs, group/alias edges, source occurrences, exact status vocabulary, evidence object on verified/partial, real anchors, 215 command-map keys and no wildcard. Check emits explicit per-class unmapped rows; it never treats absence of a join as zero gaps.
- **Negative / rollback:** duplicate ID, missing occurrence, wrong quote/span, dead file/heading, status typo, partial row with no source evidence, group child not found and wildcard command mapping each fail. Research artifacts only; rollback is restore the prior generated crosswalk, not to edit source ledgers.
- **Operator-script contract:** the future validator accepts only `--check`; it parses the complete argv before opening any source/output, writes nothing in check mode, rejects unknown/extra args, and follows the strict sibling argument pattern.

