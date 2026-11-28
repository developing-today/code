# PM service, Workers edition (component A)

The hosted edition of the PM service: the laptop aggregator's
capabilities re-implemented in TypeScript as one hosted service on
Cloudflare Workers, consuming the **same data model** as the laptop
edition (#9060), so both editions read the same world.

Design: `doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition/`
(component A of epic #9078, under #9079). Implementation slices are
§8 of that design; this package grows one slice at a time.

## Slice 1 — PM tenant store: schema + migrations (#12825)

`src/tenant-store/` is the store the rest of the service builds on:

- `migrations/0001_pm_tenant_store.sql` — the DDL of record. One
  schema in the tenant's Turso DB (component E tenant plane, #9083
  §2.2) holding the whole PM store: board registry rows (#9060 §1
  `BoardDef`, stored as data), collector snapshots, the merged item
  pool, hierarchy/link records (#9060 §11), Metadata copies (#9060
  §12), roadmap rows, milestones + release-milestone close-out
  snapshots (#9060 §4), reconcile findings, and the writer-of-record
  lease row (design §5).
- `migrate.ts` — the migration runner: applies pending migrations in
  version order, each in a transaction, and records them in
  `pm_schema_migrations` (the schema-version record). Runtime-agnostic
  over a minimal `SqlDb` surface.
- `load.ts` — loads the `NNNN_name.sql` files as migrations.
- `schema.ts` — the wire shapes the store holds (`BoardDef`,
  `LinkRecord`, `PmMetadataBlock`, `WriterLease`) and their
  validators; the conformance contract against #9060 and the laptop
  collector's output types.
- `sqlite.ts` — a bun:sqlite `SqlDb` adapter for tests/local tooling
  (a tenant DB is SQLite-shaped wherever it lives; Turso/libSQL
  speaks the same dialect).

Invariants carried by the schema and pinned by the test suite:

- **Tenant plane only.** These migrations are never applied to the
  central tenancy DB — no PM content is central (E §2.3).
- **No tokens or keys in any table.** The suite fails if any column
  name in the schema names a token, secret, password, credential, or
  key value.
- **Shapes match #9060 exactly.** Registry / pool / link-record /
  Metadata fixtures in the suite are the collector's landed output
  shapes (`pm-stack/aggregator/src/model.ts`, `board-registry.ts`).

Run the suite:

```sh
bun test        # from this directory
bun run typecheck
```

## Slice 2 — Hosted collector (#12826)

`src/collector/` is the registry-driven multi-board ingest of
design §3, running on Workers Cron Triggers:

- `pool.ts` — the same GraphQL pool query the laptop collector runs
  (fields, views, `parent`, `blockedBy`/`blocking`, labels,
  milestones, `subIssuesSummary`) and the parser that maps it to the
  `ProjectPoolItem` / `ProjectAudit` shapes slice 1's schema is
  pinned against. A port, not an import — this package deploys as
  one Worker bundle; the dual-edition conformance suite (design §8
  slice 8) pins the two implementations to identical output.
- `fetch.ts` — one board's paginated snapshot fetch. The board's
  stored ETag rides the first page as `If-None-Match`; a 304 ends
  the run with no page fetches and no writes. GitHub rate-budget
  headers are captured per response.
- `ingest-do.ts` — `IngestDO`, one per board: serializes the board's
  runs and holds its ingest cursors (ETag / last-snapshot watermark)
  and rate-budget state. Derived and rebuildable (design §7
  invariant 6) — losing it costs one full re-fetch, never data.
- `store.ts` — the tenant-DB writes: registry reads, snapshot
  landing (header + pool replacement per board, so a failed board
  keeps its previous pool), Metadata-copy upserts (§12), and
  ingest findings.
- `collector.ts` — `runHostedCollector`: per board, mint a per-org
  App installation token through the injected tenancy seam, fetch,
  and collect; then resolve epic/spawn ancestry over the **merged**
  pool (cross-board chains resolve) and land each board's snapshot.
  Fail-soft per board throughout: a failure records an
  `ingest-failure` finding and never empties another board's pool;
  a board whose rate budget is spent defers with an
  `ingest-deferred` finding.

Invariants pinned by `test/collector.test.ts`:

- **No token persists.** Tokens are minted per run, used for one
  board's fetch, and dropped; the suite scans every tenant-DB
  table, the DO state, and the run report for the token value.
- **Incremental second runs.** With a stored ETag, a 304 board does
  exactly one conditional request and writes nothing.
- **Writer provenance.** Landed snapshots record `writer='hosted'`
  and the board's writer-lease epoch (design §5/§7).
