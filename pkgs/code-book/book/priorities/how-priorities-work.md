# How priorities work

The merged list is not a wishlist sorted by enthusiasm. It is the output of
a stated doctrine, applied once across the whole project on 2026-11-23/24
and recorded so the next re-ranking can argue with the reasoning instead of
restarting from taste. This chapter states the doctrine: the item types,
the tier logic, how items enter the list, and how reprioritisation is
recorded — with real examples from the census that built the list.

## The four item types

Every item carries one type, and the type says what "done" means:

- **Capability set** — a capability set to reach. Done when the capability
  works end to end, not when its first slice lands. Example: item 8, Atlas
  ingestion fully automated, is not done because a staging tier exists; its
  acceptance is continuous ingestion with a recount procedure.
- **Problem** — a problem to solve. Done when the failure mode stops
  occurring, with evidence. Example: item 1, queue correctness, is measured
  in repaired packets, closed defect issues and proven placement criteria,
  not in effort spent.
- **Milestone** — a milestone. Done at a verifiable event. Example: item 6,
  the Talos cluster booted live and accepting data — a state a reader could
  in principle check, not a percentage.
- **Shift goal** — a goal that marks a shift. Done when the new way is the
  default and the old way is retired or demoted. Example: item 3, one honest
  transport — the accreted per-surface transports are the old way, and the
  item is not done while three programmes still build their own.

## The tier logic: substrate before surfaces

The seven tiers are a dependency argument, not a ranking of importance:

0. **Execution substrate (fleet).** The queue, build capacity and transport
   carry every other item's work. A defect here corrupts evidence
   everywhere, so it ranks first regardless of how unglamorous it is.
1. **Physical substrate (lab).** The cluster is assumed by the in-cluster
   build item, by self-hosted ID servers and by the local backup leg.
   Physical work is also the slowest to start, so its prerequisites
   (switches, presenting servers) rank ahead of everything that consumes
   them.
2. **Data substrate.** Backups before ingest automation before coverage
   before placement: each step mutates or moves the corpus, and the order
   guarantees a restore story exists before the mutations begin.
3. **Search intelligence.** Atlas and ID together, after the data they eat
   from is trustworthy. The census states the reason for item 13's place
   explicitly: a reranker evaluated on a frozen, provenance-poor corpus
   would bake in today's gaps.
4. **ID platform build-out.** Store before hosting before federation before
   shells: each item is written against the interface or service of the one
   before it.
5. **Gateway, PM and fork surfaces.** Valuable, heavily used — and consumers
   of the tiers above rather than prerequisites for them. Item 28 (the
   trust and capacity layer) is the clearest case: placed late
   deliberately, because the manual doctrine it systematises already works.
6. **The record itself.** Books and design registers describe and gate the
   work; they do not precede it. Item 30 is last by construction.

Within a tier, dependency order wins; ties break on leverage — how much
other work the item unblocks. Item 12's rationale is the pattern: it ranks
first in Tier 3 because items 13 and 15 both presume it.

## How items enter the list

The 2026-11-23/24 list was built by merging, in one pass:

- Drew's seed list, filed on the programme tracker #13905 — the seeds are
  marked in Census B and keep their seed status in each book's chapter.
- The four programme investigations of 2026-11-23 (fleet #13272, Atlas
  #13241, ID #13240, mcpx #13264), each a full census of its system.
- The newer programme issues named in the brief (#13779, #13752, #13844,
  #13765, #13774, #13880).
- The ID-100 list (46 newly enqueued items, 29 already tasked, a sweep to
  82 of 100).
- The AUDIT-7 umbrella (#13871) — its lanes are cited, not re-derived: Lane
  1 swept 199 open PRs and merged 101; Lanes 5 and 6 counted 312 issues
  demanding design; Lane 7 censused 18,594 stray-work items.
- One item entered during the census itself: #13880, the ID-apps epic, was
  filed 2026-11-24T00:09Z while Census B was being compiled, and was taken
  in as item 15 rather than left for a future list.

An item that enters later enters the same way: a tracker or census names
it, it is typed, it is placed by the tier logic, and the placement rationale
is written down at entry. A long-term direction that belongs inside an
existing item rides that item's line instead of inflating the list — the
multi-transport goal Drew recorded on 2026-11-23 rides item 3 (see [The
merged priority list](the-merged-list.md)).

## How reprioritisation is recorded

Reprioritisation is a recorded event with a reason, never a silent edit.
The census carries these examples, and this book preserves them as the
pattern:

- **Sequenced behind a sibling by verdict.** Item 20's scaling path is
  explicitly sequenced behind currencies v1 by standing triage verdicts —
  recorded as a reprioritisation (v1 first), not as neglect.
- **Correctly blocked, by name.** Item 26's backlog-derivation work (#5007)
  carries a verdict of record: correctly blocked on phase ordering — do not
  start yet. The list respects the verdict instead of ranking the item
  higher to make it move.
- **A miss, recorded as a miss.** Item 6's predecessor cluster was torn
  down (confirmed 2026-10-26); the list records a reset, not a delay.
  Item 8's miss is likewise on record: canonical counts froze at the
  2026-10-31 publish because ingestion stayed manual, and the freeze is the
  stated reason the recount procedure is an acceptance item.
- **Gates, not dates.** Several items were paced at census by platform
  flag holds (#13106, #13084, #13036, the #10237-chain close).
  Correction (2026-11-23, re-verified against live state): those holds
  have since cleared — #13106, #13084, #13036 and all three of the
  #10237 → #10252 → #10269 chain closed as completed on 2026-11-23.
  The census records them as gates with reasons, and — per standing
  rule — does not re-ask them as a batch. Where the record has no timing, the census says "no timing in
  record"; 22 of the 30 items carry no projected date, and the list prints
  none for them. The one hard near-term date in the census is item 28's:
  the quota observation interval can first elapse 2026-11-30 at 02:30:06.420
  UTC.
- **A near-miss of process, named as such.** Item 26's #9335 landed its
  acceptance and was live-verified 2026-11-13, then sat awaiting close —
  recorded as a process near-miss, not as unfinished work.

## Keeping the list honest

Statuses use a fixed vocabulary — done, in flight, designed, not started —
each with its evidence. "Designed" means a design of record exists and
implementation has not landed; "in flight" means code or queue work is
actively landing. Where a seed has no single owning issue, the list says so
plainly (items 4 and 23) rather than inventing one — both have since
been filed (2026-11-23): #13920 owns item 4 and #13918 owns item 23.
The re-census rule —
when this list is next rebuilt — is in [Keeping this book
current](../development/keeping-this-book-current.md).
