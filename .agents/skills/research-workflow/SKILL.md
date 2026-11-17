---
name: research-workflow
description: Route a research package (doc/<UTC>_research_<slug>/) through the research-to-delivery workflow. Reads machine state with the workflow tool, finds each concept's stage, dispatches the stage skill to parallel worker subagents, verifies and merges their PRs, restarts stalled workers and stops where the user asked. Use when asked to run, resume, continue or check a research workflow or package, to find what is ready, running or blocked, to take a request from intake through research, design, planning or implementation, or to run the next stage for every ready concept. It never writes the content itself; for one stage's own work load that stage's skill (request-intake, research-prepare, concept-research, notes-reconcile, issue-handoff, design-orchestrate, feature-plan, implementation-orchestrate).
---

# Research workflow (router)

Moves a research package from S0 (intake) to S8 (implement). It reads machine state, decides what runs next, dispatches one stage skill per unit of work to parallel workers and lands their PRs. The stage contract is in [stages.md](../../../planning/workflow/stages.md), and the dispatch and merge rules are in [collaboration.md](../../../planning/workflow/collaboration.md) and [ordering.md](../../../planning/workflow/ordering.md). The router is an orchestrator. It writes only merge commits, the generated views, lease and log entries, and issue/PR bookkeeping. It never writes concept, research, design, plan or code content.

## Set up

```sh
source /home/user/.local/share/plaza-tools/git-env.sh     # the profile's git_env
rw() { python3 -I scripts/research_workflow.py "$@"; }    # run from the repository root
PKG=doc/<UTC>_research_<slug>                             # the package folder
LOG="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-router"; mkdir -p "$LOG"
```

Each log line is `router <UTC> <kind>: <text>` and is appended to `$LOG/router.md`. If a command below fails with `invalid choice` or an unknown flag, read [references/tool-fallbacks.md](references/tool-fallbacks.md).

## Steps

1. **Find the package and the stopping point.** `rw check --repo-root .` lists every package. If the user handed over new input and there is no package for it yet, the next stage is S0 (see the S0 rule in [references/routing.md](references/routing.md)). Take the stopping point from the user's words using the phrase list in `references/routing.md`. If the user didn't state one, finish the current stage for every ready concept, then stop.
   Done when `$LOG/router.md` has a `start` line naming the package path and the stop stage.

2. **Read state.** Run:

   ```sh
   rw status --package "$PKG" --live      # per concept: stage, run status (derived from git and GitHub), issues, PRs
   rw ready --package "$PKG"              # ready set and waves; excludes running, leased and claimed concepts
   rw check --package "$PKG" --remote-notes
   ```

   Run status `pushed`, `pr-open` and `merged` come from `--live`. Never take them from a worker's own claim.
   Done when you have, for every concept, its stage, its run status, its ready-or-blocked state with the blocking ids, and its pending notes.

3. **Choose one action per concept.** Map stage and run status to an action with the table in `references/routing.md`. The action is one of dispatch, verify-and-merge, monitor, restart, wait (name the blocker), ask the human (batched), or done. Concepts past the stop stage are left alone. Apply the barriers listed there: S3 waits for every S2 to merge, and `type: hint` concepts never go to S2. A conflict between concepts goes to the decision protocol in [decisions.md](../../../planning/workflow/decisions.md), not to you.
   Done when every concept has exactly one action, written as one log line.

4. **Dispatch ready work in parallel.** Concepts are dispatched in `ready` order, and the number running at once never exceeds the profile's `max_parallel_writers` (5 by default). For each one:

   ```sh
   rw lease acquire --package "$PKG" --stage S1 --resource C003 --owner prepare-c003-7f2a --purpose "S1 research-prepare"
   ```

   Then fill in [references/worker-brief.md](references/worker-brief.md) and launch every worker in the same turn as a background subagent. Pass paths and URLs, not summaries. Size each unit to one fresh session. If a unit won't fit, split it before dispatching.
   Done when every dispatched concept has a lease, its brief is in `$LOG/<worker>.md`, and its subagent is running, with nothing over the cap.

5. **Verify and merge finished workers.** When a worker reports, run the triple check in [references/merge-and-restart.md](references/merge-and-restart.md). The remote branch, the PR head and the SHA the worker reported must all be equal. The worker must also have finished, and the PR must be mergeable and green. Then merge with `gh pr merge <n> --merge --match-head-commit <sha>` and confirm with `git merge-base --is-ancestor <sha> origin/main`. A PR that is behind or conflicting goes back to its worker. Never fix a worker's branch yourself.
   Done when every reported PR is either merged and confirmed as an ancestor, or returned to its worker with the reason logged.

6. **Restart stalled workers and sweep orphans.** A stalled worker has no log line for 20 minutes, no live lease (`rw lease list --package "$PKG"`) and no open PR. Restart it on the same branch with its original brief: merge, never reset. A branch with commits ahead of main and no PR is an orphan. Use the commands in `references/merge-and-restart.md`.
   Done when no stalled worker and no orphan branch is left without an action.

7. **Refresh the generated views.** After a wave merges, run `rw render --package "$PKG"` on a fresh router branch (`research/<slug>/router-<n>`) and run `rw check --package "$PKG"`. Open that PR and merge it the same way. Workers leave the shared run table to you, which keeps their PRs from conflicting.
   Done when the run table in `design.md` on `main` matches `rw status --package "$PKG" --live`.

8. **Continue or stop.** If the stop stage is reached, or nothing is ready or running, stop and report. Otherwise go back to step 2. Wait for subagent completion notices instead of polling in a sleep loop.
   Done when the turn report below lists every concept's stage and run status.

## Single-concept shortcut

A package with one concept keeps the full layout (no special case in [records.md](../../../planning/workflow/records.md)) but skips the ceremony:

- one worker runs S1 and then S2 in the same session if it fits;
- S3 is just the check `rw check --package "$PKG" --require-resolved`;
- no waves are planned, and the run table has one row.

Details are in `references/routing.md`.

## Parallelism and ownership

There is one worker per concept per stage, each in its own worktree and branch with a pushed empty branch, and up to the cap at once. Every file has one writer. Wide files (`main.md`, `design.md`, indexes, `AGENTS.md`) change only through PRs, and `changes.md` changes by appending a single line. Notes and receipts are new files. The full rules are in [collaboration.md](../../../planning/workflow/collaboration.md) (rules 1 to 10); this skill does not restate them. Questions for the human are batched, ranked by effect, and each one has a default ([decisions.md](../../../planning/workflow/decisions.md)). Work continues on the defaults.

## Should load / should not load

- Load for: resuming or continuing a package; "what's ready / blocked / running"; running the next stage across concepts; taking a request through to a named stage; merging worker PRs; restarting stalled workers.
- Don't load for these, which belong to the named skill: new raw input with no package yet (`request-intake`); one concept's preliminary research (`research-prepare`); one concept's full research (`concept-research`); notes and receipts (`notes-reconcile`); writing issues (`issue-handoff`); running a design or implementation wave (`design-orchestrate`, `implementation-orchestrate`); general web research with no package.

## References

- [references/routing.md](references/routing.md): read in steps 1 and 3 (stop phrases, stage-to-skill table, actions, barriers, hints, nesting).
- [references/worker-brief.md](references/worker-brief.md): read in step 4, before every dispatch.
- [references/merge-and-restart.md](references/merge-and-restart.md): read in steps 5 and 6.
- [references/tool-fallbacks.md](references/tool-fallbacks.md): read only when a command named here is missing.

## Turn report

End every turn with:

```
Done: <stages run, PRs merged (number and SHA), workers dispatched or restarted>
Verified: <command -> result>, e.g. rw status --package ... --live -> C001..C014 at S1 merged
Remaining: <concepts not at the stop stage, with their next action>
Questions (non-blocking): <question -> default in use>, or none
Blocked on user: <only what nothing else can proceed without>, or none
```
