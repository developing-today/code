<!-- Moved verbatim from AGENTS.md (Research, design and delivery workflow) by task 15116 (issue #8430) so the root rulebook fits its 8,000-byte budget. -->

### Parallel writers

These apply whenever other agents may share the repository. Reasons and detail: [`planning/workflow/collaboration.md`](planning/workflow/collaboration.md).

- **Boot:** `source /home/user/.local/share/plaza-tools/git-env.sh` (commits fail without it), then a fresh worktree from `origin/<base>`. Push the empty branch first; open the PR after the first commit.
- **One writer per file.** Wide files (`AGENTS.md`, `doc/README.md`, indexes, a package's `main.md` and `design.md`) change only by PR from your own branch.
- **Add-only records are new files** (notes, receipts, decisions), or one line in a union-merged `changes.md`. Never append to a shared file by hand.
- **Never** `git stash` (all worktrees share it; use a WIP commit), force push, rebase, `git add .` (stage explicit paths), `gc`/`prune` or `sudo`. Merge commits only; merge the base in before opening or landing a PR.
- **Final message:** branch, PR URL, last pushed SHA, what was verified and what was not.
- **A merged branch is dead** (GitHub deletes it). Follow-up work goes on a new branch and PR.

### Subsession working pattern

How a session works through subsessions. Mechanics verified against the
production server on 2026-11-06; the review flow is the standing
design-mill protocol.

- **Worktrees always, directory explicit.** Every session works in a
  worktree on its own branch, never the primary checkout. A subsession
  inherits its parent's directory and, unless a model is passed, its
  parent's route: launch children from the directory their work belongs
  in, and state the directory and route in the launch prompt anyway, so
  the record shows intent.
- **Scope the task to the context it needs.** A child gets a crisp spec
  and only the context that task requires; the parent keeps the whole
  picture. Independent tasks launch in parallel. Follow up with a finished
  subsession by its session id instead of relaunching cold.
- **Foreground waits; background notifies.** A foreground `subagent` call
  returns the child's final output inline. A background launch returns at
  once and its completion arrives as its own turn: do not poll in a hot
  loop, sleep, or duplicate the running child's work.
- **Own reviewer before return.** Work that launches reviewer/judge
  subsessions returns to its parent only when the worker and the reviewer
  agree it is complete and correct; disagreement loops through gap-fill
  sessions and re-review. Orchestrators run their own reviewers for total
  scope, cohesion, completeness, and end-to-end correctness, guiding
  implementer sessions to fill gaps — orchestrators do not implement.
- **Verify what a child did.** For anything that writes, read its diff and
  artifacts, not just its final message; the final message is enough for
  pure lookups.
- **Detached sessions never call the `question` tool.** There is no user
  watching: the call blocks the child and its foreground parent, nobody is
  notified, and the fleet retry can re-ask the same question in a loop.
  Record the question with the default you chose and continue.
- **A session cannot change its own model.** The route ref set at creation
  is what it runs (the serving leaf inside a route varies per step). If a
  task needs a different head, launch a subsession on that head; switching
  models mid-session also discards the prompt cache.
