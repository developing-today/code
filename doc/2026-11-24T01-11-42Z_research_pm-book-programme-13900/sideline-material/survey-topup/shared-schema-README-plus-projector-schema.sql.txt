# Shared schema v1: aggregator actions and outcomes

Status: the PostgreSQL DDL here is the original contract for the
action/event tables and the #6815 add-on stores. The live copies of these
tables are in Turso (the Supabase project they were first applied to has
been deleted; the Turso schema was created during the 2026-11-18 recovery,
see `doc/2026-11-18_supabase-turso-recovery.md`). They are the canonical
backing store when the aggregator runs with `PM_AGG_STORE_BACKEND=turso`;
with the default `file` backend the JSON files remain authoritative. The
aggregator reaches them only through `aggregator/src/turso-stores.ts`, which
translates the PostgreSQL-dialect statements in
`aggregator/src/remote-stores.ts` to SQLite (`public.` prefixes dropped,
`jsonb`/`timestamptz` casts removed).

The target uses the `public` schema and the table-group names from
[#1744](https://github.com/developing-today/code/issues/1744):
`public.agg_actions` is owned by the PM aggregator, and
`public.shared_events` is the cross-service event record. The database is a
new backing-store target under #3142, not a change in ownership. The
aggregator remains the source of truth for actions and their outcomes.

## `public.agg_actions`

One row per action record from the version-1 `actions.json` envelope. IDs are
text because action IDs are application-generated strings, not UUIDs.

| Column                                           | Type / nullability                      | JSON action field                         | Contract                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------ | --------------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`                                             | `text` primary key                      | `id`                                      | Stable action identity; never reused or deleted.                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `client_token`                                   | `text`, nullable                        | `clientToken`                             | Per-form idempotency token. A partial unique index covers non-null values; null remains valid for legacy pre-token records.                                                                                                                                                                                                                                                                                                                                                                            |
| `kind`, `verb`                                   | `text`, not null                        | `kind`, `verb`                            | Kinds are `triage`, `status-request`, `session`, `preference`; the check constraint permits only the current `VERBS_BY_KIND` pairs in `../aggregator/src/actions.ts`.                                                                                                                                                                                                                                                                                                                                  |
| `object_ref`                                     | `text`, not null                        | `objectRef`                               | Non-empty object identity.                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `target_ref`, `session_id`, `session_source`     | `text`, nullable                        | `targetRef`, `sessionId`, `sessionSource` | Optional routing/target references.                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `payload`                                        | `jsonb`, not null                       | `payload`                                 | Must be a JSON object; verb-specific validation stays at the application boundary.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `author`                                         | `jsonb`, not null                       | `author`                                  | Must contain a supported `kind`, non-empty `id`, and non-empty `label`. This is self-asserted authorship, **not authenticated identity**.                                                                                                                                                                                                                                                                                                                                                              |
| `created_at`                                     | `timestamptz`, not null                 | `createdAt`                               | Creation time; no `updated_at` field exists in the source contract.                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `state`                                          | `text`, not null, default `new`         | `state`                                   | Check-constrained to `new`, `applied`, `rejected`, `superseded`. A result timestamp is recorded in `applied_at` for applied and consumer-rejected transitions; a check constraint enforces this (`applied` requires `applied_at`, `new` forbids it, `rejected` requires it unless the row is a rejected duplicate — `ActionsStore.add()` creates duplicate attempts directly in state `rejected` with `duplicateOf`/`resultNote` and no consumer timestamp, and those records must import losslessly). |
| `applied_at`, `applied_by_run`, `result_note`    | `timestamptz`, `text`, `text`, nullable | `appliedAt`, `appliedByRun`, `resultNote` | First consumer result time, run ID, and optional explanation.                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `supersedes_action_id`, `duplicate_of_action_id` | `text`, nullable                        | `supersedes`, `duplicateOf`               | Optional self-references with restrictive foreign keys. History is superseded/rejected, never deleted.                                                                                                                                                                                                                                                                                                                                                                                                 |

The primary key, allowed kind/verb/state checks, JSON object checks, and
partial unique `client_token` index are database constraints. The consume-once
rule remains a transactional writer contract: a future result writer must
update only `WHERE state = 'new'` and append its outcome in the same database
transaction. The schema migration does not change that writer.

## `public.shared_events`

This table preserves the aggregator's v2 audit-event fields and adds a
structured action link for DB-authored action outcomes. `id` remains the
event's independent stable text ID; it is not the action ID.

| Column                            | Type / nullability                      | Event field                   | Contract                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------------------------- | --------------------------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                              | `text` primary key                      | `id`                          | Stable event identity.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `event_at`, `ingested_at`         | `timestamptz`, not null                 | `at`, `ingestedAt`            | When the event happened and when the store accepted it; they are distinct. `ingested_at` defaults to database time.                                                                                                                                                                                                                                                                                                                                                                                      |
| `event_at_raw`, `ingested_at_raw` | `text`, nullable                        | `at`, `ingestedAt` (verbatim) | The exact timestamp strings the event hash was computed over (`canonicalFields` in `../aggregator/src/events.ts` hashes the literal strings). `timestamptz` normalizes spellings (`2026-11-05T03:00:00+03:00` and `2026-11-05T00:00:00Z` are the same instant but different hash inputs), so importers MUST store the original strings here and DB-authored writers MUST store the exact strings they feed the hash; without them, exported rows cannot be hash-verified against the JSON rollback path. |
| `actor`, `kind`, `ref`, `summary` | `text`, not null                        | same                          | Current stream fields. Action results use `action-applied` / `action-rejected`, run actor, the action's `objectRef`, and the current summary format.                                                                                                                                                                                                                                                                                                                                                     |
| `attribution`, `source`           | `jsonb`, not null                       | same                          | Attribution must have a supported confidence and boolean `asserted`; source must identify its writer. Defaults preserve current unknown/unasserted attribution and `pm-stack/aggregator` source.                                                                                                                                                                                                                                                                                                         |
| `attribution_raw`, `source_raw`   | `text`, nullable                        | same (verbatim)               | The exact JSON serializations the event hash was computed over: `canonicalFields` includes `attribution` and `source` via `JSON.stringify` of the original objects, and `jsonb` does not preserve object-key ordering, so a round-trip through `jsonb` alone cannot reproduce every accepted original event's hash. Importers MUST store the original serializations here; DB-authored writers MUST store the exact strings they feed the hash.                                                          |
| `corrects_event_id`               | `text`, nullable                        | `corrects`                    | Soft reference, intentionally not a foreign key: event retention can prune the corrected record.                                                                                                                                                                                                                                                                                                                                                                                                         |
| `migrated_from_version`           | `smallint`, nullable                    | `migratedFromVersion`         | If set, must be `1`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `prev_hash`, `hash`               | `text`, not null                        | same                          | Lowercase 64-character SHA-256 chain values. The migration validates shape; serialization and chain ordering remain a single-writer responsibility.                                                                                                                                                                                                                                                                                                                                                      |
| `action_id`                       | `text`, nullable FK to `agg_actions.id` | new DB linkage                | Set for DB-authored action-result events; null is allowed for existing JSON event rows, which have no structured action ID.                                                                                                                                                                                                                                                                                                                                                                              |
| `idempotency_key`                 | `text`, nullable                        | new DB linkage                | For a DB-authored action outcome, use `action-result:<action-id>`. Unique when present. Existing unlinked JSON events may leave it null.                                                                                                                                                                                                                                                                                                                                                                 |

An action outcome must either be unlinked (both fields null, for existing JSON
events) or have both fields set with the matching stable key. Other event kinds
may use an idempotency key without an action link. A partial unique index on
`action_id` allows at most one linked outcome per action; the key index makes
retries idempotent. The two linkage columns are nullable for compatibility
with current JSON outcomes: the
`/api/actions/result` writer currently emits a random event ID and summary but
does not pass an action ID to `recordStreamEvent`. A later DB writer must
populate both fields. This schema-only slice does not alter that behavior or
backfill the unlinked event file. Because the old outcome records cannot be
reliably joined back to actions from their summary text, a future data-move
slice must choose whether to retain those records in `events.json` or import
them as explicitly unlinked history; it must not guess the linkage.

The action author and event actor are provenance claims, not authentication.
Current events default to `{confidence: "unknown", asserted: false}` and
`{identity: "pm-stack/aggregator"}`. Do not upgrade these to authenticated
provenance without the separate #941 trust-hardening work.

RLS is enabled on both tables (PostgreSQL DDL only; Turso has no RLS). The
migration grants `service_role` the minimum table privileges needed by the current contracts:
actions can be read/inserted/updated but not deleted; events can be
read/inserted and pruned under the existing event-retention policy, but not
updated. No client-facing RLS policy or production database write is part of
this change.

## Migration, fallback, and rollback

There is no shared Postgres migration runner yet; migrations are applied by
hand in version order. `migrations/20261105_0001_action_event_store.up.sql`
creates `agg_actions` and `shared_events` with their constraints, indexes, and
grants. `migrations/20261116_0001_aggregator_remote_stores.up.sql` (#6815)
adds, in dependency order after it:

| Table                           | Holds                                                                                                           |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `public.agg_activity_ingestion` | per-source checkpoints, dedupe seen ids, source status                                                          |
| `public.agg_activity_gaps`      | the activity gap ledger (gaps resolve in place)                                                                 |
| `public.agg_usage_snapshots`    | append-only usage observations, dedupe key `service\|metric\|ts\|value`                                         |
| `public.agg_store_documents`    | aggregator-owned JSON documents (`aa-last-good`; namespaced C7 report events use an append-only typed contract) |

`migrations/20261122_0001_shared_comments.up.sql` (#1744, Move 1 step 2)
adds, after those:

| Table                    | Holds                                                                                        |
| ------------------------ | -------------------------------------------------------------------------------------------- |
| `public.shared_comments` | dashboard comments (the DashComment record), the shared cross-service comment table of #1744 |

Each paired `.down.sql` refuses to drop a non-empty table; the #6815 down
migration drops its tables in reverse dependency order. The refusal is
deliberate protection against an accidental data-loss rollback.

Cutover and rollback are the aggregator's `PM_AGG_STORE_BACKEND` switch
(`file` default → `turso`), documented in
[the aggregator README](../aggregator/README.md#store-backend-turso-6815).
Switching to `turso` makes the tables the write path; the local JSON
stores are preserved as a rebuildable cache and remain the rollback target.
The migration test proves the paired PostgreSQL DDL and rollback on a
disposable PostgreSQL 16 instance; it does not connect to any remote
database.

This supplies reversible DDL for the #6815 slice plus the
`shared_comments` contract. It does **not** finish the
remaining `agg_*` / `shared_*` contracts (views, roadmap annotations,
the projector's `pmp_*` groups), decide the projector's P1 location,
or complete P1 / Move 1; #1744 acceptance criterion 3 remains unchecked.

## Disposable test

Run from the repository root:

```sh
bash pm-stack/shared-schema/test-migration.sh
```

The script starts its own temporary PostgreSQL 16 container, applies the up
migrations in order, proves key/foreign-key/check/idempotency behavior
(including legacy-null compatibility, the #6815 store-table constraints,
and the #1744 shared_comments constraints),
proves each down migration refuses non-empty tables, clears only its
disposable fixtures, applies the down migrations in reverse, and verifies
the tables are gone. It does not connect to Turso or the PM Compose
database.
===MIGRATIONS===
CREATE TABLE public.agg_actions (
CREATE TABLE public.shared_events (
CREATE TABLE public.orch_runs (
CREATE TABLE public.orch_items (
CREATE TABLE public.agg_activity_ingestion (
CREATE TABLE public.agg_activity_gaps (
CREATE TABLE public.agg_usage_snapshots (
CREATE TABLE public.agg_store_documents (
CREATE TABLE public.shared_comments (
===PROJECTOR-SCHEMA===
-- PM projector + link registry schema (libSQL / Turso).
-- The projector is glue: GitHub stays the source of truth for code work,
-- Kaneo/Grist stay the source of truth for their own layers, and this
-- database only holds links and projections. It is disposable — every row
-- can be rebuilt from GitHub + the link writes. It never writes back.
--
-- PROJECTION CONTRACT (#4302, audited in #4303 — the README's "Projection
-- contract" section carries the writer inventory behind this statement):
-- A projection writes only what the event proves. An absent field proves
-- nothing — it neither defaults a value nor erases a stored one — and the
-- only timestamps a row carries are ones its own payload reported.
-- Deliveries dedupe on the delivery id, which intake requires; a comment's
-- guard key is the last-update time its payload reports (submitted_at for
-- reviews); deletion is a state every projected kind can reach, and it is
-- absorbing — no later write resurrects a deleted row.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- GitHub items (issues and PRs), keyed by repo-qualified ref "owner/repo#N".
-- A ref is unique and stable across repos; it is the join key everywhere.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_items (
  ref               TEXT PRIMARY KEY,        -- "owner/repo#123"
  repo              TEXT NOT NULL,           -- "owner/repo"
  number            INTEGER NOT NULL,
  kind              TEXT NOT NULL,           -- 'issue' | 'pr'
  url               TEXT NOT NULL,           -- html_url
  api_url           TEXT NOT NULL,           -- api url, for reconciliation
  title             TEXT NOT NULL,
  state             TEXT NOT NULL,           -- 'open' | 'closed'
  state_reason      TEXT,                    -- 'completed' | 'not_planned' | null
  draft             INTEGER NOT NULL DEFAULT 0, -- PRs only
  merged            INTEGER NOT NULL DEFAULT 0, -- PRs only
  author            TEXT,                    -- login
  labels            TEXT NOT NULL DEFAULT '[]', -- JSON array of label names
  assignees         TEXT NOT NULL DEFAULT '[]', -- JSON array of logins
  created_at        TEXT NOT NULL,           -- ISO-8601 (UTC)
  updated_at        TEXT NOT NULL,
  closed_at         TEXT,
  merged_at         TEXT,
  source_updated_at TEXT NOT NULL,           -- updated_at, monotonic guard
  fetched_at        TEXT NOT NULL            -- ISO-8601 when we last wrote it
);

CREATE INDEX IF NOT EXISTS pmp_items_repo ON pmp_items(repo);
CREATE INDEX IF NOT EXISTS pmp_items_state ON pmp_items(state);
CREATE INDEX IF NOT EXISTS pmp_items_kind ON pmp_items(kind);

-- Item deletion markers (#6782): a deleted issue/PR must stay deleted.
-- GitHub numbers are never reused within a repo, so a durable marker keyed
-- by ref makes deletions absorbing for stale webhook/reconcile replays
-- (the item-level mirror of the pmp_comments tombstone contract).
CREATE TABLE IF NOT EXISTS pmp_item_deletions (
  ref               TEXT PRIMARY KEY,        -- "owner/repo#123"
  source_updated_at TEXT NOT NULL,           -- the deleted item's updated_at at the delete event
  fetched_at        TEXT NOT NULL            -- ISO-8601 when the marker was written/refreshed
);

-- ---------------------------------------------------------------------------
-- Comments across all four repos, including PR-interior comments that Kaneo
-- never syncs (reviews and review/line comments). This is the unified
-- comment-activity projection (aggregator requirement #2).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_comments (
  id             TEXT NOT NULL,              -- GitHub comment/review id
  ref            TEXT NOT NULL,              -- item ref this comment belongs to
  kind           TEXT NOT NULL,              -- 'issue_comment' | 'review' | 'review_comment'
  author         TEXT,
  body           TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT,
  url            TEXT,
  -- review comments only:
  path           TEXT,
  line           INTEGER,
  diff_hunk      TEXT,
  -- reviews only:
  review_state   TEXT,                       -- 'commented' | 'approved' | 'changes_requested' | 'dismissed'
  submitted_at   TEXT,
  commit_id      TEXT,
  deleted        INTEGER NOT NULL DEFAULT 0, -- set to 1 on GitHub delete events
  source_updated_at TEXT NOT NULL,           -- last-update time the payload reports (updated_at; submitted_at for reviews) — monotonic guard key, never fabricated
  fetched_at     TEXT NOT NULL,
  PRIMARY KEY (id, kind)
);

CREATE INDEX IF NOT EXISTS pmp_comments_ref ON pmp_comments(ref);
CREATE INDEX IF NOT EXISTS pmp_comments_kind ON pmp_comments(kind);
CREATE INDEX IF NOT EXISTS pmp_comments_created ON pmp_comments(created_at);

-- ---------------------------------------------------------------------------
-- The link registry (aggregator requirement #1): repo-qualified GitHub
-- issue/PR references attached to any board item or roadmap row, regardless
-- of which repo the item belongs to. Subjects are external keys:
--   kaneo:<projectSlug>/<taskId>      board items
--   grist:<docId>/<tableId>/<rowId>   roadmap rows
--   gh:<ref>                          (an item, for one-hop "related" links)
-- relation is a loose, human-chosen label: 'tracks' | 'implements' | 'relates'.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_links (
  subject    TEXT NOT NULL,           -- external key above
  ref        TEXT NOT NULL,           -- "owner/repo#N"
  relation   TEXT NOT NULL DEFAULT 'relates', -- 'tracks' | 'implements' | 'relates'
  note       TEXT,
  created_by TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY (subject, ref, relation)
);

CREATE INDEX IF NOT EXISTS pmp_links_subject ON pmp_links(subject);
CREATE INDEX IF NOT EXISTS pmp_links_ref ON pmp_links(ref);

-- ---------------------------------------------------------------------------
-- Webhook deliveries: idempotency + audit. The GitHub delivery GUID is the
-- unique key, so a redelivered webhook is applied at most once.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_deliveries (
  delivery_id TEXT PRIMARY KEY,
  event       TEXT NOT NULL,
  action      TEXT,
  ref         TEXT,                   -- item ref touched, if any
  received_at TEXT NOT NULL,
  outcome     TEXT NOT NULL,          -- 'applied' | 'duplicate' | 'ignored' | 'error'
  detail      TEXT
);

-- Keep the first failed attempt separately so a later successful retry can
-- update pmp_deliveries without erasing the original failure evidence.
CREATE TABLE IF NOT EXISTS pmp_delivery_errors (
  delivery_id TEXT PRIMARY KEY,
  event       TEXT NOT NULL,
  received_at TEXT NOT NULL,
  outcome     TEXT NOT NULL CHECK (outcome = 'error'),
  detail      TEXT
);

-- Preserve error rows created by earlier versions before retries can replace
-- their current outcome in pmp_deliveries. INSERT OR IGNORE makes this safe on
-- every idempotent migrate() run.
INSERT OR IGNORE INTO pmp_delivery_errors (delivery_id, event, received_at, outcome, detail)
SELECT delivery_id, event, received_at, outcome, detail
FROM pmp_deliveries
WHERE outcome = 'error';

-- ---------------------------------------------------------------------------
-- Reconciliation cursors, one per (repo, stream). Idempotent upserts keyed by
-- repo-qualified IDs are the safety net; these cursors make the poll cheap.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_sync_state (
  repo    TEXT NOT NULL,
  stream  TEXT NOT NULL,              -- 'issues' | 'prs' | 'issue_comments' | 'review_comments' | 'reviews'
  cursor  TEXT,                       -- opaque continuation token / etag / since
  updated_at TEXT NOT NULL,
  PRIMARY KEY (repo, stream)
);

-- ---------------------------------------------------------------------------
-- Push events: recorded for audit and for later exec-strip aggregation.
-- The projector does not write back to GitHub; pushes are read-only here.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_push_events (
  delivery_id TEXT PRIMARY KEY,
  repo        TEXT NOT NULL,
  ref         TEXT,                   -- git ref pushed, e.g. "refs/heads/main"
  head_sha    TEXT,
  before_sha  TEXT,
  pusher      TEXT,
  created_at  TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Hot list (#5204): an append-only, normalized change ledger over the same
-- webhook stream the projector already consumes, plus per-consumer
-- checkpoints. Rows are normalized projections only -- never full raw
-- webhook bodies -- and rebuildable by replaying deliveries. `seq` is the
-- monotonic cursor consumers paginate on; `event_id` is stable per
-- (delivery, sub-event) so a redelivery of the same x-github-delivery
-- appends nothing twice, while distinct deliveries stay distinct rows.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_hot_events (
  seq              INTEGER PRIMARY KEY AUTOINCREMENT, -- monotonic cursor
  event_id         TEXT NOT NULL UNIQUE,   -- stable id: "<delivery>:<sub-event>"
  delivery_id      TEXT,                   -- x-github-delivery, when known
  repo             TEXT NOT NULL,          -- "owner/repo"
  event            TEXT NOT NULL,          -- webhook event name
  action           TEXT,                   -- webhook action, when the event has one
  ref              TEXT,                   -- item ref "owner/repo#N", or git ref for push
  actor            TEXT,                   -- sender / author login, when available
  source_url       TEXT,                   -- html_url of the source object, when available
  source_timestamp TEXT,                   -- source created/updated time, when available
  received_at      TEXT NOT NULL           -- ISO-8601 when the projector received it
);

CREATE INDEX IF NOT EXISTS pmp_hot_events_repo ON pmp_hot_events(repo);
CREATE INDEX IF NOT EXISTS pmp_hot_events_ref ON pmp_hot_events(ref);

-- Per-consumer sweep checkpoints. A checkpoint only ever moves forward and
-- never past the high-water cursor the server issued for that sweep, so a
-- failed sweep resumes from its old checkpoint and replays safely.
CREATE TABLE IF NOT EXISTS pmp_consumer_checkpoints (
  consumer   TEXT PRIMARY KEY,
  cursor     INTEGER NOT NULL DEFAULT 0,   -- last acked pmp_hot_events.seq
  updated_at TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Read model (views). Session K (aggregator UI) consumes these directly from
-- the same libSQL database, or over the HTTP API.
-- ---------------------------------------------------------------------------

-- Every link, joined to the item's live state and title.
CREATE VIEW IF NOT EXISTS pmp_v_links_items AS
  SELECT l.subject, l.ref, l.relation, l.note, l.created_by, l.created_at,
         i.repo, i.number, i.kind, i.url, i.title, i.state, i.state_reason,
         i.draft, i.merged, i.author, i.labels, i.updated_at, i.closed_at
  FROM pmp_links l
  LEFT JOIN pmp_items i ON i.ref = l.ref;

-- One unified activity stream per subject: every comment on every linked
-- item, newest first, with the item's ref and the link relation included.
CREATE VIEW IF NOT EXISTS pmp_v_subject_activity AS
  SELECT l.subject, l.ref, l.relation,
         c.id, c.kind, c.author, c.body, c.created_at, c.updated_at, c.url,
         c.path, c.line, c.review_state, c.deleted,
         i.title AS item_title, i.state AS item_state, i.kind AS item_kind
  FROM pmp_links l
  JOIN pmp_items i ON i.ref = l.ref
  JOIN pmp_comments c ON c.ref = l.ref
  ORDER BY c.created_at DESC, c.id DESC;
