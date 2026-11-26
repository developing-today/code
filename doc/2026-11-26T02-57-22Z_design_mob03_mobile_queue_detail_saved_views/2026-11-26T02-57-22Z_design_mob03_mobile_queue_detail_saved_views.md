# MOB-03 — Mobile queue, item detail and saved views over one read model

Issue: #9470 (slice MOB-03 of #9201). Status: design record. It is not
an authorization to implement. Requirement IDs: C-R083, H-R001, H-R013,
O-R013.

## 0. Governing decisions consumed

| Source                                 | What this record takes from it                                                                                                          |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| #9201 MOB-03 slice text                | The view list, the owned files, the named red test `work_mobile_views_match_reference_filter`, the 360 / 768 / 1280 px and 200% oracle. |
| #9201 §3.1, §3.5                       | One read model; absent is never zero; a stale or failed source shows facet-level state; no silent collapse.                             |
| MOB-00 (#9467, harness on main)        | `playwright.config.ts` and `test/browser/mobile-work-view.spec.ts`, fixture mode, network isolation, no screenshot-only pass.           |
| MOB-01 record D4–D7                    | `pm_work_view/v1`, the five fact states, conflict codes, freshness roll-up.                                                             |
| MOB-02 record D3, D6                   | `projection.column` is a `BoardColumn` or `null`; `null` renders as "Unknown" with its reason codes.                                    |
| MOB-08 record §4.1, §4.4               | `RelationsSection` (`hard_blocker_refs`, `blocking_refs`, `parent_refs`) and the view receipt.                                          |
| #1276 (closed, `src/views.ts` on main) | The saved-views store, `ViewFilters`, `vf.*` share URLs, per-person default view.                                                       |
| #9070 (genealogy read model, on main)  | Item detail shows the genealogy header as it does today.                                                                                |
| MOB-04 (#9471)                         | Owns the compact responsive component system. MOB-03 uses it and adds no component of its own.                                          |
| MOB-07 (#9474)                         | Owns the session capability panel. MOB-03 reserves its place and keyboard stop on the detail page.                                      |

## 1. Verified state on main `d2656174`

- `src/views.ts` (#1276) holds `ViewFilters` with keys `repo, area,
  horizon, owner, state, epic, status, kind`, the pure
  `itemMatchesView` / `applyView`, `viewQuery` / `viewUrl` /
  `filtersFromQuery`, and `ViewsStore` (one JSON file, atomic writes).
  Mutations go through the actions store (`save-view`, `rename-view`,
  `delete-view`, `set-default-view`).
- `src/board-view.ts` maps a `GhItem` to `ViewItemLike` with
  `owner: item.author`. So the saved-view `owner` dimension means the
  GitHub author today.
- `src/queue-view.ts` builds the `/queue` page from the orchestration
  section; `reconciledInProgress` derives In Progress from one of three
  signals (`project-status`, `active-session`, `open-pr`).
- `src/render.ts` is 8,819 lines; `src/ui.css` 454 lines. Tap targets
  carry a 40px floor (#1180, `test/mobile-targets.test.ts`).
- `test/browser/mobile-work-view.spec.ts` has one test,
  `pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract`, covering
  `/queue` and `/item/<id>` at 360 and 768 px and keyboard reach of a
  queue card title. It does not cover 1280 px, 200% zoom, a filter,
  a back control or a capability panel.
- `src/work-projection.ts` (MOB-01) is on open PR #15188, not on main.
  `src/work-lifecycle-policy.ts` (MOB-02),
  `src/work-relations-activity.ts` (MOB-08) and
  `src/work-mobile-views.ts` do not exist.

## 2. Intent

Every mobile list is a pure filter over the same `PmWorkView[]`. The
browser shows exactly the refs the pure filter returns, in the same
order, at every width. An item whose deciding fact is unknown is never
quietly put in a list or quietly left out of it: it is counted and
shown as "can't place".

## 3. Ownership

- **Owned:** new `src/work-mobile-views.ts`, new
  `test/work-mobile-views.test.ts`, new fixtures under
  `test/fixtures/work-mobile-views/`, the new render functions in
  `src/render.ts`, and new rules in `src/ui.css`.
- **Needed edits outside the owned list, named:** one route
  registration for `/work` and `/work/<authority_id>/<work_id>` in
  `src/server.ts`, and new steps in `test/browser/mobile-work-view.spec.ts`
  (MOB-00's file; additions only, its existing assertions unchanged).
- **Read-only:** `views.ts`, `board-view.ts`, `work-projection.ts`,
  `work-lifecycle-policy.ts`, `work-relations-activity.ts`.
- **Not touched:** `queue-view.ts`, `work-items.ts`, `actions.ts`, the
  `/queue`, `/board` and `/attention` pages. They keep their behavior;
  MOB-03 adds `/work` beside them.
- **Added nowhere:** no database, view store, cache, queue, endpoint
  beyond the two pages, scheduler, GitHub client or writer.

## 4. Decisions

### D1. One pure module, one function per question

`work-mobile-views.ts` exports:

```ts
mobileView(views: readonly PmWorkView[], id: MobileViewId, ctx: ViewContext): MobileViewResult
groupWorkViews(views: readonly PmWorkView[], by: "repo" | "project" | "owner"): WorkGroup[]
matchesSavedFilters(view: PmWorkView, filters: ViewFilters): boolean
```

```ts
interface ViewContext { now: string; recent_window_ms: string; cold_after_ms: string; viewer: ViewerRef | null }
interface MobileViewResult {
  id: MobileViewId;
  refs: WorkKey[];                 // matched, in D5 order
  undecided: { ref: WorkKey; deciding_fact: FactKey; state: "absent" | "invalid" | "error" | "stale" | "policy_undefined" }[];
  unavailable: null | { reason: "fact_not_sourced"; fact: FactKey };   // the whole view cannot be computed
  freshness: "live" | "fixtures" | "stale" | "error" | "absent";      // MOB-01 D7 over the matched + undecided items
}
```

No I/O, no clock, no module state. `ctx.now` and both windows are
passed in, as in MOB-01 D1. Durations are decimal strings, per the
MOB-01 wire rule.

### D2. The view catalogue

Each view names its **deciding facts**. A row matches only when every
deciding fact is `present` with `freshness` `live` or `fixtures` and
the predicate holds. Otherwise D3 applies.

| `MobileViewId`   | Deciding facts                              | Matches when                                                                                                    |
| ---------------- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `all`            | none                                        | always (every view in the input)                                                                                |
| `triage`         | `project`                                   | Project Status is `null` or `Todo`, and Area or Horizon is `null`                                               |
| `needs_triage`   | `github`                                    | the issue carries the `needs-triage` label (verbatim), whatever its Project fields                              |
| `ready`          | `projection.column` (MOB-02)                | column is `Ready`                                                                                               |
| `high`           | `intrinsic_urgency`                         | urgency is present and the #9200 `high_or_above` predicate holds; `policy_undefined` is undecided, never "high" |
| `critical_path`  | `relations`, `projection.column`            | `blocking_refs` is non-empty and column is not `Done`                                                           |
| `blocking`       | `relations`                                 | `blocking_refs` is non-empty                                                                                    |
| `my_work`        | `claim`, `session`                          | `claim=claimed` with the viewer's lane, or `session=bound{verified:true}` to the viewer                         |
| `active_agent`   | `claim`, `session`                          | `claim=claimed` and `session=bound{verified:true}`                                                              |
| `review`         | `projection.column`                         | column is `Review`                                                                                              |
| `blocked_parked` | `projection.column`                         | column is `Blocked` or `Parked`                                                                                 |
| `recent`         | `github` (`observed` revision `updated_at`) | the issue's `updated_at` is within `recent_window_ms` of `now`                                                  |
| `cold_backlog`   | `github`, `projection.column`               | column is `Backlog` and `updated_at` is older than `cold_after_ms`                                              |

Notes:

- `critical_path` in V1 is "blocks something and is not done". A
  longest-chain measure needs the whole graph and a cost per item; it
  is a named follow-up, not guessed here.
- `high` waits on #9200. Until its predicate is exported, every row
  with a present urgency is `undecided` with state `policy_undefined`,
  and the view says so (D3). It never reads a `priority:high` label as
  high.
- `my_work` and `active_agent` depend on `claim` and `session`, which
  MOB-01 leaves `absent/not_sourced` until a broker read and MOB-07
  exist. While every input row has `claim` absent with reason
  `not_sourced`, the view returns `unavailable: {reason:
  "fact_not_sourced", fact: "claim"}` and an empty `refs`. The page
  says "Claims aren't connected yet", never "Nothing assigned to you".
- `needs_triage` reads the label verbatim. It does not reuse
  `metadata-ingestion.ts`'s internal `needs-triage` outcome, which is a
  different signal.

### D3. Undecided rows are counted, not dropped

When a row's deciding fact is not decidable (absent, invalid, error,
stale, or `policy_undefined`), the row goes to `undecided` with that
fact and state. The page shows them under the list as one collapsed
group, "N can't be placed", each row naming its reason in words ("Project
data is stale", "GitHub couldn't be read"). A stale fact is undecided,
not matched on its last value: a list that claims to be "Ready" must
not rest on stale data. `all` has no deciding fact and so no
undecided rows.

### D4. Grouping

`groupWorkViews` groups by:

- `repo`: the locator's `owner/repo` of the primary `source_refs`
  entry. Two refs with the same locator and different non-null
  `repository_id` are two groups (MOB-01 D3), labelled with both.
- `project`: the Project #4 board id from the `project` fact; rows
  with no project fact form a "No project" group, never merged into
  another.
- `owner`: the GitHub author, which is what the saved-view `owner`
  dimension means on main (§1). MOB-03 keeps that meaning and labels
  the group "Author" in the UI so it is not read as an assignee.

Groups are sorted by label, then rows by D5.

### D5. Order

Within a view: column order (`Inbox/Triage, Ready, In Progress, Review,
Blocked, Parked, Backlog, Done, Unknown`), then `updated_at`
descending, then `(authority_id, work_id)`. Fixed, deterministic, and
the same in the pure result and the rendered page. Urgency does not
sort rows until #9200 defines it.

### D6. Saved views reuse #1276

- A saved view stays a `ViewFilters` set in `ViewsStore`. MOB-03 adds
  no store and no filter key.
- The page's URL carries the mobile view as `mv=<MobileViewId>` beside
  the existing `vf.*` params, so a shared link reproduces both.
- Saving from `/work` saves the `vf.*` filters through the existing
  `save-view` action. The `mv` choice is not persisted in V1, because
  `views.ts` (not owned) validates filter keys and rejects unknown
  ones. Persisting `mv` is a named follow-up for `views.ts`'s owner:
  add an optional `view` key to `ViewFilters`.
- `matchesSavedFilters` maps `PmWorkView` to `ViewItemLike` by the same
  field meanings as `ghItemLike`, then calls `itemMatchesView`. One
  matching rule, reused.
- A saved filter on a fact that is not present excludes the row and
  counts it in D3's undecided group under that filter's fact.

### D7. The pages

- `/work?mv=<id>`: a filter control (the 13 views, plus group-by), the
  list, the undecided group, and the view's freshness line. Each row
  shows title, locator, column (or "Unknown" with reason codes), and
  one chip per conflict code.
- `/work/<authority_id>/<work_id>`: item detail. Heading is the title;
  then the genealogy header (#9070); the facts table with each fact's
  state and freshness in words; conflicts; relations from MOB-08; a
  region with id `capability-panel` that MOB-07 fills, rendered with a heading link and
  "Session details aren't connected yet" until then; and a back
  link to the list URL the viewer came from.
- Keyboard order on both pages: skip link → filter → list rows → back
  → capability panel. All are native links, buttons or form controls.
- Layout uses MOB-04's components and its breakpoints. At 360 px and
  at 640 px (1280 px at 200% zoom), no element is wider than the
  viewport; long locators and titles wrap.

## 5. Negative cases

| Case                  | Expected                                                                                                                                   |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| stale input           | `ready` with a stale project facet → row is undecided, not in `refs`                                                                       |
| duplicate / replay    | the same `(authority_id, work_id)` twice in the input → one row; second copy is a test failure in the input validator, not silently merged |
| ambiguous resource    | `identity_ambiguous` → row shows the conflict chip; `repo` grouping splits it (D4)                                                         |
| unauthorized actor    | `my_work` with `viewer: null` → `unavailable`, never another person's list                                                                 |
| partial remote effect | not applicable: MOB-03 issues no writes                                                                                                    |
| unknown version       | an input view with an unknown `schema_version` → excluded and counted, with reason                                                         |
| no false completed    | `column: null` with a terminal-looking facet → shown as "Unknown", never in a Done list                                                    |

## 6. Oracles and tests

`test/work-mobile-views.test.ts`:

1. `work_mobile_views_match_reference_filter` (named red test). For
   each fixture and each `MobileViewId`, `mobileView(...).refs` equals
   the fixture's expected refs, in order.
2. `work_mobile_views_count_undecided_rows`. Stale, error and absent
   deciding facts land in `undecided` with the right fact and state.
3. `work_mobile_views_high_is_undecided_until_9200`. Label-only
   urgency is never in `high`.
4. `work_mobile_views_unsourced_claim_is_unavailable_not_empty`.
5. `work_mobile_views_group_split_on_ambiguous_identity`.
6. `work_mobile_views_saved_filters_reuse_item_matches_view`. A
   `ViewFilters` set gives the same answer through
   `matchesSavedFilters` as `itemMatchesView` on the mapped item.
7. `work_mobile_views_order_is_deterministic`. Shuffled input gives
   byte-identical output.
8. `work_mobile_views_reject_unknown_schema_version`.
9. `work_mobile_views_dedupe_is_a_validation_error`.

Fixtures reuse MOB-01's `test/fixtures/pm-work-view/*.view.json` as
inputs and add `test/fixtures/work-mobile-views/expected.json`
(view id → refs, undecided).

Browser, added to `test/browser/mobile-work-view.spec.ts`, test name
`pmo_mob_03_work_views_render_reference_refs`:

- For `all`, `ready` and `blocked_parked`: the refs rendered on
  `/work?mv=<id>` (each row carries `data-work-key`) equal the refs in
  `test/fixtures/work-mobile-views/expected.json`, in order. Test 1
  proves that file equals the pure result, so the browser is checked
  against the pure filter without a new JSON endpoint.
- Widths: 360, 768 and 1280 CSS px, and 640 CSS px for 1280 px at 200%
  zoom (the WCAG 1.4.10 reflow equivalence). No horizontal overflow at
  any of them, on the list and on the detail page.
- Keyboard: from the top of `/work`, Tab reaches the filter, then a
  row link; Enter opens detail; Tab then reaches the back link and the
  `capability-panel` region's heading link.

The run lists tests by exact name first; a filter that matches zero
tests fails.

## 7. Compatibility and reversal

Additive. `/queue`, `/board`, `/attention` and the saved-view store are
unchanged. Removing the module, the two render functions, the CSS
rules and the route removes `/work` and nothing else. The
`expedite=15` / `next-up=25` spellings appear verbatim as source
values and are not folded into `high` or `do_now`.

## 8. Requirement trace

| ID     | How this record serves it                                                                                |
| ------ | -------------------------------------------------------------------------------------------------------- |
| C-R083 | One read model for every list (D1, D2); the browser oracle checks rendered refs against the pure result. |
| H-R001 | Responsive queue and detail at 360 / 768 / 1280 px and 200% zoom (D7, §6).                               |
| H-R013 | Saved views reuse the #1276 store and share URLs (D6).                                                   |
| O-R013 | Undecided rows and unavailable views are shown with reasons, never collapsed (D3, D2 notes).             |

## 9. Gates

1. This record merged.
2. MOB-01 code on main (#15188) for `PmWorkView`.
3. MOB-02 code on main for `projection.column`; until then the
   column-based views (`ready`, `review`, `blocked_parked`,
   `cold_backlog`, `critical_path`) return every row as undecided with
   `column_policy_not_installed`.
4. MOB-08 code on main for `relations`; until then `blocking` and
   `critical_path` are undecided.
5. MOB-04 components for the layout; MOB-03 can land its pure module
   and tests before them.

## 10. Non-goals

- Any action, verb or write from these pages (MOB-05/06).
- The session capability panel's content (MOB-07).
- A longest-path critical path (follow-up once a cost per item exists).
- Urgency sorting or a `high` rule of its own (#9200).
- Replacing `/queue`; moving existing pages onto `PmWorkView` is a
  later step with its own record.
