# Shared schema: orchestration state (#5005, phase 1 of #3142)

Status: schema contract and local migration only. This migration has
**not** been applied to Supabase `muse-dev2`; no rows have been copied,
and the orchestration files remain authoritative.

Target-store decision (recorded on
[#5005](https://github.com/developing-today/code/issues/5005) with the
document evidence): orchestration state is new PM state, so it starts
in Supabase `muse-dev2` under the #3142 placement, in the `public`
schema with a new `orch_*` table group — the conventions
[SHARED-SCHEMA-MIGRATION-PLAN.md](../SHARED-SCHEMA-MIGRATION-PLAN.md)
inherits from #1744. The #6581 Supabase→Turso wave plan migrates
existing Supabase table sets and does not classify orchestration
state; if a future wave classifies it, these tables move by the same
shadow → parity → flip pattern and the file export below is the
fallback until that flip verifies.

The state lives in three files today, all in the goal workspace's
`hidden_files/`: `orchestration.json` (the rendered state the #898
dashboard contract serves), `orchestrator-state.json` (the pass's
carried state), and `orchestrator-log.md` (the run log). The contract
maps them onto two tables. The remaining `orchestration.json`
sections (`dependencies`, `goals`, `actions`) are per-run render
content, not classification state: they live in the run's `document`
only, which is also what keeps the export byte-shape identical to
today's file.

## `public.orch_runs` — the run log

One row per orchestration pass; append-only (grants are `SELECT,
INSERT` — history is never rewritten or deleted through the service
role).

| Column | Type / nullability | `orchestration.json` field | Contract |
|---|---|---|---|
| `run_id` | `text` primary key | `runId` | Stable run identity (`orch-<generatedAt>` today). The document's `runId` must equal it. |
| `generated_at` | `timestamptz`, not null | `generatedAt` | When the pass produced the state. |
| `document_version` | `integer`, not null, ≥ 1 | `version` | Export document schema version; the document's `version` must equal it. |
| `provenance` | `jsonb`, not null, object | `provenance` | The sources the pass read, verbatim. |
| `document` | `jsonb`, not null, object | (the whole file) | The complete export document for this run. The generated export is the newest run's `document`, so the #898 dashboard shape is preserved by construction, not by a renderer. |
| `item_count` | `integer`, not null, ≥ 0 | `tiers` lengths | `len(tiers.needsDrew) + len(tiers.needsOrchestrator)` for the run; a cheap parity probe against `orch_items` after a snapshot replace. |

## `public.orch_items` — current classification state

One row per classified item; the table always equals the newest
run's tiers. Item fields beyond the columns below (`horizon`,
`question`, `recommendedAction`, and anything later passes add) live
verbatim in `item` — the columns are the queryable classification
surface, the JSONB is the lossless record, the same split the #6103
contract uses for action payloads.

| Column | Type / nullability | Item field | Contract |
|---|---|---|---|
| `id` | `text` primary key | `id` | Stable item identity; the `item` document's `id` must equal it. Never reused. |
| `tier` | `text`, not null | (the `tiers` key) | Check-constrained to `needsDrew`, `needsOrchestrator` — the two tiers the document carries today. A new tier is a contract change, not data. |
| `kind`, `title`, `source`, `state`, `reason` | `text`, not null | `kind`, `title`, `source`, `state`, `reason` | The classification itself. `kind`/`state` must be non-empty. |
| `ref` | `text`, nullable | `ref` | The item's governing reference (e.g. `gh:owner/repo#n`), when it has one. |
| `since`, `last_seen_at` | `timestamptz`, not null | `since`, `lastSeenAt` | First classified / most recently seen; `last_seen_at >= since` is enforced. |
| `item` | `jsonb`, not null, object | (the whole item) | The item verbatim. |
| `first_seen_run_id`, `last_seen_run_id` | `text`, not null, FK → `orch_runs` | — | The runs that first and most recently carried the item; `ON DELETE RESTRICT` keeps the log's referents honest. |

## Writer contract (the cutover slice's obligations, fixed here)

One transaction per pass, in this order:

1. Insert the `orch_runs` row (run id, generated-at, provenance, the
   complete document, item count).
2. Replace `orch_items` with the document's tiers: delete the
   previous snapshot, insert one row per item, carrying
   `first_seen_run_id` forward for items that already existed.
3. Regenerate `orchestration.json` from the run's `document` (until
   the reader flip, the file is still written every pass — it is
   both the export and the rollback path).

Until that slice lands and verifies (a pass reads its state back
from the tables and the dashboard serves the generated export
unchanged), the files stay authoritative and the writer is the
hourly orchestration pass exactly as today. No flag-day: the reader
flip is a separate slice after the writer has run dual for at least
one full cycle, per the migration plan's reversibility rule.

Rollback at every stage is "point the pass back at the files": the
files are never behind, because step 3 keeps writing them.

## What this slice is not

- No DDL applied to `muse-dev2`, no rows copied, no writer or reader
  changed — the P1 schema-vetting shape the plan's precondition P1
  uses (compare the #6103 actions contract in [README.md](README.md)).
- No Turso placement: see the decision record above. The fleet-queue
  database is untouched by this work, as #6581's end state requires.
