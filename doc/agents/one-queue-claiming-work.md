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

