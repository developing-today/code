# The story so far

This chapter is the project's history as the records support it, rolled up
by month and quarter. Its source is Census A (the retrospective census,
compiled 2026-11-23, landed as PR #13928), whose spine is GitHub's merged-PR
record, corroborated by the dated records under `doc/`, the daily memory
notes, and the stewardship cycle log. [Week by week](week-by-week.md)
carries the weekly record; [Capabilities: what we can do
now](capabilities.md) carries the dated capability timeline.

## The honestly covered period — read this first

The sources do not cover the same ground, and this book does not pretend
they do.

- **Fully supportable, week by week: 2026-10-19 to 2026-11-23 (W43–W48).**
  GitHub, the daily notes, the cycle log and `doc/` all overlap.
- **Supportable but thinner: 2026-09-28 to 2026-10-18 (W40–W42).** The daily
  notes and `doc/` carry it; merged-PR counts are near zero, so "what
  shipped" claims rest on design records and notes.
- **Design-record only: 2026-03-20 to 2026-04-01 (W12–W14).** Eight dated
  `doc/` records and nothing else. The records are dated March–April, but
  the GitHub spine shows no merged PRs in that window at all, so they read
  as design work later committed under its original dates, not as a
  verifiable March build history. The primary source wins.
- **Uncovered: 2026-04-02 to 2026-09-27 (W14–W39).** No dated records, no
  daily notes, and zero merged PRs were found in any mined source for those
  25 weeks. This book states that as a gap. It does not reconstruct it, and
  it does not describe the period as either quiet or busy — there is no
  evidence for either word.
- Inside the covered period there is one smaller gap: no daily notes exist
  for 2026-11-13 to 2026-11-21. GitHub and `doc/` cover those days; the
  weekly entries for W46 and W47 rest on them and say so.

The shape of the whole record, in one line: eight March–April design
records, a 25-week silence, and then an exceptionally dense eight weeks —
2026-09-28 to 2026-11-23 — in which the fleet, Atlas, the PM stack and the
books were built out. The verifiable substance of 2026 is the autumn.

## Month rollups

### March 2026 — design records only

Six dated records (W12–W13): collaborative cursors, content modes, peer
discovery, file-list UI, metadata tags, testing architecture. No GitHub,
memory or log corroboration exists. The honest March rollup is one
sentence: design records exist under March dates; nothing else about March
can be verified from the mined sources.

### April 2026 — two records, then silence

Two records dated 2026-04-01 (image upload, multi-instance), then no
evidence of any kind until 2026-09-28. April cannot carry a fuller rollup.

### May to September 2026 — uncovered, except September's last four days

No mined source covers May through 2026-09-27. September's covered tail is
W40 (from 2026-09-28): the nix/mcpx watch-and-rebuild programme — a
15-minute loop tracking upstream mcpx, with milestone rebuilds on each
upstream release — and the Nix persistence work. A "September" rollup would
be a rollup of four days, and is labelled as such here.

### October 2026 — the platform takes shape

Covered 2026-10-05 to 2026-10-31, dense from 2026-10-19. The ID/worlds
platform design landed as phased records (phases 0–8, W41–W42); the PM
aggregator gave the project its first live dashboard (W43); the full queue
cutover, Atlas on Turso in production, and the live `/quota` loop all
landed in W44. October's merged-PR spine: 2, then 59, then 304 in the code
repo across W41–W43, with 946 spanning Oct 26 – Nov 1, and the forks at
181–366 merges per week in the same span. The month's hardest problems were
mcpx daemon lifecycle correctness and a deploy chain repeatedly blocked by
mcpx and ID test breaks. Its institutional fact: stewardship began as a
standing hourly cycle on 2026-10-24 and has run ever since. What we could
do on Oct 31 that we could not on Oct 1: run work through a fleet queue,
watch it and its quota cost live, and search a production corpus.

### November 2026 — fleet-owned infrastructure, then the books

Covered 2026-11-01 to 2026-11-23, with daily notes missing 2026-11-13 to
2026-11-21. The fleet built its own infrastructure: laptop-agent, the queue
and secrets brokers, and the repo-queue to fleet migration (W45); the lab's
first Cloudflare Tunnel (2026-11-06); queue durability — a SQLite
coordination replica and a Turso-to-D1 backup with restore drill (W47); and
the documentation capstone on a single day, 2026-11-23: three books
completed (fleet's already stood; Atlas at 57 chapters, ID at 67, mcpx
audited and aligned) and four investigation programmes run. November's
spine to Nov 23: about 1,554, 1,152, 1,142 and 526 merged PRs in the code
repo in W45–W48. The month's hardest problems were migrations (Supabase to
Turso, repo-queue to fleet), the laptop channel collapse and rebuild
(W47), and the platform execution-risk flag regime (W48). What we could do
on Nov 23 that we could not on Nov 1: operate the fleet end to end on
fleet-owned infrastructure, and hand a new reader complete books instead of
a pile of design records.

## Quarter rollups — what they can honestly say

- **Q1 2026 (Jan–Mar):** only W12–W13's six design records fall in this
  quarter, all in its last two weeks. January and February are entirely
  uncovered.
- **Q2 2026 (Apr–Jun):** two records dated 2026-04-01; the rest is
  uncovered. No honest Q2 rollup exists beyond that sentence.
- **Q3 2026 (Jul–Sep):** uncovered until 2026-09-28. The quarter's only
  supportable content is W40's first days.
- **Q4 2026 (Oct–Dec, in progress):** the quarter the record actually
  describes — 2026-10-05 to 2026-11-23 of it so far, with the caveats
  above. A Q4 rollup built from the October and November rollups is honest.
- **Year 2026:** a year rollup cannot honestly be written from these
  sources, and this book does not write one. What exists is stated above:
  eight design records, a 25-week silence, and eight dense weeks.

## The arc

Read across the covered weeks, the project's centre of gravity moved three
times. First, design: October opened with the ID/worlds platform designed
on paper before it was built. Second, substrate: the queue cutover, the
brokers, laptop-agent and the migrations replaced borrowed infrastructure
with fleet-owned infrastructure, because agent work at this volume cannot
run on per-tool scripts and other people's queues. Third, the record: once
the substrate held, the project spent a deliberate burst — all of
2026-11-23 — turning its pile of 619 dated design records into books a
reader can actually use, and censusing every programme into the merged list
this book carries. The next arc is the merged list itself: substrate first,
then the physical cluster, then the data and search work that the autumn
designed but did not finish.
