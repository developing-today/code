# Collaboration

How parallel agents share one repository without losing work. The skills cite these rules by number ("collaboration rule 7") instead of restating them. File formats are in [records.md](records.md), values in [vocabulary.md](vocabulary.md), and the base branch, git environment and cap in the profile ([profile.md](profile.md)).

## The rules

1. **One writer per file.** A concept file, its outgoing notes, its receipts and a live log each have one writer at a time; [records.md](records.md#package-layout) says who. Wide files (`main.md`, `design.md`, `AGENTS.md`, `doc/README.md`, catalogs, roadmaps) change only through a PR from your own branch.
2. **Add-only records.** What several agents add to is a set of new files (notes, receipts, decisions) or one line each in the union-merged `changes.md`. Never append to the end of a shared file by hand.
3. **Worker boot.** Every writer starts in its own worktree and branch:

   ```sh
   source /home/user/.local/share/plaza-tools/git-env.sh   # commits fail without it
   git fetch -q origin
   git worktree add -b <branch> <worktree_root>/<name> origin/<base>
   cd <worktree_root>/<name> && git push -u origin <branch>
   ```

   - Push the empty branch before editing, so the name is taken. Open the PR after the first commit; GitHub refuses a PR with no diff.
   - Never `git stash` (all worktrees share one stash; use a WIP commit), `gc` or `prune`, `sudo`, force push, rebase, or `git add .` (stage explicit paths). Merge commits only.
   - Merge `origin/<base>` into your branch, and push, before opening the PR and again before it lands. Conflicts are then resolved in your worktree, and `changes.md` merges with git's union driver, which GitHub's merge button may skip.

4. **A merged branch is dead.** GitHub deletes it on merge. Follow-up work goes on a new branch (`-2`, `-3`) and a new PR. Never base a PR on a branch that will be deleted first: target the trunk, or an `integration/` branch that merges last.
5. **Live state stays outside git.** Leases, heartbeats and push SHAs go in the live directory ([Live coordination](#live-coordination)). Facts that must last (notes, receipts, decisions, history) go in git.
6. **Final message.** Every worker ends with its branch, PR URL, last pushed SHA, what it verified (commands and real output) and what it did not.
7. **Orchestrators only merge.** Workers write content. The orchestrator merges, files the decision records reviewers return, and keeps issues and state current. It never pushes to a worker's branch; it sends the worker back.
   - Merge a worker PR only when the worker has finished (its log ends with `end`), the SHA triple check holds (`git ls-remote origin <branch>` = PR head = reported SHA), and the PR is mergeable, not a draft, based on the trunk, green, and inside the worker's owned paths. Afterwards confirm `git merge-base --is-ancestor <sha> origin/<trunk>`.
   - Restart rule: a worker with no log line for 20 minutes, no live lease and no open PR is stalled. Restart it with its original brief on the same branch, merging what is there, never resetting.
   - A branch with commits ahead of the trunk and no PR is an orphan. Restart its worker, or log it abandoned; never delete it.
   - Commands: [merge-and-restart.md](../../.agents/skills/research-workflow/references/merge-and-restart.md).
8. **Dispatch only what is ready, within the cap.** A unit starts only when `ready` lists it: every `requires` target merged, no live lease, no open PR or pushed branch for it ([ordering.md](ordering.md#waves)). At most `max_parallel_writers` writers run at once (profile; 5). Read-only reviewers hold no worktree or lease and do not count. Each unit gets one fresh session; size it to fit one, and split it if it would not.
9. **Paths, not summaries.** Pass file paths, SHAs and issue and PR URLs between agents. Never paste a summary of something the receiver can read itself.
10. **Every PR is green on its own base.** A failure counts as pre-existing only if the base's latest run shows it. CI is meant to check only the packages a PR changes, with a scheduled job for the whole tree; today it checks the whole tree on every PR ([Tool status](records.md#tool-status)).

## Live coordination

Leases and heartbeats change every few minutes and every worktree must see them at once, so they live in the git common directory, which all worktrees of a clone share. Nothing here is committed and nothing here is the record.

- **Directory.** `$(git rev-parse --git-common-dir)/agent-work/<package>-<stage>/` per stage, `agent-work/<package>/` for package-wide locks, and `agent-work/<package>-router/` for the router. If the common directory is not writable, use the profile key `agent_work_dir`.
- **Locks.** `locks/<resource>.lock/`, created with `mkdir`, which fails when the directory exists, so exactly one writer gets it. It holds a file `owner` with one line `<name> <acquired> <expires> <purpose>`, times in UTC.
  - A lease lasts 20 minutes. The holder renews it before then by rewriting `<expires>`, and removes the directory when done.
  - Anyone may take over an expired lease: rename the directory to `<resource>.lock.stale-<UTC>` (only one rename succeeds), then create it again.
  - Resources: `<C001>` for a concept at its stage, taken before the worker is dispatched; package-wide `ids` while allocating a concept or decision ID; package-wide `file-<path>` for a wide file, with each `/` written as `__`.
  - Until `lease acquire|release|list` exists ([Tool status](records.md#tool-status)), make the directory and write `owner` by hand.
- **Heartbeat logs.** `<name>.md`, one per writer, appended only by that writer. Each line is `<name> <UTC> <kind>: <text>`.
  - Worker kinds: `start` first, `heartbeat` at least every 10 minutes, `push <sha>` after every push, `end` last.
  - Router kinds: `dispatch`, `merge`, `restart`, `abandoned`. The router keeps each worker's brief in its own directory as `<worker>.md`, so a restart can reuse it.

## Failure scenarios

Each was found in a review of the first version of this workflow; the prevention is the rule or mechanism above that stops it.

- **Every status change edits one shared state block, so parallel PRs always conflict.** Per-concept state blocks with one writer, generated views, and `changes.md` lines (rules 1, 2). The tool on `main` still uses one block ([Tool status](records.md#tool-status)).
- **`ready` lists concepts that are already running, so work is dispatched twice.** `ready` reads the trunk and skips leased and running units; dispatch takes the lease first (rule 8).
- **A worker commits after its PR merged, or reports done without pushing.** Final message with the last pushed SHA, the SHA triple check, dead merged branches, and the orphan sweep (rules 4, 6, 7).
- **Auto-delete strands PRs stacked on a branch that merges first.** Target the trunk or an `integration/` branch that merges last (rule 4).
- **Notes on unmerged branches are invisible, so reconcile reports everything resolved.** `check --remote-notes` scans every `origin/*` ref, note-only PRs merge fast, receipts record the scanned SHAs, and a later note reopens S3 ([records.md](records.md#receipts)).
- **Claims exist only in git, so two orchestrators take the same unit or file.** `mkdir` leases, including `file-<path>` locks for wide files (rule 5).
- **The shared stash, a missing git environment, or `gc` breaks worktree isolation.** The boot block and its prohibitions (rule 3).
- **Whole-tree validation couples unrelated workers, so red becomes normal.** Green on its own base, changed-package checks, and the pre-existing failure rule (rule 10).
- **Appends at the end of wide files conflict on every parallel PR.** Per-concept sections, one file per entry, and `merge=union` only for line lists (rules 1, 2).
- **No machine-readable stage or liveness, so the orchestrator stalls or starts implementing.** Enumerated run statuses derived from git and GitHub ([vocabulary.md](vocabulary.md#run-status)), heartbeats, the restart rule, and orchestrators that only merge (rules 5, 7).
