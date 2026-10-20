---
name: design-orchestrate
description: Coordinate parallel subagents across research issues, reconcile trade-offs, manage PR merges, and produce design records. Use during Stage 6 design synthesis.
---

# Design Orchestrate (Stage 6)

Coordinate multi-agent research execution, resolve design conflicts, log architectural decisions, and draft formal design packages. Detailed rules live in [/planning/workflow/collaboration.md](/planning/workflow/collaboration.md) and [/planning/workflow/decisions.md](/planning/workflow/decisions.md).

## Procedure

1. **Ready Frontier & Dispatch**:
   Inspect unblocked child issues. Dispatch worker subagents into dedicated worktrees. Cap concurrency at `max_parallel_writers`.

2. **Monitor Worker PRs**:
   Track worker PRs. Verify that `git ls-remote` matches the PR head and the worker's reported last pushed SHA before executing merge commits (`--no-ff`).

3. **Resolve Disagreements**:
   If workers propose conflicting designs:
   - Do not halt the run.
   - Launch read-only reviewer and critic subagents ([/planning/workflow/templates/prompts/critic.md](/planning/workflow/templates/prompts/critic.md)) with merged context.
   - Record the chosen direction as an `agent-preliminary` decision in `decisions/DXXX-<slug>.md`.
   - Add genuine trade-offs to the minimal human-review list.

4. **Challenge Pass**:
   Launch an investigator subagent ([/planning/workflow/templates/prompts/investigator.md](/planning/workflow/templates/prompts/investigator.md)) to challenge whether decisions were framed too narrowly.

5. **Consolidate Design Record**:
   Write the formal design record in `doc/<UTC>_design_<slug>/` following [/planning/templates/design.md](/planning/templates/design.md).

## Turn Report

End every turn with:

- Done: <orchestration round summary, merges, decisions logged>
- Verified: <git log merge evidence, check command output>
- Remaining: <handoff to feature-plan>
- Questions (non-blocking): <question -> default>
- Blocked on user: <consequential human review items only>
