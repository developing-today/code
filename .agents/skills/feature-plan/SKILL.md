---
name: feature-plan
description: "Turn a settled design into an implementation plan in planning/plans/ from planning/templates/plan.md, then publish one implement issue per step and the orchestration issue that runs them (S7 of a workflow research package, worked from a plan issue). Steps are granular and numbered, shared interfaces come first, and each step states its tier (crucial, recommended or contingent), goal, what it enables, owned files, the failing test it starts from and a done check a command can run. Hard blockers are kept apart from soft rework-if-early edges, file collisions become blocking edges, and steps form waves with [P] parallel markers. Decisions that depend on implementation details are deferred with defaults. A fresh-context validator checks the plan, a zero-context reader checks each issue, and issues carry blocked-by links. Use when given a plan issue, or asked to plan, break down, sequence or ticket a design. Running the steps is implementation-orchestrate; settling the design is design-orchestrate."
---

# Feature plan (S7)

You turn one settled design, usually one sub-feature named by a plan issue, into a plan that parallel workers can build without asking anything: granular numbered steps, interfaces first, hard and soft edges, waves, tiers, tests and deferred decisions with defaults. Then you publish one implement issue per step and one orchestration issue. You plan; you never design and never implement. If the design is wrong or silent, defer the question with a default, tell the design side, and keep planning.

The stage contract is in [stages.md](../../../planning/workflow/stages.md), edge and wave semantics and the step fields in [ordering.md](../../../planning/workflow/ordering.md), relation kinds and tiers in [DEPENDENCIES.md](../../../planning/DEPENDENCIES.md), branch, lease and PR rules in [collaboration.md](../../../planning/workflow/collaboration.md), and decision sources and coverage in [decisions.md](../../../planning/workflow/decisions.md). The plan follows the [plan template](../../../planning/templates/plan.md); the issues follow the [implement](../../../planning/workflow/templates/issue-implement.md) and [orchestration](../../../planning/workflow/templates/issue-orchestration.md) templates. This skill does not restate those rules.

## Set up

```sh
source /home/user/.local/share/plaza-tools/git-env.sh     # the profile's git_env
rw() { python3 -I scripts/research_workflow.py "$@"; }    # run from the repository root
PKG=doc/<UTC>_research_<slug>; ITEM=C001.4                # package; the item in the plan issue's marker
PLAN=planning/plans/<sub-feature slug>.md                 # the plan this run writes
NAME=planner-<plan issue number>-<4 hex>                  # your agent name
LIVE="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-S7"; mkdir -p "$LIVE/issues"
```

Each log line is `$NAME <UTC> <kind>: <text>`, appended to `$LIVE/$NAME.md`, with a `heartbeat` at least every 10 minutes. If a command fails with `invalid choice` or an unknown flag, use the router's [tool fallbacks](../research-workflow/references/tool-fallbacks.md). Outside a research package (a design record with no package), skip the `rw` commands and the package records; every other step holds.

## Steps

1. **Boot and claim.** Create your own worktree and branch from a fresh `origin/main` (the plan issue's branch name, else `research/<slug>/s7-<item>` with the item in lowercase, such as `research/docs-pipeline/s7-c001.4`), push it empty, take the lease `$ITEM` (`rw lease acquire`), record the trunk SHA and the UTC time, and append `start`.
   Done when `git ls-remote` shows the branch, the lease is listed, and the log has `start` with the SHA.

2. **Verify the entry state.** The plan issue is open. Its design orchestration issue is closed. The consolidated design record, its decision records and `human-review.md` are on `origin/main`. No other branch, PR or plan file exists for `$ITEM` (run the marker search in the [templates README](../../../planning/workflow/templates/README.md) and `git ls-remote origin 'refs/heads/research/<slug>/*'`). If a condition fails, comment `Blocked: <what>` on the plan issue and stop.
   Done when every condition is checked against the recorded SHA, with the output in the log.

3. **Read the design and map the code.** Read the plan issue's "Read first" list in order. Then launch read-only subagents in parallel, one per area of code the design touches, each returning the files, symbols and tests involved as `file:line` at the recorded SHA. Pass them paths and URLs, not summaries.
   Done when every section of the design maps to code paths with `file:line` evidence, or is marked "new file".

4. **Fence the goal and scope.** Copy the plan template to `$PLAN` and fill the header, Goal and Scope as in [references/plan-outline.md](references/plan-outline.md). Every `Out` item names where it is tracked. Commit, push, and open a draft PR into `main` before writing the steps (collaboration rule 3).
   Done when the draft PR exists and its head equals your last pushed SHA.

5. **Pin interfaces first.** List every interface two or more steps will share: types, schemas, file formats, CLI flags, config keys, paths and module boundaries. Each gets one owner, a step behind a contract test or the plan's own text, and every step that uses it requires that owner. No other step defines or changes it.
   Done when the plan's Interfaces section gives each shared interface exactly one owner that comes before all of its users.

6. **Write granular steps.** Number them in one sequence in dependency order, so a step only ever requires lower numbers. Each is one PR and one fresh session, in the step format of `references/plan-outline.md`: tier, goal, what it enables, owned files, the decisions and items it cites, the failing test it starts from, a done check as a command, edges and size.
   Done when every step has every field and none needs more than one PR.

7. **Mark edges, waves and `[P]`.** Use `Requires:` only when one step's output is a real input to another, and `After:` with the rework it costs to start early. Two steps that write one file: reassign the file, or make the collision a `Requires:` that names it. Prefer the arrangement with the fewest hard edges. Group the steps into waves, mark `[P]` on each step that shares no edge and no file with the others in its wave, and write the critical path, peak parallelism against `max_parallel_writers` and the estimate in sessions.
   Done when every `After:` states its rework, every file collision is reassigned or a hard edge, and the critical path is written down.

8. **Tier and order by value.** Mark each step `crucial` (the goal fails without it), `recommended` (worth doing now, the goal holds without it) or `contingent` (only if its named trigger fires). No crucial step may require a non-crucial one: raise the blocker's tier or drop the edge. Within a wave, crucial steps come first, then by what they enable.
   Done when every step has a tier, every contingent step names its trigger, and no crucial step requires a lower tier.

9. **Write tests and evidence.** For each step, name the test that fails before the change and passes after it; workers write it first. Name the integration or end-to-end checks the last wave runs and the profile's `validation_commands`.
   Done when every done check is a runnable command and every step names a test that would fail at the recorded SHA.

10. **Defer what depends on implementation.** In "Deferred decisions", give each open question its default, the step whose worker settles it, the step it must be settled before, and what changes if the answer differs. List the `human-review.md` defaults the plan builds on, with the steps that would redo work if a human reverses one. Product or architecture questions go back to the design side as a comment on the design orchestration issue; the plan continues on the default.
    Done when every open question from the design is answered by a step, deferred with every field, or listed as a human-review default.

11. **Validate with fresh context.** Fill Rollout and rollback, Risks and Progress, run `scripts/plan_check.py`, push, and give the plan to one fresh read-only validator with the prompt in [references/validation.md](references/validation.md). Fix every blocking finding and re-run with a new validator, at most three rounds; what still blocks becomes a deferred decision or a tracked `Out` item.
    Done when `plan_check.py` prints `ok` and the last validator return has no blocking finding, both saved in the log.

12. **Publish the implement issues.** Follow [references/issues.md](references/issues.md): draft the bodies in parallel with read-only subagents, one step each, give each draft to a fresh [zero-context reader](../../../planning/workflow/templates/prompts/zero-context-reader.md), then create the issues in wave order, blockers first, with `--blocked-by` and the plan issue as `--parent`. Contingent steps get no issue until their trigger fires.
    Done when every crucial and recommended step has an open implement issue that ends with its marker, whose `blockedBy` equals the step's `Requires:`, and whose last zero-context read has no blocking question.

13. **Publish the orchestration issue.** Fill the orchestration template's S8 lines with the graph in wave order, run the zero-context reader on it, publish it, and comment its URL on every implement issue and on the plan issue.
    Done when it is open, ends with its marker, and every implement issue links it.

14. **Record and report.** Write the issue links into the plan and its Progress section, add the plan to the plan catalog, and update the package records (`references/issues.md`, "Recording"). Run `prettier --write` on your files and `rw check --repo-root .`, merge `origin/main`, push, mark the PR ready, comment it on the plan issue, `rw lease release`, and append `end <last pushed sha>`. Never merge your own PR.
    Done when the check passes and `git ls-remote origin <branch>` equals the PR head and the SHA you report.

## Parallelism and ownership

One agent per plan issue, so sub-features plan in parallel. You are the only writer of `$PLAN`, its line in the plan catalog, your item's lines in `main.md` and blocks in `design.md`, the concept state block while you hold its lease, and the issues you create. Mapping, drafting, validating and zero-context subagents are read-only leaves that return text; none of them writes files or creates issues. You never edit design or decision records: send a note to the concept (`rw note`) and comment on the design orchestration issue instead ([collaboration.md](../../../planning/workflow/collaboration.md), rules 1 to 10).

If the harness cannot launch subagents, do the mapping, drafting and reading yourself one at a time, but still give the plan and every issue body to a reader that did not write them, and say so in the turn report.

## Should load / should not load

- Load for: "work plan issue #540"; "turn this approved design into a granular implementation plan"; "break the C003 design into numbered steps with tests"; "sequence these steps into waves and mark what can run in parallel"; "make the implement tickets and the orchestration issue for this plan".
- Don't load for these, which belong to the named skill: settling the design, merging design PRs or deciding between options (`design-orchestrate`); running implement issues, merging builder PRs or the final reviews (`implementation-orchestrate`); writing research or design child issues (`issue-handoff`); finding the next stage of a package (`research-workflow`); writing code for one step (follow its implement issue).

## References

- [references/plan-outline.md](references/plan-outline.md): read in steps 4 to 10 (sections, the step format, interfaces, edges, waves, tiers, deferred decisions).
- [references/validation.md](references/validation.md): read in step 11 (the mechanical checks, the validator prompt, the stop rule).
- [references/issues.md](references/issues.md): read in steps 12 to 14 (titles and markers, links before the plan merges, drafting, creating, the orchestration issue, recording).
- [scripts/plan_check.py](scripts/plan_check.py): run in step 11 and after every fix.

## Turn report

End every turn with:

```text
Done: <plan path; steps, waves and tiers counted; issues created or reused, with URLs>
Verified: <plan_check.py -> ok>, <validator round n -> no blocking finding>, <zero-context reads -> no blocking question>, <rw check --repo-root . -> ok>, <ls-remote = PR head -> ok>
Remaining: <"implementation-orchestrate on <orchestration issue URL>", or what is left>
Questions (non-blocking): <deferred decision or question -> default in use>, or none
Blocked on user: <only a design that leaves the goal impossible to plan>, or none
```

The final turn also gives the branch, the PR URL, the last pushed SHA, the critical path and the estimate in sessions.
