# HYD-06 — Per-source incremental activity revisions and tombstones

Issue: [#9508](https://github.com/developing-today/code/issues/9508)
(slice `HYD-06` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.** This
record elaborates the activity-revision slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the revision, observation, and tombstone contract an
implementation issue can be scoped against. It does not re-decide
anything that package settled, and it does not authorize code, schema,
migration, deployment, or live acceptance; those remain separately
gated stages (see §9).

Governing decisions consumed, not re-opened:

- **FDR-PMO-004** (immutable hydration, per-source cursors,
  attribution): "Track source-native event/resource ID separately from
  resource revision/updated marker, delivery ID, occurrence time,
  observed time, and source update time. Edits revise the same logical
  item and append an observation. Deletions/minimizations are recorded
  when authoritative; otherwise periodic full reconciliation is a
  bounded detection strategy with an explicit visibility gap." Its
  revision oracle — edit an old comment after the creation cursor has
  advanced, and the result is one logical comment with a new revision
  and an appended observation, not a duplicate creation or an omission
  — is this slice's acceptance test.
- **Component #9207 §4** (incremental activity and cursor correctness):
  activity identity separates occurrence and edits; a comment's
  ascending ID creation ordering does not detect edits to old IDs;
  updated-since queries may surface changed items but overlap reduces
  races without proving completeness against arbitrary edits and
  deletions; maintain tombstones/minimized/deleted observations only
  where an authoritative API or event provides them, and report the
  visibility gap where it does not.
- **HYD-05** ([#9507](https://github.com/developing-today/code/issues/9507),
  design record merged 2026-11-23, PR #13297): owns the page walk, the
  cursor key and its states, the atomic page+cursor commit, caps and
  explicit truncation. This slice consumes that contract unchanged: it
  never redefines a cursor, never advances one, and never synthesizes a
  continuation. HYD-05 lands raw page items with page provenance; this
  slice assigns revision semantics over those landings.
- **JRN-01** ([#9463](https://github.com/developing-today/code/issues/9463),
  closed 2026-11-21, design merged PR #12398): owns the append-only
  journal event envelope and correction semantics (§3.3.1 of the #9208
  component). This slice names the append seam it writes through and
  invents no second journal.
- **AUTH-05** (#9208): owns the trusted principal/provenance record in
  `src/work/authority.rs`. Attribution _of activity to a Fleet session_
  is HYD-07's ([#9509](https://github.com/developing-today/code/issues/9509),
  design record merged 2026-11-23); this slice records external
  observations and never claims session attribution.

Verified state at writing (2026-11-23, origin/main `de34307cca9`, the
merge base this branch was cut from):
`pkgs/fleet/crates/fleet-queue/src/work/activity.rs` and
`.../src/work/journal.rs` do not exist; `fleet-queue` has no activity,
revision, or journal module (`src/work/` holds `authority.rs`,
`completion.rs`, `contracts.rs`, `degraded.rs`, `lifecycle.rs`,
`mod.rs`, `operations.rs`, `routing.rs`, `selection.rs`,
`sqlite_replica.rs`); neither acceptance test
(`pmo_hyd_refresh_observes_edit_behind_creation_cursor`,
`pmo_hyd_replay_deduplicates_revision_not_creation`) is implemented
anywhere on main; and the granular prerequisites HYD-05
([#9507](https://github.com/developing-today/code/issues/9507), open as
its implementation tracker) and CLI-05
([#9502](https://github.com/developing-today/code/issues/9502), open)
remain unresolved, while JRN-01
([#9463](https://github.com/developing-today/code/issues/9463)) is
closed with its design merged. The design below therefore specifies
contracts against those slices' stated outputs, not against running
code.

## 1. Intent

A hydration or `changes` read is only useful if it can answer "what
happened since I last looked" without lying in either direction:

- **Missed edits.** Comments are mutable at the source. An edit to a
  three-week-old comment raises no new comment ID, so a cursor built on
  creation order walks straight past it. A reader told "no changes"
  shows stale content as current.
- **False changes.** Re-reading a page and re-inserting its items files
  the same comment twice: the user sees a phantom new comment, and
  counts, sorting, and attribution all drift.
- **Invisible deletions.** A comment removed or minimized between
  reads simply stops appearing. Inferring "deleted" from absence in a
  paginated list is unsound — the page may have shifted, the item may
  be hidden by the caller's permission class, or the list may be
  truncated.

So the slice's unit is the **observation**, not the row: every landed
item is compared against what is already known for that source-native
identity, and the difference is recorded as an appended fact —
`created`, `revised`, `tombstoned`, `minimized`, `restored` — while the
logical item stays one item. Deletion facts are recorded only when the
source states them, and a source that cannot state them carries an
explicit visibility gap rather than a confident "nothing was deleted".

## 2. Activity record model

### 2.1 Identity

A logical activity item is identified by source-native identity
qualified by the resource the cursor key already scopes
(HYD-05 §2.1):

- **source-native ID** — comment ID, event ID, review ID, commit SHA,
  or file path within its PR diff;
- **resource** — provider, owner/repo, issue/PR number (the same
  number in another repository never collides);
- **endpoint class** — which list the item arrived on
  (`github.issue.comments`, `github.issue.timeline`,
  `github.pr.reviews`, …).

Two landings with the same identity are the same logical item. Identity
never includes the item's body: an edit must not mint a new identity,
and that is exactly the failure this slice exists to prevent.

### 2.2 The four times

Each item and each observation carries these, kept distinct
(FDR-PMO-004):

| Time                | Meaning                                                                                      | Trust                            |
| ------------------- | -------------------------------------------------------------------------------------------- | -------------------------------- |
| `occurred_at`       | when the source says the event happened (comment `created_at`, commit author/committer time) | source-claimed                   |
| `source_updated_at` | the source's own last-modified marker (`updated_at`, `edited_at`)                            | source-claimed; **not** a cursor |
| `observed_at`       | when Fleet's ingestion durably saw this item                                                 | Fleet-authoritative              |
| `first_observed_at` | `observed_at` of the observation that first created the record                               | Fleet-authoritative              |

`source_updated_at` orders _observations of edits_ and is evidence; it
is never used as a high-water mark or a cursor (HYD-05 §8: legacy
`last_checked_at` is not trusted, and no timestamp is a continuation).
`observed_at` is the only time Fleet asserts on its own authority, and
it is the tie-breaker when source markers are equal (§2.4).

### 2.3 Revision and content digest

Each observation stores a digest of the item's canonicalized content
(the fields that constitute its body-of-record for that endpoint:
body text, state, labels as ordered as the source gives them, title —
whatever the endpoint's contract names). The digest is computed over
exact normalized bytes, not display text.

A landing whose digest equals the latest known digest for that identity
is a **repeat**, not a revision: it updates nothing beyond its ingest
record (HYD-05 §4). A landing whose digest differs is a **revision**: a
new observation is appended with `revision = previous + 1`, the prior
observation stays addressable and unchanged, and a current-item
projection selects the newest valid observation (§3).

Digests make equality cheap and deterministic; they are never treated
as proof of _what_ changed, only of _whether_ the body-of-record
changed. Fields outside the digest (for example, a label the endpoint
returns inconsistently) must be listed as such in the implementation
issue rather than silently excluded.

### 2.4 Equal timestamps

Sources routinely return equal `source_updated_at` values for distinct
edits, and some return `updated_at` earlier than a previous
observation (clock skew, eventual consistency). Ordering therefore
never depends on `source_updated_at` alone:

1. If `source_updated_at` differs, the later value is the newer
   observation.
2. If it is equal or out of order, the **digest** decides: a different
   digest is a revision regardless of the marker.
3. If the digest is also identical, it is a repeat, not a revision.
4. Only the digest-equal, marker-equal case is a no-op.

That rule is the named negative case "equal `updated_at`": it must not
produce a duplicate creation, and it must not swallow a real edit whose
marker happens to be unchanged.

## 3. Observation projection and the journal append seam

### 3.1 Two layers, one fact store

- **Activity records** (this slice, `src/work/activity.rs`) hold the
  normalized item, its identity, its four times, and its ordered
  observations with digests.
- **The append-only journal** (JRN-01 contract, `src/work/journal.rs`)
  is the durable, ordered, per-work event stream every other
  lifecycle fact also appends to. This slice writes `external.observed`
  and `activity.revised` / `activity.tombstoned` /
  `activity.minimized` events through the JRN-01 envelope — event ID,
  work `(authority_id, work_id)`, per-work `sequence` as a canonical
  decimal string, `schema_version`, kind, `occurred_at`, provenance,
  `visibility`, and `before`/`after` refs.

There is one journal and one activity store, not two. The append seam
is the JRN-01 event append; this slice owns the _payload kind and the
revision rule_, JRN-01 owns _the envelope and its correction
semantics_. A revision is an append of a new event that references the
prior activity observation (`supersedes_event_id`-style ref is used for
journal corrections; activity revisions carry
`supersedes_observation_id` and never rewrite the prior observation).

### 3.2 Visibility and privacy

External activity is **untrusted public source data**. Journal
visibility for these events is `internal` by default: they are
Fleet-authoritative facts about external content, readable by work
readers, and never mirrored to GitHub (JRN-01: no automatic mirror).
Private Fleet notes remain private; this slice does not read, embed,
or re-emit note bodies, and the golden serialization test asserts a
private note field cannot appear in an activity or journal payload.

The item's body text is stored as the untrusted data it is, labeled by
source/actor/time (component #9207 §4 trust boundary). Nothing in it
gains authority by being hydrated.

### 3.3 Bounded growth

Observations are append-only per identity: the store keeps the ordered
observation refs and their digests, not a copy of every historical body
unless retention policy keeps them. The current-item projection is the
latest valid observation; an audit view returns the chain. A
retention/redaction policy may tombstone an old observation's payload
while preserving its identity and digest — the same rule JRN-01 sets
for corrections. Replay of activity history is bounded by
`schema_version`; this slice makes no full-work-state reconstruction
claim.

## 4. Detecting edits behind the creation cursor

This is the core of the slice, and it is explicitly **two
mechanisms**, neither of which alone is a completeness proof:

1. **Updated-since sweep.** For each cursor key, walk the endpoint with
   the source's changed-items affordance where one exists (a
   `since`/`updated_at`-ordered list, a timeline) over an **overlap
   window** that starts strictly before the last `source_updated_at`
   Fleet observed for that key. Overlap is a positive control: it
   re-presents items already known, which dedupe, and it catches edits
   whose marker moved into the window.
2. **Periodic authoritative reconciliation.** A bounded full
   reconciliation of the resource (driven by the same caller that
   drives hydration — no new scheduler) re-walks the list to catch
   what the sweep cannot see.

**Overlap is not proof.** A bounded overlap window is a probability
reduction, not a guarantee: an edit older than the window, or one whose
marker the source does not bump, is invisible to the sweep. The design
therefore states per resource/endpoint whether the source _can_ expose
"changed since" and with what lag; where it cannot, that endpoint
carries a declared **coverage limitation**, and reconciliation cadence
is the only mitigation. A `changes` result must render a
coverage-limited endpoint as "no _observed_ changes", never as "no
changes".

Shifting pages are expected (HYD-05 §4): an insertion during a
reverse-chronological walk slides later pages, which can re-present or
delay items but cannot move the walk past an unlanded one, because the
walk follows the server's continuation. This slice adds no ordering
assumption of its own.

## 5. Tombstones, minimization, and the deletion gap

A comment that disappears from a list is **not** evidence of deletion:
the list may be truncated (HYD-05 §6), the page may have shifted, or
the permission class may hide it. So:

- **Recorded deletion/tombstone** — only when the source states it: a
  timeline event (`deleted`, `referenced`), a state field
  (`isMinimized`, a `deleted` flag), or an authoritative webhook the
  projector already dedupes. The record appends a `tombstoned` or
  `minimized` observation with `observed_at` and the source evidence
  ref; the logical item and its prior observations remain addressable.
- **Minimized** is distinct from deleted: the item still exists and may
  be restored. A `restored` observation follows the same append rule.
- **Absent deletion** — the item stops appearing with no source
  statement. Nothing is inferred. The endpoint's coverage limitation is
  reported (§4), and reconciliation is what may later surface an
  authoritative deletion event. This is the named negative case "absent
  deletion".
- **Deletion gap reported** — the `changes` result carries an explicit
  `deletion_visibility: reported | gap` field per endpoint, so a reader
  can tell "no deletions were reported by a source that reports them"
  from "this source cannot tell us".

## 6. Cursor and commit semantics (consumed from HYD-05)

This slice re-states nothing here; it consumes:

- the cursor key (resource, endpoint, query shape, API-version
  fingerprint, auth-scope class, generation) and its
  `absent`/`in-progress`/`complete`/`unknown` states;
- the rule that **cursor advances only after the page's items are
  durably committed**, with the two-store staged-intent path where
  object store and DB cannot transact;
- the rule that a cursor without its committed page data is `unknown`
  and forces a full snapshot;
- caps ending a walk as `truncated`, never `complete`.

Activity observations are created **inside** that same page commit:
the page's items, their normalized records and observations, and the
cursor advance land together or not at all. A crash before the commit
re-fetches and dedupes; a crash after resumes at the next page. There is
no interleaving in which an item is landed without its observation or
an observation without its page provenance.

## 7. Acceptance oracle, specified

The implementation issue's two first failing tests, from the slice,
in `pkgs/fleet/crates/fleet-queue/src/work/activity.rs`'s test target,
run by the slice's done command (after the `-- --list` step confirms
each filter matches exactly that test — a zero-test filter pass is not
acceptance):

1. `pmo_hyd_refresh_observes_edit_behind_creation_cursor`
2. `pmo_hyd_replay_deduplicates_revision_not_creation`

### 7.1 Fixture

A fake list API (the same fixture style as HYD-05) serving a comment
list, where a comment is created, the walk advances its cursor past it,
and the API **then edits that old comment** without changing its
creation position. A second page carries an unrelated new comment.
Replay the walk (and re-feed the same pages) twice.

Reference model asserts:

- after the refresh, the edited comment is **one** logical item with a
  **new revision** and one appended observation whose digest differs —
  not a second comment, not an omission;
- the new comment lands once;
- replaying the same pages produces no new observation and no new
  item (idempotent on identity + digest);
- the item's `occurred_at` is unchanged while `observed_at` advances;
- the cursor advanced only after both pages' items were durable, and a
  crash injected before page-2 commit leaves no landed item without
  its cursor position (reusing HYD-05's crash model).

### 7.2 Negative fixtures

| Case                                                   | Expected behavior                                                                                                            |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| Edit behind creation cursor (the two named tests)      | One item, new revision, appended observation; replay is a no-op                                                              |
| Equal `updated_at` on distinct edits                   | Digest decides; any real content change is a revision, marker equality alone never hides it                                  |
| Shifting pages during walk                             | Re-presentation dedupes; no item skipped; no duplicate creation                                                              |
| Absent deletion (item disappears, no source statement) | No tombstone inferred; `deletion_visibility: gap` reported for that endpoint                                                 |
| Authoritative deletion/minimize event                  | `tombstoned`/`minimized` observation appended; prior observations and identity retained                                      |
| Minimized then restored                                | `restored` observation appended; item remains one logical item                                                               |
| Repeat page / duplicate delivery                       | Repeat ingestion is a no-op beyond the ingest record; webhook delivery ID deduped separately from resource revision          |
| Changed body with unchanged digest-covered fields only | Implementation issue must name digest-covered fields; the fixture pins the expected behavior explicitly, no silent exclusion |

No mock-only claim of deployed or live behavior: the oracle runs
against the fake API and the real store transaction path; live
acceptance remains a separate stage (§9).

## 8. Compatibility and migration

- **Unknown cursor forces a full snapshot** (HYD-05 §8, restated as
  consumed): the first walk under this design backfills activity
  records from the full list; there is no conversion from legacy
  `last_checked_at` and no fabricated high-water mark.
- **Existing result shapes are extended, not renamed.** The
  `changes`/`activity.*` outcomes gain additive fields
  (`revision`, `observation_kind`, `deletion_visibility`,
  `coverage_limitation`); the ratified `expedite=15` / `next-up=25`
  values are untouched and remain distinct from `do_now`.
- **Persisted-shape gate.** The logical fields are fixed here
  (identity, four times, digest, observation chain, kind); storage
  waits for the QTI crosswalk and the named schema owner — this design
  picks no table, column, or migration.
- **Rollback disables incremental reading only:** turning the
  capability off leaves landed records, observations, and journal
  events in place, returns callers to their prior read behavior, and
  retains all generations. No history is deleted.
- **No new schedule on any surface**; `doc/fleet-schedule-registry.md`
  is intentionally untouched.

## 9. Gates and sequencing

Implementation of this design starts only when **all** of the
following hold (slice runtime gates, restated and resolved against the
current tree):

1. This record is merged.
2. HYD-05's cursor/commit contract is in place — its design record is
   merged (PR #13297, 2026-11-23) and #9507 remains the tracker for the
   implementation that supplies the page walk and cursor this slice
   commits inside.
3. JRN-01's journal contract is in place — #9463 closed 2026-11-21 with
   its design merged (PR #12398); the append seam of §3.1 is scoped
   against that envelope.
4. CLI-05 ([#9502](https://github.com/developing-today/code/issues/9502),
   lifecycle transition and checkpoint/park/resume/handoff receipts)
   resolves its `work/contracts.rs`/receipt surface this slice's
   `changes` output rides.
5. The shared-file sequencing edge is honored: HYD-05 owns
   `pkgs/fleet/crates/fleet-queue/src/work/activity.rs` and lands its
   page-iterator/cursor model to that file first; this slice adds the
   revision model to the same file only after that lands.
6. A separately scoped implementation issue is filed against this
   record and the §7 oracles; #9508 itself authorizes no code.

Production adoption, any schema/migration step, and live acceptance
remain separate stages with their own evidence, per the slice body.

## 10. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records only which part of this design speaks to each
group — it claims design coverage, not implementation.

| IDs                                                                                    | Where addressed                                                                                                                       |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| C-R119, C-R120, C-R121, C-R122                                                         | §2, §4, §6 — per-source cursors and their commit rule (consumed from HYD-05); revisions and observations distinguished from creations |
| C-R111, C-R112, C-R118                                                                 | §3, §8 — the activity/changes read surface and its additive outcomes                                                                  |
| C-R005, C-R009, C-R107, C-R108                                                         | §4, §5 — changed-item detection over the existing list surfaces; deletion/minimize evidence                                           |
| C-R150, C-R151                                                                         | §2, §6, §8 — durable provenance/times and legacy-cursor treatment                                                                     |
| H-R014, H-R016, H-R017, H-R024, H-R031                                                 | §7, §8, §9 — oracle, compatibility, gates carried into the implementation issue                                                       |
| O-R022, O-R023, O-R028, O-R030, O-R049, O-R050                                         | §4, §5, §7 — candidate/example occurrences behind overlap-not-proof, deletion gap, and the negative fixtures                          |
| P-R009, P-R012, P-R015, P-R016, P-R018, P-R027, P-R039, P-R050, P-R051, P-R065, P-R066 | §2, §3, §6 — proposal occurrences behind the record model, the journal seam, and the atomic commit                                    |

## 11. Non-goals

- No new queue, database, scheduler, service, resolver, executable,
  GitHub client, or creation-outbox producer (slice boundary). No
  polling loop; reconciliation is caller-driven.
- No cursor, page-walk, cap, or continuation semantics — HYD-05's; this
  slice consumes them. No second journal — JRN-01's envelope and
  correction semantics are used as-is.
- No session attribution classification — HYD-07's, over the records
  this slice produces. A record says what the source said and when
  Fleet saw it; it never asserts a Fleet session caused it.
- No write-side preconditions or effect verification — REL-06/REL-03.
  This slice ingests reads.
- No change to #6533 or its live acceptance; no edit to another
  slice's owned files. `activity.rs` (after HYD-05 lands its model) and
  the journal append seam are the only implementation surfaces, and
  only under the §9 gates.
- No inference of deletion from absence, and no edit detection claimed
  from creation order, under any fallback.
