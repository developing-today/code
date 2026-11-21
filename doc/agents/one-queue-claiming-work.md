<!-- Moved verbatim from AGENTS.md by task 15116 (issue #8430) so the root rulebook fits its 8,000-byte budget. -->

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

## Who holds an issue (assignee is not a hold)

Every open issue and PR here used to be auto-assigned to `dezren39`
(Drew) by a sweeper workflow, so the assignee says nothing about
ownership. Lanes read it as "Drew-owned" and parked work for an
"explicit ownership release" that never comes (#7616 collected 50+
park notes in two days). The rule:

- An issue or PR is **held for a human only** when it carries a hold
  label: `held`, `user/owned` or `do-not-auto-edit`. `do-not-merge`
  blocks merging, not working. "Drew-owned" in any runbook means one of
  these labels, nothing else.
- **Assignees never hold work**, whoever they are, and neither does a
  `work/assigned` label. Which worker holds a task is the queue's
  atomic claim, not a label or an assignee. Never park, and never wait
  for a "release" or "handoff", because of an assignee.
- **A PR or branch authored by `dezren39` is not Drew's either.** Fleet
  agents push and open PRs under that account, so authorship is fleet
  work product, not a hold. A PR is someone else's only if a live queue
  claim holds it or it carries a hold label.
- **There is no ownership handoff.** Never park "awaiting ownership
  handoff", "owner release", "owner coordination" or "reassignment",
  and never carry such a gate forward in a resume brief or queue
  checkpoint: nobody gives one. A queue task's own earlier branch,
  worktree or draft PR is its own work to inspect and continue. (On
  2026-11-21 seven parked tasks were still waiting on such a gate
  after the assignments were cleared; their checkpoints were reset.)
- Check before parking on ownership (exit 0 = workable, 3 = held):

  ```bash
  python3 pm-stack/lane-guard/lane_guard.py hold --repo <owner/repo> --issue <n>
  ```

- **One park note, not one per cycle.** Before posting a parked or
  picked-up agent note, run the note check and skip the comment on
  exit 3; record the park in the queue checkpoint instead.

  ```bash
  python3 pm-stack/lane-guard/lane_guard.py note --repo <owner/repo> \
    --issue <n> --task <task-id> --state parked
  ```

  When it says to post, start the note with the `header` line it
  prints. Progress notes (PR opened, blocked, completed) always post.

- A task too big for one run is not a reason to park without code:
  land the next reviewable slice as a PR (or a checkpointed branch)
  and park with that slice named as the resume point.

## Decision records bind this file

Every durable ruling behind the rules above lives in the fleet decision
records index, [`doc/decisions/README.md`](../decisions/README.md) —
resume-by-session (FDR-0001), the manual-claim override (FDR-0002),
multi-task claim and take-over (FDR-0003), manual-only items (FDR-0004),
run identity and attribution (FDR-0005), the park remediation ladder
(FDR-0006), justified count caps (FDR-0007), parks with a term and an
owner release (FDR-0008), the park reboot test (FDR-0009), and
completion evidence (FDR-0010).

**A change to claim, park, resume, take-over, stop-authority, cap, or
attribution semantics requires a decision record in the same body of
work** — the same PR, or the same run for non-PR work. Editing this
file without that record is an incomplete change, and a change that
contradicts an accepted record must supersede it rather than diverge
from it in silence.

## Implementation target and usable work window (2026-11-18)

`payload.repo` identifies the tracking issue's repository. When implementation
belongs elsewhere, producers set `payload.implementation_repo` to its explicit
`owner/name`. `--filter 'repo = code'` and `repo != code` select that implementation
target first, falling back to the tracking repo for ordinary legacy packets.
Issue references and duplicate-work identity continue to use the tracking repo.
A packet explicitly flagged `cross-repo-change` without this target is quarantined
at claim time with a repair reason, not handed to another mismatched worker.
Never infer the target from a title, or use an enqueue dedupe hit as a packet edit.

For a bounded session, pass `--remaining-budget-minutes <usable-minutes>` on EACH
claim, recomputing the remaining usable work window rather than passing the whole
initial window repeatedly. Producers declare `required_budget_minutes` for the
minimum uninterrupted time to the next safe checkpoint. Existing broker matching
excludes requirement-bearing tasks unless the declared remaining window fits;
a fitting worker can bypass a capacity park's timer. `timebox_minutes` is NOT an
estimate or a requirement. Do not invent an hours requirement from `effort/l`.
Ownership/handoff holds remain independent: a longer budget is not authorization.
See [FDR-0011](../decisions/FDR-0011-implementation-target-and-work-window.md).
