# atlas-search Worker

The cross-database search surface (developing-today/code#6788):
a Cloudflare Worker that pulls all Atlas databases for search
through the fleet search service; for each query it stores a
**Durable Object**, automatically inserts the query as an
**embedding** (the pinned Snowflake space — see `src/space.ts`),
and caches the result **indefinitely** (no TTL/eviction in v1).

## Shape

- `src/index.ts` — gate, normalization, keying, routing.
- `src/cache_do.ts` — the `QueryCache` DO: one per query key;
  check-and-fill runs inside the DO so a fan-out happens once.
  State machine: absent → embedded → complete; the embedding is
  persisted before the search runs.
- `src/service.ts` — calls to `fleet serve search`
  (`POST /v1/embed`, `POST /v1/search`) via the
  `atlas-search.security.cab` tunnel hostname + Cloudflare
  Access service token. The service enforces this client's
  `personal` tier ceiling; the Worker holds no Turso credentials
  and keeps no shard list (new `atlas-core-*` databases join the
  fan-out through the service's manifest discovery).
- `src/key.ts` — DO key = sha256 over the normalized query plus
  the pinned config fingerprint (ceiling, model revision, vector
  table, k). A cached entry is never served across ceilings or
  embedding spaces.
- `src/admission.ts` — the D1 hot-cache admission policy (#13103,
  #9134 design §3/§5): heat counters, heavy-query immediate
  admission, per-tenant quotas (tier weights 1:4:16, 5% floor),
  entry size cap, negative-result TTL, and the LRU eviction sweep.
  Pure policy, not yet wired into the read path (follow-on 4).

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
is a binary refresh of the laptop's `atlas-search.service`; the
Worker degrades to 502 (nothing cached) while the service is
unreachable.
