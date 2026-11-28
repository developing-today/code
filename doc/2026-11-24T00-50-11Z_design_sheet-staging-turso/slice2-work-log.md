# Slice 2 work log — schema and idempotence verification (design §5 order 2, #6581)

## Scope (from design addendum §5 order 2)

Inspect deployed DDL and code for all staging + curated tables; run safe
isolated tests for batch association, unique keys, dedupe upserts,
append-only guards, ambiguous commit recovery. No production cutover.
Add-only record: new test file + dated repair-plan doc. No data movement,
no D1 action, no secrets.

## Fresh live state (revalidated 2026-11-24)

- #6581 OPEN (umbrella, execution owner Drew).
- #5692 OPEN + status/blocked + queue/blocked + work/assigned (sheet staging) — the slice's home wave.
- #5691 CLOSED (mirror analytics out of scope).
- #5690 OPEN task/triage, #5693 OPEN status/parked (other waves, untouched).
- Slice 1 landed: PR #14078 (merge 333e8e1a), manifest
  `doc/2026-11-24T00-50-11Z_design_sheet-staging-turso/inventory-19-tables.md`
  - contract test `scripts/tests/test_sheet_staging_inventory.py` (5/5).
- Nine-point design: PR #13930 merged (D1 ruling still pending — Turso is
  the working destination; no D1 action in this slice).

## PG source-of-truth DDL (repo, pkgs/fleet/migrations/)

- 0017_workbook_final_captures.sql: automation/research/finance/home (?)
  staging triples — see DDL finding below.
- 0004_ainventory.sql: ainventory staging pair (append-only, no dedupe key
  on sheet_rows; ingest_batches is parent metadata).
- 0011_engineering.sql + 0016_engineering_events.sql: engineering staging
  triple + engineering_records curated.
- 0003_ideas.sql: ideas.ideas curated.
- 0013_aa_event_log.sql: ainventory event log parent.

## A. Deployed/source DDL key inventory

Source of record: checked-in PG DDL in `pkgs/fleet/migrations/` (repo read
2026-11-24; deployed Turso DDL not directly inspected — see §E).
Turso naming: `schema.table` → `schema_table` (design §4).

- 0017 (automation / research / finance, identical triples):
  - `sheet_rows`: UNIQUE (source_id, import_batch, sheet_name, row_index)
    (named `<schema>_sheet_rows_dedupe_key`). `source_id bigint NOT NULL`
    with NO REFERENCES clause — FK is conventional, not enforced.
  - `events`: UNIQUE (import_batch, sheet_name, row_index) (batch-row key)
    AND UNIQUE (dedupe_key). Both constraints present in all three schemas.
  - `ingest_batches`: `id uuid PRIMARY KEY DEFAULT gen_random_uuid()`,
    no natural/unique key beyond PK. Creation is not idempotent.
- 0004 (ainventory workbook mirror):
  - `sheet_rows`: UNIQUE (source_id, import_batch, sheet_name, row_index)
    WITH `source_id REFERENCES ainventory.sources(id)` — enforced FK.
  - `benchmark_results`: UNIQUE (source_id, import_batch, model_name,
    benchmark, metric, effort). NOTE: retired — `aa-turso-sql.ts`
    (AA_PAYLOAD_SQL_REVISION=3 comment) records the table dropped after
    full-row EXCEPT verification (configs 35/35, cai 27/27); canonical
    read path is now `measurement_events`.
  - No `ingest_batches` in 0004; the batch parent lives in 0013.
- 0011 (engineering ledger mirror):
  - `sheet_rows`: UNIQUE (source_id, import_batch, sheet_name, row_index),
    FK to `engineering.sources(id)` enforced.
  - `records` (curated): UNIQUE (source_id, import_batch, sheet_name,
    row_index), FK to sources enforced. No dedupe_key column — identity
    IS the staging-row key.
- 0016 (engineering.events + ingest_batches):
  - `events`: UNIQUE (dedupe_key) ONLY. No (import_batch, sheet_name,
    row_index) key — unlike the 0017 events triples.
  - `ingest_batches`: uuid PK, no natural key (same shape as 0017/0013).
- 0003 (ideas mirror):
  - `ideas.ideas`: `dedupe_key text NOT NULL UNIQUE`, FK to
    `ideas.sources(id)` enforced. `sources.name UNIQUE`.
- 0013 (ainventory append-only event log):
  - `ingest_batches`: uuid PK, no natural key.
  - `measurement_events` / `price_events` / `provider_policies`: NO unique
    dedupe constraint. Append-only enforced procedurally: BEFORE
    UPDATE/DELETE triggers raise (`reject_event_mutation`) + REVOKE
    UPDATE/DELETE from service_role/authenticated/anon. Currency resolved
    by views `v_current_measurements` / `v_current_prices` ordering
    (source_date, ingested_at, id) — newest-wins, id tiebreak.
- home domain: NO DDL anywhere in repo (confirms slice-1 manifest note:
  repo-absent by design). Turso `home_*` tables unverified — §E.

## B. Canonical writer conflict/identity semantics

- Batch identity: the writer's stable key is `sources.import_batch`
  (uuid). `AA_BATCH_INDEX_SQL` (`aa-turso-sql.ts:521`) LEFT JOINs
  `ingest_batches` for optional metadata only — a missing parent row must
  never hide a batch (`aa-batches.ts:4-6`). Carrier collapse uses a stable
  fingerprint of batch-bound semantic data; `source_ref`,
  `inserted_counts`, names, timestamps are explicitly NOT dedupe keys
  (`aa-batches.ts:8-12`).
- Staging retries: the unique (source_id, import_batch, sheet_name,
  row_index) key is the conflict target; safe retry = INSERT OR IGNORE
  on that key (proven §C). Same row in a NEW batch is a new row by design.
- Curated retries: 0017 events carry BOTH keys — retry must conflict on
  either; `engineering.records` retries conflict on the staging-row key;
  `ideas.ideas` retries conflict on `dedupe_key`. 0016
  `engineering.events` has only `dedupe_key` — a same-key redelivery
  dedupes, but nothing stops two different dedupe_keys for the same
  logical row.
- Batch creation: uuid PKs with no natural key (0017/0016/0013) mean
  double-create diverges; canonical discipline is read-before-create
  (SELECT id …; INSERT only if absent), proven in §C.
- Ambiguous commit: read back by the stable key (staging-row key and/or
  dedupe_key) before retrying; the suite replays a bounded identical
  input and asserts unchanged counts.
- Recovery writers (`scripts/recovery/import_*.py`) are additive-only
  with post-insert count checks; no destructive sync.

## C. Isolated-target proofs (sqlite3, file-local temp DB)

Suite: `scripts/tests/test_sheet_staging_idempotence.py` — 9/9 PASS
(python3, 2026-11-24, worktree `oc-code-04/6581-slice2-schema-idempotence`):

- TestStagingBatchAssociation (4): duplicate source row idempotent;
  same row in new batch is a new row; row without source refused (FK);
  orphan-detector query (LEFT JOIN ingest_batches) returns [].
- TestBatchCreationIsNotIdempotent (2): double-create diverges (2 rows);
  read-before-create converges (1 row).
- TestCuratedDedupeUpsert (2): same dedupe_key never dupes; same
  batch-row key with rotated dedupe_key never dupes.
- TestAmbiguousCommitRecovery (1): replay of identical staging+curated
  input after read-back changes nothing (1,1).
  No production, staging, or shared DB touched; no network; no secrets.

## D. Mismatches with explicit repair plan

1. 0017 `sheet_rows.source_id` has NO REFERENCES clause (0004/0011 do).
   Orphan rows are conventionally impossible but not DB-refused. REPAIR
   (slice 3, per domain): either add the FK (migration) or keep the
   writer-side orphan-detector query from §C as the gate; do not assume
   the DB refuses them. No DDL change in this slice.
2. `engineering.events` (0016) lacks the (import_batch, sheet_name,
   row_index) key the 0017 events triples carry. Same logical row with a
   fresh dedupe_key inserts twice. REPAIR: slice 3 must either adopt the
   0017 dual-key shape or document that dedupe_key construction alone
   guarantees row-identity; writer change per-domain, one PR each.
3. `measurement_events` / `price_events` have no unique key at all —
   dedupe is view-ordering, not constraint. Safe only with the
   append-only triggers + REVOKE posture from 0013. REPAIR: the Turso
   target must be inspected for equivalent guards (triggers do not port
   to SQLite/Turso verbatim); until proven, writers stay additive-only
   and retries stay read-before-write. Slice 3 proves guard behavior per
   domain on the real target.
4. `ainventory.benchmark_results` is retired (dropped) but still appears
   in older manifests as curated scope. REPAIR: none needed in code;
   slice-1 manifest readers must treat `measurement_events` as canonical
   (already the case in `aa-turso-sql.ts` REVISION 3).
5. home domain: zero repo DDL, zero repo writer/reader. REPAIR: slice 1
   follow-up must source the home DDL/writer from outside the repo
   (deployed Worker, archive, or owner) before any slice-3 writer work
   for home. Home is NOT cleared for writer changes.

## E. Remote coverage gaps (unverified, not declared)

- Deployed Turso `fleet-platform` DDL not inspected in this slice (no
  live read performed): unique indexes, FK equivalents, and append-only
  guard installation on the TARGET are unproven. The §C proofs run on an
  isolated SQLite target only.
- Remote producers/readers beyond the repo (deployed Workers, schedules,
  external consumers for all 19 tables) not inventoried here — slice 1
  already marks 17/19 reader paths `unknown`; this slice does not change
  that.
- The conflicting D1 proposal (Nov-23 audit) remains undecided by Drew;
  no D1 action taken or implied.
- No production destructive test used; no data moved; no credentials
  touched.
