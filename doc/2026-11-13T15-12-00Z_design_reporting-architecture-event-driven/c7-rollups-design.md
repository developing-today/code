# C7 — Rollups, design pages, per-project spaces, reports timeline

Status: component design and implementation sequence; consumes the program
design in [`note.md`](note.md), C6 page mechanics, #9060 genealogy, and C8
Atlas ingestion. This document does not re-specify those components.

## Decision summary

1. **Period boundaries use `America/Chicago` calendar time.** Daily periods
   are `[00:00, next 00:00)` CT; weekly periods are Monday 00:00 to the next
   Monday 00:00 CT; monthly, quarterly, and annual periods use calendar
   boundaries in CT. Generate the initial period document at its opening
   boundary and refresh it as source events arrive. The amendment's
   midnight-vs-08:00 ambiguity is resolved in favor of midnight: the existing
   daily convention and S4.8 explicitly say midnight-to-midnight. 08:00 is
   only the normal end of the daily soft-edit window, not a period boundary.
2. **Reports are permanent, versioned records with a mutable current view.**
   Store structured source records and append-only event history; render
   current pages from those records. At rollover, freeze a dated snapshot in
   an unpinned library space. Never silently rewrite a frozen snapshot.
3. **Automated updates append; they do not overwrite human-edited prose.**
   Amendment KV arrays use indexed child keys and an `in` append operation
   with duplicate detection/optional locking. Each rendered block has a
   stable unique ID, related timestamp, added timestamp, and optional links
   to every report receiving it. If the current rendered block differs from
   its stored form (whitespace-insensitive), automation flags conflict and
   skips it; the reader can compare, choose either side, edit each, or
   combine. Automation never edits prior KV array entries.
4. **Status counts are point-in-time and explicit.** `done`, `in-progress`,
   `blocked`, and `queued` come from canonical PM-stack issue state at the
   report's `as_of` time. Exclude cancelled/duplicate closures from
   implemented-done counts and report them separately. Every count links to
   its members; unknown/unmapped state is shown separately, not guessed.
5. **C7 owns composition and navigation, not source-of-truth ingestion.**
   C7 consumes permanent PM-stack data and #9060's genealogy, C6's mobile
   page mechanics, and C8's search/embedding. Report links remain metadata
   links and data returned to C7 is source-attributed and timestamped.

## Page and record model

Each report record has a stable ID, kind, project/domain, period start/end,
timezone, `as_of`, template version, generation time, source cursor, ordered
event/block references, human edits, conflict state, and finalization state.
The current view is a projection; the event log and prior snapshots are
permanent. Late-arriving outcomes are recorded on the new period with a link
to the originating item/period and an explicit confirmed/disproved/changed
relationship. They do not mutate old closed narrative except through the
restricted post-window operations below.

### Rollups and templates

- **Daily:** one page per date and project, plain-language prose answering
  what changed, what pivoted and why (named issue/design), and permanent
  remediations. Include timestamped activity with hourly/event-log grouping,
  linked quota and generic reports, excerpts/media where available, and
  bubbling candidates for the weekly. A daily period opens at midnight CT;
  its soft edit window ends at 08:00 CT the next morning (the standing
  convention selected from the 02:00/05:00/08:00 examples).
- **Weekly:** Monday-to-Monday CT. Roll up daily narratives backward with
  reach-back context through the prior week's Friday, including later use
  and issues caused by prior work. Sections: handled last week, biggest
  changed issues/designs, planned next week and forward-day sketches, queue
  prioritization/custom report, and links to spaces/issues/PRs. Preserve
  project prose and an aggregate overview. Soft window: 2 calendar days.
- **Monthly / quarterly / annual:** calendar CT rollups. Monthly and
  quarterly windows are 7 days; annual is a new amendment family and uses a
  7-day window after year end. Include roadmap/features, struggles, process
  improvements, cost/capability changes, traffic patterns, surprises and
  learnings. Monthly ideas/design report covers started/fully designed/
  closed designs, implemented vs cancelled, automation suggestions, idea
  kernels/rejections/alternatives and design-mill prose with visual artifacts
  as available. Annual adds plan/SaaS/core statistics, dollar and PR
  efficiency, major changes, mistakes/surprises, and next-year ideas.
- **Dynamic period variables:** template variables resolve once per report
  generation and are stored as numeric ISO values (e.g. `2026-11`) for all
  references in that report. The amendment's before/after-15th previous vs
  current month is a _suggested selection policy_, not an implicit default
  period: generation commands require an explicit period or use the latest
  closed period. This avoids a report changing identity based on generation
  date. A caller may request the suggested policy explicitly.
- **Generation surface:** expose explicit fleet generation commands for
  daily/weekly/monthly/quarterly/annual reports. Re-running is idempotent for
  a given period/template/source cursor and updates the living current view;
  it does not duplicate events or replace manually edited blocks.
- **Bubbling:** each day marks weekly candidates, each week monthly
  candidates, each month quarterly, each quarter annual. Importance defaults
  to the selected grain and lower grains; explicit level sets override.
  Quarter pages begin collecting from quarter start.

### Late edits, conflicts, snapshots

Before each grain's soft-window deadline, normal edits and detail completion
are permitted, with every edit versioned. After that deadline only: (a)
naive quote blocks with source/date, (b) append-at-end entries, (c) a top
banner pointing at a later correction, (d) event-log injections in their
chronological slot, or (e) post-finalization feedback. Late outcomes belong
to the current page and cross-link the earlier hypothesis. Snapshot at period
close into a dated, unpinned, frozen library item named with original page
name plus date/week/year-quarter. A correction to a snapshot is a new linked
correction record, never an in-place edit.

The amendment's KV start is the minimum durable write layer: scalar facts
use `fleet kv set/put`; period families include `major_items`, `predictions`,
`summary`, and `related_items`; array elements use indexed keys, with
automation writing only through the prefix-based `in` append operation.
Array display supports index/reverse/date ordering, grid common-key columns
plus catch-all JSON, arbitrary known-property sort/column visibility, and
expand/collapse of nested values. Conflict UX provides find/resolve,
either/or, edit-each, git-style compare, and reviewer-assisted combination.
The surface defaults to prose and folded detail; this machinery is
progressive disclosure, not the default reading experience.

## Design pages, project spaces, and timeline

- **Design page:** one canonical page per design, linked to its parent/roadmap
  genealogy and origin. Render subdesign tree and why-needed; status ladder
  (`idea → planned → initial issue → issue built out → fully designed →
implementation issues`); implementation counts and linked members by
  done/in-progress/blocked/queued; reasons for blocked states; daily/weekly
  change notes. Long-running designs get linked monthly/quarterly pages.
  Genealogy and metadata are read from #9060/#9073/#9074, not duplicated.
- **Project space:** stable landing record for Fleet, PM stack, Atlas, and ID,
  with exists/wanted/in-progress/designed, inspiration/end state, open
  research, report/design links, and links to related spaces. A missing
  source section remains visibly unavailable rather than invented.
- **Timeline:** quarter → month → week → day, with current quarter visible.
  Overall activity is first and has 1–3 positives/negatives, folded details,
  and deeper inline expansion without navigation. Domain weekly columns cover
  ID; Fleet/queues/PM stack; Atlas/Turso/Cloudflare including LLM and
  tertiary-cloud quotas. Dated special reports attach to their day; designs
  show creation, phase start/end, and priority-pause milestones. Monthly and
  quarterly branches link their ideas reports. Search/tree discovery spans
  names and text; C6 supplies the page index and mobile mechanics.
- Event log grouping puts classified count blocks between major items.
  Issue closures classify implemented vs cancelled/duplicate; PR states are
  separate. Default grouping is natural per-project lines with expandable
  issue/PR/task counts. A per-period exact-boundary checkbox records the
  chosen grouping when precise ordering matters; uncertain boundaries carry
  source timestamps and are never presented as exact.

## Statement disposition map

| Statement | Design element / disposition                                                                             |
| --------- | -------------------------------------------------------------------------------------------------------- |
| S3.17     | Timeline, tree/sitemap, and full name/text search; see C6 for shared index implementation.               |
| S4.1      | Daily, weekly, monthly series; quarterly and annual added by the issue/amendment.                        |
| S4.2      | Frozen dated period snapshots in unpinned library spaces.                                                |
| S4.3      | Four named project spaces and required state/link sections.                                              |
| S4.4      | Per-project daily prose, pivots and durable remediations.                                                |
| S4.5      | Weekly sections and linked work/plans.                                                                   |
| S4.6      | Backward daily rollup plus handled-last-week/planned-next/forward sketch.                                |
| S4.7      | Prior-Friday reach-back with usage and downstream issues.                                                |
| S4.8      | Midnight-to-midnight CT; 08:00 next morning selected as soft-close deadline.                             |
| S4.9      | Late outcome appears on new report and links to old hypothesis.                                          |
| S4.10     | Weekly 2 days; monthly/quarterly 7 days (within requested ranges); annual 7 days as amendment extension. |
| S4.11     | Restricted post-window forms and immutable snapshots/correction records.                                 |
| S4.12     | Out of C7; C8 ingests C7 pages and exposes Atlas search.                                                 |
| S4.13     | Design pages, genealogy, rationale, status ladder.                                                       |
| S4.14     | Point-in-time linked status counts, child-to-parent rollup, reasons and daily/weekly updates.            |
| S4.15     | Monthly/quarterly pages for long-running designs.                                                        |
| S4.16     | Linked quota/generic pages, excerpts/media and timestamp/event-log rollup.                               |
| S4.17     | Tree/sitemap plus name/text search; C6 supplies shared index mechanics.                                  |
| S4.18     | Permanent PM-stack-backed records, custom/report index, troubleshooting grouping, timeline.              |
| S4.19     | Nested timeline, overview first, fold/unfold in place.                                                   |
| S4.20     | Domain weeklies as adjacent columns/subitems.                                                            |
| S4.21     | Special reports attach to date; designs expose phase milestones.                                         |
| S4.22     | Monthly/quarterly thematic rollups.                                                                      |
| S4.23     | Monthly idea/design report and visual artifacts, progressively detailed.                                 |

## Implementation sequence and verification

This is intentionally split: C7 spans storage semantics, generation,
multiple page families, timeline, and integrations. Recommended leaves under
#9575 (all must link back to this design):

1. **Canonical report records, KV append, period generation and snapshots**
   (foundation; includes stable IDs, idempotency, conflict detection, CT
   boundaries, soft-window enforcement). First implementation leaf; all
   renderers depend on this.
2. **Daily/weekly/monthly/quarterly/annual rollup composer** (requires 1;
   templates, bubbling, classified events, reach-back, explicit generation
   CLI, worked Modal example).
3. **Design pages and implementation rollups** (requires canonical records
   and genealogy read interfaces; status-count oracle fixtures include
   cancelled/duplicate exclusion).
4. **Project spaces and reports timeline/index integration** (requires 1–3;
   C6 index/mobile and C8 ingestion are interface dependencies, not reworked).
5. **Edit-window conflict and correction UX** (can begin after 1; late
   operation allowlist, immutable snapshots, either/or/diff/combine behavior,
   indexed-array grid controls).

The order is a hard dependency chain for the first two; leaves 3 and 5 may
proceed in parallel after 1; leaf 4 follows 2 and consumes 3. Do not mark
the component complete until read-back proves at least one real-day page and
its weekly parent are generated from stored data, one full design page has
linked member counts, current-quarter timeline is discoverable, and negative
tests reject off-window arbitrary rewrites, duplicate append, silent
snapshot mutation, guessed issue-state counts, and Atlas/C6/C8 ownership
duplication. Fixtures include a late-arriving confirmation/disproof and the
Modal two-day living item from the amendment.

## Open implementation details (defaults)

- PM-stack API names and storage schema are determined by leaf 1 after
  inspecting existing permanent record APIs; no parallel report database.
- Exact grouping boundaries default to source event time and deterministic
  `(timestamp, stable_id)` ordering; exact-mode records the user-selected
  boundary. This gives stable reruns without claiming uncertain chronology.
- Annual, explicit generation command, per-period KV family and conflict
  controls are amendment additions. They are kept in the design but can be
  sequenced after the core four-grain experience if delivery requires it.
- C6 owns anchors/comments/template markers and mobile layout. C8 owns
  ingestion, embedding and Atlas search. C7 tests the integration contracts
  but does not duplicate those implementations.
