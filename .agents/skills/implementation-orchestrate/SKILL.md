---
name: implementation-orchestrate
description: Orchestrate parallel builders across git worktrees to execute implementation plans, verify tests, and merge PRs. Use during Stage 8 feature implementation.
---

# Implementation Orchestrate (Stage 8)

Coordinate parallel worker subagents across git worktrees to build planned feature phases, driving test-driven development, code review, and merge-commit landing. Detailed rules live in [/planning/workflow/collaboration.md](../../../planning/workflow/collaboration.md).

## Procedure

1. **Frontier Dispatch**:
   Query ready implementation issues where all `requires` blockers are merged. Boot one builder subagent per issue in an isolated worktree.

2. **Builder Protocol**:
   Ensure builders follow the boot protocol: source git env, create and push an empty branch, write red tests first, implement, format, and open a PR.

3. **Verify & Merge**:
   When a builder reports done:
   - Check that `git ls-remote` matches the PR head and reported SHA.
   - Verify that automated test criteria pass.
   - Merge the PR using a merge commit (`gh pr merge --merge --auto`).
   - Clean up the worker worktree (`git worktree remove`).

4. **Iterate Across Waves**:
   Recompute the frontier as PRs merge. Dispatch newly unblocked tasks until all phases are complete.

5. **Final Review & Verification**:
   Launch a read-only verifier subagent to check the full diff against the original plan and run end-to-end test suites.

## Turn Report

End every turn with:

- Done: <implementation progress, merged PRs, active builders>
- Verified: <automated test outputs, git merge ancestry>
- Remaining: <remaining phases or validation>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
