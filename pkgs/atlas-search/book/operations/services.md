# Running the service and the Worker

Source: `home/common/default.nix`, `pkgs/atlas-search/crates/atlas-search/src/server.rs`, `pkgs/fleet/crates/fleet-search/src/serve.rs` and `pkgs/fleet/crates/fleet-search/src/server.rs`, and `pkgs/atlas-search/worker/` (`cloudflare.config.ts`, `wrangler.toml`, `src/index.ts`, `src/search.ts`, `src/service.ts`, `src/cache_do.ts`) under https://github.com/developing-today/code/blob/main/home/common/default.nix — provenance only; this chapter is complete without following it.

Atlas serving has two halves on two machines. The **service** is a long-lived process on the laptop that owns the corpus connection, the resident embedding model, and the search itself. The **Worker** is a Cloudflare Worker in front of it: the hosted front door that authenticates callers, caches whole query results, and forwards anything new to the laptop. The concepts are drawn in [The service and the Worker](../concepts/service-and-worker.md); this chapter is the operator version — units, ports, checks, and restarts. The HTTP surfaces themselves are in [The service HTTP API](../reference/service-api.md) and [The Worker HTTP API](../reference/worker-api.md).

## The laptop service: one unit, two binaries

The unit is the systemd **user** service `atlas-search.service`, declared in `home/common/default.nix` (not under `nixos/`):

| Property    | Value                                                                                                                                             |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Description | `Atlas search service (hybrid corpus search, serve mode)`                                                                                         |
| Ordering    | `After` / `Wants` = `network-online.target`                                                                                                       |
| Restart     | `Restart = on-failure`, `RestartSec = 3`                                                                                                          |
| Enablement  | `Install.WantedBy = [default.target]` — added at the step-6 cutover; in step 5 the unit landed with no Install section so it could not auto-start |

**Which binary is live.** The unit's `ExecStart` is selected by a home-manager flag, `atlasSearchServeViaFleet`:

- Flag `true` (the production setting): `ExecStart` is `fleet serve search` — the serve half rehomed into the fleet binary (#5604). **This is the live service.**
- Flag `false`: `ExecStart` is `atlas-search serve` — the standalone binary. This is the rollback shape, not the live one.

The unit name, the configuration, the port, and the wire protocol are identical in both shapes; rollback is that single flag pointed back, followed by a home-manager switch and a unit restart. Two verbs never moved: `atlas-search init` (writes the service config) and `atlas-search fetch-model` (provisions the model) stay on the standalone `atlas-search` on PATH in both shapes. Any runbook step that says "restart atlas-search" is talking about the unit; debugging the standalone binary's serve path is debugging the rollback.

What the serve process holds, and why it exists as a process at all: one lazily loaded ONNX embedder session for the Snowflake model (about a second to load, a few hundred megabytes resident, against roughly 35 seconds of cold start for the Python path), a connection pool to the corpus stores, the job registry and in-memory temp store (below), and a worker pool for branch execution. It resolves the same read-only Turso token the Python resolves — by name from Bitwarden, or the `turso` CLI mint cache — and **holds no write credential for any Turso database**.

### Port and exposure

The service binds **`127.0.0.1:18760`** — loopback only. The single source of truth is `atlasSearchPort = 18760` in `home/common/default.nix`, and the code defaults agree everywhere: the server listen default, the `init` template, and every client's default service URL (`http://127.0.0.1:18760`) are all 18760.

Exposure beyond loopback is layered in front of that bind, never by widening it:

- Tailnet callers reach it through a `tailscale serve` mapping on the laptop's MagicDNS name. That mapping is created imperatively at activation; it is deliberately not declared in the home config, because tailscale serve state is imperative tailscaled state in this fleet, and a mapping in front of a not-yet-started service would itself be a live exposure change.
- Hosted callers never reach the port directly at all: they reach the Worker, and the Worker reaches the service through the tunnel hostname `atlas-search.security.cab` plus a Cloudflare Access service token (below).

The service is never a public listener in its own right.

### Configuration and client identities

Service configuration lives in **`~/.local/state/atlas-search/service.json`**, mode 0600, written by `atlas-search init`. It holds the listen address and the per-client token table: each named client has its own bearer token (named here, never valued) and a **ceiling cap** — the highest sensitivity ceiling that client may ever search under. The state directory also holds the model snapshot (`model/`), traces, and feedback records; nothing else writes there.

The clamp is the security property: the server computes `effective = min(requested, token ceiling cap)`, records the effective ceiling on the job, and reports it in the result envelope. Raising a cap is a local edit by the user in that file — never something a request can do. The v1 client set and caps, as designed and configured:

| Client                                                             | Ceiling cap      |
| ------------------------------------------------------------------ | ---------------- |
| The user's CLI on the laptop (`drew-cli`)                          | `secret-bearing` |
| OpenChamber sessions                                               | `personal`       |
| Fork / OpenCode agents                                             | `personal`       |
| Sandbox scripts and Muse lanes via the tailnet (`sandbox-scripts`) | `personal`       |
| The Worker (`cf-worker`)                                           | `personal`       |

Aster is explicitly not a v1 client. The sensitivity ladder and the context intersection that further narrows a request are in [Sensitivity tiers and client ceilings](../concepts/sensitivity.md); the full variable and file inventory is in [Configuration and environment](../reference/configuration.md).

Client side, a seat presents its token from the environment variable named `ATLAS_SEARCH_TOKEN`, or from **`~/.config/atlas-search/client.json`** (mode 0600, one token string field). A seat with no client config cannot use the service route at all — its searches fall back to standalone, loudly; see [Troubleshooting](troubleshooting.md).

### Health checks

Laptop service, `GET /health` — no auth, no corpus data:

```json
{ "status": "ok", "embedder_state": "...", "queue_depth": 0, "active_branch_executions": 0, "active_jobs": 0 }
```

`embedder_state` is the shared embedder's load state (the design vocabulary is cold / warming / ready). This is the liveness, embedder-state, and queue-depth surface in one call. Operator checks, in order:

```
$ systemctl --user status atlas-search.service
$ curl -s http://127.0.0.1:18760/health
$ journalctl --user -u atlas-search.service -n 100
```

A service that answers `/health` with `embedder_state` not yet ready will still serve FTS-only work; vector and hybrid searches wait on the embedder. A service that does not answer at all is down for via-service clients (which fall back to standalone) and fatal for the Worker (which has no fallback — it returns 502).

## The Worker in production

The Worker is the package in `pkgs/atlas-search/worker/`: name `atlas-search`, deployed to the fleet Cloudflare account's workers.dev subdomain, reached at `https://atlas-search.developing-today.workers.dev`. Its deploy source of truth is **`cloudflare.config.ts`**; `wrangler.toml` drives local development and tests only, and the two files must be kept in step (bindings were verified field-for-field, including the shared D1 placeholder id below).

### Routes and the gate

Five public routes: `GET /health` (open, liveness only, returns `{ "status": "ok", "service": "atlas-search-worker" }`), `POST /v1/search`, `GET /v1/cache/inspect`, `POST /v1/cache/purge`, and `POST /mcp` — the one-tool MCP surface in [The Atlas MCP server](../reference/mcp.md). Everything else is a 404.

Every route except `/health` is gated. REST routes use the shared `GATE_TOKEN`. `/mcp` uses the optional `AGENT_TOKENS` secret when configured: a JSON object maps stable agent ids to distinct bearer tokens, and the Worker derives `agent_id` from the matching credential (never from a request field or header). With that map configured, the shared token is not accepted on `/mcp`; without it, the legacy `GATE_TOKEN` resolves to `hosted-per-user`. Token comparisons hash both values and XOR-fold the digests. Valid MCP searches emit a structured Worker log event with the authenticated agent id, query SHA-256, k, status, and cache outcome; raw query text is not logged. Per-agent backend ceilings and corpus scoping are not implemented by this slice.

### Bindings, variables, and secrets (names only)

| Binding / variable  | Kind           | Target / value                                                                                                                                                                                                                 |
| ------------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `QUERY_CACHE`       | Durable Object | Class `QueryCache` in this Worker, SQLite-backed (`new_sqlite_classes`, migration tag `v1`) — the per-query cache; see [The service and the Worker](../concepts/service-and-worker.md)                                         |
| `HOT_CACHE`         | D1 database    | Database `atlas-hot-cache`. The committed `database_id` in both config files is a local placeholder (`00000000-0000-0000-0000-000000000001`), replaced with the real id when the database is created at deploy time            |
| `SERVICE_BASE`      | Plain variable | `https://atlas-search.security.cab` — the tunnel hostname of the laptop service. Every service call is `fetch` against this base                                                                                               |
| `HOT_CACHE_ENABLED` | Plain variable | `"false"` as committed. Only the literal string `"true"` enables the D1 store — and today that still changes nothing, because the store is not wired into the request path (#13105); see [Troubleshooting](troubleshooting.md) |

Secrets are set with `wrangler secret put` or uploaded with the new version at deploy time; values are never in either config file:

| Secret name                                 | Purpose                                                                                                                                                               |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GATE_TOKEN`                                | The inbound bearer gate for REST routes and the legacy MCP fallback                                                                                                  |
| `AGENT_TOKENS`                              | Optional JSON secret object mapping agent ids to distinct MCP bearer tokens; when set, disables the shared-token MCP fallback                                         |
| `SERVICE_TOKEN`                             | The service's `cf-worker` client token (service-side cap `personal`), presented as a bearer on every service call                                                     |
| `ACCESS_CLIENT_ID` / `ACCESS_CLIENT_SECRET` | The Cloudflare Access service token for `atlas-search.security.cab`, presented as the `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers on every service call |

At deploy time these are assembled from fleet secrets under the names `atlas.search-worker-gate-token`, `atlas.search-cf-worker-token`, `cloudflare.access-atlas-search-client-id`, and `cloudflare.access-atlas-search-client-secret`, plus `cloudflare.api-token-account-wide` for the `cf` CLI itself. The Worker holds **no Turso credentials and no shard list**: fan-out, shard discovery, manifest reads, and merging all live in the laptop service.

Outbound, the Worker pins its identity at compile time: ceiling `personal`, disclosure context `hosted-per-user`, model `Snowflake/snowflake-arctic-embed-m-v2.0` at 768 dimensions. No request field, header, or environment variable can change any of them, and the Worker verifies the service's reported `effective_ceiling` is `personal` before anything is cached. There are no application-level timeouts in the Worker — outbound calls run unbounded, subject only to Cloudflare platform limits.

### Deploying and rolling back the Worker

Deploys go through the sanctioned `cf` binary's versions flow, not a bare `wrangler deploy`:

1. Assemble a throwaway secrets file from the fleet secrets, by name.
2. Create the new version with that secrets file (`cf workers versions create --secrets-file …`; the package's `deploy` script is `cf workers versions create`), so no version ever deploys without its gate and service tokens.
3. Verify the bindings on the new version.
4. Create a deployment at 100%.

Rollback is a new deployment at the previous version id — no upload needed. Local development is `wrangler dev`; the test suite is `vitest run` (the package's `test` script), and `tsc --noEmit` is its `typecheck` script.

## Restarts: what is lost, what survives

Restart the laptop service with:

```
$ systemctl --user restart atlas-search.service
```

A restart of the serve process loses exactly the state that was designed to be perishable:

- **The temp store, entire.** Every job's fused ranking lives only in the serve process's memory, with a TTL of 10 minutes from completion and a 60-minute hard cap from submit. A restart takes in-flight jobs, queued jobs, and finished-but-unpaged rankings with it. Clients see their job handles fail and simply submit again — that resubmission is the recovery procedure, by design. Nothing in the temp store survives a restart, by requirement: job results hold ceiling-gated snippets, and their lifetime is meant to match the job, not the disk.
- **The warm embedder.** The ONNX session is resident state; the restarted process reloads it lazily (about a second) on the first vector or hybrid search.
- **Connections and queues.** The Turso connection pool, the branch-execution slots, and any queued work behind the backpressure limits (saturation is answered `429` with a retry hint; a job queued beyond its useful life fails as expired) are all rebuilt empty.

A restart does **not** lose: the service config or client table (`service.json` on disk), the provisioned model snapshot, traces or feedback records (files under the same state directory), any corpus store (the monolith, the shards, the canonical `atlas.db` — the service is a reader of all of them), or the backup (a separate unit; see [Backup, restore and the census](backup-and-census.md)).

A Worker redeploy is the mirror image: it loses nothing durable. The Durable Object cache lives in the objects' own SQLite storage, not in the Worker code version, so cached answers survive a deploy — which is precisely why stale answers are a purge operation, not a redeploy operation; see [Troubleshooting](troubleshooting.md). The D1 hot cache, being unwired, is unaffected either way.
