# VAL-01 — Migration, rollback, graph repair, and privacy validation

<!-- pmo-9196:VAL-01 -->

**Issue title:** `design(pm-orchestration): Migration, rollback, graph repair, and privacy validation`  
**Design parent:** #9208 · **Scope:** #9208 relations/journal/authority · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs (ALLOCATED):** C-R128, P-R008. **Slice status:** curated implementation/research slice. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

validate additive graph/journal migration/backfill, rollback, cycle quarantine, event preservation, privacy and Project ownership.

## Curated requirement trace

- **C-R128 — design-only (requirement)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `C-196`; citations: 02_charter_v1_1.md#C-196:151; design: components/9208-typed-dependencies-notes-journal-field-authority.md#2-typed-relationship-contract; outcome: Derive priority labels from canonical score, and accept authorized label edits through a conflict-aware normalization policy; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited
- **P-R008 — design-only (candidate)**; basis: exact frozen implementation_slice_keys inverse join; support rationale: none; occurrences `P-130, P-131, P-132, P-133, P-134, P-135, P-136, P-137, P-138, P-139, P-140, P-141, P-142, P-143, P-144, P-145, P-146, P-147, P-148, P-149, P-150, P-151, P-152, P-153, P-154, P-155, P-156, P-157, P-158, P-159, P-160, P-161, P-162, P-163, P-164, P-165, P-166, P-167, P-168, P-169, P-170, P-171, P-172, P-173, P-174, P-175, P-176, P-177, P-178, P-179, P-180, P-181, P-182, P-183, P-184, P-185, P-186, P-187, P-188, P-189, P-190, P-191, P-192, P-193, P-194, P-195, P-196, P-197, P-198, P-199, P-200, P-201, P-202, P-203, P-204, P-205, P-206, P-207, P-208, P-209, P-210, P-211, P-212, P-213, P-214, P-215, P-216, P-217, P-218, P-219, P-220, P-221, P-222, P-223, P-224, P-225`; citations: 03_full_orchestration_proposal.md#P-130:197, 03_full_orchestration_proposal.md#P-131:197, 03_full_orchestration_proposal.md#P-132:199, 03_full_orchestration_proposal.md#P-133:201, 03_full_orchestration_proposal.md#P-134:203, 03_full_orchestration_proposal.md#P-135:205, 03_full_orchestration_proposal.md#P-136:207, 03_full_orchestration_proposal.md#P-137:208, 03_full_orchestration_proposal.md#P-138:209, 03_full_orchestration_proposal.md#P-139:210, 03_full_orchestration_proposal.md#P-140:211, 03_full_orchestration_proposal.md#P-141:212, 03_full_orchestration_proposal.md#P-142:213, 03_full_orchestration_proposal.md#P-143:214, 03_full_orchestration_proposal.md#P-144:216, 03_full_orchestration_proposal.md#P-145:218, 03_full_orchestration_proposal.md#P-146:220, 03_full_orchestration_proposal.md#P-147:221, 03_full_orchestration_proposal.md#P-148:222, 03_full_orchestration_proposal.md#P-149:223, 03_full_orchestration_proposal.md#P-150:224, 03_full_orchestration_proposal.md#P-151:225, 03_full_orchestration_proposal.md#P-152:226, 03_full_orchestration_proposal.md#P-153:227, 03_full_orchestration_proposal.md#P-154:229, 03_full_orchestration_proposal.md#P-155:231, 03_full_orchestration_proposal.md#P-156:233, 03_full_orchestration_proposal.md#P-157:234, 03_full_orchestration_proposal.md#P-158:235, 03_full_orchestration_proposal.md#P-159:236, 03_full_orchestration_proposal.md#P-160:237, 03_full_orchestration_proposal.md#P-161:238, 03_full_orchestration_proposal.md#P-162:239, 03_full_orchestration_proposal.md#P-163:240, 03_full_orchestration_proposal.md#P-164:241, 03_full_orchestration_proposal.md#P-165:242, 03_full_orchestration_proposal.md#P-166:243, 03_full_orchestration_proposal.md#P-167:244, 03_full_orchestration_proposal.md#P-168:245, 03_full_orchestration_proposal.md#P-169:247, 03_full_orchestration_proposal.md#P-170:249, 03_full_orchestration_proposal.md#P-171:251, 03_full_orchestration_proposal.md#P-172:253, 03_full_orchestration_proposal.md#P-173:255, 03_full_orchestration_proposal.md#P-174:256, 03_full_orchestration_proposal.md#P-175:257, 03_full_orchestration_proposal.md#P-176:258, 03_full_orchestration_proposal.md#P-177:259, 03_full_orchestration_proposal.md#P-178:260, 03_full_orchestration_proposal.md#P-179:261, 03_full_orchestration_proposal.md#P-180:263, 03_full_orchestration_proposal.md#P-181:265, 03_full_orchestration_proposal.md#P-182:267, 03_full_orchestration_proposal.md#P-183:269, 03_full_orchestration_proposal.md#P-184:270, 03_full_orchestration_proposal.md#P-185:271, 03_full_orchestration_proposal.md#P-186:272, 03_full_orchestration_proposal.md#P-187:273, 03_full_orchestration_proposal.md#P-188:274, 03_full_orchestration_proposal.md#P-189:275, 03_full_orchestration_proposal.md#P-190:277, 03_full_orchestration_proposal.md#P-191:279, 03_full_orchestration_proposal.md#P-192:281, 03_full_orchestration_proposal.md#P-193:283, 03_full_orchestration_proposal.md#P-194:284, 03_full_orchestration_proposal.md#P-195:285, 03_full_orchestration_proposal.md#P-196:286, 03_full_orchestration_proposal.md#P-197:287, 03_full_orchestration_proposal.md#P-198:288, 03_full_orchestration_proposal.md#P-199:289, 03_full_orchestration_proposal.md#P-200:290, 03_full_orchestration_proposal.md#P-201:292, 03_full_orchestration_proposal.md#P-202:294, 03_full_orchestration_proposal.md#P-203:296, 03_full_orchestration_proposal.md#P-204:298, 03_full_orchestration_proposal.md#P-205:300, 03_full_orchestration_proposal.md#P-206:302, 03_full_orchestration_proposal.md#P-207:303, 03_full_orchestration_proposal.md#P-208:304, 03_full_orchestration_proposal.md#P-209:305, 03_full_orchestration_proposal.md#P-210:306, 03_full_orchestration_proposal.md#P-211:307, 03_full_orchestration_proposal.md#P-212:308, 03_full_orchestration_proposal.md#P-213:309, 03_full_orchestration_proposal.md#P-214:310, 03_full_orchestration_proposal.md#P-215:312, 03_full_orchestration_proposal.md#P-216:314, 03_full_orchestration_proposal.md#P-217:316, 03_full_orchestration_proposal.md#P-218:318, 03_full_orchestration_proposal.md#P-219:319, 03_full_orchestration_proposal.md#P-220:320, 03_full_orchestration_proposal.md#P-221:321, 03_full_orchestration_proposal.md#P-222:322, 03_full_orchestration_proposal.md#P-223:323, 03_full_orchestration_proposal.md#P-224:324, 03_full_orchestration_proposal.md#P-225:326; design: components/9204-queue-selection-bounded-exploration.md#eligibility-and-deterministic-normal-selection, components/9207-context-hydration-incremental-activity.md#22-versioned-manifest-normative-shape-candidate, components/9208-typed-dependencies-notes-journal-field-authority.md#2-typed-relationship-contract, components/9209-leases-fencing-outbox-direct-completion.md#4-outbox-operation-model; outcome: Define logical internal responsibilities for Queue Broker, Hydrator, GitHub Adapter, Activity Reconciler, Dependency Resolver, append-oriented Journal, Mutation Coordinator, and durable Outbox; reuse existing infrastructure rather than require separate services.; residual: See status reason and missing_evidence; implementation slices are local keys, not issue IDs.; missing evidence: exact implementation/parser tests for the full behavior, deployment/live acceptance unless separately cited

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

- `pkgs/fleet/crates/fleet-queue/src/work/relations.rs`

Explicit read-only consumers (not writable ownership): none declared.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: CEN-04, #9073, #9074
File-collision sequencing edges added by stable topological serialization: none.
Existing issue references retained as non-blocking runtime/evidence gates: none.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: #9198→CEN-04.

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback`
- **Oracle:** The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.
- **Done command:** `confirm test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback`; actual DB migration target must separately pass the owner-documented upgrade/rollback command before rollout (not guessed here).`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

source tests, DB migration, staging, deployed canary and live remote verification separately.

Negative coverage: ambiguous ref, cycle, missing principal, telemetry mislabeled journal, private leak, rollback after remote effect.

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.
- Rollback: Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `components/9208-typed-dependencies-notes-journal-field-authority.md`, slice `VAL-01` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: see the pinned census and component evidence links above.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): none.
- Existing external requirement refs not assigned a new curated ID: none.
- Command allocation: `none; this is not a command-owning slice`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

### VAL-01 — Migration, rollback, graph repair, and privacy validation

**Tier:** crucial release validation. **Goal:** validate additive graph/journal migration/backfill, rollback, cycle quarantine, event preservation, privacy and Project ownership. **Owns:** migration logic tests in `src/work/relations.rs`/`journal.rs`; actual versioned migration file remains at existing path established by #9198. **Hard blockers:** Queue Table Improvements, #9198 schema owner, DEP/JRN/AUTH contracts; #9073/#9074 only if metadata schema touched. **First failing oracle:** `pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback` tests ambiguous links/events, upgrade, write, disable/rollback and prior-data read. **Done command:** confirm test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback`; actual DB migration target must separately pass the owner-documented upgrade/rollback command before rollout (not guessed here). **Negative:** ambiguous ref, cycle, missing principal, telemetry mislabeled journal, private leak, rollback after remote effect. **Acceptance:** source tests, DB migration, staging, deployed canary and live remote verification separately.

## Shared-file sequencing

Added hard sequencing edges (the predecessor owns the same exact writable file):

- blocked by `PRI-04` — `shared-file:pkgs/fleet/crates/fleet-queue/src/work/relations.rs`
