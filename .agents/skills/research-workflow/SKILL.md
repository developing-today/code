---
name: research-workflow
description: Discover the current stage of a research collection, inspect readiness, and dispatch the next lifecycle stage or worker. Use when resuming work on a request, finding what is unblocked, or checking package status.
---

# Research Workflow Router

This skill inspects collection state and routes work to the appropriate lifecycle skill. Detailed contracts live in [/planning/workflow/README.md](/planning/workflow/README.md).

## Procedure

1. **Locate Collection & Read Status**:

   ```sh
   python3 scripts/research_workflow.py status --package <package-dir>
   ```

   Check the current stage (`intake`, `prepare`, `research`, `reconcile`, `handoff`, `design`, `plan`, `implement`).

2. **Compute Ready Frontier**:

   ```sh
   python3 scripts/research_workflow.py ready --package <package-dir>
   ```

   Inspect unblocked concepts and active concurrency constraints.

3. **Dispatch to Stage Skill**:
   - Stage `intake` -> call `request-intake` if not yet prepared.
   - Stage `prepare` -> call `research-prepare` per ready concept.
   - Stage `research` -> call `concept-research` per ready concept.
   - Stage `reconcile` -> call `notes-reconcile` per concept.
   - Stage `handoff` -> call `issue-handoff` to create parent/fan-out issues.
   - Stage `design` -> call `design-orchestrate` to coordinate research PRs and decisions.
   - Stage `plan` -> call `feature-plan` to create granular implementation plans.
   - Stage `implement` -> call `implementation-orchestrate` across ready frontiers.

4. **Report Progress**:
   Print the ready frontier, active leases, and next action.

## Turn Report

End every turn with:

- Done: <summary of inspection or dispatch>
- Verified: <command -> output>
- Remaining: <next stage or pending work>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
