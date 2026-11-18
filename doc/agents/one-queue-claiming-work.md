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
