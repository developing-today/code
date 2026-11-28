# Orchestration issue template

Opened at S5 to run every child issue of a concept through design (S6, `design-orchestrate`), and at S7 to run every implement issue of a plan (S8, `implementation-orchestrate`). Title: `Handoff: Orchestrate <design or implementation> of <concept title> (<package>·C001) as an orchestrator using subagents`. Keep the S6 or the S8 lines and delete the other. Run the marker search in the [README](README.md) first; the marker's `stage` is `S5` or `S7`.

```markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

Every issue in the graph below is merged and validated, and the main reference and meta design show each one's result. S6: design records, decision records and [human-review.md](https://github.com/<issue_repository>/blob/<trunk>/<package path>/human-review.md) are complete, and the plan issue is open. S8: the feature works as the plan says, on `<trunk>`.

## Your role: orchestrator, not implementer

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- You plan, dispatch, track and decide how work proceeds. Workers design, implement, review and validate. Your only writes are merge commits, state and receipt files, and issue and PR bookkeeping.
- Run every ready issue at once, up to `max_parallel_writers`, one worker each with its own branch and worktree. Issues that share files run one after another.
- No worker reviews its own work: a fresh reviewer subagent reviews every PR against its issue.
- Re-verify what workers report (`git ls-remote`, `git log`, CI) before building on it, and carry their hedges forward.

## Read first

1. [Orchestrator brief](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/templates/prompts/orchestrator-brief.md): the loop, the merge gate and the restart rule.
2. [Meta design](https://github.com/<issue_repository>/blob/<trunk>/<package path>/design.md): "Initial research stages" (S6) or the plan's steps (S8), waves and run table.
3. S6: [parent issue #<n>](https://github.com/<issue_repository>/issues/<n>). S8: [plan](https://github.com/<issue_repository>/blob/<trunk>/planning/plans/<slug>.md).
4. [Collaboration rules](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/collaboration.md) and [decision protocol](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/decisions.md).

## Current state (verified <UTC> against `<trunk SHA>`)

- <n> issues: <n> merged, <n> open PRs, <n> not started. `python3 scripts/research_workflow.py status --live --package <package path>` output: <summary>.

## The graph

1. [#<n> <title>](https://github.com/<issue_repository>/issues/<n>) — blocked by: none. Parallel with: #<n>.
2. [#<n> <title>](https://github.com/<issue_repository>/issues/<n>) — blocked by: #<n> (requires: <reason>).

## First steps

1. Boot a read-only worktree on `origin/<trunk>` (`source <git_env>`, `git worktree add --detach <worktree_root>/<slug> origin/<trunk>`).
2. Run `ready --package <package path>`, `status --live` and `lease list`; dispatch every ready issue with the worker brief.
3. Write a `start` line in the coordination directory and comment here when each PR opens or merges.

## Done when

- Outcome: the goal above is true.
- Verified by: every graph issue is closed by a merged PR whose SHA is an ancestor of `origin/<trunk>`; `check --package <package path> --require-resolved` passes; the run table shows `validated` for each. S6: the challenge pass ran and every reopened decision was re-investigated. S8: `validation_commands` from the profile pass on `<trunk>`.
- Boundaries: no implementation, no design writing, no deciding a disagreement yourself: launch the reviewer, critic and investigator prompts with both sides' material.
- Stop as blocked: only for a fork that changes everything after it. Otherwise continue on the preliminary choice, list it in `human-review.md`, and keep going.
- Report here and in your final message: Done, Verified, Remaining, Questions (with defaults), Blocked on user.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"<S5 or S7>","kind":"orchestration"} -->
```
