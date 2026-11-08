---
name: design-orchestrate
description: "Run S6 of a workflow research package as an orchestrator that never designs or implements. Takes one concept's design child issues to a settled design by dispatching the ready frontier to workers (own worktree, empty branch pushed first, merge-commit PRs), watching heartbeats, restarting stalled or unpushed workers on the same branch, and merging only after the SHA triple check. Keeps uncontested preliminary decisions, sends every disagreement to read-only reviewer, critic and investigator subagents, runs the challenge pass on decisions made from too narrow a set of options, and keeps working on preliminary choices instead of halting. Ends with a minimal human-review.md, a feature-plan issue per sub-feature, and main reference and meta design updates by PR. Use when given a design orchestration issue, or asked to run, merge, settle or close a concept's design issues. For creating those issues use issue-handoff; for writing the plan use feature-plan."
---

# Design orchestrate (S6)

You take one concept's design child issues to a settled design. You dispatch, watch, verify, merge and record. Workers write every design record, and read-only reviewer, critic and investigator subagents settle disagreements. You never write design prose or code, and you never decide a disagreement yourself. Work never waits on a human: it continues on the preliminary choice, because in the user's words "rework isn't ideal, that is a better approach than halting until a human response".

The stage contract is in [stages.md](../../../planning/workflow/stages.md), the decision protocol in [decisions.md](../../../planning/workflow/decisions.md), the worker, lease and merge rules in [collaboration.md](../../../planning/workflow/collaboration.md), and scheduling in [ordering.md](../../../planning/workflow/ordering.md). Records and issues follow the [templates](../../../planning/workflow/templates/README.md); subagents get the [prompts](../../../planning/workflow/templates/prompts) verbatim. This skill does not restate those rules.

## Set up

```sh
source /home/user/.local/share/plaza-tools/git-env.sh     # the profile's git_env
rw() { python3 -I scripts/research_workflow.py "$@"; }    # run from the repository root
PKG=doc/<UTC>_research_<slug>; CID=C001                   # the package and the concept
LIVE="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-S6"; mkdir -p "$LIVE/briefs"
```

Each log line is `root <UTC> <kind>: <text>`, appended to `$LIVE/root.md`. If a command fails with `invalid choice` or an unknown flag (`lease`, `status --live` and `check --remote-notes` may be missing), use the router's [tool fallbacks](../research-workflow/references/tool-fallbacks.md).

## Steps

1. **Boot and claim.** Make a detached worktree on `origin/main` for reading and a bookkeeping branch `research/<slug>/s6-<concept>-books` (the concept in lowercase), pushed empty. Take the concept lease (`rw lease acquire`), record the trunk SHA and the UTC time, and append `start` to `root.md`.
   Done when the lease is listed and `root.md` has a `start` line with the SHA.

2. **Verify entry and read the graph.** The orchestration issue is open and lists the child issues, and `rw check --package "$PKG" --require-resolved` passes with no notes in flight. From the issue bodies and the concept's "Initial research stages" in `design.md`, read each child's blockers, parallel-safe list, owned files and design record path. Every record goes in one folder, `doc/<UTC>_design_<concept_slug>/`. If two children share a path or a path is missing, comment the fix on those issues before dispatching.
   Done when `root.md` holds the graph, one line per child, and no two children own the same path.

3. **Pin shared names first.** If ready children must agree on terms, paths, config keys or interfaces, dispatch one wave-0 worker that writes them into the consolidated record's "Shared names" section, and block the others on it. Parallel workers otherwise invent incompatible answers.
   Done when the wave-0 PR is merged, or `root.md` says why nothing is shared.

4. **Dispatch the frontier.** A child is ready when every blocker's PR is merged into `origin/main` (go by merged state, not issue state) and no lease, open PR or remote branch exists for it. Start one worker per ready child, up to `max_parallel_writers`, all in the same turn, with the brief in [references/dispatch.md](references/dispatch.md). Children that share files run one after another. Repeat after every merge.
   Done when every ready child under the cap has a worker with a pushed branch, a saved brief and a `start` line.

5. **Watch and restart.** Whenever a worker returns, and at the start of every turn, read the heartbeats, `rw status --live` and `rw lease list`, and apply the restart table in `references/dispatch.md`: a stalled worker, one that reported done without a pushed SHA or PR, and one with an open PR but no `end` line all restart on the same branch and worktree. Then run the orphan sweep. Wait for completion notices instead of polling in a sleep loop.
   Done when every live worker's last heartbeat is under 20 minutes old or it was restarted, and the sweep finds nothing unexplained.

6. **Merge with the SHA triple check.** Run the gate in [references/merge.md](references/merge.md): `git ls-remote` equals the PR head equals the reported SHA, the worker has finished, the PR is mergeable on a fresh base, and checks are green. Merge with a merge commit at that head, confirm `git merge-base --is-ancestor`, close the issue and remove the worktree. A failing PR goes back to its worker; never push to a worker's branch.
   Done when every merge has its gate line and its ancestry line in `root.md`.

7. **Triage the decisions each merge brings.** Read the merged facet's decision records and open questions, using the triage table in [references/decisions.md](references/decisions.md). Keep what nobody contests, correct a misattributed `Source`, send open questions back to their facet, and route every conflict to step 8. Comment each kept decision on the issues that build on it.
   Done when each candidate is marked `kept`, `retagged`, `returned` or `routed` in `root.md`.

8. **Route disagreements.** For each conflict, build the merged material from both sides once, then launch the [reviewer](../../../planning/workflow/templates/prompts/reviewer.md), [critic](../../../planning/workflow/templates/prompts/critic.md) and [investigator](../../../planning/workflow/templates/prompts/investigator.md) in parallel on it. Combine their returns by the rules in `references/decisions.md` and record each result as an `agent-preliminary` decision on a bookkeeping PR. Running workers continue on that default.
   Done when every conflict has a merged decision record and every genuine toss-up is queued for `human-review.md`.

9. **Challenge pass.** Once every crucial and recommended child has merged, give every decision record to one fresh [challenger](../../../planning/workflow/templates/prompts/challenge-pass.md). It finds decisions made from too narrow a set of options. Reopen each flagged one with the investigator, record the outcome, and scope any changed choice with `git grep -l D<nnn>` into follow-up issues. Re-challenge only the reopened decisions, once.
   Done when every decision has a `sound` line or a recorded re-investigation.

10. **Consolidate.** Dispatch one editor worker for the consolidated design record (outline in [references/closeout.md](references/closeout.md)), then a fresh reviewer on its PR. Restart the editor on the same branch for each must-fix, at most twice; what is left becomes a follow-up issue. Merge through the gate.
    Done when the consolidated record is on `origin/main` and the reviewer's must-fix list is empty or tracked.

11. **Write `human-review.md`.** Generate it from the decision records with the [template](../../../planning/workflow/templates/human-review.md): only genuine toss-ups, ranked by effect, each with its default, alternatives and reversal cost, plus batched questions that each carry a default. Never ask what the transcript already states.
    Done when every entry links its record and every unlisted preliminary decision says `Human review: Not needed: <why>` in its own record.

12. **Open a feature-plan issue per sub-feature.** Split the consolidated design into sub-features as `references/closeout.md` describes. Each crucial or recommended one gets an item (`rw item add`) and a plan issue from the [plan issue template](../../../planning/workflow/templates/issue-plan.md), checked by a parallel [zero-context reader](../../../planning/workflow/templates/prompts/zero-context-reader.md). Contingent ones get an item with their trigger and no issue yet.
    Done when each plan issue is open, ends with its marker, and its last zero-context read has no blocking question.

13. **Update the main reference and meta design by PR.** On the bookkeeping branch, mark S6 and what it found in the concept's `main.md` section, add the S6 result block under the concept in `design.md`, update the concept state, run `rw render`, and add one `changes.md` line (details in `references/closeout.md`). Merge it through the gate, comment the summary on the orchestration issue, close it, `rw lease release`, and append `end`.
    Done when `rw check --package "$PKG"` passes on `origin/main`, the PR's SHA is an ancestor, and the orchestration issue is closed.

## Parallelism and ownership

Your git writes are merge commits and bookkeeping PRs from your own branch: decision records that capture what the disagreement subagents concluded, `human-review.md`, `rw item` and `rw render` output, the concept state block while you hold its lease, and `changes.md` lines. Everything else belongs to workers: one per child, each the only writer of the files its issue lists. Reviewer, critic, investigator, challenger and zero-context subagents are read-only leaves with no worktree, so they fan out freely. Pass paths, SHAs and issue URLs, never pasted summaries ([collaboration.md](../../../planning/workflow/collaboration.md), rules 1 to 10).

If the harness cannot launch subagents, do the work yourself one child at a time, on the same branches and through the same gate, and say so in the turn report.

## Should load / should not load

- Load for: "run design orchestration issue #512"; "take C003 through design"; "the C003 design PRs are in, merge them and make the preliminary decisions"; "the storage and sync facets disagree, settle it"; "run the challenge pass before planning"; "write human-review.md" or "open the plan issues for C003".
- Don't load for these, which belong to the named skill: creating the child or orchestration issues (`issue-handoff`); working one design child issue as its worker (follow the issue body); writing the plan or the implement issues (`feature-plan`); running implement issues (`implementation-orchestrate`); finding which stage a package is in (`research-workflow`); a design question outside a workflow package.

## References

- [references/dispatch.md](references/dispatch.md): read before step 4 (names, the worker brief, sizing, liveness, the restart table, the orphan sweep).
- [references/merge.md](references/merge.md): read before step 6 (the gate, pre-existing failures, stale bases, cleanup).
- [references/decisions.md](references/decisions.md): read before step 7 (triage, combining returns, D numbers, the challenge pass, human-review rules, reversal).
- [references/closeout.md](references/closeout.md): read before step 10 (the consolidated record, sub-features, plan issues, the bookkeeping PR).

## Turn report

End every turn with:

```text
Done: <merges with PR numbers and SHAs, workers started or restarted, decisions recorded (D ids), issues opened>
Verified: <ls-remote = PR head = reported SHA -> ok>, <merge-base --is-ancestor -> ok>, <rw check --package -> ok>, ...
Remaining: <children running or not started, decisions in flight, close-out steps left>
Questions (non-blocking): <toss-up -> default in effect>, or none
Blocked on user: <only a fork that changes everything after it, with no other work left>, or none
```

The final turn also gives the bookkeeping branch, its PR URL and last pushed SHA, the consolidated record path, `human-review.md` and the plan issue URLs.
