# Multi-Agent Collaboration Protocol

This document defines the 10 binding collaboration rules for orchestrators and worker subagents operating across parallel git worktrees.

## The 10 Collaboration Rules

1. **One Writer Per File**: Every document, concept file, outgoing note, and receipt has exactly one owning writer at a time. Wide shared files (`main.md`, `design.md`, `AGENTS.md`, `doc/README.md`, catalogs) must only be modified via pull requests under active mutex leases.
2. **Add-Only Records**: Cross-agent communication must use new files (`notes/`, `receipts/`, `decisions/`) or union-merged lines (`changes.md`). Never append directly to the end of a shared file owned by another agent.
3. **Worktree Boot Protocol**: Every parallel worker must execute in a dedicated worktree on a dedicated branch branched from the declared integration baseline.
   - Sourcing the local git environment: `source /home/user/.local/share/plaza-tools/git-env.sh`.
   - Disabling automatic garbage collection: `git config --worktree gc.auto 0`.
   - Pushing an empty branch to origin immediately upon creation to reserve the ref: `git push -u origin <branch>`.
   - Making the first commit and opening a PR before proceeding with extensive changes.
   - Guardrails: strictly no `git stash` (use WIP commits), no `git reset --hard`, no rebase, no force-push, no `sudo git`, and stage explicit paths only (`git add -- <paths>`).
4. **Dead Branch on Merge**: Because repository settings automatically delete head branches upon merge, merged branches are immediately considered dead. Post-merge follow-up work must be performed on a newly branched ref (e.g. `<branch>-2`).
5. **Live Coordination & Atomic Leases**: Non-committed live coordination data lives in `$(git rev-parse --git-common-dir)/agent-work/<package>-<stage>/`:
   - Mutex locks use atomic directories: `mkdir locks/<resource>.lock/` containing an `owner` file (`<name> <acquired> <expires> <purpose>`) with a 20-minute expiry and atomic rename-based takeover.
   - Append-only heartbeat logs: `<name>.md` with lines `<name> <UTC> <kind>: <text>`, beginning with `start` and ending with `end`.
6. **Worker Completion Report**: Every worker subagent must terminate with a structured report containing:
   - Branch name and open PR URL.
   - Exact last pushed commit SHA (`git rev-parse HEAD`).
   - Verified automated test commands and output.
   - List of unverified assumptions or open questions.
7. **Orchestrator Merge Gate**: An orchestrator merges a worker's PR only when:
   - The remote ref (`git ls-remote origin <branch>`) matches the open PR's head SHA and matches the worker's reported last SHA.
   - The worker subagent is confirmed idle/finished.
   - Required status checks are green.
   - Merging uses a merge commit (`--no-ff`), followed by verification that the commit is an ancestor of the base (`git merge-base --is-ancestor`).
   - Stalled workers (no heartbeat for 20 minutes, no active lease, no open PR) are restarted with their original brief on the same branch.
8. **Ready Frontier Dispatch**: A concept is ready to dispatch only when all of its hard prerequisites (`requires`) are fully merged, no active lease exists, and no open PR or remote branch is currently working on it. Maximum concurrency is capped by `max_parallel_writers` (default: 5).
9. **Context Pointers Over Pasted Summaries**: Orchestrators communicate with subagents using file paths, commit SHAs, and issue URLs. They avoid pasting large text transcripts or intermediate summaries into prompts.
10. **Green on Own Base**: Every PR must pass all required checks against its own base. CI checks validate only the packages modified by the PR; global whole-tree audits run on scheduled workflows.
