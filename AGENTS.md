# AGENTS.md

This is the always-loaded rulebook index; it is size-budgeted (8,000 bytes, checked by
`scripts/research_workflow.py check`). Detail lives in the topic files linked from each
section — read the topic file before acting on that section's rules.

## Web retrieval fallback

Full rules: [`doc/agents/web-retrieval-fallback.md`](doc/agents/web-retrieval-fallback.md).
In short: use the normal/default request first; on a 403, 429, or
bot-block, retry with a normal browser User-Agent, then rotate others
(list and site matrix in
[`ai-crawler-site-access-table.md`](./ai-crawler-site-access-table.md#user-agents)).
Never use an alternate User-Agent to debug real HTTP behaviour or to
bypass authentication or access controls, and never treat an
alternate-UA success as proof the resource works for ordinary clients.
Document what works in the access table, and archive genuinely scarce
sources at `https://web.archive.org/save/<url>`.

## Owner preferences

Full text: [`doc/agents/owner-preferences.md`](doc/agents/owner-preferences.md).
In short: use subagents in parallel where independent; never block the
main thread on a long wait (short waits, background long commands); the
owner does not care if secrets or other sensitive values end up in chat
logs — do not mention that or file rotation tasks for it. Velocity:
unblock yourself, test selectively, run full suites only for very large
or critical changes, and file GitHub issues for unresolved gaps instead
of blocking. CI-first verification per
[`.github/ci-policy.md`](.github/ci-policy.md); heavy laptop builds go
through `~/tools/fleet-build-lock` — see the topic file for the build
discipline (target broker, narrowest Rust verification).

## Testing doctrine

Three tools, used where they fit:

- **PBT (property-based testing):** state invariants that must always hold, then let a generator hammer the code with thousands of random inputs to find counterexamples. Where code has meaningful invariants — parsers, serializers, state machines, replication, ID/crypto formats — state the invariants and cover them with property tests. In Rust here that is `proptest` (already a dev-dependency of `pkgs/id`).
- **MBT (model-based testing):** where a system's behavior has a spec or design model, build the model and generate tests — and their expected results — from it, instead of hand-writing cases only.
- **Oracles:** the decision procedure that tells you whether actual output matches expected — the hard part of automated testing. Every automated test names its oracle: a reference implementation, the model, an invariant check, or a golden corpus. A test with no decidable oracle is a smell to fix, not ship.

Example-based tests remain fine for smoke and acceptance coverage; they do not replace PBT/MBT where those fit.

## Operator script argument contract (#3352)

Full text: [`doc/agents/operator-script-argument-contract.md`](doc/agents/operator-script-argument-contract.md).
In short: every operator script declares its whole command line and
parses strictly against that declaration before any subprocess,
network, file write, lock, or deploy step; anything unaccounted for is
a usage error with a non-zero exit. New scripts copy the strict pattern
from a covered sibling and state the contract in their header comment.

## Worker Git preflight

At the start of each task, before relying on Git names such as `origin/main`, run `bash "$(git rev-parse --show-toplevel)/scripts/worker-preflight.sh"`. If it reports `refs/heads/origin/main`, stop and escalate; do not delete the branch or alter refs as part of task work. That local branch shadows the remote-tracking ref.

## Fleet schedules

The unified fleet schedule registry is `doc/fleet-schedule-registry.md`
(#4988). Creating, changing, or retiring a schedule on any surface —
Muse cron, OpenCode/OpenChamber scheduled tasks or laptop systemd
timers — updates that registry in the same pass (same PR /
same run). A schedule that exists on a surface but not in the registry
is a defect.

## Filing an issue outside a creator run

**File and enqueue in the same pass.** Any lane that files a GitHub issue
outside a creator run — a work lane filing a blocker mid-task, an audit
lane, a hand filing — enqueues that issue's triage packet in the same
pass, or records in the filing comment why it is deliberately not
queued. An issue filed without a queue item is invisible to every worker
lane: nothing dispositions it, and it sits open until someone notices by
hand (#7016).

```bash
repo-queue enqueue <packet.json> --queue repo-health-<repo>-pilot
```

The standing backstop is the coverage reconciliation
(`scripts/fleet/queue-reconcile/`, `doc` in its README): every open issue
must hold a **live** task — `queued`, `claimed`, `parked` or `blocked` —
in its own pilot queue. A `done` task is not coverage, and neither is a
task in a `*-shadow` queue. When the reconciliation reports uncovered
issues, the rule above was missed somewhere; fix the filing path, not
just the import.

## The one queue: claiming work

Full text: [`doc/agents/one-queue-claiming-work.md`](doc/agents/one-queue-claiming-work.md).
In short: there is ONE fleet queue; per-repo `queue` values are stream
labels, not separate queues. The default claim is unfiltered —
`repo-queue claim --worker <worker> --lane <lane>` returns the
highest-priority eligible item across all repos. Matching is the union
of the task's requirements and the claimer's claim-time `--filter`
expression, empty by default on both sides; filters are never registry
bindings, and claimer identity is recorded, never itself a filter.
Designated filtered lanes (5 code-only, 3 not-code per surface) are the
only exceptions; the sets are recorded on #8097.

## Research, design and delivery workflow

Use it when a request, from one sentence to pages of voice-transcribed notes, should become researched documents, designs, plans or parallel PRs. Contracts: [`planning/workflow/`](planning/workflow/README.md). Where documents go: [`planning/DOCUMENTATION_ARCHITECTURE.md`](planning/DOCUMENTATION_ARCHITECTURE.md). Tool: `python3 -I scripts/research_workflow.py` (`check`, `status`, `ready`).

Pick the skill by situation ([stages](planning/workflow/stages.md)):

- Resuming, or unsure which stage a package is at: `research-workflow` (reads state, dispatches).
- A new request to capture: `request-intake` (S0).
- One concept: `research-prepare`, `concept-research`, then `notes-reconcile` (S1-S3).
- Parent, fan-out or orchestration issues: `issue-handoff` (S4-S5).
- Design, plan, build: `design-orchestrate`, `feature-plan`, `implementation-orchestrate` (S6-S8).

Orchestrators do not implement: they merge, keep state and issues current, and start workers.

### Parallel writers and subsessions

Full text: [`doc/agents/parallel-writers-and-subsessions.md`](doc/agents/parallel-writers-and-subsessions.md);
reasons in [`planning/workflow/collaboration.md`](planning/workflow/collaboration.md).
In short: worktrees always (boot the git env, push the empty branch
first), one writer per file, add-only records are new files, never
`git stash` / force push / rebase / `git add .`, merge commits only.
Subsession work is scoped to the context it needs, launched from its
own worktree directory; the parent verifies what a child did from its
diff; detached sessions never call the `question` tool; a session
cannot change its own model — launch a subsession on the needed head.
