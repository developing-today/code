# Census and architecture checkpoint — task 15381

**Active:** `summit-direct`, session `ses_e5e2d10e06effeEylSDDVaN0PL`.
Documentation batch: https://github.com/developing-today/code/pull/9225
Branch: `design/pm-orchestration-buildout`; last pushed architecture commit `3472c8f08a`.
Census pin remains `a038f084dbeb525494d7bb928ad91f597db2ac51`.

## Findings that change the design detail, not its scope

- Distributed priority has a concrete current compatibility gap: pinned
  `pm-stack/dispatch/src/validate.ts` snaps `15→10` and `25→20`, while dated #7030
  ratification and broker readback preserve `expedite=15` / `next-up=25`. This is a
  producer/consumer contract problem, not permission to renumber the live queue.
- Claim SQL orders by priority and creation time without a final ID tie; list adds an ID
  tie. V1 must not be advertised as already satisfying the proposed stable total order.
- The existing PM queue view is a board/Project projection, not live broker claim order.
- Current worker-ID leases and bounded creation-outbox mechanisms are useful foundations,
  not proof of monotone fencing, full lifecycle commands, or direct verified completion.
- #6533 remains the separate live launch-and-bind acceptance fixture; #8187's closed
  baseline and a merged fix do not discharge it. #7030's priority brief is already ratified.

## Artifacts and quality checks

- Five-source preservation ledger committed: 3,571 mechanically segmented occurrences,
  hashes and exact character/byte spans. Its checker passes. This is **not yet** the curated
  atomic requirement crosswalk; that semantic pass is continuing, and no zero-unmapped
  implementation claim is made.
- Track A/B census and primary-source prior-art drafts produced; domain writers are expanding
  the versioned interfaces, failure cases, test oracles and granular issue specifications.
- Shared architecture committed: canonical existing broker; distributed authenticated
  commands; field authority; versioned observations; explicit state/lease/directive separation;
  internal modules in the existing Fleet package; no replacement infrastructure.
- Parent review has already caught and returned inaccurate CLI absence claims, grouped
  rather than per-operation command rows, an incorrect dependency-cycle predicate, and
  ambiguous requirements/filter union wording for correction before final acceptance.
- The initial generated ledger had redundant oversized section locators and Markdown
  whitespace. Compaction/correction is in progress; the batch will not merge on the seed's
  mechanical check alone.

QUESTION: Exact-name provenance for “Queue Table Improvements” is still unlocated after
repo text/history and GitHub searches. Default: continue autonomous recovery against #3424
and related schema records, retain a tracked research gap, and gate shared-schema changes
on a concrete compatibility disposition. Do not assume similarly shaped queue work is the
named original, and do not stall the rest of the design waiting for a human pointer.

**Designed / implemented / deployed / live-verified:** this run is producing design and
issue artifacts only. No production queue/dispatch/schema change or new service is being
made. #9196 and its implementation children remain open.
