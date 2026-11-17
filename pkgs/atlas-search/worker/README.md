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

## Endpoints

- `GET /health` — open liveness.
- `POST /v1/search` `{query, k?}` — gated (`Authorization:
  Bearer <GATE_TOKEN>`). Returns `{cache: "miss"|"hit", key,
  query, result, meta}`.
- `GET /v1/cache/inspect?query=..&k=..` — gated; DO metadata
  readback (embedding presence/dims/validity, result presence,
  hit count, timings). The proof/audit surface.

## Secrets (names only; set with `wrangler secret put`)

`GATE_TOKEN`, `SERVICE_TOKEN` (the service's `cf-worker` client
token), `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET` (Access
service token for `atlas-search.security.cab`).

## Run

```sh
npm install
npm test        # vitest + @cloudflare/vitest-pool-workers
npm run deploy  # wrangler deploy (fleet Cloudflare account)
```

## Service dependency

The miss path needs `POST /v1/embed` on the fleet search service
(added in the same change as this Worker, in both `server.rs`
copies under the #5433 parity guard). Deploying the service side
is a binary refresh of the laptop's `atlas-search.service`; the
Worker degrades to 502 (nothing cached) while the service is
unreachable.
