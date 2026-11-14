# DSP-06 — Canonical dispatch reservation, direct launch and bind

<!-- pmo-9196:DSP-06 -->

**Issue title:** `design(pm-orchestration): Canonical dispatch reservation, direct launch and bind`  
**Design parent:** #9203 · **Scope:** #9203 direct dispatch/events · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R005, C-R009, C-R087, C-R088, C-R090, C-R091, C-R093, C-R107, C-R152, H-R001, H-R006, H-R012, H-R016, H-R017, H-R021, H-R023, H-R024, H-R031, H-R034, H-R035, H-R041, H-R043, H-R045, H-R049, H-R054, H-R056, H-R058, H-R060, H-R062, O-R011, O-R014, O-R023, O-R038, O-R040, O-R041, O-R047, O-R053, O-R063, O-R064, O-R065, O-R075, O-R076, O-R077, P-R009, S-R002, S-R004. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

reserve once against canonical task, apply #9178/#9179 gates, invoke existing `fleet dispatch launch` / service operation, and bind actual session. -

## Curated requirement trace

(Compacted for GitHub 65,536-character issue-body limit during task 15381 publisher recovery, 2026-11-13. Full per-occurrence citations, design refs, and outcome text for every ID below are preserved in census/requirements-crosswalk.jsonl in this design directory, merged via PR #9225.)

- **C-R005 — partially implemented (requirement)**; occurrences `C-148, C-149, C-150, C-151, C-152, C-153, C-154, C-155, C-156, C-157, C-158, C-159`; full trace: census/requirements-crosswalk.jsonl
- **C-R009 — partially implemented (requirement)**; occurrences `C-012`; full trace: census/requirements-crosswalk.jsonl
- **C-R087 — open implementation (requirement)**; occurrences `C-116`; full trace: census/requirements-crosswalk.jsonl
- **C-R088 — design-only (requirement)**; occurrences `C-117`; full trace: census/requirements-crosswalk.jsonl
- **C-R090 — partially implemented (requirement)**; occurrences `C-119`; full trace: census/requirements-crosswalk.jsonl
- **C-R091 — partially implemented (requirement)**; occurrences `C-120`; full trace: census/requirements-crosswalk.jsonl
- **C-R093 — partially implemented (requirement)**; occurrences `C-122`; full trace: census/requirements-crosswalk.jsonl
- **C-R107 — partially implemented (requirement)**; occurrences `C-147`; full trace: census/requirements-crosswalk.jsonl
- **C-R152 — partially implemented (requirement)**; occurrences `C-228`; full trace: census/requirements-crosswalk.jsonl
- **H-R001 — design-only (requirement)**; occurrences `H-029`; full trace: census/requirements-crosswalk.jsonl
- **H-R006 — design-only (requirement)**; occurrences `H-115, H-116, H-117`; full trace: census/requirements-crosswalk.jsonl
- **H-R012 — partially implemented (requirement)**; occurrences `H-069, H-070, H-071, H-072`; full trace: census/requirements-crosswalk.jsonl
- **H-R016 — design-only (requirement)**; occurrences `H-075`; full trace: census/requirements-crosswalk.jsonl
- **H-R017 — partially implemented (requirement)**; occurrences `H-139, H-140, H-141, H-142, H-143, H-144, H-145, H-146, H-147, H-148, H-149, H-150, H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R021 — unverified (requirement)**; occurrences `H-059, H-060, H-061, H-062, H-063, H-064, H-065`; full trace: census/requirements-crosswalk.jsonl
- **H-R023 — design-only (requirement)**; occurrences `H-083, H-084, H-085, H-086, H-087`; full trace: census/requirements-crosswalk.jsonl
- **H-R024 — design-only (requirement)**; occurrences `H-151, H-152`; full trace: census/requirements-crosswalk.jsonl
- **H-R031 — design-only (requirement)**; occurrences `H-089, H-090`; full trace: census/requirements-crosswalk.jsonl
- **H-R034 — design-only (history)**; occurrences `H-155, H-157, H-158, H-159, H-160, H-161, H-162, H-163`; full trace: census/requirements-crosswalk.jsonl
- **H-R035 — design-only (history)**; occurrences `H-164, H-165`; full trace: census/requirements-crosswalk.jsonl
- **H-R041 — design-only (requirement)**; occurrences `H-062`; full trace: census/requirements-crosswalk.jsonl
- **H-R043 — design-only (requirement)**; occurrences `H-064`; full trace: census/requirements-crosswalk.jsonl
- **H-R045 — design-only (requirement)**; occurrences `H-083`; full trace: census/requirements-crosswalk.jsonl
- **H-R049 — design-only (requirement)**; occurrences `H-086`; full trace: census/requirements-crosswalk.jsonl
- **H-R054 — design-only (requirement)**; occurrences `H-062`; full trace: census/requirements-crosswalk.jsonl
- **H-R056 — design-only (requirement)**; occurrences `H-064`; full trace: census/requirements-crosswalk.jsonl
- **H-R058 — design-only (requirement)**; occurrences `H-083`; full trace: census/requirements-crosswalk.jsonl
- **H-R060 — design-only (requirement)**; occurrences `H-084`; full trace: census/requirements-crosswalk.jsonl
- **H-R062 — design-only (requirement)**; occurrences `H-086`; full trace: census/requirements-crosswalk.jsonl
- **O-R011 — design-only (candidate)**; occurrences `O-450, O-451, O-452, O-453, O-454, O-455, O-456, O-457, O-458, O-459, O-460, O-461, O-462, O-463, O-464, O-465, O-466, O-467, O-468, O-469, O-470, O-471, O-472, O-473, O-474, O-475, O-476, O-477, O-478, O-479, O-480, O-481, O-482, O-483, O-484, O-485, O-486`; full trace: census/requirements-crosswalk.jsonl
- **O-R014 — design-only (candidate)**; occurrences `O-582, O-583, O-584, O-585, O-586, O-587, O-588, O-589, O-590, O-591, O-592, O-593, O-594, O-595, O-596, O-597, O-598, O-599, O-600, O-601, O-602, O-603, O-604, O-605, O-606, O-607, O-608, O-609, O-610, O-611, O-612, O-613, O-614, O-615, O-616, O-617, O-618, O-619, O-620, O-621, O-622, O-623, O-624, O-625, O-626, O-627, O-628, O-629, O-630, O-631, O-632, O-633, O-634, O-635`; full trace: census/requirements-crosswalk.jsonl
- **O-R023 — design-only (candidate)**; occurrences `O-846, O-847, O-848, O-849, O-850`; full trace: census/requirements-crosswalk.jsonl
- **O-R038 — design-only (candidate)**; occurrences `O-1178, O-1179, O-1180, O-1181, O-1182, O-1183, O-1184, O-1185, O-1186, O-1187, O-1188, O-1189, O-1190, O-1191, O-1192, O-1193, O-1194, O-1195, O-1196, O-1197, O-1198, O-1199, O-1200, O-1201, O-1202, O-1203, O-1204, O-1205, O-1206, O-1207, O-1208, O-1209, O-1210, O-1211, O-1212, O-1213, O-1214, O-1215, O-1216, O-1217, O-1218, O-1219, O-1220, O-1221, O-1222, O-1223, O-1224, O-1225, O-1226, O-1227, O-1228, O-1229, O-1230, O-1231, O-1232, O-1233, O-1234, O-1235, O-1236, O-1237, O-1238, O-1239, O-1240, O-1241`; full trace: census/requirements-crosswalk.jsonl
- **O-R040 — design-only (history)**; occurrences `O-016, O-017, O-018, O-019, O-020, O-021, O-022, O-023, O-024`; full trace: census/requirements-crosswalk.jsonl
- **O-R041 — design-only (history)**; occurrences `O-025, O-026, O-027, O-028, O-029, O-030, O-031, O-032, O-033, O-034, O-035, O-036, O-037, O-038, O-039, O-040, O-041, O-042, O-043, O-044, O-045, O-046, O-047, O-048, O-049, O-050, O-051`; full trace: census/requirements-crosswalk.jsonl
- **O-R047 — design-only (history)**; occurrences `O-142, O-143, O-144, O-145, O-146, O-147, O-148, O-149, O-150, O-151, O-152, O-153, O-154, O-155, O-156, O-157, O-158`; full trace: census/requirements-crosswalk.jsonl
- **O-R053 — design-only (history)**; occurrences `O-006, O-011`; full trace: census/requirements-crosswalk.jsonl
- **O-R063 — design-only (candidate)**; occurrences `O-598, O-599, O-600, O-601, O-602, O-603, O-604, O-605, O-606, O-607, O-608, O-609, O-610, O-611, O-612, O-613, O-614, O-615, O-616, O-617, O-618, O-619`; full trace: census/requirements-crosswalk.jsonl
- **O-R064 — design-only (candidate)**; occurrences `O-620, O-621, O-622, O-623, O-624, O-625, O-626, O-627, O-628`; full trace: census/requirements-crosswalk.jsonl
- **O-R065 — design-only (candidate)**; occurrences `O-629, O-630, O-631, O-632, O-633, O-634, O-635`; full trace: census/requirements-crosswalk.jsonl
- **O-R075 — design-only (candidate)**; occurrences `O-598, O-599, O-600, O-601, O-602, O-603, O-604, O-605, O-606, O-607, O-608, O-609, O-610, O-611, O-612, O-613, O-614, O-615, O-616, O-617, O-618, O-619`; full trace: census/requirements-crosswalk.jsonl
- **O-R076 — design-only (candidate)**; occurrences `O-620, O-621, O-622, O-623, O-624, O-625, O-626, O-627, O-628`; full trace: census/requirements-crosswalk.jsonl
- **O-R077 — design-only (candidate)**; occurrences `O-629, O-630, O-631, O-632, O-633, O-634, O-635`; full trace: census/requirements-crosswalk.jsonl
- **P-R009 — design-only (candidate)**; occurrences `P-227, P-228, P-229, P-230, P-231, P-232, P-233, P-234, P-235, P-236, P-237, P-238, P-239, P-240, P-241, P-242, P-243, P-244, P-245, P-246, P-247, P-248, P-249, P-250, P-251, P-252, P-253, P-254, P-255, P-256, P-257, P-258, P-259, P-260, P-261, P-262, P-263, P-264, P-265, P-266, P-267, P-268, P-269, P-270, P-271, P-272, P-273, P-274, P-275, P-276, P-277, P-278, P-279, P-280, P-281, P-282, P-283, P-284, P-285, P-286, P-287, P-288, P-289, P-290, P-291, P-292, P-293, P-294, P-295, P-296, P-297, P-298, P-299, P-300, P-301, P-302, P-303, P-304, P-305, P-306, P-307, P-308, P-309, P-310, P-311, P-312, P-313, P-314, P-315, P-316, P-317, P-318, P-319, P-320, P-321, P-322, P-323, P-324, P-325, P-326, P-327, P-328, P-329, P-330, P-331, P-332, P-333, P-334, P-335, P-336, P-337, P-338, P-339, P-340, P-341, P-342, P-343, P-344, P-345, P-346, P-347, P-348, P-349, P-350, P-351, P-352, P-353, P-354, P-355, P-356, P-357, P-358, P-359, P-360, P-361, P-362, P-363, P-364, P-365, P-366, P-367, P-368, P-369, P-370, P-371, P-372, P-373, P-374, P-375, P-376, P-377, P-378, P-379, P-380, P-381, P-382, P-383, P-384, P-385, P-386, P-387, P-388, P-389, P-390, P-391, P-392, P-393, P-394, P-395, P-396, P-397, P-398, P-399, P-400, P-401, P-402, P-403, P-404, P-405, P-406, P-407, P-408, P-409`; full trace: census/requirements-crosswalk.jsonl
- **S-R002 — design-only (candidate)**; occurrences `S-014, S-015, S-016, S-017, S-018`; full trace: census/requirements-crosswalk.jsonl
- **S-R004 — partially implemented (candidate)**; occurrences `S-023, S-024, S-025, S-026, S-027, S-028`; full trace: census/requirements-crosswalk.jsonl
  Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`
- `pkgs/fleet/crates/fleet-dispatch/src/lib.rs`

Explicit read-only consumers (not writable ownership): pkgs/fleet/crates/fleet-queue/src/work/contracts.rs, pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: REL-03, #8581
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: #9028 (runtime gate: reconcile the current PM PostgREST producer bypass before canonical reservation/cutover; offline contracts and fixtures may proceed), #9178 (upstream claim/manual/takeover/resume semantics must be verified before mutation/cutover; offline compatibility models may proceed), #9179 (upstream park/resume safety semantics must be verified before mutation/cutover; offline fixtures may proceed), #9241 (bounded QTI search receipt is complete but original pointer remains unlocated; compatibility review may proceed from the receipt without waiting for issue closure).
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: none.

Source-local candidate aliases normalized for this packet: DSP-03 → REL-03.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`
- **Oracle:** generated crash-cut MBT proves concurrent operation has one task/session; duplicate event reuses IDs; hard gate/cap/lease rejects; timeout after remote create binds existing session after readback without a second launch. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`; PM service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`. Gated on #9028 and #9178/#9179; no parallel queue/scheduler. -
- **Done command:** `generated crash-cut MBT proves concurrent operation has one task/session; duplicate event reuses IDs; hard gate/cap/lease rejects; timeout after remote create binds existing session after readback without a second launch. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`; PM service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`. Gated on #9028 and #9178/#9179; no parallel queue/scheduler. -`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

generated crash-cut MBT proves concurrent operation has one task/session; duplicate event reuses IDs; hard gate/cap/lease rejects; timeout after remote create binds existing session after readback without a second launch. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`; PM service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`. Gated on #9028 and #9178/#9179; no parallel queue/scheduler. -

Negative coverage: stale input, duplicate/replay, ambiguous resource, unauthorized actor, partial remote effect, unknown version, and no false verified/completed result where applicable.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9203-direct-dispatch-slash-commands-event-routing.md`, slice `DSP-06` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `agent.dispatch, agent.slash.start, work.handoff`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### DSP-06 — Canonical dispatch reservation, direct launch and bind

- **Requirements:** C-2/C-8/C-13, H-SURVIVE-A/H-SCENARIOS, O §§11/14/17/20. **Goal:** reserve once against canonical task, apply #9178/#9179 gates, invoke existing `fleet dispatch launch` / service operation, and bind actual session.
- **Owned files:** `pkgs/fleet/crates/fleet-dispatch` adapter plus proposed `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`/`lifecycle.rs` contract integration, dispatcher tests and operator runbook. Do not patch Supabase and Turso paths independently; route integration of any CLI verb is a separate shared `main.rs`/registry glue slice.
- **Failing test to start:** `pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate` in the queue/dispatch contract test targets.
- **Done/oracle:** generated crash-cut MBT proves concurrent operation has one task/session; duplicate event reuses IDs; hard gate/cap/lease rejects; timeout after remote create binds existing session after readback without a second launch. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`; PM service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`. Gated on #9028 and #9178/#9179; no parallel queue/scheduler.
- **Dependencies:** DSP-03/04; #9028, #8581, #9178/#9179; #9241 (local alias `QTI-RECON-01`) gates schema-coupled implementation only.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `DSP-05` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/operations.rs`
