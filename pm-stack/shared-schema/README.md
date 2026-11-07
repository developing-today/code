# Shared schema v1: aggregator actions and outcomes

Status: schema contract and local migration only. This migration has **not**
been applied to Supabase `muse-dev2`; no rows have been copied, and the
aggregator's JSON files remain authoritative.

The target uses the `public` schema and the table-group names from
[#1744](https://github.com/developing-today/code/issues/1744):
`public.agg_actions` is owned by the PM aggregator, and
`public.shared_events` is the cross-service event record. The database is a
new backing-store target under #3142, not a change in ownership. The
aggregator remains the source of truth for actions and their outcomes.

## `public.agg_actions`

One row per action record from the version-1 `actions.json` envelope. IDs are
text because action IDs are application-generated strings, not UUIDs.

| Column | Type / nullability | JSON action field | Contract |
|---|---|---|---|
| `id` | `text` primary key | `id` | Stable action identity; never reused or deleted. |
| `client_token` | `text`, nullable | `clientToken` | Per-form idempotency token. A partial unique index covers non-null values; null remains valid for legacy pre-token records. |
| `kind`, `verb` | `text`, not null | `kind`, `verb` | Kinds are `triage`, `status-request`, `session`, `preference`; the check constraint permits only the current `VERBS_BY_KIND` pairs in `../aggregator/src/actions.ts`. |
| `object_ref` | `text`, not null | `objectRef` | Non-empty object identity. |
| `target_ref`, `session_id`, `session_source` | `text`, nullable | `targetRef`, `sessionId`, `sessionSource` | Optional routing/target references. |
| `payload` | `jsonb`, not null | `payload` | Must be a JSON object; verb-specific validation stays at the application boundary. |
| `author` | `jsonb`, not null | `author` | Must contain a supported `kind`, non-empty `id`, and non-empty `label`. This is self-asserted authorship, **not authenticated identity**. |
| `created_at` | `timestamptz`, not null | `createdAt` | Creation time; no `updated_at` field exists in the source contract. |
| `state` | `text`, not null, default `new` | `state` | Check-constrained to `new`, `applied`, `rejected`, `superseded`. A result timestamp is recorded in `applied_at` for both applied and rejected transitions; a check constraint enforces this (`applied`/`rejected` require `applied_at`, `new` forbids it). |
| `applied_at`, `applied_by_run`, `result_note` | `timestamptz`, `text`, `text`, nullable | `appliedAt`, `appliedByRun`, `resultNote` | First consumer result time, run ID, and optional explanation. |
| `supersedes_action_id`, `duplicate_of_action_id` | `text`, nullable | `supersedes`, `duplicateOf` | Optional self-references with restrictive foreign keys. History is superseded/rejected, never deleted. |

The primary key, allowed kind/verb/state checks, JSON object checks, and
partial unique `client_token` index are database constraints. The consume-once
rule remains a transactional writer contract: a future result writer must
update only `WHERE state = 'new'` and append its outcome in the same database
transaction. The schema migration does not change that writer.

## `public.shared_events`

This table preserves the aggregator's v2 audit-event fields and adds a
structured action link for DB-authored action outcomes. `id` remains the
event's independent stable text ID; it is not the action ID.

| Column | Type / nullability | Event field | Contract |
|---|---|---|---|
| `id` | `text` primary key | `id` | Stable event identity. |
| `event_at`, `ingested_at` | `timestamptz`, not null | `at`, `ingestedAt` | When the event happened and when the store accepted it; they are distinct. `ingested_at` defaults to database time. |
| `event_at_raw`, `ingested_at_raw` | `text`, nullable | `at`, `ingestedAt` (verbatim) | The exact timestamp strings the event hash was computed over (`canonicalFields` in `../aggregator/src/events.ts` hashes the literal strings). `timestamptz` normalizes spellings (`2026-11-05T03:00:00+03:00` and `2026-11-05T00:00:00Z` are the same instant but different hash inputs), so importers MUST store the original strings here and DB-authored writers MUST store the exact strings they feed the hash; without them, exported rows cannot be hash-verified against the JSON rollback path. |
| `actor`, `kind`, `ref`, `summary` | `text`, not null | same | Current stream fields. Action results use `action-applied` / `action-rejected`, run actor, the action's `objectRef`, and the current summary format. |
| `attribution`, `source` | `jsonb`, not null | same | Attribution must have a supported confidence and boolean `asserted`; source must identify its writer. Defaults preserve current unknown/unasserted attribution and `pm-stack/aggregator` source. |
| `corrects_event_id` | `text`, nullable | `corrects` | Soft reference, intentionally not a foreign key: event retention can prune the corrected record. |
| `migrated_from_version` | `smallint`, nullable | `migratedFromVersion` | If set, must be `1`. |
| `prev_hash`, `hash` | `text`, not null | same | Lowercase 64-character SHA-256 chain values. The migration validates shape; serialization and chain ordering remain a single-writer responsibility. |
| `action_id` | `text`, nullable FK to `agg_actions.id` | new DB linkage | Set for DB-authored action-result events; null is allowed for existing JSON event rows, which have no structured action ID. |
| `idempotency_key` | `text`, nullable | new DB linkage | For a DB-authored action outcome, use `action-result:<action-id>`. Unique when present. Existing unlinked JSON events may leave it null. |

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

RLS is enabled on both tables. The migration grants the Supabase
`service_role` the minimum table privileges needed by the current contracts:
actions can be read/inserted/updated but not deleted; events can be
read/inserted and pruned under the existing event-retention policy, but not
updated. No client-facing RLS policy or production database write is part of
this change.

## Migration, fallback, and rollback

There is no shared Postgres migration runner yet.
`migrations/20261105_0001_action_event_store.up.sql` creates only these two
tables, constraints, indexes, and grants. Its paired `.down.sql` refuses to
drop either table while it contains rows, then drops `shared_events` before
`agg_actions`. The refusal is deliberate protection against an accidental
data-loss rollback.

For a future store move, retain `actions.json` and `events.json` as the
authoritative write and read path while DB writes are only a mirror. Enable DB
reads only after parity checks pass; on a DB outage or read-parity failure,
fall back to those files. A later writer cutover must preserve a replayable
JSON copy. If rollback happens after any DB-only writes, stop DB writes, export
and replay missing rows to JSON in action/event order, verify IDs, client
tokens, outcomes, and event hashes, then route reads and writes back to the
files. Empty the target tables only after that verification; the down
migration refuses to drop non-empty tables. No reader, writer, migration
runner, or live row is changed by this slice.

This supplies the first column contract and reversible DDL artifact for P1 /
Move 1 (`actions.json` → `agg_actions` + action outcomes in `shared_events`).
It does **not** apply the DDL to `muse-dev2`, migrate or cut over the action
store, finish the remaining `agg_*` / `shared_*` contracts, decide the
projector's P1 location, or complete P1 / Move 1. #1744 acceptance criterion
3 remains unchecked.

## Disposable test

Run from the repository root:

```sh
bash pm-stack/shared-schema/test-migration.sh
```

The script starts its own temporary PostgreSQL 16 container, applies the up
migration, proves key/foreign-key/check/idempotency behavior (including
legacy-null compatibility), proves the down migration refuses non-empty
tables, clears only its disposable fixtures, applies down, and verifies the
tables are gone. It does not connect to Supabase or the PM Compose database.
