# HYD-05 — Pagination, API caps, and resumable durable page ingestion

Issue: [#9507](https://github.com/developing-today/code/issues/9507)
(slice `HYD-05` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.** This
record elaborates the page-ingestion slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the cursor and commit contract an implementation issue
can be scoped against. It does not re-decide anything that package
settled, and it does not authorize code, schema, migration,
deployment, or live acceptance; those remain separately gated stages
(see §9).

Governing decisions consumed, not re-opened:

- **FDR-PMO-004** (immutable hydration, per-source cursors,
  attribution): "Use a cursor key scoped by canonical resource,
  endpoint, query shape, API version, auth-scope class, and
  generation. Fetch, validate, durably persist page events/artifact
  refs and cursor continuation as one transaction where supported;
  cursor advances only after all page data is durable. If artifact
  store and DB cannot transact together, stage immutable objects,
  persist an ingest intent, verify hashes, then commit event+cursor;
  garbage collect only unreferenced staging data under retention
  policy." Its cursor MBT — duplicate/reordered pages and a crash on
  page N cannot skip a durably unprocessed item; repeat ingestion is
  idempotent — is this slice's acceptance test. FDR-PMO-004 also
  fixes the honesty rule this slice inherits: caps are explicit
  capability limits, never truncation hidden from the caller, and
  partial/truncated state is never presented as complete.
- **Component #9207, HYD-05 slice text**: goal (follow `Link`
  continuation; persist page data/cursor atomically only after
  durable processing), owned files, acceptance, negative cases, and
  the migration rule (unknown cursor forces full snapshot; legacy
  `last_checked_at` not trusted; no new scheduler). This record
  specifies those, it does not replace them.
- **REL-06** ([#9516](https://github.com/developing-today/code/issues/9516),
  design record merged 2026-11-23): owns the _write_ side of the
  GitHub surface — effect-specific preconditions and read-back
  verdicts for single operations. A REL-06 read-back is a
  single-effect verification GET; it is not page ingestion and owns
  no cursor. HYD-05 owns the _read_ side: bulk list ingestion for
  hydration/activity. Neither slice re-decides the other; both ride
  the existing `fleet gh` adapter surface — no new GitHub client.
- **HYD-06** ([#9508](https://github.com/developing-today/code/issues/9508))
  and **HYD-07** ([#9509](https://github.com/developing-today/code/issues/9509),
  design record merged 2026-11-23) are downstream consumers of this
  slice's output: HYD-05 durably lands raw page items; HYD-06 owns
  normalized records, revisions, and tombstones over them; HYD-07's
  classifier reads the records HYD-06 produces. HYD-05 defines no
  revision semantics and no attribution.

Verified state at writing (2026-11-23, origin/main `e8237c0b7`):
`pkgs/fleet/crates/fleet-queue/src/work/activity.rs` does not exist,
no cursor or page-iterator model for activity ingestion exists in
`fleet-queue` (`turso_queue.rs` is the queue broker store and carries
no activity cursor), the acceptance test
`pmo_hyd_page_ingest_and_cursor_commit_atomically` is not implemented
anywhere on main, and the granular prerequisites HYD-01
([#9458](https://github.com/developing-today/code/issues/9458)),
CEN-04 ([#9434](https://github.com/developing-today/code/issues/9434)),
REL-03 ([#9513](https://github.com/developing-today/code/issues/9513)),
and the REL-02 shared-file predecessor
([#9512](https://github.com/developing-today/code/issues/9512)) are
all still open. `Link`-header following exists today only in the
hosted worker's GitHub surface (`pkgs/fleet/hosted-worker/src/github.ts`)
and the PM Stack aggregator — neither persists an ingestion cursor.
The design below therefore specifies contracts against those slices'
stated outputs, not against running code.

## 1. Intent

Every hydration and activity read walks a paginated GitHub list:
issue comments, timeline, PR commits, PR files, reviews. The naive
implementations lose data in both directions:

- **Skip.** Advance an in-memory cursor (or worse, a
  `last_checked_at` timestamp) as pages arrive, persist items
  afterwards: a crash between the two skips every unprocessed item
  permanently, and the next walk starts past them.
- **Duplicate-or-truncate.** Re-walk from the start on every failure
  and treat "no `next` link seen yet" as "complete": API caps (a PR
  lists at most 250 commits and 3,000 files) and configured budgets
  silently truncate, and the caller is told the history is whole
  when it is not.

So the walk's unit of durability is the page: a page's items and the
continuation that follows it commit together, or neither does. The
cursor is a server-issued continuation, never an offset, a page
number, or a timestamp — offsets shift under insertion, and
timestamps are not cursors (FDR-PMO-004: an old comment can be edited
behind any time watermark; HYD-06 owns catching those edits, and it
can only do so over pages this slice actually landed).

## 2. Cursor model

### 2.1 Cursor key

One cursor per walk domain, keyed exactly as FDR-PMO-004 scopes it:

- canonical resource (owner/repo + issue/PR number — the same number
  in different repos never collides),
- endpoint (the allocated list commands: `github.issue.comments`,
  `github.comment.list`, `github.issue.timeline`,
  `github.pr.commits`, `github.pr.diff`, `github.pr.files` class),
- query shape (the full parameter set that determines the result
  set: sort, direction, per-page size, filters, and the cap profile
  of §6),
- API version fingerprint (§2.3),
- auth-scope class (which credential class made the reads — result
  sets differ by visibility),
- hydration generation the cursor belongs to.

A change to any key component is a different cursor. Reusing a
continuation under a changed query shape is a contract violation,
not a resume.

### 2.2 Cursor value and states

The stored value is the opaque continuation from the last committed
page's `Link: rel="next"` (URL or token, stored verbatim — never
parsed into page numbers, never synthesized), plus the key, the
commit timestamp (recorded as evidence, never used as a cursor),
and one of four states:

- **absent** — never ingested; a walk starts at the first page.
- **in-progress** — a continuation is committed; a walk resumes
  exactly there. This is the resumable state: process death at any
  point leaves the cursor at the last durably processed page.
- **complete** — a terminal page (no `rel="next"`) was committed
  under this key. A later refresh walk starts a new generation
  rather than mutating this cursor.
- **unknown** — the stored value fails validation (shape, key
  mismatch, fingerprint mismatch, or the cursor-without-events
  condition of §5.3). Unknown forces a full snapshot for that
  resource/endpoint (§8); it is never "repaired" by guessing.

### 2.3 Query/API version fingerprint

Each cursor carries a fingerprint of the query shape and the API
version the walk ran against (the request parameters as issued plus
the API version header value). If a resumed walk would run under a
different fingerprint — the code's query construction changed, or
the API version moved — the stored continuation is not trusted:
the cursor is `unknown` and the walk restarts as a full snapshot.
Continuations are only meaningful inside the exact request regime
that produced them.

## 3. The page walk

A walk is driven by an existing caller (`activity.reconcile`,
`work.changes`, the HYD-04 hydration adapter) — this slice adds no
scheduler and no polling loop.

1. Resolve the cursor for the key. Absent → first page. In-progress
   → the stored continuation. Unknown → full snapshot (§8).
2. Fetch one page through the existing `fleet gh` adapter surface.
3. Read the `Link` header. Absent header or absent `rel="next"` →
   this is the terminal page. A present-but-malformed `Link` header
   (unparseable, or a `next` relation whose value is not a usable
   continuation) is a **hard error**: the walk stops, the cursor
   does not advance, and the result reports `incomplete` with the
   malformed-continuation reason. Malformed is never read as
   terminal — a parsing failure must not masquerade as the end of
   the history.
4. Validate and process the page's items (§4), then commit the page
   and its continuation atomically (§5). Only after the commit does
   the walk fetch the next page.
5. The walk ends in exactly one reported state: `complete`
   (terminal page committed, no cap hit), `truncated` (a cap of §6
   stopped the walk; the committed continuation is the resume
   point), or `incomplete` (an error stopped the walk; the cursor
   sits at the last committed page). No fourth, silent outcome
   exists.

## 4. Page processing: identity, duplicates, ordering

- **Item identity is the source-native ID** (comment ID, commit SHA,
  file path within its PR diff, event ID), qualified by the cursor
  key's resource. Ingestion dedupes on that identity: a repeated
  page — from a resume overlap, a retry, or a shifted walk —
  re-lands the same identities and changes nothing. Repeat ingestion
  is idempotent by construction, not by cleanup.
- **Duplicates and reordering are expected, not errors.** Pages
  shift while a walk runs: a new comment inserts at the head of a
  reverse-chronological list and every later page slides. Because
  the cursor is the server's continuation rather than an offset,
  sliding can re-present an item (deduped) but cannot move the walk
  past an unlanded item: the continuation chain, followed link by
  link, is the completeness claim — a walk is `complete` only when
  it reaches a terminal page through committed continuations, never
  because a count matched.
- **Overlap is not proof.** A resume may deliberately re-fetch the
  last committed page as a positive control; overlap items are
  deduped like any other repeat. Overlap never substitutes for
  following the continuation.
- What a landed item _means_ — revision vs. creation, tombstones,
  minimized observations — is HYD-06's layer over these raw landings.
  This slice guarantees the raw item and its page provenance (cursor
  key, page sequence, observed-at) are durable; it assigns no
  revision semantics.

## 5. Atomic commit of page + cursor

### 5.1 Single-store path (the rule)

Where the page's items and the cursor live in the same store — the
`turso_queue.rs` transaction integration the slice owns — one
transaction commits, together: the page's items (or their durable
artifact refs), the ingest record for the page, and the cursor's
new continuation/state. The transaction either lands whole or not
at all. The cursor therefore never points past durably processed
data, and no committed page lacks its cursor advance. This is the
acceptance oracle's atomicity: crash before the commit → the page
is re-fetched and deduped; crash after → the walk resumes at the
next page; there is no interleaving in which an item is skipped.

### 5.2 Two-store path (object store + DB)

Where page payloads are staged in an artifact/object store that
cannot transact with the DB, the FDR-PMO-004 sequence applies,
unchanged:

1. Stage the page's objects immutably (content-addressed or
   generation-scoped names).
2. Persist an ingest intent in the DB naming the staged objects.
3. Verify the staged objects' hashes.
4. Commit the items + cursor continuation in one DB transaction,
   resolving the intent.

A crash between object-store success and DB commit leaves staged
orphans and an unmoved cursor: the page is re-fetched, re-staged,
and deduped on identity. Orphans are garbage-collected only when
unreferenced, under the retention policy — never by the walk
itself. The failure direction is always re-work, never loss.

### 5.3 Cursor without events is a forbidden state

A committed cursor whose page items are missing (commit corruption,
a partial restore, a store migration that carried cursors but not
items) must never be honored as a resume point: on detection the
cursor is `unknown` (§2.2) and the resource/endpoint restarts as a
full snapshot. The check is part of walk start: the cursor row and
the last page's ingest record are read together, and a cursor with
no matching committed page record does not validate. This is the
negative case the slice names explicitly — silently resuming from
such a cursor would skip exactly the items the cursor claims were
processed.

## 6. Caps and explicit truncation

Two cap families bound every walk, and both report rather than hide:

- **Source caps** — the platform's documented per-endpoint limits,
  named in the slice: a PR exposes at most **250 commits** and
  **3,000 files**. Reaching one ends that endpoint's walk at the
  cap boundary. These are properties of the source, not of Fleet's
  configuration, and no walk design can page past them.
- **Fleet caps** — configured per endpoint/query shape: maximum
  pages per walk, maximum items, maximum bytes, and a wall-clock
  budget. They are part of the query shape in the cursor key (§2.1),
  so a resume runs under the same caps that produced its cursor.

Hitting either family ends the walk as `truncated` (§3), with the
outcome naming which cap, its limit, the count reached, and the
committed continuation a later walk resumes from. `truncated` is a
first-class result the caller renders as partial — per FDR-PMO-004,
a truncated history presented as complete is the failure this slice
exists to prevent. Raising a Fleet cap is a configuration change,
not a code path: it produces a new query shape, hence a new cursor.

## 7. Acceptance oracle, specified

The implementation issue's first failing test, from the slice:
`pmo_hyd_page_ingest_and_cursor_commit_atomically`, in
`pkgs/fleet/crates/fleet-queue/src/work/activity.rs`'s test target,
run by the slice's done command (`cargo test --manifest-path
pkgs/fleet/Cargo.toml -p fleet-queue --lib --features
client,queue-broker pmo_hyd_page_ingest_and_cursor_commit_atomically`,
after the `-- --list` step confirms the filter matches exactly that
test — a zero-test filter pass is not acceptance).

Fixture: a fake list API serving at least two pages joined by a
`Link: rel="next"` chain, with crash injection at four points —
before page-2 ingest, after page-2 ingest before commit, at the
commit, and after the commit. The reference model asserts, for every
crash point: no item from any served page is skipped, the cursor
never sits past the last durably processed page, and a resumed walk
terminates `complete` with the full item set landed exactly once.

Negative fixtures, one per named case:

| Case                                  | Expected behavior                                                            |
| ------------------------------------- | ---------------------------------------------------------------------------- |
| Absent `Link` header                  | Terminal page; walk ends `complete`                                          |
| Malformed `Link` header               | Hard error; cursor unmoved; walk reports `incomplete`, never `complete`      |
| Duplicate entries within/across pages | Deduped on source-native ID; walk still `complete` at the terminal page      |
| Reordered entries/pages               | Same landed set; order is not load-bearing                                   |
| 250-commit / 3,000-file source cap    | Walk ends `truncated`, naming the cap; not `complete`                        |
| Object-store success + DB failure     | Cursor unmoved; staged orphans GC-eligible; re-walk dedupes, nothing skipped |
| Cursor without events                 | Cursor invalid → `unknown` → full snapshot for that resource/endpoint        |
| Fingerprint change on resume          | Cursor invalid → `unknown` → full snapshot (§2.3)                            |
| Repeated page (idempotency)           | Second landing is a no-op beyond the ingest record                           |

No mock-only claim of deployed or live behavior: the oracle runs
against the fake API and the real store transaction path, and live
acceptance remains a separate stage (§9).

## 8. Compatibility and migration

- **Unknown cursor forces full snapshot** (slice migration rule,
  restated): any stored cursor that fails validation — including
  every pre-design cursor shape — is `unknown`, and the first walk
  under this design is a full snapshot for that resource/endpoint.
  There is no conversion path from legacy state.
- **Legacy `last_checked_at` is not trusted.** It is never read as,
  converted into, or used to fabricate a cursor or a high-water
  mark (FDR-PMO-004: do not fabricate a high-water mark from old
  timestamps). It may survive as display metadata; it has no
  ingestion semantics.
- **No existing surface changes spelling or outcome.** The allocated
  commands (`activity.reconcile`, `work.changes`, the
  `github.*` list commands) keep their names and result envelopes;
  the walk states `complete`/`truncated`/`incomplete` are additive
  fields on existing result shapes. Shared CLI registration glue
  remains CLI-13's; the ratified `expedite=15` / `next-up=25`
  values are untouched and unrelated, and remain distinct from
  `do_now`.
- **Persisted-shape gate.** The cursor's logical fields are fixed
  here (key, continuation, fingerprint, state); its storage waits
  for the QTI crosswalk and the named schema owner (CEN-04, §9) —
  this design does not pick a table, a column, or a migration.
- **Rollback is disable-only:** turning page ingestion off leaves
  committed items and cursors in place and returns callers to their
  prior read behavior; no landed history is deleted. Projection
  rows remain rebuildable from the landed pages.

## 9. Gates and sequencing

Implementation of this design starts only when **all** of the
following hold (slice runtime gates, restated):

1. This record is merged.
2. HYD-01 [#9458](https://github.com/developing-today/code/issues/9458)
   (versioned hydration manifest and result envelope — the
   generation the cursor key scopes to, and the storage/transaction
   contract the commit of §5 integrates with), CEN-04
   [#9434](https://github.com/developing-today/code/issues/9434)
   (schema owner and migration compatibility for any persisted
   cursor field; the QTI crosswalk gate of §8), and REL-03
   [#9513](https://github.com/developing-today/code/issues/9513)
   (idempotency identity and stable receipts — the ingest-intent
   coordination of §5.2, where storage is shared with the outbox)
   are resolved.
3. The shared-file sequencing edge on
   `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` is honored:
   REL-02 [#9512](https://github.com/developing-today/code/issues/9512)
   (claim/lease contract compatibility) lands its changes to that
   file first.
4. A separately scoped implementation issue is filed against this
   record and the §7 oracle; #9507 itself authorizes no code.

Production adoption, any schema/migration step, and live acceptance
remain separate stages with their own evidence, per the slice body.
This design creates, changes, or retires **no schedule on any
surface**; `doc/fleet-schedule-registry.md` is intentionally
untouched.

## 10. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records only which part of this design speaks to each
group — it claims design coverage, not implementation.

| IDs                                                                                    | Where addressed                                                                                                        |
| -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| C-R119, C-R120                                                                         | §2, §4, §5 — per-source retrieval cursors; overlap plus deduplication; cursors advance only after durable processing   |
| C-R111, C-R112, C-R118                                                                 | §3, §8 — the activity/changes read surface this ingestion feeds; command allocation consumed as-is                     |
| C-R121, C-R122                                                                         | §4 — item identity and provenance this slice lands durably; session attribution itself is HYD-07's, over these records |
| C-R005, C-R009, C-R107                                                                 | §3, §6 — pagination behavior of the existing list surfaces, bounded by explicit caps                                   |
| C-R150, C-R151                                                                         | §2, §5, §8 — durable cursor/provenance metadata and its legacy treatment                                               |
| H-R014, H-R016, H-R017, H-R024, H-R031                                                 | §7, §8, §9 — oracle, compatibility, and gate coverage carried into the implementation issue's acceptance               |
| O-R022, O-R023, O-R028, O-R030, O-R049, O-R050                                         | §3–§6 — candidate/example occurrences behind the walk, dedupe, and cap rules and their negative cases                  |
| P-R009, P-R012, P-R015, P-R016, P-R018, P-R027, P-R039, P-R050, P-R051, P-R065, P-R066 | §2, §5 — proposal occurrences behind the cursor model and the atomic page+cursor commit                                |

## 11. Non-goals

- No new queue, database, scheduler, service, resolver, executable,
  GitHub client, or general creation-outbox producer (slice
  boundary). Walks are caller-driven; nothing here polls.
- No revision, tombstone, or minimized-record semantics — those are
  HYD-06's, over this slice's durable landings. No attribution
  classification — HYD-07's.
- No write-side preconditions or read-back verdicts — REL-06's.
  This slice verifies no effects; it ingests reads.
- No change to #6533 or its live acceptance; no edit to any other
  slice's owned files. `activity.rs` and the `turso_queue.rs`
  transaction integration are the only implementation surfaces,
  and only under the §9 gates.
- No synthesized pagination: page numbers, offsets, and timestamps
  are never manufactured into continuations, under any fallback.
