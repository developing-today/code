# AGENTS.md

## Web retrieval fallback

When retrieving public web content with `curl`, `wget`, Python HTTP clients, or similar tools:

1. Use the normal/default request first.
2. If the request returns a 403, 429, obvious bot-block page, or otherwise fails because of automated-client filtering, retry with a normal modern browser User-Agent:
   `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36`
3. If that still fails, try other well-known user agents — different sites block different agents, so it is worth rotating through several before giving up. Good next tries:
   - `WhatsApp/2.23.20.0` (link-preview fetcher; rarely blocked, often slips through news/paywall-adjacent CDNs)
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot`
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com`
4. A full sorted list of known user-agent strings, which operators publish IP ranges for verification, and a site-by-site access matrix (which agents worked or failed on which domains) lives in [`ai-crawler-site-access-table.md`](./ai-crawler-site-access-table.md#user-agents). Check that section for details and prior findings — including notes about the specific domain you are hitting.
5. Do not use altered User-Agent headers when testing or debugging the actual HTTP behaviour of an application, API, website, authentication flow, or client, because doing so could hide genuine access-control or compatibility problems.
6. Do not treat a successful response obtained with an alternate User-Agent as evidence that the resource works normally for ordinary clients.
7. Do not use User-Agent substitution to bypass authentication, account permissions, or other explicit access controls.
8. **Document what works.** When plain requests fail but an alternate path succeeds, add your findings to `ai-crawler-site-access-table.md` (the agent's subsection under _User Agents_, the domain's subsection under _Sites_, or _General retrieval tips_) at any time, regardless of what the current session is about, and include the update in your next commit. Failures are worth recording too: e.g. "UA X has known failures on example.com, but UA Y worked."
9. **Preserve scarce sources.** If a URL was genuinely useful and hard to acquire — very few copies online, or hosted somewhere unlikely to persist — submit it to `https://web.archive.org/save/<url>` (works unauthenticated). This applies to the kind of thing that has no mirror: datasheets/whitepaper/manual PDFs, technical docs on fragile personal or CMS-hosted pages, demo/example projects that exist only to illustrate how some code works, hard-to-find header files or source snippets not in any repo or package manager, one-off benchmark posts. It does **not** apply to widely-mirrored content (Wikipedia, MDN, popular repos) or anything already covered elsewhere — you can't and shouldn't archive every page you visit. Anonymous saves are rate-limited to roughly a few per minute; don't hammer it.
   - **Rare case:** if you are confident the site is still up but you are blocked or rate-limited, and you already know from some other source that the URL is valuable (a citation, an API doc you need, source code relevant to the task), you may submit it unseen and record that it wasn't directly accessible but was submitted on `<date>` — e.g. in the relevant _Sites_ subsection of `ai-crawler-site-access-table.md` — so a future agent can check `web.archive.org` for the capture. Only do this when there's a concrete reason the page matters, not speculatively.

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
  - Document unresolved ambiguities, design concerns, or known gaps by creating GitHub issues instead of blocking progress.

## Testing doctrine

Three tools, used where they fit:

- **PBT (property-based testing):** state invariants that must always hold, then let a generator hammer the code with thousands of random inputs to find counterexamples. Where code has meaningful invariants — parsers, serializers, state machines, replication, ID/crypto formats — state the invariants and cover them with property tests. In Rust here that is `proptest` (already a dev-dependency of `pkgs/id`).
- **MBT (model-based testing):** where a system's behavior has a spec or design model, build the model and generate tests — and their expected results — from it, instead of hand-writing cases only.
- **Oracles:** the decision procedure that tells you whether actual output matches expected — the hard part of automated testing. Every automated test names its oracle: a reference implementation, the model, an invariant check, or a golden corpus. A test with no decidable oracle is a smell to fix, not ship.

Example-based tests remain fine for smoke and acceptance coverage; they do not replace PBT/MBT where those fit.

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
