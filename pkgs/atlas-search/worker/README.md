# atlas-search Worker

The cross-database search surface (developing-today/code#6788):
a Cloudflare Worker that pulls all Atlas databases for search
through the fleet search service; for each query it stores a
**Durable Object**, automatically inserts the query as an
**embedding** (the pinned Snowflake space — see `src/space.ts`),
and caches the result **indefinitely** (no TTL/eviction in v1).

## Shape

- `src/index.ts` — the inbound gate and routing. Normalization
  and keying live in `src/key.ts`; index.ts calls `queryKey`
  directly only for the inspect/purge routes (search keying runs
  in `src/search.ts`).
- `src/cache_do.ts` — the `QueryCache` DO: one per query key;
  check-and-fill runs inside the DO so a fan-out happens once.
  State machine: absent → embedded → complete; the embedding is
  persisted before the search runs.
- `src/service.ts` — calls to `fleet serve search`
  (`POST /v1/embed`, `POST /v1/search`) via the
  `atlas-search.security.cab` tunnel hostname + Cloudflare
  Access service token. The service enforces this client's
  `personal` tier ceiling, and the Worker enforces it too: every
  search response's `effective_ceiling` is checked against the
  pinned ceiling and a mismatch is refused (502) before anything
  is cached. The Worker holds no Turso credentials
  and keeps no shard list (new `atlas-core-*` databases join the
  fan-out through the service's manifest discovery).
- `src/key.ts` — query normalization and DO keying. The key is
  sha256 over the canonical key material (KEY_SCHEMA 2): the
  normalized query plus `"v": 2`, the pinned ceiling, the
  disclosure context (`hosted-per-user`, #12622), model
  revision, vector table, and k. A cached entry is never served
  across ceilings, disclosure contexts, or embedding spaces;
  entries keyed under schema v1 are orphaned, never served.

## Endpoints

- `GET /health` — open liveness.
- `POST /v1/search` `{query, k?}` — gated (`Authorization:
Bearer <GATE_TOKEN>`). Returns `{cache: "miss"|"hit", key,
query, result, meta}`.
- `GET /v1/cache/inspect?query=..&k=..` — gated; DO metadata
  readback (embedding presence/dims/validity, result presence,
  hit count, timings). The proof/audit surface.
- `POST /v1/cache/purge` `{query, k?}` — gated with the same
  `GATE_TOKEN`; removes exactly that normalized query/config cache
  entry. The query key is a SHA-256 digest, so per-prefix purge
  cannot enumerate matching entries without a separate index and is
  intentionally not provided.
- `POST /mcp` — gated with the same `GATE_TOKEN`; a stateless
  MCP (Model Context Protocol) Streamable HTTP endpoint (#8625)
  exposing one tool, `search`, over the exact same execution
  path as `POST /v1/search` (same pinned context and ceiling,
  same Durable Object cache — an MCP search and a REST search
  for the same query share one cache entry). Cache purge and
  inspect are deliberately not MCP tools: purge is a write and
  inspect is an operator/audit surface. Callers share the one
  hosted identity (`hosted-per-user`); per-agent identity and
  scoping are #8625's later slices. The laptop's mcpx connects
  to this endpoint as its `atlas` server (remote transport,
  bearer auth expanded from the daemon's environment).

## Deferred cache policy triggers (#6805)

- Add corpus generation to the cache key at the first same-space
  re-ingest that changes more than 5% of chunks, or on a user-visible
  stale-result complaint.
- Add an LRU sweeper when there are 10,000 cached queries or 1 GB of
  aggregate cache storage. Until then, entries have no TTL.

## D1 tenant hot cache (#9134, flag off)

The `HOT_CACHE` D1 binding and the store module in
`src/hot_cache.ts` implement §8 item 1 of the D1 tenant hot cache
design (`doc/2026-11-23T08-06-00Z_design_atlas_d1_tenant_hot_cache/`):
the §5 `hot_cache` schema (migration `migrations/0001_hot_cache.sql`)
plus get/put/touch/evict primitives. The tier is gated by
`HOT_CACHE_ENABLED` (default `"false"`), but the store is not yet
wired into the request path at all: nothing in the Worker calls
it outside tests, so the flag alone does not activate the cache —
with the flag on and the binding present, the Worker still never
touches D1 and the read path is unchanged. Freshness
(#13101), admission/eviction (#13103), and read-path integration
(#13105) are separate follow-ons. The committed `database_id` is a
local placeholder: the real `atlas-hot-cache` database is created at
deploy time and its id replaces the placeholder in both
`wrangler.toml` and `cloudflare.config.ts`.

Invalidation (§8 item 5, #13106) is implemented in
`src/invalidation.ts` over the tenant-state and purge-audit tables
of migration `migrations/0002_hot_cache_invalidation.sql`:
consumption of the per-tenant invalidation events that ride on
ingest completion records (the design's v1 piggyback transport)
drops the tenant's Class C entries and sets forced-revalidation,
and the audited operator purge deletes all of a tenant's classes as
a store operation. Neither is wired into the request path yet:
event consumption is invoked by the ingest-completion consumer and
the purge is operator-only, never called by automated paths;
read-path integration (#13105) is what consults the recorded state.

## Secrets (names only; set with `wrangler secret put`)

`GATE_TOKEN`, `SERVICE_TOKEN` (the service's `cf-worker` client
token), `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET` (Access
service token for `atlas-search.security.cab`).

## Run

```sh
npm install
npm test        # vitest + @cloudflare/vitest-pool-workers
```

## Deploy (cf versions flow)

Deploys go through the sanctioned `cf` binary's versions flow —
the same one `pkgs/pkarr-worker` and `pkgs/message-bridge` use —
never a silent overwrite. `cloudflare.config.ts` is the deploy
source of truth; `wrangler.toml` drives local dev and tests.
Keep the two in step.

```sh
# CLOUDFLARE_API_TOKEN from the fleet secrets route
# (cloudflare.api-token-account-wide), exported transiently.
cf workers versions create --tag git-<sha12> --message "<sha + summary>" \
  --secrets-file <file holding GATE_TOKEN, SERVICE_TOKEN,
                  ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET>
cf workers versions get <new-version-id>   # check the bindings carried
cf workers deployments create --worker atlas-search \
  --version <new-version-id> --percentage 100
```

The secrets file is a throwaway `.env` assembled from fleet
secrets (`atlas.search-worker-gate-token`,
`atlas.search-cf-worker-token`,
`cloudflare.access-atlas-search-client-id`,
`cloudflare.access-atlas-search-client-secret`) and deleted
after the upload; values never land in the repo. Rollback is a
new deployment at the previous version id — no upload needed.

## Service dependency

The miss path needs `POST /v1/embed` on the fleet search service
(added in the same change as this Worker, in both `server.rs`
copies under the #5433 parity guard). Deploying the service side
is a binary refresh of the laptop's `atlas-search.service` — whose
ExecStart is `fleet serve search` (atlasSearchServeViaFleet, #5604),
so the binary refreshed is the fleet binary the unit runs; the
Worker degrades to 502 (nothing cached) while the service is
unreachable.
