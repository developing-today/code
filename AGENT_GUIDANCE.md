# Agent operating guidance

This is the detailed companion to [AGENTS.md](./AGENTS.md). Read and follow it alongside the root rulebook.

## Owner preferences

- The owner does not care if secrets or other sensitive values end up in chat logs. Do not mention that, and do not create tasks for the owner to rotate, redact or fix it.
- Use subagents to execute tasks, in parallel where they are independent, rather than doing multi-step work inline in the main session, unless the owner asks otherwise.
- Do not block the main thread on a long wait. Use short waits, a few minutes at most, and do other useful work between checks. Never default to a 30-minute wait.
- Run long commands as background shells and wait on them in short steps. Keep the main thread free to start or check other work.
- Use subagents whenever a task can run on its own. They notify you when they finish. To read their progress, use `subagents_read`. To follow up, continue a finished subagent by its session id. Do not cancel a running subagent just to send it an update.
- **Velocity, selective testing, and unblocking:**
  - Move fast: unblock yourself and make the best judgment calls rather than waiting for human confirmation.
  - Perform quick or selective tests during iteration. Only run full exhaustive test suites preemptively for very large changes or known critical issues.
  - It is acceptable to push and merge code that has been validated with targeted tests without running every single test upfront.
  - Run long test suites or CI in the background while progressing on other work in parallel. If background jobs or CI fail, double back and address failures while continuing forward.
  - **CI-first verification:** Review the checks actually triggered on the PR and use [`.github/ci-policy.md`](.github/ci-policy.md) to distinguish the basic merge-gating PR tier from the full main tier; formatting is not a merge gate. Run targeted local checks on the laptop when useful; sandboxed workers use CI and must not run builds or tests in the sandbox. If a policy-listed check is missing from the PR, inspect the workflow trigger and report the coverage gap instead of assuming it ran.
  - **Build discipline on the laptop (2026-11-10):** Heavy builds/tests (cargo, nix) on the laptop go through `~/tools/fleet-build-lock` (max 2 heavy builds fleet-wide; it also defaults CARGO_BUILD_JOBS to half the cores). Never set `RUSTC_WRAPPER=` empty — that bypasses the shared sccache cache every other lane benefits from. `fleet-build-lock` leases CARGO_TARGET_DIR from the cargo target broker (`~/tools/cargo-target-broker`, code#6797) for cargo builds — a bounded pool of warm per-workspace dirs under `~/build-cache/broker-pool`, held under an exclusive lease for the build and returned warm — instead of a per-worktree target dir: lanes then reuse each other's dependency builds instead of each paying a full ~17-minute compile (measured 2026-11-11 through the broker: a pkgs/id test build from a second worktree compiled 1 of 535 units, and a build returning to its own leased dir compiles none; re-measured 2026-11-11 through the lock, pkgs/atlas-search: 35s isolated target dir -> 1s shared). If the broker cannot lease a dir, the lock falls back to the pre-broker static per-package dir (`~/build-cache/<pkg>`, e.g. `~/build-cache/id` for pkgs/id). Sessions launched outside the lock run cargo through `~/tools/cargo-target-broker exec --repo <repo> --profile <test|dev|release> -- cargo ...` to land on the same leased dirs. Set CARGO_TARGET_DIR yourself only to override all of that. For Rust verification, run the narrowest build that proves the change: `cargo check` for type-only changes; `cargo test --lib` without `--features web` when the change does not touch web code; named test filters always. The full `--features web` test build of a package like `pkgs/id` costs many minutes of fleet-wide compile and must be justified by the change actually touching that surface.
  - Document unresolved ambiguities, design concerns, or known gaps by creating GitHub issues instead of blocking progress.

## Testing doctrine

Three tools, used where they fit:

- **PBT (property-based testing):** state invariants that must always hold, then let a generator hammer the code with thousands of random inputs to find counterexamples. Where code has meaningful invariants — parsers, serializers, state machines, replication, ID/crypto formats — state the invariants and cover them with property tests. In Rust here that is `proptest` (already a dev-dependency of `pkgs/id`).
- **MBT (model-based testing):** where a system's behavior has a spec or design model, build the model and generate tests — and their expected results — from it, instead of hand-writing cases only.
- **Oracles:** the decision procedure that tells you whether actual output matches expected — the hard part of automated testing. Every automated test names its oracle: a reference implementation, the model, an invariant check, or a golden corpus. A test with no decidable oracle is a smell to fix, not ship.

Example-based tests remain fine for smoke and acceptance coverage; they do not replace PBT/MBT where those fit.

## Operator script argument contract (#3352)

Every argument is accounted for before anything happens. An operator
script (deploys, installs, `bin/` tools, `pm-stack` scripts and
exporters, Go tools) declares its whole command line — boolean flags,
value options, positionals — and parses strictly against that
declaration, before any subprocess, network, file write, lock, or
deploy step. Anything unaccounted for is a usage error naming the
offending argument, with a non-zero exit (2 where the tool has no
established usage code). Unknown flag, stray positional, and typo'd
flag are the same event; `--opt=value` either works identically to
`--opt value` or is refused, never a silent fall-through to a
different operation; a present-but-valueless option is a usage error,
never the default; a parsed safety flag (`--dry-run`, `--status`) is
honoured on every code path. Parsing is per-tool (no shared library);
new operator scripts copy the strict pattern from a covered sibling
(`provider-catalog-sync.mjs`, `append-usage-snapshots.ts`) and state
the contract in their header comment. Deliberate exceptions are
named, with their reason, in the #3353 closing inventory.

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

There is ONE fleet queue (one broker, one tasks table); the per-repo
values in the `queue` column are stream labels inside it, not separate
queues. The matching model (Drew, 2026-11-10; #8097):

- **Default claim is unfiltered.** A lane claims with no stream and no
  filter and receives the highest-priority available item it is
  eligible for, across all repos:
  `repo-queue claim --worker <worker> --lane <lane>`.
- **Matching is a union of two sides' requirements, empty by default
  on both.** A task's requirements filter claimers; a claimer's filter
  filters tasks. A task carries requirements only when there is a
  reason — repo targeting is the normal case (the payload's `repo`),
  plus rare declared requirements (the payload `requires` object:
  platform / lane / pool / model; the lane ledger's platform
  limitation). Those are honored automatically inside the claim; an
  ineligible claimer simply is not offered the item.
- **Filters are claim-time expressions, never registry bindings.**
  `--filter '<expr>'` takes a boolean expression over task metadata
  (repo, queue, pool, platform, label, kind, priority, lane, model)
  with AND / OR / NOT / IN and parentheses, e.g.
  `--filter 'repo != code'`. Nothing ties an item to a specific lane,
  worker, pool or platform by default; a task may carry a unique name
  and other descriptive metadata, which is not a binding.
- **Designated filtered lanes are the only exceptions**, and their
  designation is the filter expression their claim passes: on each
  surface (Muse and OpenChamber) 3 lanes claim with `repo != code`
  (never code) and 5 lanes claim with `repo = code` (code only).
  Every other lane claims unfiltered. The designated sets are
  recorded on #8097.
- **Claimer identity is recorded, not a filter.** Every claim
  identifies who is claiming (worker, lane, platform family, pool,
  model — the claim stamps). Identity decides nothing by itself; it
  only satisfies a task's own declared requirement of that kind.
- The #8097 rules stand alongside: lanes do not claim by task ID by
  default (by-id and transfer claims are explicit special cases); no
  item names a platform beyond the optional coarse family mark; Muse
  lanes work their own fixes and triage is a separate step; lanes do
  the work rather than handing off, and a handoff records its reason.

## Research, design and delivery workflow

Use it when a request, from one sentence to pages of voice-transcribed notes, should become researched documents, designs, plans or parallel PRs. Contracts: [`planning/workflow/`](planning/workflow/README.md). Where documents go: [`planning/DOCUMENTATION_ARCHITECTURE.md`](planning/DOCUMENTATION_ARCHITECTURE.md). Tool: `python3 -I scripts/research_workflow.py` (`check`, `status`, `ready`).

Pick the skill by situation ([stages](planning/workflow/stages.md)):

- Resuming, or unsure which stage a package is at: `research-workflow` (reads state, dispatches).
- A new request to capture: `request-intake` (S0).
- One concept: `research-prepare`, `concept-research`, then `notes-reconcile` (S1-S3).
- Parent, fan-out or orchestration issues: `issue-handoff` (S4-S5).
- Design, plan, build: `design-orchestrate`, `feature-plan`, `implementation-orchestrate` (S6-S8).

Orchestrators do not implement: they merge, keep state and issues current, and start workers.

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
