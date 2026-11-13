# Queue Table Improvements provenance — research gap

Created 2026-11-13 for the PM Stack & Fleet orchestration design mill (#9196), under the requirement crosswalk/current-state census (#9198).

## Why this is tracked

The recovered original handoff names “Queue Table Improvements” as existing work that must be reconciled before shared-schema changes. The design packet preserves that as H-QTI, with H-RECONCILE and charter anchors C-1/C-2. The census explicitly requires finding the original by name and not assuming equivalence. This gap is now a separate, bounded research issue: [#9241](https://github.com/developing-today/code/issues/9241), `research(pm-orchestration): recover Queue Table Improvements provenance and schema crosswalk`.

The issue includes the durable marker `<!-- pmo-9196:QTI-PROVENANCE -->`, is a native sub-issue of #9198, and is intentionally research-only. The original issue title/name search did not find the source; that is not proof of absence across private or external systems. Do not block owner-independent recovery while waiting for Drew.

## Existing evidence to extend, not duplicate

- Pinned source tree: `a038f084dbeb525494d7bb928ad91f597db2ac51`.
- `census/track-a.md` §§1 and “Ready-to-file research slice” records repository text/history, GitHub issue/PR, staged source, and candidate crosswalk results.
- `census/anchors.json` `queueTableImprovementsDiscovery` records the exact search surfaces/terms and `unlocated-unverified` disposition.
- The exact-name searches did not find a primary QTI pointer. They found candidate, not equivalent, work: #3424/#8180 (durable task model, six pieces), #8579/#8602 (schema audit), and related issues. A census worker is separately investigating #3424 and #8579; this research issue must not duplicate that substantive candidate research.
- The original source pointer and source anchors are in `sources/04_recovered_original_combined_design.md`, `sources/01_handoff_pm_stack_fleet_orchestration.md`, and `sources/02_charter_v1_1.md`. H/C occurrence IDs are in `inventory/statements.md` and should be retrieved rather than guessed.

## Required outcome

Produce a dated, reproducible search receipt and a field-owner/schema crosswalk before any shared-schema migration. Either locate the primary source and give its durable pointer plus alias rationale, or document “not located within enumerated accessible surfaces,” access limitations, and concrete recovery steps. Compare original shared fields/contracts with current owners, implementations, and issue scopes at the pinned commit. Mark unknowns as unknown; do not infer equivalence from similar shapes. Use archival issue bodies/history/docs and current owners as evidence; no owner oracle is required.

Acceptance oracle: primary-source identity evidence for any source mapping, plus pinned-code/issue evidence for every current-state mapping. No production, schema, queue, Project, migration, or deployment changes are in scope.
