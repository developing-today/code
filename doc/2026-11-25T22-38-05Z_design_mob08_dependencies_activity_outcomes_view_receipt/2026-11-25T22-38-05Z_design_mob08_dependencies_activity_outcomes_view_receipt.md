# MOB-08 — Dependencies, activity, outcomes and explicit view receipt

Design record for #9475 (`pmo-9196:MOB-08`), parent #9201, program #9196.
It covers the read side of controls 14 and 15 of #9201 §3.3:
"inspect hard/advisory dependencies" and "inspect recent activity and
verified completion outcomes". It also covers the views those controls
feed: Blocking others, Parked/Blocked, Recently changed and outcomes.

This is a design record only. It authorizes no code. Section 9 lists
the gates for a separately scoped implementation issue.

## Governing decisions consumed

| Source                                                                  | What this record takes from it                                                                                                                                                                                                                       |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9201 §3.1 (component `9201-pm-projection-mobile-control.md`)           | `relations: { parent_refs, hard_blocker_refs, advisory_refs }`. Visible Done needs the completion policy's verified terminal receipt. Each view is a filter over one read model and discloses its filter and source freshness.                       |
| #9201 §3.5                                                              | One stale source: render what is available with a facet-level label. Absent is never zero.                                                                                                                                                           |
| #9207 §4 and the HYD-05 / HYD-06 records                                | Cursor identity. A cursor advances only after its page is durably committed. Activity identity is source-native, and an edit is a revision of the same item, never a new one. The four times are kept apart.                                         |
| MOB-01 record (`doc/2026-11-25T01-21-26Z_design_mob01_work_read_model`) | D5: `relations` come from DEP-03/DEP-04 edges when present, else `ProjectPoolItem.parentRef/blockedBy/blocks`, and provider hierarchy is advisory. D3: a locator is not an identity. `SourceRead<T>` keeps "not connected" apart from "read failed". |
| MOB-02 record                                                           | Board column is a projection with reasons. Blocked and parked keep their state; urgency never promotes them.                                                                                                                                         |
| REL-09 (`fleet-queue/src/work/completion.rs`, #9519)                    | A completion receipt pins an activity watermark and reports its freshness limit instead of claiming clean reconciliation.                                                                                                                            |
| #7016 and #8579                                                         | Task outcomes are durable truth keyed by task. Issue state is a GitHub mirror. #7016 counted 766 issues whose tasks were done while the issue stayed open: task completion and issue closure are separate facts.                                     |

## 1. Verified state on main `664c77ac`

- `pm-stack/aggregator/src/model.ts` `ProjectPoolItem` carries GitHub's
  three native relations: `parentRef`, `blockedBy[]` and `blocks[]`.
  It also carries `edgeDiagnostics[]` (an endpoint that was
  `unavailable` or `malformed`, kept so incomplete data differs from a
  ref outside the pool), `epicRef` and `historicalParentRefs`. A
  missing edge means GitHub has none, not an error. Refs are
  `owner/repo#n` display keys.
- `src/genealogy.ts` (#9070, closed) derives ancestors, spawned-by and
  release from verified pool edges and labels. It reads `parentRef`
  chains. It does not touch `blockedBy`.
- `src/orchestration-dependency.ts` is the wire vocabulary for a
  dependency's status. `blocked` there means a prerequisite that is
  itself waiting.
- `src/activity.ts` (#3321) ingests watchdog, Kaneo and GitHub hot-list
  deliveries. Each `ActivityDelivery` has a stable per-source
  `sourceId` (the dedupe key) and the source's `at`. A persisted
  per-source `SourceCheckpoint` and seen-id set make a repeat a no-op.
  `ActivitySourceStatus` keeps `cursor` (processed locally),
  `checkpointCursor` (durably acknowledged) and `highWater` apart, and
  its `state` is `unconfigured | current | lagging | outage |
unrecoverable`. Known outage windows become `GapRecord`s, rendered as
  holes and never as coverage.
- `src/outcome-attribution.ts` and `src/outcome-view.ts` (#10910) hold
  one immutable outcome fact per hot-list event, with `taskId`,
  `coverage: complete | partial` and source-gap facts. Rollups over
  partial facts are lower bounds. An empty ledger and a gap-only ledger
  are not zero activity.
- `src/work-projection.ts` (MOB-01's module) is **not on main**. The
  MOB-01 record merged, and #9468 is open with no code. So no
  `RelationSnapshot` or `PmWorkView` type exists yet.
- `pkgs/fleet/crates/fleet-queue/src/work/activity.rs`, which the issue
  names as a read-only consumer, **does not exist**. HYD-05 owns
  creating it, and HYD-06 (#9508) adds the revision model to it after.
- No typed edge model exists beyond GitHub's three relations. DEP-02
  (#9503, typed edge model and direction) is open with no merged
  record.
- `.github/workflows/pm-aggregator-tests.yml` runs `bun test` and
  `tsc --noEmit` on `workflow_dispatch` only.

Gap: each page that shows dependencies or activity reads these sources
its own way. No module says, for one work item, which edges gate it,
what changed since a point, and which completions are verified, with a
statement of which sources and cursors that answer rests on.

## 2. Intent

One pure module turns relation, activity and outcome readings into
three view sections and one receipt. It reads only what the callers
pass in. It writes nothing, persists no cursor and adds no endpoint.

## 3. Ownership

| File                                                                      | Role                                                                                                          |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `pm-stack/aggregator/src/work-relations-activity.ts`                      | Owned. The pure projections in §4 and the receipt in §4.4.                                                    |
| `pm-stack/aggregator/test/work-dependencies-activity.test.ts`             | Owned. The oracles in §6.                                                                                     |
| `src/model.ts`, `src/activity.ts`, `src/outcome-*.ts`, `src/genealogy.ts` | Read-only. Types and existing stores are consumed, never changed.                                             |
| `src/work-projection.ts`                                                  | MOB-01's. When it lands, MOB-01's `relations` fact is filled from §4.1's output; this slice does not edit it. |
| `fleet-queue/src/work/activity.rs`                                        | HYD-05 and HYD-06. When it exists, its records are another activity input to §4.2.                            |

The issue body's "original slice text" names `src/work-projection.ts`
and a proposed `src/work-activity-view.ts`. The curated owned-files list
above it names `src/work-relations-activity.ts`. This record follows the
curated list. `work-projection.ts` stays MOB-01's, so two slices never
write the same file.

## 4. Decisions

### 4.1 Relations: typed, one-way, never collapsed

`projectRelations(item, pool)` returns:

```ts
interface RelationEdge {
  kind: "parent" | "child" | "blocked_by" | "blocks" | "advisory";
  strength: "hard" | "advisory";
  target: RefKey | null; // null only for an unresolved endpoint
  target_state: "open" | "closed" | "merged" | "unknown";
  resolution: "in_view" | "out_of_view" | "unresolved";
  source: "github_native" | "dep_typed";
  observed_at: string;
}
interface RelationsSection {
  parent_refs: RefKey[];
  hard_blocker_refs: RefKey[]; // blocked_by edges whose target is not closed or merged
  advisory_refs: RefKey[];
  blocking_refs: RefKey[]; // this item's open "blocks" targets
  edges: RelationEdge[]; // every edge, sorted
  cycles: RefKey[][];
}
```

Rules:

1. **A parent is never a blocker.** `parentRef` becomes a `parent` edge
   with strength `advisory` and goes only to `parent_refs`. Children
   go only to `edges`. This is the slice's named red test.
2. **Only `blocked_by` gates.** A GitHub "blocked by" edge is `hard`.
   A hard blocker whose target state is `unknown` stays in
   `hard_blocker_refs`. An unreadable blocker is never treated as
   resolved.
3. **An unresolved endpoint is kept.** Each `edgeDiagnostics` entry
   becomes an edge with `resolution: "unresolved"`, and a hard one still
   counts as a blocker. A target outside the loaded pool is
   `out_of_view` with state `unknown`.
4. **Provider hierarchy is advisory** (MOB-01 D5). Epic membership and
   `historicalParentRefs` appear as `advisory` edges and never gate.
5. **Typed edges win when present.** When DEP-02's typed edges exist for
   an item, they are the edge set and GitHub's native edges for that
   item become evidence on the same edges, never a second copy. Until
   DEP-02 lands, `source` is always `github_native`.
6. **Cycles are reported, not resolved.** A `blocked_by` cycle within the
   pool is listed in `cycles`. Nothing here propagates urgency (that is
   #9200's), so a cycle can't escalate anything.
7. **Identity is the full ref.** `a/x#5` and `b/y#5` never merge, and an
   edge is matched on the whole ref, never on the number.

The **Blocking others** view is the items whose `blocking_refs` is
non-empty. **Parked/Blocked** uses `hard_blocker_refs` together with the
lifecycle fact. Neither view recomputes eligibility (MOB-01 D5 keeps the
broker's verdict as is).

### 4.2 Activity: per-source cursors, deterministic order

`projectActivity(deliveries, sources, since?)` takes deliveries from any
number of sources, plus each source's status as `activity.ts` reports
it. It returns the visible activity and a per-source cursor reading.

Each delivery is normalized to:

```ts
interface ActivityItem {
  source: string;
  source_id: string; // source-native id (HYD-06 §2.1)
  position: string | null; // source sequence as a decimal string, when the source has one
  revision: number; // 1 for the first landing of an id; +1 per distinct digest
  digest: string; // over the item's body-of-record
  occurred_at: string | null; // source-claimed
  source_updated_at: string | null;
  observed_at: string; // Fleet-authoritative
  ref: RefKey | null;
  kind: string;
  visibility: "public" | "private";
}
```

Rules:

1. **Repeat versus revision** (HYD-06 §2.3 and §2.4). Same
   `(source, source_id)` and same digest is a repeat and changes
   nothing. A different digest is a revision of the same item. The view
   shows the newest revision once and keeps the earlier ones
   addressable, never as new rows.
2. **A cursor never passes a gap.** For a positioned source, the view's
   cursor is the highest position below which every position was
   delivered, or the source's `checkpointCursor` if that is lower. A
   delivery above a missing position is shown, but the cursor stays
   below the gap and the source reads `lagging`. Positions are compared
   as decimal integers, never as strings or floats.
3. **A late delivery still shows.** An item at or below the cursor that
   was not seen before is shown in its correct place. Dedupe uses
   identity, not position, so a late item is never dropped as "already
   past".
4. **Order is a function of content, not arrival.** Visible order is
   `occurred_at` descending (falling back to `observed_at` when the
   source gives none), then `source`, then `source_id`, then revision.
   Any arrival permutation of the same deliveries produces the same
   view. This is the oracle in §6.
5. **Absent is not empty.** A source that is `unconfigured`, `lagging`,
   `outage` or `unrecoverable`, or has an open `GapRecord` inside the
   requested window, is named in the receipt (§4.4). An empty
   Recently changed list with such a source is reported as incomplete,
   never as "nothing changed".
6. **Private is filtered here, not later.** A `private` item is left out
   unless the caller passes a principal holding the note-read role.
   Each item left out is counted in the receipt, with no content.
7. **Nothing is persisted.** The cursor in this section is a value the
   view computes from its inputs. A durable new cursor or history store
   waits on #8581 and the QTI crosswalk (#9241). The existing
   `activity.ts` checkpoint file is read, never written.

**Recently changed** is this section with `since` set, over every work
item, and nothing else.

### 4.3 Outcomes: a task's completion is not the issue's

`projectOutcomes(work, receipts, issue, facts)` returns:

```ts
interface OutcomesSection {
  task_outcomes: {
    task_id: string;
    outcome: "verified_complete" | "pending_sync" | "partial" | "conflict" | "uncertain" | "rejected" | "none";
    receipt_ref: string | null;
    freshness: "current" | "limited" | "unknown"; // REL-09's Freshness (Current | Limited), carried; "unknown" when no receipt pins one
  }[];
  issue_state: "open" | "closed" | "unknown";
  display_done: boolean;
  discrepancies: ("issue_open_task_done" | "issue_closed_no_receipt" | "receipt_freshness_limited")[];
  outcome_facts: { total: number | null; partial: number; source_gap: boolean };
}
```

Rules:

1. **Receipts are keyed by task.** A verified receipt for one task says
   that task is done. It never sets `issue_state` and never makes the
   issue Done.
2. **`display_done` needs both.** It is true only when the issue is
   closed **and** the completion policy's verified terminal receipt
   exists for the work and its freshness is not `limited` (#9201 §3.1,
   REL-09). A closed issue alone, a merged PR alone, a `status:done`
   label or a closed queue task is never Done.
3. **The two lag classes are named.** An open issue whose tasks are all
   verified complete is `issue_open_task_done` (the #7016 class). A
   closed issue with no verified receipt is `issue_closed_no_receipt`.
   Both show as conflict or pending, never Done.
4. **Outcome facts keep their honesty.** Counts come from
   `rollupOutcomeFacts`. A partial count is a lower bound. A gap-only
   ledger gives `total: null` and `source_gap: true`, never 0.
5. **A downstream's own report is not a receipt.** A `verified_complete`
   with no readback behind it (MOB-09's `unbacked_verified`) is shown
   as `pending_sync`.

### 4.4 The explicit view receipt

Every view built from these sections returns a receipt. The view
doesn't infer it, and the caller doesn't assemble it.

```ts
interface ViewReceipt {
  schema_version: "pm_view_receipt/v1";
  view: string; // "recently_changed", "blocking_others", ...
  filter: Record<string, string>; // canonical, sorted keys; exactly what was applied
  generated_at: string;
  sources: {
    source: string;
    state: "current" | "lagging" | "outage" | "unrecoverable" | "unconfigured" | "error";
    cursor: string | null;
    checkpoint_cursor: string | null;
    observed_at: string | null;
  }[];
  open_gaps: { source: string; from: string; to: string | null }[];
  hidden_private: number;
  complete: boolean; // true only when every source is current and no gap overlaps the window
  digest: string; // over the sorted view rows + this receipt minus digest and generated_at
}
```

The receipt is what "explicit view receipt" means in this slice: the
view states which filter, sources and cursors its rows rest on. A reader
can then tell "nothing changed" from "we could not read GitHub". The
`digest` makes two builds comparable. The same inputs give the same
digest whatever their arrival order (§4.2 rule 4).

## 5. Negative cases

| Case                   | Behavior                                                                                                                                           |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stale input            | Source shows `lagging` or `error` in the receipt. `complete` is false. Rows still render with that label.                                          |
| Duplicate or replay    | Same identity and digest is a repeat. No new row, no revision bump, same digest.                                                                   |
| Ambiguous resource     | Equal numbers in two repos stay two refs. An edge endpoint that can't be resolved is `unresolved` and kept.                                        |
| Unauthorized actor     | Private activity is left out and counted. Relations and outcomes hold no private text.                                                             |
| Partial remote effect  | A task outcome of `pending_sync` or `partial` stays as is. `display_done` is false.                                                                |
| Unknown version        | A delivery with no usable position leaves the cursor where it was. A receipt with unknown freshness gives `freshness: "unknown"`, never `current`. |
| False verified or done | Closed issue without a receipt, or a receipt for one task of many, is never Done. A parent edge never gates. An unknown blocker never clears.      |

## 6. Oracles and tests

Command: `cd pm-stack/aggregator && bun test test/work-dependencies-activity.test.ts`.
CI runs it through `pm-aggregator-tests.yml` by `workflow_dispatch` on
the implementation branch. Each test name below must show up in the
run's output before acceptance. A filter that matches zero tests is not
acceptance.

Reference graph oracle (a hand-built pool whose expected edge list is
written out in the test):

1. `work_relations_keep_parent_separate_from_hard_blocker` (named red test)
2. `work_relations_unknown_blocker_still_gates`
3. `work_relations_unresolved_endpoint_is_kept`
4. `work_relations_advisory_and_epic_never_gate`
5. `work_relations_cycle_is_reported_not_propagated`
6. `work_relations_equal_numbers_in_two_repos_stay_apart`

Per-source cursor simulation (a generated stream with duplicates, edits,
late and out-of-order deliveries; the oracle is a reference model that
replays the stream in position order):

7. `work_activity_cursor_never_passes_unprocessed_gap`
8. `work_activity_duplicate_delivery_is_noop`
9. `work_activity_edit_is_revision_not_new_item`
10. `work_activity_late_event_is_visible_in_order`
11. `work_activity_any_arrival_order_gives_same_view` (permutation property, seeded)
12. `work_activity_private_hidden_without_role`

Completion versus closure (the #7016 / #8579 distinction):

13. `work_outcome_task_completion_is_not_issue_done`
14. `work_outcome_closed_issue_without_receipt_is_not_done`
15. `work_outcome_limited_freshness_is_not_done`
16. `work_outcome_gap_only_ledger_is_not_zero`

Receipt:

17. `work_view_receipt_names_filter_sources_and_cursors`
18. `work_view_lagging_source_marks_empty_view_incomplete`

## 7. Compatibility and reversal

The change is additive: one new module, one new test file. No existing
export, page, route, fixture, stored file or schema changes. The
existing `expedite=15` and `next-up=25` spellings are not read here.
Reversal removes the module. Because nothing is persisted, there is no
data to roll back.

## 8. Requirement trace

| ID     | How this record serves it                                                                                                              |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| C-R081 | Controls 14 and 15 from a phone or browser: inspect dependencies (§4.1) and recent activity and outcomes (§4.2, §4.3).                 |
| C-R119 | Per-source cursor, event identity, occurred and observed times, last successful sync and provenance, all surfaced in §4.2 and §4.4.    |
| O-R012 | Blocked and parked keep their blockers. Views are filters over one model and say which (§4.1, §4.4).                                   |
| P-R018 | Source-specific cursors that never pass unprocessed activity, dedupe of overlapping retrieval, and distinct timestamp meanings (§4.2). |

The trace is design-level. Each ID's own missing evidence (parser
tests for the full behavior, deployed and live acceptance) stays open
until an implementation and its acceptance are recorded.

## 9. Gates

Implementation starts only on a separately scoped issue, and only when:

1. This record is merged.
2. MOB-01's `work-projection.ts` types are on main (#9468), so §4.1's
   output fills MOB-01's `relations` fact instead of defining a second
   one. Until then the module can be built over `ProjectPoolItem`
   directly. The MOB-01 wiring is a later, separate step.
3. DEP-02 (#9503) is merged, or the implementation issue states that
   only GitHub's native relations are in scope (§4.1 rule 5).
4. Persistent cursor or history semantics are out of scope, so #8581
   and the QTI crosswalk (#9241) don't gate this slice. They gate
   anything that would persist §4.2's cursor.

Any live route, persisted field, deployed behavior or live acceptance
needs its own evidence and approval.

## 10. Non-goals

- No new store, queue, cursor file, endpoint or GitHub client.
- No urgency propagation along blocker edges (#9200).
- No eligibility computation (the broker's verdict is shown as is).
- No change to `activity.ts` ingestion, `outcome-*.ts` facts or
  `genealogy.ts`.
- No page or UI change. MOB-03 owns the views' layout.
