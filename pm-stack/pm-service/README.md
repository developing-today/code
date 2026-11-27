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

## Slice 3 — Hosted read API parity (#12828)

`src/read-api/` serves the #9060 §6 shapes from the tenant DB —
the hosted half of design §4's parity list. Read-only throughout:
the store layer issues SELECTs only, and no token is read, minted,
or persisted on this path (E §2.3).

- `genealogy.ts` — the genealogy read model (#9060 §6), ported from
  the laptop aggregator's #9070 implementation: `ancestors[]`
  (parent / historical-parent / primary-parent / spawned-by /
  roadmap / priority), the release derivation, the sentence
  renderer, and the roadmap-row enrichment. A port, not a redesign;
  chain breaks render honestly ("with no epic yet"), never as
  fabricated links.
- `releases.ts` — milestone close-out accounting (#9060 §4), ported
  from the laptop's #9067 implementation: program milestones get
  retroactive counts; release milestones get the close-out record
  (counts split closed / carried-over, the release date, the
  description line). Plus the assembler that serves a stored
  `pm_release_snapshots` row in the same shape.
- `store.ts` — the tenant-DB reads: board summaries (registry +
  latest snapshot audit per board), the merged pool (current
  `pm_items` rows), roadmap rows (stored genealogy overlay wins
  per-field where set; unset fields derive over the pool), and
  release snapshots (stored records first, computed retroactive
  accounting for the rest).
- `api.ts` — the four endpoints and their fetch handler:
  `/api/projects.json` (`projects[]` + per-board audit + the merged
  pool), `/api/item/<id>.json` (item + `ancestors[]` + `release` +
  the genealogy sentence; unknown ref → structured 404),
  `/api/roadmap.json` (rows with `priority`/`afterRefs`/
  `releaseRef`), `/api/releases.json` (milestone snapshots).

Invariants pinned by `test/read-api.test.ts`:

- **The §8 worked example renders end-to-end.** The crate-split
  chain (step #4403 → epic #3447 → design #3195 → roadmap item
  #2056) serves its full ancestor chain, its release, and the
  genealogy sentence through `/api/item/<id>.json`.
- **Honest absence.** A board with no snapshot serves null audit; an
  item with no chain serves empty ancestors and the chain-break
  sentence; an unknown ref 404s. Nothing is fabricated.

## H4 slice — surface C agent request acceptance (#14262)

`src/agent-front-door.ts` layers the component H design §§4.2/4.5
policy on the shared H1 JWT verifier. `resolveAgentFrontDoor(token,
tenantId, deps)` accepts only an agent-only token, with a single
audience and `scope === binding_id === requested tenant` (never `*`).
Its signed lifetime is at most 900 seconds. It returns only the
`agent` role, even when the owning account has broader live roles.

The injected `loadCredential(jti)` must read **live central tenancy
metadata on every invocation**, never a cached token or secret. The
metadata must match the resolved owner account/binding, tenant and
audience; its lifetime is at most 30 days and contains the token's
lifetime. Missing, revoked or inconsistent rows deny immediately.
`resolveAccountId` and `loadRoles` remain H1's trusted live-tenancy
adapters; the latter must reject inactive accounts/bindings and load
tenant-scoped grants. The audience is trusted deployment configuration,
not caller input. Every accepted request awaits an attribution audit
(`jti`, owner account/binding, tenant); an audit failure denies.
Actions still need their own operation/outcome audit at the API layer.

This is an unwired acceptance seam, **not a production surface**.
Provisional provider contract: `jti` names the revocable credential
metadata handle, `scope` is the tenant binding, `roles` is `["agent"]`.
No token/secret values are stored by this module.

Focused oracle tests live in `test/front-door.test.ts` alongside H1:

```sh
cd pm-stack/pm-service && bun test test/front-door.test.ts
```

## H4 slice 2 — issuance, metadata store, exchange, surface C HTTP (#14262)

`src/agent-credentials.ts`, `src/agent-credential-store.ts`, and
`src/surface-c-api.ts` complete the H4 server-side path on top of the
acceptance seam above:

- **Issuance** (`issueAgentCredential`): minted from the H2 owner
  session (the front-door-resolved principal). Only `owner` /
  `orchestrator` sessions mint; the tenant is the session's own
  binding, exactly one tenant, never `'*'`; TTL is at most 30 days,
  rotation by re-issuance. The secret is generated at issuance,
  returned exactly once, and never stored.
- **Metadata store**: the central tenancy plane (E's `credential`
  home), NOT a tenant-DB migration — the tenant plane holds no
  credential rows. A row is metadata (`jti`, owner
  `account_id`/`binding_id`, `tenant_id`, audience, lifetime,
  revocation stamp) plus a SHA-256 digest of the secret so the
  exchange can verify a presented secret; presenting the digest
  itself does not authenticate. In-memory and `SqlDb` adapters are
  provided; every read is live, so revocation by `jti` lands within
  one request.
- **Exchange** (`exchangeClientCredentials`): the surface C
  client-credentials flow. A live, unrevoked, unexpired credential
  plus its secret exchanges for an audience-bound token with a
  15-minute TTL and no refresh token; signing is injected (the
  provider owns the keys). All failures are one `invalid_client`.
- **HTTP** (`createSurfaceCHandler`): `POST /api/agent-credentials`
  (owner session; the 201 is the only response that ever shows a
  secret), `DELETE /api/agent-credentials/<jti>` (owner-scoped
  revocation), `POST /oauth/token` (the exchange), and
  `GET /api/agent/whoami` — a protected resource accepted only
  through `resolveAgentFrontDoor`, returning the agent principal
  plus its audit attribution (`jti` + owning account/binding).

Remaining before H4 closes: the browser issuance UI (the API above
is its backend) and wiring this handler into the deployed Worker
entry with the real H2 session resolver and provider signer.

Oracle tests live in `test/agent-credentials.test.ts`:

```sh
cd pm-stack/pm-service && bun test test/agent-credentials.test.ts test/front-door.test.ts
```

Dispatch this command remotely using Fleet; no production credentials
are needed (tests sign ephemeral fixture tokens).

## CF-5c — async serving substrate + Worker entry (#14969)

The read API was born over the synchronous `SqlDb` surface, whose
only adapter is bun:sqlite — fine for tests and local tooling, but
a Worker cannot run it, and the tenant plane (Turso/libSQL over
HTTP) is async. This slice adds the async twin without touching
the synchronous path's behaviour:

- `src/tenant-store/libsql-http.ts` — `AsyncSqlDb` (the one method
  the read path needs) and `libsqlHttpDb`, its adapter over the
  libSQL server protocol (Hrana v3 over HTTP, plain fetch). The
  tenant DB URL/token live only in the adapter's closure, supplied
  from Worker secrets; nothing on the read path reads, logs, or
  persists a token (E §2.3).
- `src/read-api/async-store.ts` — the four read loads awaited.
  Every SQL string and row→shape mapping is imported from
  `store.ts` / `collector/store.ts` (whose mappings were extracted
  for exactly this sharing), so the served shapes cannot drift.
- `src/read-api/async-api.ts` — the same four routes over the
  async store; `test/read-api-async.test.ts` pins every route's
  status and body byte-identical to the synchronous handler's.
- `src/worker.ts` + `wrangler.toml` — the Worker entry. `/health`
  reports `{ ok, service, version, deployedAt, ageSeconds }` with
  the VERSION and DEPLOYED_AT vars the
  deploy workflow stamps per deploy; until the
  `PM_TENANT_DB_URL` / `PM_TENANT_DB_TOKEN` secrets are set, the
  API routes answer a structured 503 (an unconfigured store is an
  honest state, never invented data).
- `.github/workflows/pm-service-deploy.yml` — the design §4
  versions standard: test, `wrangler versions upload` stamped
  with the full commit SHA and timestamp, health gate after
  promotion (`scripts/check-health.mjs`), and rollback by promoting
  the prior version if the exact-SHA or age check fails. Cloudflare
  does not provide Version URLs for Workers with Durable Objects.

```sh
cd pm-stack/pm-service && bun test && bun run typecheck
```

## CF-5d — M5 read waves over tenant-DB projections (#14970)

First wave of the CF-5 programme's M5 surfaces. Per-surface record:

- **Overview — moved.** `GET /api/overview.json`: pool-wide totals
  (boards / items / open / closed / merged / drafts) and breakdowns
  by state, status, area, horizon, kind, and content kind, plus the
  board summaries it derives from.
- **Queue — moved.** `GET /api/queue.json`: the open pool grouped
  by project Status, split into `blocked` (held by a still-open
  pool item) and `readyRefs` (no live blocker) — the hosted half of
  the console's Now/queue family.
- **Stayed (later waves):** board and roadmap already serve as the
  §6 routes above; program, quota, events, today, digest, reports,
  /aa, and /chat remain on their current stacks. Their data is not
  in the tenant projections yet, so their waves land their
  projection first — nothing here fabricates them.

Both new routes are pure derivations in `src/read-api/waves.ts`
over the projections the existing routes already read (merged pool

- board summaries), served by both handlers (`api.ts` /
  `async-api.ts`); the async parity suite pins them byte-identical.

* **Freshness visible:** every wave response carries `freshness` —
  the per-board latest snapshot stamps and the newest as
  `latestSnapshotAt` (null when no snapshot has landed).
* **Write-amplification check (#14865 gate):** zero new writes.
  This wave adds no projection, table, or write path; it issues
  only the SELECTs the existing routes already issue, so the
  collector's snapshot landing remains the sole writer.
* **Live cutover** follows the CF-5c deploy: the routes ship with
  the Worker on its next versions deploy; until the tenant secrets
  land (#14969's remaining acceptance) the Worker serves them as
  the designed honest 503, like the §6 routes.

## Slice 4 — dashboard surfaces (#12829)

`src/dashboard/dashboard.ts` serves the design §4 V1 parity list as
server-rendered HTML, read through the same async loads the JSON API
uses, so a page and its JSON twin cannot disagree. The Worker routes
dashboard paths first and falls through to the API for the rest.

| Path          | Page                                                                                |
| ------------- | ----------------------------------------------------------------------------------- |
| `/`           | every board in the registry, its latest snapshot and status counts                  |
| `/board/<id>` | one board's items from its latest snapshot                                          |
| `/item/<ref>` | item detail: genealogy header (root first), sentence, release, after, fields        |
| `/roadmap`    | overall priorities (rows grouped by priority) and the per-project roadmap (by area) |
| `/releases`   | milestone close-out cards with carried-over epics                                   |
| `/attention`  | open reconcile findings plus the coverage checks over each board's snapshot         |
| `/quota`      | names quota/usage as not yet in the tenant store                                    |

GETs only (anything else is a 405), SELECTs only, no token on the
path. Absence renders as absence: a board with no snapshot says so,
an unknown board or item is a 404 page, and quota shows no numbers it
cannot back (#14065). Pages behind the same 503 as the API until the
tenant store secrets are set.

Not in this slice, each tracked separately: quota/usage data in the
tenant store, the Kaneo/Pad/Grist embeds, dispatch intake, the console
gate, push notifications and the AA extras.
