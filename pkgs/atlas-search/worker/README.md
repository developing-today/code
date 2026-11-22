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
- `POST /v1/cache/purge` `{query, k?}` — gated with the same
  `GATE_TOKEN`; removes exactly that normalized query/config cache
  entry. The query key is a SHA-256 digest, so per-prefix purge
  cannot enumerate matching entries without a separate index and is
  intentionally not provided.

## Deferred cache policy triggers (#6805)

- Add corpus generation to the cache key at the first same-space
  re-ingest that changes more than 5% of chunks, or on a user-visible
  stale-result complaint.
- Add an LRU sweeper when there are 10,000 cached queries or 1 GB of
  aggregate cache storage. Until then, entries have no TTL.

## Secrets

`GATE_TOKEN`, `SERVICE_TOKEN` (the service's `cf-worker` client
token), `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET` (Access
service token for `atlas-search.security.cab`). The upload workflow
reads these from the protected GitHub environment and includes them in
the uploaded version. It does not read the laptop's SOPS or Bitwarden
stores.

## Run

```sh
npm install
npm test        # vitest + @cloudflare/vitest-pool-workers
npm run dev     # wrangler dev
```

## Production version flow

Production changes use two explicit, manually dispatched GitHub Actions
workflows on `main`:

1. **Atlas Search Worker — upload version** builds and uploads a
   git-stamped Cloudflare Worker Version. It does not change traffic. The
   run summary prints the resulting version ID.
2. **Atlas Search Worker — promote version** requires that version ID as
   input, verifies it is deployable, and only then creates a 100% traffic
   deployment. It reads back the active split and checks the existing
   `/health` response. There is no automatic promotion or push/PR deploy.

Both jobs use the `atlas-search-production` GitHub Environment. Configure
required reviewers for that environment and provide its `CLOUDFLARE_API_TOKEN`
secret, `CLOUDFLARE_ACCOUNT_ID` variable, and Worker secrets named
`ATLAS_SEARCH_GATE_TOKEN`, `ATLAS_SEARCH_SERVICE_TOKEN`,
`ATLAS_SEARCH_ACCESS_CLIENT_ID`, and `ATLAS_SEARCH_ACCESS_CLIENT_SECRET`.
The workflows intentionally fail before contacting Cloudflare when a
required value is missing. This PR declares the environment reference but
does not create or change GitHub repository settings.

`cloudflare.config.ts` describes the deployed Worker for `cf`; `wrangler.toml`
continues to drive local development and Vitest. Keep the Worker name,
compatibility date, `SERVICE_BASE`, `QUERY_CACHE` Durable Object binding,
SQLite-backed `QueryCache` export, and secret names aligned between them.

## Service dependency

The miss path needs `POST /v1/embed` on the fleet search service
(added in the same change as this Worker, in both `server.rs`
copies under the #5433 parity guard). Deploying the service side
is a binary refresh of the laptop's `atlas-search.service`; the
Worker degrades to 502 (nothing cached) while the service is
unreachable.
