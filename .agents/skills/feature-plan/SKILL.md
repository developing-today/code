---
name: feature-plan
description: Turn approved designs into granular end-to-end implementation plans with numbered steps, test contracts, and worktree partitions. Use during Stage 7 feature planning.
---

# Feature Plan (Stage 7)

Translate an approved design or consolidated research into a granular, execution-ready implementation plan. Full specification lives in [/planning/templates/plan.md](/planning/templates/plan.md) and [/planning/workflow/ordering.md](/planning/workflow/ordering.md).

## Procedure

1. **Review Design & Boundaries**:
   Read the design record, decision records, and acceptance criteria. Fence the scope: clearly state what the feature will _not_ do.

2. **Define Test Seams First**:
   Specify automated test commands and observable outputs that will fail before the change and pass after it.

3. **Phase & Step Breakdown**:
   Break implementation into phases (`1.0`, `2.0`, ...) and numbered steps (`1.1.1`, `1.1.2`, ...). Mark parallel-capable steps with `[P]`.

4. **Map Worktree Partitions & Dependency Edges**:
   Assign file ownership partitions to prevent concurrent builders from modifying the same files. Declare explicit `requires` and `shares-files-with` relations.

5. **Create Implementation Issues**:
   Publish granular implementation issues and generate the implementation orchestration issue. Save the plan to `planning/plans/<slug>.md`.

## Turn Report

End every turn with:

- Done: <plan summary, phase count, issue links>
- Verified: <plan links and consistency check output>
- Remaining: <handoff to implementation-orchestrate>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
