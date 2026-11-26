# The shared schema

The shared schema is the contract layer for the stores that more than one PM component writes. It lives in `pm-stack/shared-schema/`, with its inventory map at `pm-stack/SHARED-SCHEMA-MAP.md` and its migration plan at `pm-stack/SHARED-SCHEMA-MIGRATION-PLAN.md`. Its centre is orchestration state: the record of what the orchestration pass has published — tiers, items, and the state other surfaces read back — written to the Turso `fleet-platform` store by one writer, in one transaction per pass.

This chapter states what the survey establishes about the layer: the writer, the write rule, the migrations and guards, the table inventory (survey top-up 1, T2), and the single-writer rules it supports elsewhere in the stack.

## Why a contract layer exists

The stack's components share stores. The aggregator publishes orchestration state; the orchestration pass consumes dashboard actions and performs Project writes; decision events, quota records, alerts, and mirror data land in the same `fleet-platform` store family. Without a shared contract, each writer invents its own table shapes and its own write pattern, and a reader cannot tell which writer owns a row or whether a multi-row update landed whole.

The shared schema answers with three mechanisms: a map that inventories the shared stores, migrations that change their shape in the open, and a single writer for orchestration state that commits each pass as one transaction.

## The orchestration writer

`pm-stack/shared-schema/orchestration-writer.py`, specified in `pm-stack/shared-schema/ORCHESTRATION.md`, is the writer for orchestration state to Turso `fleet-platform`. It was built under issues #5005 and #3142. Its rule is in its description: one transaction per pass. A pass that publishes a new orchestration state commits all of that pass's writes together or not at all, so a reader never reads half a pass.

That rule pairs with the stack's wider single-writer rule for portfolio state:

- The orchestrator is the single writer of GitHub Project fields. Agents and the dashboard do not edit Project fields directly; the aggregator reads the pool through the fleet feed (`pm-stack/README.md`).
- A dashboard status change is therefore not a write. It is a `status-request` action, consumed by the orchestration pass, which performs the write itself and reports the outcome back through `POST /api/actions/result`. The item's next published tier entry carries the acknowledgement.
- Hierarchy and dependency edges are kept distinct in the same spirit: hierarchy is native GitHub sub-issue parents, dependencies are native `blockedBy` and `blocking` relations, and prose is a substitute for neither (`pm-stack/README.md`; the dependency handling lives in `aggregator/src/orchestration-dependency.ts`, `epic-graph.ts`, and `graph.ts`).

## Migrations and guards

The directory carries `migrations/` — the schema's changes as migrations — and `ainventory-turso-guards.sql`, a set of SQL guards for the Turso inventory. Migration and orchestration tests sit alongside them in `pm-stack/shared-schema/`. The book's rule for this layer follows from its shape: a shared-store change lands as a migration in this directory, with its test, not as an ad hoc change inside one component's code.

The live Turso copies are kept current by a runner in the same directory (#14256): `turso_migrations.py` translates each PostgreSQL-dialect migration to SQLite under the aggregator's own translation conventions, refusing constructs it has no rule for, and `apply-store-migrations.py` applies the pending set in version order — one transaction per migration, each recorded in the store's `pm_schema_migrations` ledger so a re-run is a reported no-op. The translation is pinned by reviewed golden files, and the row-level contract is proven against the translated schema by `test_turso_migrations.py`. The `pm-store-migrate` workflow is the dispatch surface: a stand-in store proves the set in CI, and a named store from the platform inventory runs only with its token secret configured and its name repeated as confirmation.

The table inventory, from `pm-stack/SHARED-SCHEMA-MAP.md`, `pm-stack/shared-schema/migrations/`, and `pm-stack/projector/schema.sql` (survey top-up 1, T2):

**The projector's tables** (owner/writer: the projector; the full treatment is in [The projector and projections](projector.md)): 10 tables — `pmp_items`, `pmp_item_deletions`, `pmp_comments`, `pmp_links`, `pmp_deliveries`, `pmp_delivery_errors`, `pmp_sync_state`, `pmp_push_events`, `pmp_hot_events`, `pmp_consumer_checkpoints` — plus 2 views, `pmp_v_links_items` and `pmp_v_subject_activity`.

**The shared-schema migration tables** — 9 tables, and their live copies are in Turso: the PostgreSQL DDL in `pm-stack/shared-schema/` is the original contract, the Supabase project they were first applied to has been deleted, and the Turso schema was created during the 2026-11-18 recovery (`pm-stack/shared-schema/README.md`). They are canonical when the aggregator runs with `PM_AGG_STORE_BACKEND=turso`; with the default `file` backend the JSON files remain authoritative. The nine: `agg_actions` (aggregator — one row per `actions.json` action record), `shared_events` (aggregator, cross-service — the cross-service event record with a SHA-256 hash chain), `orch_runs` (orchestration writer — one append-only row per orchestration pass), `orch_items` (orchestration writer — current tier and classification state, replaced per run in the writer's transaction), `agg_activity_ingestion` (aggregator — per-source ingestion checkpoints and dedupe), `agg_activity_gaps` (aggregator — the activity gap ledger), `agg_usage_snapshots` (aggregator and usage writers — append-only usage observations), `agg_store_documents` (aggregator — keyed JSON documents), and `shared_comments` (aggregator — dashboard comments, the shared cross-service comment table of #1744).

**Not yet tables:** `shared_items` and `shared_links` are target group names only — the shared-schema README states the DDL does not finish the remaining `agg_*` / `shared_*` contracts, and #1744 acceptance criterion 3 remains unchecked. The canonical `shared_links` source today is the projector's `pmp_links`. Several aggregator-owned stores are still files, not tables (among them `fleet-feed.json`, `comments.json`, `actions.json`, `roadmap-annotations.json`, `views.json`, the aggregator `registry.json`, and the quota caches), each with its `agg_` target group named in the map.

## What the shared schema is not

- **It is not the projector's schema.** The projector's link registry has its own schema at `pm-stack/projector/schema.sql`, owned by the projector service. See [The projector and projections](projector.md).
- **It is not Kaneo's Postgres.** Kaneo's integration settings, including the status-transition configuration, live in Kaneo's own Postgres and are set by nothing in this repository. The store-migration plan at `pm-stack/STORE-MIGRATION-PLAN-6816.md` defines a gated cutover path off Kaneo Postgres toward Supabase, with presync and promote modes, a fence, and an exact-parity gate — a plan for a migration, recorded as gated, not a completed move.
- **It is not a hierarchy type system.** No shared enum defines the roadmap ladder. Project pool Kind is exactly `Task` or `Epic` in `aggregator/src/model.ts`; programs are a document scheme and an Area value; "aggregator" names a service (D10). The shared schema carries shared _store_ contracts, and the ladder's types live in the systems that own each level. See [The roadmap hierarchy: initiatives, projects, epics, aggregators](roadmap-hierarchy.md).
- **It is not the quota store's whole story.** Quota-side records — pool guidance, forecasts, and reweight proposals — are recorded as records and pull requests, not applied as silent reprioritisation. Detail beyond that statement is UNVERIFIED in the survey this chapter draws on.

## Reading the layer

For a change to shared state, read in this order: `SHARED-SCHEMA-MAP.md` for which store and which writer own the record; `ORCHESTRATION.md` for the orchestration writer's contract; the migration files for the shape as it stands; and [The aggregator: one state, many sources](aggregator.md) for how the published state is read back into pages. [Architecture](architecture.md) places the layer among the stack's ledgers, sync, and shared contracts.

Sources. Survey files `~/workspace/pm-book/survey/apps.md` (§25, §36, §1 store-migration pointer), `~/workspace/pm-book/survey/roadmap.md` (§1, §3, §4), `~/workspace/pm-book/survey/disagreements.md` (D10); repo paths `pm-stack/shared-schema/ORCHESTRATION.md`, `pm-stack/shared-schema/orchestration-writer.py`, `pm-stack/shared-schema/migrations/`, `pm-stack/shared-schema/ainventory-turso-guards.sql`, `pm-stack/SHARED-SCHEMA-MAP.md`, `pm-stack/SHARED-SCHEMA-MIGRATION-PLAN.md`, `pm-stack/STORE-MIGRATION-PLAN-6816.md`, `pm-stack/README.md`.
