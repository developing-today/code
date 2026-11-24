# The shared schema as a library

`pm-stack/shared-schema/` is the shared-store contract layer of the PM stack. Components that write shared orchestration state to the Turso `fleet-platform` store do it under this package's contracts, in its transaction shape, under its guards. This chapter describes the package at the depth the surveys support: its parts, its writer, its consumers as named in code and docs, and the map that indexes the shared stores.

## The parts

The package directory carries:

- `ORCHESTRATION.md` — the orchestration contract document for the package.
- `orchestration-writer.py` — the writer for orchestration state to Turso `fleet-platform`. Its rule is one transaction per pass (#5005, #3142).
- `migrations/` — the schema migrations for the shared stores the package covers.
- `ainventory-turso-guards.sql` — guard SQL for the Turso inventory surface, under the file's own name as surveyed.
- Migration and orchestration tests, shipped with the package.

The inventory context for the package is `pm-stack/SHARED-SCHEMA-MAP.md`. The migration plan that sits next to it is `pm-stack/SHARED-SCHEMA-MIGRATION-PLAN.md`. Read the map before the migrations: the map names which store a contract belongs to.

A fuller export list — every table, view, and guard the package defines, with signatures — is UNVERIFIED from the surveys. The surveys name the files above and the writer's transaction rule, and no complete export catalogue. A chapter that enumerated tables from the migrations alone, without the map's store assignments, invents the package's public surface.

## The writer's contract

`orchestration-writer.py` writes orchestration state in one transaction per pass. That shape has three consequences a consumer relies on:

1. A pass lands whole or not at all. A reader never reads half a pass.
2. Ordering between passes is transaction order in the store. There is no second ordering channel.
3. A failed pass leaves the previous pass's state in place. Recovery is a new pass, not a repair of a partial one.

The orchestration state this writer carries is the same state the dashboard actions act on: dashboard actions are consumed by the orchestration pass, and the pass is the Project's single writer for Project fields. See [Tutorial: follow an item from issue to done](../tutorials/follow-an-item.md) for the action path an item takes into that pass.

## Who consumes it

Consumers named in the surveys:

- The orchestration pass itself, writing to `fleet-platform` through the writer.
- The aggregator, as the reader of the shared stores: the aggregator's projections join orchestration state with its other sources, and its quota-check helper (`pm-stack/quota-check/turso-mirror-read.py`, #6828) reads the `fleet-platform` and `fleet-mirror` stores per database over the libsql HTTP API, with per-database tokens fetched by name. That helper is explicitly not the Turso CLI's org token route.
- The decision-events pipeline, as a writer to the same store family: decision-set activity events land in `fleet-platform` in the `decision_set_events` table, through `aggregator/scripts/sync-decision-events.ts` and the decision-events Worker.

The complete consumer list is UNVERIFIED from the surveys. The surveys name the writer, the store, and the readers above; they do not enumerate every script that imports the package or applies its migrations. Treat any consumer not named here as unconfirmed until its import or its store writes are read in code.

## The package in the stack's store picture

`fleet-platform` is the PM and dashboard plane store: agent mirror, quota, aggregator ingest families, alerts, and decision events. It is the store the #13844 Turso audit assigns the largest share of estate writes. `fleet-mirror` is the lagging second copy of the agent-mirror dataset. Both stores are PM-book data stores; their estate context is in the coverage ledger for this programme.

The shared schema is the contract layer over that picture, not a store itself. Code that needs the shape of a shared table reads the package's migrations and the map. Code that needs the data reads the store.

## Using the package

1. Read `pm-stack/SHARED-SCHEMA-MAP.md` for the store and table in question.
2. Read the migration that defines the table, in `pm-stack/shared-schema/migrations/`.
3. Write through the package's writer shape: one transaction per pass, for orchestration state.
4. Run the package's migration and orchestration tests with the change.

Step 3 is the rule that keeps the package a library and not a naming convention: a writer that bypasses the transaction shape breaks the three consequences in the section above for every reader.

Sources. survey/apps.md §23, §25, §36; survey/roadmap.md §3–§4; coverage-ledger.md Table B (`fleet-platform`, `fleet-mirror`). Ground truth: laptop clone at 4d937b3, 2026-11-23.
