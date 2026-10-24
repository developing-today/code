# Plan issue template

The S6 orchestrator opens one per concept once its design is settled and the challenge pass is done; S7 (`feature-plan`) works it. Title: `Plan: <concept title> (<package>·C001)`. Run the marker search in the [README](README.md) first.

````markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

[`planning/plans/<slug>.md`](https://github.com/<issue_repository>/blob/<trunk>/planning/plans/<slug>.md) turns the design of **<concept title>** into granular, ordered steps, each sized for one PR and one fresh session, with hard blockers, soft order and parallel groups marked; every step has an implement issue; and one orchestration issue runs them.

## Read first

1. [Design records](https://github.com/<issue_repository>/blob/<trunk>/<design record path>) and [decision records](https://github.com/<issue_repository>/tree/<trunk>/<package path>/decisions).
2. [human-review.md](https://github.com/<issue_repository>/blob/<trunk>/<package path>/human-review.md): preliminary decisions the plan builds on.
3. [The concept file](https://github.com/<issue_repository>/blob/<trunk>/<package path>/concepts/C001-<slug>.md) and [transcript](https://github.com/<issue_repository>/blob/<trunk>/<package path>/transcript.txt) segments `<T007-T009>`.
4. [Plan template](https://github.com/<issue_repository>/blob/<trunk>/planning/templates/plan.md), [ordering](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/ordering.md), [feature-plan skill](https://github.com/<issue_repository>/blob/<trunk>/.agents/skills/feature-plan/SKILL.md).

## Current state (verified <UTC> against `<trunk SHA>`)

- Design orchestration [#<n>](https://github.com/<issue_repository>/issues/<n>) closed; <n> design PRs merged.
- Preliminary decisions awaiting a human: <D007, D009, or "none">.

## Your role

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- You write the plan. Use parallel subagents to draft implement issues, one step each, and a zero-context reader on each draft.
- Keep decisions that depend on implementation details open in the plan, each with when and by whom it will be settled.

## Owned outputs

- Writes: `planning/plans/<slug>.md`, the plan's row in the plan catalog, the concept's `issues` and `changes.md` lines, through one PR from `<branch>`.
- Implement issues (one per step) and the S7 orchestration issue.
- Must not touch: design and decision records (send a note if they are wrong).

## Prerequisites

- Blocked by: design orchestration [#<n>](https://github.com/<issue_repository>/issues/<n>).
- Parallel-safe with: plan issues of other concepts, unless they share files: <list>.

## First steps

1. Boot a fresh worktree and push the empty branch:
   ```sh
   source <git_env>
   git -C <checkout> fetch -q origin
   git -C <checkout> worktree add -b <branch> <worktree_root>/<branch-slug> origin/<trunk>
   cd <worktree_root>/<branch-slug> && git push -u origin <branch>
   ```
2. List the steps with their files; mark `requires`, `recommended-after` (with the rework) and parallel groups.
3. Write the plan, open the PR, then fan out the implement issues.

## Done when

- Outcome: the goal above is true.
- Verified by: each step names its files, its done check and its blockers; each step has an implement issue with its marker; the orchestration issue lists the graph; `check --repo-root .` passes; the PR is merged.
- Boundaries: planning only. No implementation.
- Stop as blocked: if the design leaves a step impossible to scope, write it as an open decision with a default, comment here, and continue with the rest.
- Report here and in your final message: branch, PR URL, last pushed SHA, issue URLs, what is verified, what is not.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S6","kind":"plan"} -->
````
