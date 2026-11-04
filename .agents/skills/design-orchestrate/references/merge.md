# The merge gate

Read before merging any worker PR. `implementation-orchestrate` uses the same gate. Rule 7 in [collaboration.md](../../../../planning/workflow/collaboration.md) owns the requirement; this file is the procedure.

## Gate

Merge a PR only when every line below holds. Record the outputs in `root.md` as one `merge` line, for example `root <UTC> merge: #612 research/docs-pipeline/s6-c001.3 sha=<s> gate=ok ancestor=ok`.

```sh
b=<branch>; n=<PR number>; s=<SHA from the worker's final message>
git fetch -q origin
test "$(git ls-remote origin "refs/heads/$b" | cut -f1)" = "$s"        # 1. remote = reported
test "$(gh pr view "$n" --json headRefOid -q .headRefOid)" = "$s"      # 2. PR head = reported
tail -n 1 "$LIVE/<worker>.md" | grep -Eq ' end(:| |$)'               # 3. the worker has finished
gh pr view "$n" --json state,isDraft,baseRefName,mergeable,mergeStateStatus
#    4. state OPEN, isDraft false, baseRefName <trunk>, mergeable MERGEABLE
gh pr checks "$n"                                                      # 5. checks done and green (see below)
gh pr merge "$n" --merge --match-head-commit "$s"                      # 6. merge commit at exactly that head
git fetch -q origin && git merge-base --is-ancestor "$s" "origin/<trunk>"   # 7. landed
```

- Never use `--squash`, `--rebase` or `--admin`. `--match-head-commit` makes GitHub refuse the merge if anyone pushed after you checked.
- If line 1 or 2 fails, the worker is still pushing or never pushed. Restart it ([dispatch.md](dispatch.md)); do not merge an older head.
- If line 3 fails, the worker is still running. Wait for its final message, because a worker that commits after the merge loses work to a deleted branch.

## Checks

- Wait for pending checks to finish. `gh pr checks "$n" --watch` blocks until they do.
- A failing check is pre-existing only when the trunk's latest run of the same workflow fails the same way: `gh run list --repo <repo> --branch <trunk> --workflow <workflow file> --limit 1 --json conclusion,headSha,url`. Then merge, name the trunk run in the PR comment, and make sure an issue tracks it. Otherwise the worker fixes it: restart it with the failing log URL.
- A docs-only PR that triggers no checks counts as green.
- Never weaken, skip or disable a check to get green.

## Stale base

`mergeable` only catches textual conflicts. Two workers can each be green and still break each other: one renames a key the other uses, or both add the same entry to an index file. Before the gate, compare:

```sh
git diff --name-only "$(git merge-base origin/<trunk> "origin/$b")" origin/<trunk>   # changed on trunk since the worker last merged it
gh pr diff "$n" --name-only                                                         # changed by this PR
```

If the two lists share a file, or this PR builds on an interface another merged PR changed, restart the worker. It merges `origin/<trunk>`, re-runs its checks, pushes, and reports a new SHA. Then run the gate again.

## Conflicts in generated views

A conflict inside a `<!-- workflow:view ... -->` block in `main.md` or `design.md` is never resolved by hand. The worker merges `origin/<trunk>`, takes either side of the block, runs `render --package <pkg>`, and commits the result.

## After the merge

1. Comment on the issue: PR, merge commit, `ancestor=ok`. Close the issue if the PR did not close it (`gh issue close <n> --reason completed --comment <...>`).
2. Remove the worker's worktree, without forcing: `git -C <checkout> worktree remove <path>`, then `git -C <checkout> branch -d <b>`. If either refuses (dirty tree, unmerged commits), leave it and log `cleanup-skipped <reason>`; the orphan sweep will show it.
3. Never run `git gc`, `git prune` or `git worktree prune`, and never delete a remote branch by hand. Auto-delete removes merged heads.
4. Recompute the frontier and dispatch again.
