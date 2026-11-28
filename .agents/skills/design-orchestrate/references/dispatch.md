# Dispatching, watching and restarting workers

Read before dispatching the first worker. `implementation-orchestrate` uses the same rules for its builders and points here. The binding rules are in [collaboration.md](../../../../planning/workflow/collaboration.md); this file is the procedure.

## Names, branches and worktrees

- **Worker name.** `<role>-<issue number>-<4 hex>`, for example `designer-512-a3f9` or `builder-640-0c1e`. Names are unique for the whole run. A restarted worker keeps its name and adds `-r<n>`.
- **Branch.** `research/<slug>/s<stage>-<unit>`, with `<slug>` the package slug and the unit in lowercase, for example `research/docs-pipeline/s6-c001.3` for a facet or `research/docs-pipeline/s8-c001.4-step-3` for a plan step ([vocabulary.md](/planning/workflow/vocabulary.md#identifiers)). If the issue names a branch, use that one. A restart uses the same branch. Follow-up work after a merge uses `<branch>-2`, because a merged branch is dead.
- **Worktree.** `<worktree_root>/<branch with / replaced by ->`. A restart reuses the same worktree, so local commits survive.
- **Base.** Always a fresh `origin/<trunk>` at dispatch. The worker checks that its first `HEAD` equals `git rev-parse origin/<trunk>` after a fetch. A worktree created many commits behind makes its evidence worthless.

## Worker brief

Each brief is under about 4 KB and holds pointers only. Fill in every `<...>`:

```text
You are <name>, the <designer|builder> for <issue URL>. The issue body is your full brief. Read it first, then the files it links.

Repository: <checkout path>. Trunk: <trunk>. Branch: <branch>. Worktree: <worktree path>.
Boot:
  source <git_env>
  git -C <checkout> fetch -q origin
  git -C <checkout> worktree add -b <branch> <worktree path> origin/<trunk>
  cd <worktree path> && git config --worktree gc.auto 0 && git push -u origin <branch>
  test "$(git rev-parse HEAD)" = "$(git rev-parse origin/<trunk>)"
  Make your first commit, push it, and open a draft PR into <trunk> before extensive changes.
Lease: run `lease acquire` on <issue-<n> or concept-<id>> and renew it before 20 minutes pass. Run `lease release` at the end.
  (If the tool has no `lease`, use the mkdir lock in collaboration rule 5.)
Live log: append lines to <live>/<name>.md in the form `<name> <UTC> <kind>: <text>`:
  `start` first, with this brief verbatim after it; `heartbeat` at least every 10 minutes;
  `push <sha>` after every push; `blocked` with the reason; `end <last pushed sha>` last.
You own only: <files the issue lists>. Do not touch: <everything else, especially other workers' files, main.md and design.md outside your item lines>.
Shared names and decisions to build on: <paths on origin/<trunk>>.
Every 15 minutes, and before each push: run `git fetch`, read new comments on your issue, and pick up any decision or shared name posted there.
Rules: no git stash, no rebase, no force push, no reset --hard, no gc or prune, no sudo, no `git add .`. Stage explicit paths. Merge commits only.
Before you report: format your files, run `git fetch origin && git merge --no-edit origin/<trunk>`, re-run your checks, push, and mark the PR ready for review.
Do not fix problems that already exist on <trunk>. Name them in the PR and the final message.
You are a leaf. Read-only subagents for research are fine; do not start writers or orchestrators.
Final message: branch, PR URL, last pushed SHA (`git rev-parse HEAD`, equal to `git ls-remote origin <branch>`), what is verified (command -> result), what is not, and new work you found but did not do.
```

Write the brief to `$LIVE/briefs/<name>.md` before launching, so a restart can reuse it verbatim. Launch workers in the background and in parallel. Log `dispatch <name> <issue> <branch>` in `root.md`.

## Sizing

Each unit must fit one fresh session, well inside its context. A worker that runs out of context, or returns "partial" twice, means the issue is too big. Split it. Open new child issues with their own markers through issue bookkeeping, give them `Blocked by:` lines, and close the original with links. Never let a worker split its own issue by starting writers.

## Liveness and the restart rules

Read the state of each worker whenever one returns and at the start of every turn. Wait for completion notices rather than polling in a sleep loop:

```sh
tail -n 1 "$LIVE/<name>.md"                    # last line and its time
git ls-remote origin refs/heads/<branch>        # what is actually pushed
gh pr list --repo <repo> --head <branch> --state all --json number,state,headRefOid
rw lease list --package "$PKG"                  # live leases (fallback: the mkdir locks)
```

| Situation                                                                                                   | Action                                                                                                                                              |
| ----------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| No log line for 20 minutes, no live lease, subagent not running                                             | Restart: same brief, same branch, same worktree, plus the continuation note below.                                                                  |
| Worker returned "done", but `git ls-remote` does not show its reported SHA, or no PR exists                 | Restart to push and open the PR. Never treat unpushed work as done.                                                                                 |
| PR open, no `end` line, worker gone                                                                         | Restart to finish and report.                                                                                                                       |
| Worker returned `blocked`                                                                                   | Read the reason. Fix what bookkeeping can fix (a missing decision, a stale blocker) and restart; otherwise mark it `blocked` and continue the rest. |
| Worker pushed after its PR merged (a branch that `delete_branch_on_merge` deleted has come back with no PR) | Treat the new commits as follow-up work: open a PR from that branch, or move them to `<branch>-2`. Never let them be lost.                          |

Continuation note added to a restarted brief:

```text
Continue an interrupted run. Branch <branch> is at <ls-remote SHA> on origin; your worktree <path> may hold local commits (`git log origin/<branch>..HEAD`) and uncommitted files. Keep them: commit them, merge origin/<branch> and origin/<trunk>, never reset. PR: <url or "none yet">. Finish the issue and report as the brief says.
```

If the worktree is gone, recreate it from the remote branch (`git worktree add <path> <branch>`) and say so in the note. Log `restart <name> <reason>` in `root.md`.

## Orphan sweep

Run once per watch round. It lists branches of this package that have commits ahead of the trunk and no PR:

```sh
git fetch -q origin
git for-each-ref --format='%(refname:short)' "refs/remotes/origin/research/<slug>/" | while read -r ref; do
  b=${ref#origin/}
  ahead=$(git rev-list --count "origin/<trunk>..$ref")
  pr=$(gh pr list --repo <repo> --head "$b" --state all --json number -q '.[0].number')
  [ "$ahead" -gt 0 ] && [ -z "$pr" ] && echo "ORPHAN $b ahead=$ahead"
done
```

Give each orphan to its worker: restart the worker, or open the PR yourself when the worker has finished and its final message names the SHA. Never delete an orphan branch.
