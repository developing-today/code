# DOC-01 — Operator and contributor contract documentation

<!-- pmo-9196:DOC-01 -->

**Issue title:** `design(pm-orchestration): Operator and contributor contract documentation`  
**Design parent:** #9208 · **Scope:** #9208 relations/journal/authority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R009, C-R011, C-R013, C-R015, C-R017, C-R019, C-R022, C-R023, C-R024, C-R025, C-R027, C-R028, C-R031, C-R033, C-R035, C-R128, C-R157, C-R159, C-R162, C-R164, C-R165, H-R003, H-R019, H-R020, H-R023, H-R024, H-R028, H-R029, H-R031, H-R035, O-R019, O-R020, O-R037, O-R039, O-R041, O-R043, O-R044, O-R045, P-R008, P-R026, P-R027, S-R009, S-R012. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

publish lifecycle relation/authority/error privacy runbook: add/resolve/remove blocker, view journal, correct note, repair quarantined edge, reconcile mirror conflict; document no-CAS caveat and #9178/#9179 compatibility.

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R011 — unverified (requirement)**; occurrences `C-014`; full trace: census/requirements-crosswalk.jsonl
- **C-R013 — design-only (requirement)**; occurrences `C-016`; full trace: census/requirements-crosswalk.jsonl
- **C-R015 — research (requirement)**; occurrences `C-019`; full trace: census/requirements-crosswalk.jsonl
- **C-R017 — unverified (requirement)**; occurrences `C-022`; full trace: census/requirements-crosswalk.jsonl
- **C-R019 — design-only (requirement)**; occurrences `C-024`; full trace: census/requirements-crosswalk.jsonl
- **C-R022 — open implementation (requirement)**; occurrences `C-029`; full trace: census/requirements-crosswalk.jsonl
- **C-R023 — open implementation (requirement)**; occurrences `C-030`; full trace: census/requirements-crosswalk.jsonl
- **C-R024 — design-only (requirement)**; occurrences `C-032`; full trace: census/requirements-crosswalk.jsonl
- **C-R025 — design-only (requirement)**; occurrences `C-033`; full trace: census/requirements-crosswalk.jsonl
- **C-R027 — unverified (requirement)**; occurrences `C-036`; full trace: census/requirements-crosswalk.jsonl
- **C-R028 — unverified (requirement)**; occurrences `C-037`; full trace: census/requirements-crosswalk.jsonl
- **C-R031 — unverified (requirement)**; occurrences `C-041`; full trace: census/requirements-crosswalk.jsonl
- **C-R033 — unverified (requirement)**; occurrences `C-044`; full trace: census/requirements-crosswalk.jsonl
- **C-R035 — verified existing (requirement)**; occurrences `C-046`; full trace: census/requirements-crosswalk.jsonl
- **C-R128 — design-only (requirement)**; occurrences `C-196`; full trace: census/requirements-crosswalk.jsonl
- **C-R157 — design-only (requirement)**; occurrences `C-234`; full trace: census/requirements-crosswalk.jsonl
- **C-R159 — design-only (requirement)**; occurrences `C-236`; full trace: census/requirements-crosswalk.jsonl
- **C-R162 — design-only (requirement)**; occurrences `C-240`; full trace: census/requirements-crosswalk.jsonl
- **C-R164 — unverified (requirement)**; occurrences `C-242`; full trace: census/requirements-crosswalk.jsonl
- **C-R165 — unverified (requirement)**; occurrences `C-249`; full trace: census/requirements-crosswalk.jsonl
- **H-R003 — design-only (requirement)**; occurrences `H-025, H-026, H-027`; full trace: census/requirements-crosswalk.jsonl
- **H-R019 — design-only (requirement)**; occurrences `H-033, H-034, H-035, H-036, H-037, H-038, H-039, H-040, H-041, H-042, H-043, H-044, H-045, H-046`; full trace: census/requirements-crosswalk.jsonl
- **H-R020 — unverified (requirement)**; occurrences `H-057, H-058`; full trace: census/requirements-crosswalk.jsonl
- **H-R023 — design-only (requirement)**; occurrences `H-083, H-084, H-085, H-086, H-087`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R028 — design-only (requirement)**; occurrences `H-030, H-031, H-032`; full trace: census/requirements-crosswalk.jsonl
- **H-R029 — design-only (requirement)**; occurrences `H-050, H-051, H-052, H-053, H-054, H-055, H-056`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **H-R035 — design-only (history)**; occurrences `H-164, H-165`; full trace: census/requirements-crosswalk.jsonl
- **O-R019 — design-only (requirement)**; occurrences `O-704, O-705, O-706, O-707, O-708, O-709, O-710, O-711, O-712, O-713, O-714, O-715, O-716, O-717, O-718, O-719, O-720, O-721, O-722, O-723, O-724, O-725, O-726, O-727, O-728, O-729, O-730, O-731, O-732, O-733, O-734, O-735`; full trace: census/requirements-crosswalk.jsonl
- **O-R020 — design-only (example)**; occurrences `O-736, O-737, O-738, O-739, O-740, O-741, O-742, O-743, O-744, O-745, O-746, O-747, O-748, O-749, O-750, O-751, O-752, O-753, O-754, O-755, O-756, O-757, O-758, O-759, O-760, O-761, O-762, O-763, O-764, O-765, O-766, O-767, O-768, O-769, O-770, O-771, O-772, O-773, O-774, O-775, O-776`; full trace: census/requirements-crosswalk.jsonl
- **O-R037 — design-only (example)**; occurrences `O-1143, O-1144, O-1145, O-1146, O-1147, O-1148, O-1149, O-1150, O-1151, O-1152, O-1153, O-1154, O-1155, O-1156, O-1157, O-1158, O-1159, O-1160, O-1161, O-1162, O-1163, O-1164, O-1165, O-1166, O-1167, O-1168, O-1169, O-1170, O-1171, O-1172, O-1173, O-1174, O-1175, O-1176, O-1177`; full trace: census/requirements-crosswalk.jsonl
- **O-R039 — design-only (candidate)**; occurrences `O-1242, O-1243, O-1244, O-1245, O-1246, O-1247, O-1248, O-1249, O-1250, O-1251, O-1252, O-1253, O-1254, O-1255, O-1256, O-1257, O-1258, O-1259, O-1260, O-1261, O-1262, O-1263, O-1264, O-1265, O-1266, O-1267, O-1268, O-1269, O-1270, O-1271, O-1272, O-1273, O-1274, O-1275, O-1276, O-1277, O-1278, O-1279, O-1280, O-1281, O-1282, O-1283, O-1284, O-1285, O-1286, O-1287, O-1288, O-1289, O-1290, O-1291, O-1292, O-1293, O-1294, O-1295, O-1296, O-1297, O-1298, O-1299, O-1300, O-1301, O-1302, O-1303, O-1304, O-1305, O-1306, O-1307, O-1308, O-1309, O-1310`; full trace: census/requirements-crosswalk.jsonl
- **O-R041 — design-only (history)**; occurrences `O-025, O-026, O-027, O-028, O-029, O-030, O-031, O-032, O-033, O-034, O-035, O-036, O-037, O-038, O-039, O-040, O-041, O-042, O-043, O-044, O-045, O-046, O-047, O-048, O-049, O-050, O-051`; full trace: census/requirements-crosswalk.jsonl
- **O-R043 — design-only (history)**; occurrences `O-058, O-059, O-060, O-061, O-062, O-063, O-064, O-065, O-066, O-067, O-068, O-069, O-070, O-071, O-072, O-073, O-074, O-075, O-076, O-077, O-078, O-079, O-080`; full trace: census/requirements-crosswalk.jsonl
- **O-R044 — design-only (candidate)**; occurrences `O-081, O-082, O-083, O-084, O-085, O-086, O-087, O-088, O-089, O-090, O-091, O-092, O-093, O-094, O-095, O-096, O-097, O-098, O-099, O-100, O-101, O-102, O-103, O-104, O-105, O-106`; full trace: census/requirements-crosswalk.jsonl
- **O-R045 — design-only (history)**; occurrences `O-107, O-108, O-109, O-110, O-111, O-112, O-113, O-114, O-115, O-116, O-117, O-118, O-119, O-120, O-121, O-122, O-123, O-124, O-125, O-126, O-127, O-128, O-129, O-130, O-131, O-132, O-133, O-134`; full trace: census/requirements-crosswalk.jsonl
- **P-R008 — design-only (candidate)**; occurrences `P-130, P-131, P-132, P-133, P-134, P-135, P-136, P-137, P-138, P-139, P-140, P-141, P-142, P-143, P-144, P-145, P-146, P-147, P-148, P-149, P-150, P-151, P-152, P-153, P-154, P-155, P-156, P-157, P-158, P-159, P-160, P-161, P-162, P-163, P-164, P-165, P-166, P-167, P-168, P-169, P-170, P-171, P-172, P-173, P-174, P-175, P-176, P-177, P-178, P-179, P-180, P-181, P-182, P-183, P-184, P-185, P-186, P-187, P-188, P-189, P-190, P-191, P-192, P-193, P-194, P-195, P-196, P-197, P-198, P-199, P-200, P-201, P-202, P-203, P-204, P-205, P-206, P-207, P-208, P-209, P-210, P-211, P-212, P-213, P-214, P-215, P-216, P-217, P-218, P-219, P-220, P-221, P-222, P-223, P-224, P-225`; full trace: census/requirements-crosswalk.jsonl
- **P-R026 — design-only (candidate)**; occurrences `P-1610, P-1611, P-1612, P-1613, P-1614, P-1615, P-1616, P-1617, P-1618, P-1619, P-1620, P-1621, P-1622, P-1623, P-1624, P-1625, P-1626, P-1627, P-1628, P-1629, P-1630, P-1631, P-1632, P-1633, P-1634, P-1635, P-1636, P-1637, P-1638, P-1639, P-1640, P-1641, P-1642, P-1643, P-1644, P-1645, P-1646, P-1647, P-1648, P-1649, P-1650, P-1651, P-1652, P-1653, P-1654, P-1655, P-1656, P-1657, P-1658, P-1659, P-1660`; full trace: census/requirements-crosswalk.jsonl
- **P-R027 — design-only (candidate)**; occurrences `P-1662, P-1663, P-1664, P-1665, P-1666, P-1667, P-1668, P-1669, P-1670, P-1671, P-1672, P-1673, P-1674, P-1675, P-1676, P-1677, P-1678, P-1679, P-1680, P-1681, P-1682, P-1683, P-1684, P-1685, P-1686, P-1687, P-1688, P-1689, P-1690, P-1691, P-1692, P-1693, P-1694, P-1695, P-1696, P-1697, P-1698, P-1699, P-1700, P-1701, P-1702, P-1703, P-1704, P-1705, P-1706, P-1707, P-1708, P-1709, P-1710, P-1711, P-1712, P-1713, P-1714, P-1715, P-1716, P-1717, P-1718, P-1719, P-1720, P-1721, P-1722, P-1723, P-1724, P-1725, P-1726, P-1727`; full trace: census/requirements-crosswalk.jsonl
- **S-R009 — design-only (candidate)**; occurrences `S-056, S-057, S-058, S-059`; full trace: census/requirements-crosswalk.jsonl
- **S-R012 — design-only (history)**; occurrences `S-005, S-006`; full trace: census/requirements-crosswalk.jsonl
Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.
## Owned files and boundaries

- `pkgs/fleet/README.md`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.




Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_operator_docs_match_registered_help_inventory`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `python3 -I -c 'from pathlib import Path; s=Path("pkgs/fleet/README.md").read_text(); required=("fleet dependency add","fleet work complete","private by default","no exactly-once guarantee","Queue Table Improvements"); missing=[x for x in required if x not in s]; assert not missing, missing' (must fail before documentation is added and pass afterward).`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

docs command example test or inventory consistency check passes; reviewed privacy redaction examples.

Negative coverage: docs say parent blocks child, GitHub author proves session, “optimistic locking” unsupported, exactly-once comments.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: docs-only no schema; update rollback/runbook with implementation.**
- Rollback: docs-only no schema; update rollback/runbook with implementation.**

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9208-typed-dependencies-notes-journal-field-authority.md`, slice `DOC-01` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### DOC-01 — Operator and contributor contract documentation

**Tier:** recommended. **Goal:** publish lifecycle relation/authority/error privacy runbook: add/resolve/remove blocker, view journal, correct note, repair quarantined edge, reconcile mirror conflict; document no-CAS caveat and #9178/#9179 compatibility. **Owns:** Track B docs and existing queue operator docs (paths confirmed by census; no invented doc location beyond this package). **Hard blockers:** DEP/JRN/AUTH contracts settled. **Failing oracle:** docs commands differ from registered help/golden fixtures or imply automatic mirror. **Done:** docs command example test or inventory consistency check passes; reviewed privacy redaction examples. **Negative:** docs say parent blocks child, GitHub author proves session, “optimistic locking” unsupported, exactly-once comments. **Migration/rollback:** docs-only no schema; update rollback/runbook with implementation.**
