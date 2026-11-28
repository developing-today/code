# Implement issue template

S7 (`feature-plan`) opens one per plan step; an S8 worker dispatched by the implementation orchestration issue works it. Title: `Implement: <step> (<package>·C001·step <n>)`. Run the marker search in the [README](README.md) first.

````markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

Step <n> of [the plan](https://github.com/<issue_repository>/blob/<trunk>/planning/plans/<slug>.md#<step-anchor>) is done: <what is true afterwards, in one sentence>.

## Read first

1. [The plan, step <n>](https://github.com/<issue_repository>/blob/<trunk>/planning/plans/<slug>.md#<step-anchor>): files, done check, blockers.
2. [Design record](https://github.com/<issue_repository>/blob/<trunk>/<design record path>) and decisions [D<nnn>](https://github.com/<issue_repository>/blob/<trunk>/<package path>/decisions/D<nnn>-<slug>.md).
3. [Orchestration issue #<n>](https://github.com/<issue_repository>/issues/<n>) and [collaboration rules](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/collaboration.md).

## Current state (verified <UTC> against `<trunk SHA>`)

- <what exists in the code today, with file:line; line numbers drift as `<trunk>` moves>
- Blocking steps merged: <#n in PR #n, or "none">.

## Your role

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- You are the implementer for this one step. Use read-only subagents for separable investigation; a fresh reviewer reviews your PR, not you.
- Review is light: the PR exists to surface conflicts before merge. Do not fix preexisting problems outside the step; file them.

## Owned outputs

- Writes: <files the plan assigns to this step>, and their tests.
- Branch `<branch>` from `<trunk>`, one PR into `<trunk>`, merge commits only.
- Must not touch: files of other steps, the plan, design and decision records.

## Prerequisites

- Blocked by: <#n, or "none">.
- Parallel-safe with: <#n, #n>.
- Shares files with: <"none", or the issue to finish first>.

## First steps

1. Boot a fresh worktree and push the empty branch:
   ```sh
   source <git_env>
   git -C <checkout> fetch -q origin
   git -C <checkout> worktree add -b <branch> <worktree_root>/<branch-slug> origin/<trunk>
   cd <worktree_root>/<branch-slug> && git push -u origin <branch>
   ```
2. Write the failing test the step's done check names, make a first commit and open the PR.
3. Implement until the done check passes; merge `origin/<trunk>` into the branch before asking for merge.

## Done when

- Outcome: the goal above is true on `<trunk>`.
- Verified by: <the step's done check, as a command>; <the repository's test command> passes; the PR is green on its own base.
- Boundaries: this step only. No design changes: if the design is wrong, send a note to the concept and comment here.
- Stop as blocked: if a blocker is unmerged, a requirement only a person can provide is missing, or the design does not hold, comment here with what and why, set the run status to `blocked`, and stop.
- Report here and in your final message: branch, PR URL, last pushed SHA, what is verified, what is not.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S7","kind":"implement","step":"<n>"} -->
````
