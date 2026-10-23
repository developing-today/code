# Merging, restarting and sweeping

Read this in steps 5 and 6 of the router. The rule itself is collaboration rule 7 in [collaboration.md](../../../../planning/workflow/collaboration.md). This file has the commands.

## Merge a finished worker (SHA triple check)

Merge only after the worker's final message has arrived (its subagent finished and its log ends with `end`).

```sh
BRANCH=research/<slug>/s1-c003; PR=<number>; REPORTED=<sha from the worker's final message>
git fetch -q origin
REMOTE=$(git ls-remote origin "refs/heads/$BRANCH" | cut -f1)
HEAD_SHA=$(gh pr view "$PR" --json headRefOid --jq .headRefOid)
[ "$REMOTE" = "$HEAD_SHA" ] && [ "$HEAD_SHA" = "$REPORTED" ] && echo "triple match" || echo "MISMATCH"
gh pr view "$PR" --json mergeable,mergeStateStatus,isDraft,baseRefName
gh pr checks "$PR"
git diff --name-only "origin/main...$REPORTED"     # only paths the brief let the worker write
```

Merge only when all of these hold:

- the line prints `triple match`;
- `mergeable` is `MERGEABLE`;
- `isDraft` is false;
- the base is the trunk;
- the required checks pass;
- the diff stays inside the worker's owned paths.

Then:

```sh
gh pr merge "$PR" --merge --match-head-commit "$REPORTED"
git fetch -q origin && git merge-base --is-ancestor "$REPORTED" origin/main && echo landed
```

Afterwards, log `router <UTC> merge: #<PR> <sha>` and release the lease if the worker didn't: `rw lease release --package "$PKG" --stage <S> --resource <C>`.

When a check fails, act as follows. Never push to the worker's branch yourself.

- **Mismatch:** the worker pushed after reporting, or the report is wrong. Ask the worker (resume its session) for its current SHA.
- **`BEHIND` or `DIRTY`:** send it back with "merge origin/main, resolve conflicts in your own files, push, report the new SHA".
- **Red checks:** send it back with the failing check's name and log URL.

After a merge, the branch is deleted (auto-delete is on). Any follow-up for that concept and stage uses a new branch, such as `research/<slug>/s1-c003-2`.

## Detect a stalled worker

```sh
D="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-<stage>"
tail -n 1 "$D/<name>.md"                       # last line and its UTC time
rw lease list --package "$PKG"                 # live leases and their expiry
gh pr list --head "$BRANCH" --state open --json number,url
```

A worker counts as stalled only when all three are true:

- its last log line is more than 20 minutes old;
- it holds no live lease;
- it has no open PR.

If it has an open PR but has stopped, treat it as finished and verify the PR. If it holds a live lease, it is still working.

## Restart on the same branch

If the harness can resume the worker's session, resume it. Otherwise, dispatch a new subagent with the original brief (it is in `$LOG/<name>.md`) plus:

```
Resume: branch <BRANCH> already exists on origin and may have commits.
  git fetch -q origin && git worktree add <worktree> <BRANCH>   (or reuse the existing worktree)
  git merge --no-edit origin/main                               (never reset, never rebase)
Read your log <D>/<name>.md to see what was done; continue from the first unfinished step.
```

Before dispatching, acquire the lease again under the new worker's name. Then log `router <UTC> restart: <old> -> <new> on <BRANCH>`.

## Sweep orphan branches

```sh
git fetch -q --prune origin
for ref in $(git for-each-ref --format='%(refname:strip=3)' "refs/remotes/origin/research/<slug>/"); do
  ahead=$(git rev-list --count "origin/main..origin/$ref")
  prs=$(gh pr list --head "$ref" --state all --json number --jq length)
  [ "$ahead" -gt 0 ] && [ "$prs" -eq 0 ] && echo "orphan: $ref ($ahead commits ahead, no PR)"
done
```

Handle each orphan in one of two ways:

- If its unit is not done anywhere else, restart its worker on it (above), so it opens the PR.
- If the unit was redone on another branch, log `router <UTC> abandoned: <ref> superseded by <branch or PR>` and leave the branch in place. Never delete it.
