# CEN-06 — Experiment result interpretation/report writer (separate child)

<!-- pmo-9196:CEN-06 -->

**Issue title:** `design(pm-orchestration): Experiment result interpretation/report writer (separate child)`  
**Design parent:** #9198 · **Scope:** #9198 crosswalk/census · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R161, C-R162, H-R048, O-R039. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

translate immutable CEN-05 output into a reviewable evidence report, separating measured observations from assumptions, thresholds and recommendation; do not let experiment code author its own acceptance narrative. -

## Curated requirement trace

- **H-R048 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: evidence-reporting method separate from experiment/model results; occurrences `H-085`; citations: 01_handoff_pm_stack_fleet_orchestration.md#H-085:65; design: components/9198-requirement-crosswalk-current-state-census.md#3-v2--contract-and-compatibility-crosswalk; outcome: Report designed, implemented, merged, deployed, live-verified, blocked, and pending states separately.; residual: Design contract only; implementation, adapter conformance and any runtime evidence are separate.; missing evidence: implementation tests for exact behavior, live/deployment acceptance unless separately evidenced
- **C-R161 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: evidence-reporting method separate from experiment/model results; occurrences `C-239`; citations: 02_charter_v1_1.md#C-239:188; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Execution evidence and reporting; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **C-R162 — design-only (requirement)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: evidence-reporting method separate from experiment/model results; occurrences `C-240`; citations: 02_charter_v1_1.md#C-240:190; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: The design agent should provide: current architecture map, gaps, decisions/ADRs, integrated schema/API/CLI contracts, safe migration plan, phased implementation issues with acceptance tests, risks and go/no-go gates, actual created documents/issue links, dispatch receipts, and verified status; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **O-R039 — design-only (candidate)**; basis: parent-approved semantic support mapping; not a direct implementation_slice_keys match; support rationale: evidence-reporting method separate from experiment/model results; occurrences `O-1242, O-1243, O-1244, O-1245, O-1246, O-1247, O-1248, O-1249, O-1250, O-1251, O-1252, O-1253, O-1254, O-1255, O-1256, O-1257, O-1258, O-1259, O-1260, O-1261, O-1262, O-1263, O-1264, O-1265, O-1266, O-1267, O-1268, O-1269, O-1270, O-1271, O-1272, O-1273, O-1274, O-1275, O-1276, O-1277, O-1278, O-1279, O-1280, O-1281, O-1282, O-1283, O-1284, O-1285, O-1286, O-1287, O-1288, O-1289, O-1290, O-1291, O-1292, O-1293, O-1294, O-1295, O-1296, O-1297, O-1298, O-1299, O-1300, O-1301, O-1302, O-1303, O-1304, O-1305, O-1306, O-1307, O-1308, O-1309, O-1310`; citations: 04_recovered_original_combined_design.md#O-1242:13, 04_recovered_original_combined_design.md#O-1243:13, 04_recovered_original_combined_design.md#O-1244:13, 04_recovered_original_combined_design.md#O-1245:13, 04_recovered_original_combined_design.md#O-1246:13, 04_recovered_original_combined_design.md#O-1247:13, 04_recovered_original_combined_design.md#O-1248:13, 04_recovered_original_combined_design.md#O-1249:13, 04_recovered_original_combined_design.md#O-1250:13, 04_recovered_original_combined_design.md#O-1251:13, 04_recovered_original_combined_design.md#O-1252:13, 04_recovered_original_combined_design.md#O-1253:13, 04_recovered_original_combined_design.md#O-1254:13, 04_recovered_original_combined_design.md#O-1255:13, 04_recovered_original_combined_design.md#O-1256:13, 04_recovered_original_combined_design.md#O-1257:13, 04_recovered_original_combined_design.md#O-1258:13, 04_recovered_original_combined_design.md#O-1259:13, 04_recovered_original_combined_design.md#O-1260:13, 04_recovered_original_combined_design.md#O-1261:13, 04_recovered_original_combined_design.md#O-1262:13, 04_recovered_original_combined_design.md#O-1263:13, 04_recovered_original_combined_design.md#O-1264:13, 04_recovered_original_combined_design.md#O-1265:13, 04_recovered_original_combined_design.md#O-1266:13, 04_recovered_original_combined_design.md#O-1267:13, 04_recovered_original_combined_design.md#O-1268:13, 04_recovered_original_combined_design.md#O-1269:13, 04_recovered_original_combined_design.md#O-1270:13, 04_recovered_original_combined_design.md#O-1271:13, 04_recovered_original_combined_design.md#O-1272:13, 04_recovered_original_combined_design.md#O-1273:13, 04_recovered_original_combined_design.md#O-1274:13, 04_recovered_original_combined_design.md#O-1275:13, 04_recovered_original_combined_design.md#O-1276:13, 04_recovered_original_combined_design.md#O-1277:13, 04_recovered_original_combined_design.md#O-1278:13, 04_recovered_original_combined_design.md#O-1279:13, 04_recovered_original_combined_design.md#O-1280:13, 04_recovered_original_combined_design.md#O-1281:13, 04_recovered_original_combined_design.md#O-1282:13, 04_recovered_original_combined_design.md#O-1283:13, 04_recovered_original_combined_design.md#O-1284:13, 04_recovered_original_combined_design.md#O-1285:13, 04_recovered_original_combined_design.md#O-1286:13, 04_recovered_original_combined_design.md#O-1287:13, 04_recovered_original_combined_design.md#O-1288:13, 04_recovered_original_combined_design.md#O-1289:13, 04_recovered_original_combined_design.md#O-1290:13, 04_recovered_original_combined_design.md#O-1291:13, 04_recovered_original_combined_design.md#O-1292:13, 04_recovered_original_combined_design.md#O-1293:13, 04_recovered_original_combined_design.md#O-1294:13, 04_recovered_original_combined_design.md#O-1295:13, 04_recovered_original_combined_design.md#O-1296:13, 04_recovered_original_combined_design.md#O-1297:13, 04_recovered_original_combined_design.md#O-1298:13, 04_recovered_original_combined_design.md#O-1299:13, 04_recovered_original_combined_design.md#O-1300:13, 04_recovered_original_combined_design.md#O-1301:13, 04_recovered_original_combined_design.md#O-1302:13, 04_recovered_original_combined_design.md#O-1303:13, 04_recovered_original_combined_design.md#O-1304:13, 04_recovered_original_combined_design.md#O-1305:13, 04_recovered_original_combined_design.md#O-1306:13, 04_recovered_original_combined_design.md#O-1307:13, 04_recovered_original_combined_design.md#O-1308:13, 04_recovered_original_combined_design.md#O-1309:13, 04_recovered_original_combined_design.md#O-1310:13; design: components/9198-requirement-crosswalk-current-state-census.md#2-v1-census--actual-state-and-bounded-facts; outcome: Audit current mechanisms and constraints; simulate priority/aging/clustering/hysteresis/dependencies/selection/races; verify end-to-end handoff; deliver architecture, ownership, contracts, tests, risks, and honest dispatch evidence.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/report.py`
- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/test_report.py`
- `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/priority-exploration-experiment.md`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `test_report_marks_missing_measurement_and_does_not_claim_production_benefit`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `report reproduces input/result digest, separates external source facts, measured model outputs, inference and recommendation; a reviewer can regenerate it from the immutable artifact. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

report reproduces input/result digest, separates external source facts, measured model outputs, inference and recommendation; a reviewer can regenerate it from the immutable artifact. -

Negative coverage: do not choose thresholds post hoc or mark exploration enabled from simulated benefit alone. Remove/replace report only with a new versioned artifact and preserve prior run provenance.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9198-requirement-crosswalk-current-state-census.md`, slice `CEN-06` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): H-R048, C-R161, C-R162, O-R039: evidence-reporting method separate from experiment/model results [approval parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval].
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### CEN-06 — Experiment result interpretation/report writer (separate child)

- **Tier:** recommended. **Goal:** translate immutable CEN-05 output into a reviewable evidence report, separating measured observations from assumptions, thresholds and recommendation; do not let experiment code author its own acceptance narrative.
- **Owns:** separate proposed `validation/report.py`, `validation/test_report.py`, and versioned report output `research/priority-exploration-experiment.md`. It reads an immutable result artifact; it does not edit `validation/experiment.py` or write production settings.
- **Hard dependency:** CEN-05 output schema and reproducible result artifact.
- **First failing oracle:** `test_report_marks_missing_measurement_and_does_not_claim_production_benefit` in `validation/test_report.py` rejects missing run ID, seed, sample count, oracle, uncertainty, fixture provenance or unmeasured threshold; verifies synthetic simulation is not called production result.
- **Done check:** report reproduces input/result digest, separates external source facts, measured model outputs, inference and recommendation; a reviewer can regenerate it from the immutable artifact.
- **Negative / rollback:** do not choose thresholds post hoc or mark exploration enabled from simulated benefit alone. Remove/replace report only with a new versioned artifact and preserve prior run provenance.
