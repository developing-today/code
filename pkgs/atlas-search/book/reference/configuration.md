# Configuration and environment

Source: the environment-variable call sites across [pkgs/atlas-search/crates/](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates) and [pkgs/fleet/crates/fleet-search/](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search), [worker/wrangler.toml](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/wrangler.toml) and [worker/cloudflare.config.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/cloudflare.config.ts), [crates/atlas-proto/query-parameters-v1.json](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json) — provenance only; this chapter is complete without following it.

Every Atlas configuration surface, in one place: the environment variables the binaries read, the client and service config files, the fleet config keys, the Worker config files and bindings, and the versioned query-parameter defaults. Secrets are named, never valued, throughout: for token variables this chapter gives the name and the resolution order only, because that is all any tool ever prints.

## Environment variables: all 24

### CLI-facing variables (named in help or flag definitions)

| Name                          | Used by                                                                                                                     | Default / fallback                                                                                                                                                                              |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ATLAS_SEARCH_MODEL`          | clap env for `--model` (atlas-search; the Python CLI's argparse default)                                                    | `Snowflake/snowflake-arctic-embed-m-v2.0` when unset.                                                                                                                                           |
| `ATLAS_VEC_TABLE`             | clap env for `--vec-table` (atlas-search; Python)                                                                           | `vec_snowflake_m_v2_768_f32` when unset.                                                                                                                                                        |
| `ATLAS_SEARCH_URL`            | clap env for `--service-url` (atlas-search); fleet `client::base_url` reads it in its own chain                             | `http://127.0.0.1:18760` (`DEFAULT_SERVICE_URL`); trailing `/` trimmed; empty string ignored. Fleet's chain is longer: flag → fleet.toml `[search] url` → this env → default.                   |
| `ATLAS_SEARCH_SERVICE_CONFIG` | clap env for `--config` (serve/init); the default path is resolved by `resolve_service_config_path` when the flag is absent | `~/.local/state/atlas-search/service.json`; empty string ignored.                                                                                                                               |
| `ATLAS_SHARDS`                | clap env for the `--shards` flag (search)                                                                                   | Unset = monolith backend, no shard fan-out. Grammar: `dir:&lt;path&gt;` (local `atlas-core-*.db` files) or `turso` / `turso:&lt;org&gt;` (default org `dezren39`).                              |
| `ATLAS_SEARCH_VIA`            | `service::via_env` / fleet `client::via_env`                                                                                | Only the exact value `service` requests the via-service route; anything else, including unset, = not requested. Fleet delegation explicitly removes it from the standalone child's environment. |
| `ATLAS_SEARCH_TOKEN`          | `client_token()` (via-service clients: atlas-search and fleet)                                                              | First source tried; if unset or empty, resolution falls to the client-config file (`ATLAS_SEARCH_CLIENT_CONFIG`, below). Never printed or logged.                                               |
| `ATLAS_SEARCH_CLIENT_CONFIG`  | `client_token()` file-path override                                                                                         | Default path `~/.config/atlas-search/client.json` — a JSON object with one `token` string field, expected mode 0600.                                                                            |

### Path, model and runtime variables (source-only, not in help)

| Name                           | Used by                                                                     | Default / fallback                                                                                                                                                                                                                                                                                                                                                          |
| ------------------------------ | --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ATLAS_DB`                     | `default_db_path()` (atlas-search; Python)                                  | If set and non-empty, it wins. Else `~/atlas/turso-atlas/data/atlas.db` if it exists, else the binary/script directory's `../data/atlas.db` if it exists, else the laptop path — which then errors as not found on the local backend.                                                                                                                                       |
| `ATLAS_VEC_DB`                 | `LocalBackend::new`, when `--vecdb` is absent or empty (atlas-core; Python) | Comma-separated vector DB paths, id-keyed. Else the known side-store pairs are probed (for example `~/atlas/v3/vec-snowflake.db` paired with `~/atlas/v3/corpus.db`, and `~/atlas/atlas-delta-chunks2.db`). If the corpus DB itself has the vec table, neither is consulted.                                                                                                |
| `ATLAS_MODEL_DIR`              | Model-dir resolution (atlas-search, atlas-stage promote, fleet serve)       | If set, non-empty and an existing directory, it wins. Else `~/.local/state/atlas-search/model/&lt;rev&gt;` if a directory, else the HuggingFace cache snapshot `~/.cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/&lt;rev&gt;` if a directory, else a model-not-found error. `&lt;rev&gt;` is `95c2741480856aa9666782eb4afe11959938017f`. |
| `ATLAS_ORT_LIB`                | ONNX Runtime library resolution (same three surfaces)                       | If set, non-empty and an existing file, it wins. Else `~/.local/state/atlas-search/ort/libonnxruntime.so` if a file. atlas-stage promote additionally probes the `~/atlas/v3-venv/…/libonnxruntime.so.1.31.1` path. Else an ORT-not-found error.                                                                                                                            |
| `ATLAS_SEARCH_ORT_LIB_DEFAULT` | Compile-time `option_env!` (packaged builds only)                           | Baked by `default.nix` to the build's onnxruntime store path; consulted after `ATLAS_ORT_LIB` and the state-dir candidate. Dev builds never set it and never reach the branch.                                                                                                                                                                                              |
| `HOME`                         | Every binary's home resolution                                              | Must be set and non-empty. atlas-search, atlas-core and fleet-search **panic** naming HOME rather than substituting a directory — except the trace and feedback sink `default_dir()` functions in atlas-core, which fall back to `.` (the working directory) when HOME is missing or empty.                                                                                 |

### Credential and backend variables (source-only)

Token values are never reproduced here — names and resolution order only.

| Name                                | Used by                                                                                                 | Default / fallback                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ATLAS_TURSO_TOKEN`                 | `resolve_turso_token()` (atlas-core; Python mirrors it)                                                 | First source tried. Full chain, in order: this env → Bitwarden Secrets Manager names `turso.atlas-ro-token`, `turso.atlas-read-only-token`, `atlas-ro` (via the `bws` CLI) → the cached mint at `~/.cache/atlas-search/turso-token.json` if it names db `atlas` and is under 20 hours old → a fresh read-only mint via `turso db tokens create atlas --read-only -e 7d` (cached at mode 0600). Total failure is a fatal error naming all three routes. The Turso host is a constant, not env: `atlas-dezren39.aws-us-east-1.turso.io`, db `atlas`. |
| `BWS_ACCESS_TOKEN`                  | The `bws_secret()` helper                                                                               | If unset or empty, obtained by running `source ~/tools/bws-env.sh` under bash (30-second timeout) and reading the variable from that environment. Still empty → the Bitwarden source silently yields nothing and token resolution moves on.                                                                                                                                                                                                                                                                                                        |
| `ATLAS_TURSO_TOKEN_PUBLIC`          | Shard discovery, `Tier::token_env()` — the public tier's read-only Turso token                          | Unset or empty = that tier is simply not connected. Discovery is opened only at the caller's own ceiling, so the default `personal` caller never needs the secret-tier variable.                                                                                                                                                                                                                                                                                                                                                                   |
| `ATLAS_TURSO_TOKEN_INTERNAL`        | Shard discovery — the internal tier's read-only Turso token                                             | As above.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `ATLAS_TURSO_TOKEN_SECRET`          | Shard discovery — the secret tier's read-only Turso token                                               | As above, with one extra rule: the secret-tier token is never derived from another tier's token.                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `TURSO_API_TOKEN`                   | Shard discovery, `TursoApiLister::from_env` (the constant naming the variable is `TURSO_API_TOKEN_ENV`) | Required for `turso[:&lt;org&gt;]` shard specs; unset or empty → fatal `shard discovery: TURSO_API_TOKEN is not set`.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `ATLAS_EMBED_CACHE_TURSO_URL`       | Query-embedding cache, `QueryEmbeddingCache::from_env`                                                  | Both this and the token below must be set and non-empty to enable the shared Turso cache tier; otherwise the cache is memory-only (about 10,000 entries). Cache writes are async and best-effort and never fail a search.                                                                                                                                                                                                                                                                                                                          |
| `ATLAS_EMBED_CACHE_TURSO_TOKEN`     | Query-embedding cache, `QueryEmbeddingCache::from_env`                                                  | The other half of the pair above; named only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `ATLAS_OWNER_INVENTORY_SECRET_NAME` | `OwnerInventory::load_runtime` (the constant holding the variable's name is `SECRET_REFERENCE_ENV`)     | Names the **one** Bitwarden secret key holding the owner-inventory document (`atlas-owner-inventory/v1`). The value itself is never taken from env — only the key name is. Unset or invalid → fatal `owner inventory unavailable`. Library surface; no CLI flag exposes it.                                                                                                                                                                                                                                                                        |
| `ATLAS_SEARCH_QVEC_FILE`            | Embedder load (atlas-search; fleet serve)                                                               | **Test hook**: path to a JSON array of floats substituted as a fixed query vector. Meaningless in production, per the source. An unreadable or bad-JSON file fails embedder load.                                                                                                                                                                                                                                                                                                                                                                  |

Ambient variables honoured indirectly: the proxy family (`ALL_PROXY`, `HTTPS_PROXY`, `http_proxy`, `NO_PROXY` and friends) — the via-service HTTP client is built from the process proxy environment, so service requests honour it, with `NO_PROXY` bypasses. The Python CLI additionally normalises a bracketed `NO_PROXY`/`no_proxy` to `localhost,127.0.0.1`.

## `client.json` — the client token file

Path: `~/.config/atlas-search/client.json` (override: `ATLAS_SEARCH_CLIENT_CONFIG`). Expected mode 0600. The entire format is one field:

```json
{ "token": "<the client token for this seat's identity>" }
```

Resolution, in `client_token()`: `ATLAS_SEARCH_TOKEN` first; if unset or empty, this file. The token names the seat's identity to the service — the identity itself (its name and ceiling cap) lives server-side in `service.json`; the file carries no identity name and no ceiling, and nothing a client edits here can widen what the service will serve it. A seat with no `client.json` and no `ATLAS_SEARCH_TOKEN` cannot use the service route at all and falls back to standalone. See [The service HTTP API](service-api.md) for what the token authorises.

## `service.json` — the service config

Path: `~/.local/state/atlas-search/service.json` (flag `--config`, env `ATLAS_SEARCH_SERVICE_CONFIG`). Written by `atlas-search init` at mode 0600; `serve` refuses a config that is not mode 0600. Shape (token values shown as placeholders — `init` generates them fresh, and `--force` rotates every one):

```json
{
  "listen": "127.0.0.1:18760",
  "vec_table": "vec_snowflake_m_v2_768_f32",
  "clients": [
    { "name": "drew-cli", "token": "<generated>", "ceiling_cap": "secret-bearing" },
    { "name": "openchamber", "token": "<generated>", "ceiling_cap": "personal" },
    { "name": "fork-server", "token": "<generated>", "ceiling_cap": "personal" },
    { "name": "sandbox-scripts", "token": "<generated>", "ceiling_cap": "personal" }
  ]
}
```

| Field                   | Meaning                                                                                                                                                                                     |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `listen`                | The loopback listen address. A `--listen` serve flag overrides it, loopback only.                                                                                                           |
| `vec_table`             | The vector table the service searches.                                                                                                                                                      |
| `clients[].name`        | The identity name, unique in the table.                                                                                                                                                     |
| `clients[].token`       | That identity's bearer token (value never printed by any tool).                                                                                                                             |
| `clients[].ceiling_cap` | The identity's ceiling cap: the service clamps every requested ceiling to `min(requested, ceiling_cap)`. The ladder is `shareable` &lt; `personal` &lt; `local-only` &lt; `secret-bearing`. |

Serve-time validation failures — missing config, config not mode 0600, an empty client table, duplicate client names, a bad `ceiling_cap` — are config-load failures: exit 2, with `run atlas-search init first` guidance where applicable. A `shards` spec in this config additionally enables serve-mode shard fan-out with periodic re-discovery (default refresh 60 seconds). Serve-time lifetime overrides `--job-ttl-ms` (default 600000 — the 10-minute completed-job TTL) and `--hard-ttl-ms` (default 3600000 — the 60-minute submit hard cap) live here in effect though they are flags, not file fields; see [The service HTTP API](service-api.md).

## fleet.toml `[search]` — the fleet client's keys

The fleet binary's own config file carries one Atlas section, with exactly two keys:

| Key   | Values                    | Meaning                                                                                    |
| ----- | ------------------------- | ------------------------------------------------------------------------------------------ |
| `via` | `service` \| `standalone` | The seat's preferred route. Any other value is ignored and route resolution falls through. |
| `url` | A service base URL        | The service address for the via-service route.                                             |

Where they sit in resolution: for the URL, `--service-url` flag → fleet.toml `[search] url` → `ATLAS_SEARCH_URL` → the default `http://127.0.0.1:18760`. For the route itself: explicit `--via-service`/`--standalone` flag → `ATLAS_SEARCH_VIA=service` → fleet.toml `[search] via` → the seat default (standalone on the serving host — detected by the presence of `service.json` — via-service elsewhere). No `ATLAS_*` env var overrides fleet.toml except as listed here. See [fleet search and fleet serve search](fleet-search.md).

## The Worker: `wrangler.toml`, bindings and vars

Two files describe the Worker, and they must be kept in step: `wrangler.toml` drives local dev and tests, and `cloudflare.config.ts` is the deploy source of truth — it mirrors `wrangler.toml` in the `cf` config DSL (same worker name, compatibility date, entrypoint, bindings and vars). Secrets are in neither file: a deploy uploads them with the new version, so no version deploys without its gate and service tokens.

Header facts: worker name `atlas-search`, entrypoint `src/index.ts`, compatibility date `2025-09-15`, `workers_dev = true` (deployed to the fleet Cloudflare account's workers.dev subdomain).

Plain vars (`[vars]`):

| Name                | Value in file                       | Purpose                                                                                                                            |
| ------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `SERVICE_BASE`      | `https://atlas-search.security.cab` | Base URL of the fleet search service, at its Cloudflare Tunnel hostname. Every service call is a fetch to this base plus the path. |
| `HOT_CACHE_ENABLED` | `"false"`                           | The D1 hot-cache master flag; see "The D1 master flag" below.                                                                      |

Bindings:

| Binding name  | Kind           | Target                                                                                                                                                                                                                                         | Purpose                                                            |
| ------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `QUERY_CACHE` | Durable Object | Class `QueryCache` in this Worker (SQLite-backed; migration tag `v1`, `new_sqlite_classes = ["QueryCache"]`)                                                                                                                                   | The per-query cache object namespace — one instance per cache key. |
| `HOT_CACHE`   | D1 database    | Database name `atlas-hot-cache`; the id committed in both config files is a local placeholder (`00000000-0000-0000-0000-000000000001`), replaced with the real id when the database is created at deploy time; `migrations_dir = "migrations"` | The D1 tenant hot cache.                                           |

Secrets (names only; set via `wrangler secret put` or the `cf` versions secrets file):

| Name                   | Purpose                                                                                                                                    |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `GATE_TOKEN`           | The inbound bearer gate for REST routes and the legacy MCP fallback.                                                                      |
| `AGENT_TOKENS`         | Optional JSON secret object mapping stable agent ids to distinct MCP bearer tokens; configured maps disable the shared MCP fallback.        |
| `SERVICE_TOKEN`        | The fleet search service's `cf-worker` client token (service-side `ceiling_cap: personal`), presented by the Worker on every service call. |
| `ACCESS_CLIENT_ID`     | Cloudflare Access service token, id half, for the `atlas-search.security.cab` tunnel hostname.                                             |
| `ACCESS_CLIENT_SECRET` | Cloudflare Access service token, secret half, for the same hostname.                                                                       |

Per the Worker README's deploy section, the fleet-secrets names these are assembled from at deploy time are `atlas.search-worker-gate-token`, `atlas.search-cf-worker-token`, `cloudflare.access-atlas-search-client-id` and `cloudflare.access-atlas-search-client-secret` (names only), plus `cloudflare.api-token-account-wide` for the `cf` CLI itself.

The complete list of environment members the Worker code reads is the `Env` interface: `QUERY_CACHE` (required), `HOT_CACHE` (optional — declared optional so a deploy without the database provisioned still runs the pre-cache path), `HOT_CACHE_ENABLED` (optional string), `GATE_TOKEN`, `AGENT_TOKENS` (optional), `SERVICE_BASE`, `SERVICE_TOKEN`, `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET`. All configuration arrives via the `env` argument. The MCP authentication helper receives the gate token and optional agent-token map; after authentication, the MCP handler receives only the resolved agent id. The service client sees only `{ SERVICE_BASE, SERVICE_TOKEN, ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET }`.

## The D1 master flag: `HOT_CACHE_ENABLED`

`HOT_CACHE_ENABLED` is the Worker's one feature flag: the D1 hot-cache master flag. Its semantics are literal — only the exact string `"true"` enables it; `"false"`, unset, or any other value is off. Even then, the store exists only when the flag is on **and** the `HOT_CACHE` binding is present; otherwise the caller's signal is to use the pre-cache (Durable Object) path. The committed value in both config files is `"false"`.

**Source-wins note.** Where the README's flag phrasing implies that turning the flag on activates D1 reads and writes, the source says otherwise, and the source wins: in the current code, **nothing in the request path calls the hot-cache store at all** — its only callers are its own tests. The flag therefore currently changes nothing in the request path, on or off; read-path integration is follow-on work, not a flag flip. The D1 store as implemented is a storage layer only, with no TTL and no automatic eviction sweep; see [Data stores and schemas](data-stores.md).

## `query-parameters-v1.json` — the defaults contract

One file is the single source for every search-parameter default shared by the CLI, the service and their clients: `crates/atlas-proto/query-parameters-v1.json`. It is embedded into `atlas-proto` at build time and parsed for every default — a missing or invalid default panics at first use, so a broken contract fails loudly instead of silently changing ranking. Its verbatim contents:

```json
{
  "k": 10,
  "pool": 100,
  "expand": 0,
  "parameters_version": 1,
  "shard_top_k": 100,
  "global_top_k": 100,
  "fts_weight": 1.0,
  "vector_weight": 1.0,
  "jaccard_depth": 0,
  "jaccard_threshold": 0.0,
  "rrf_k": 10,
  "planner_enabled": false,
  "spell_enabled": false,
  "query_class_hint": null,
  "variant": "raw",
  "raw": false,
  "no_cache": false,
  "wait": false,
  "hydrate": 0,
  "raw_mode": "off"
}
```

Read it as the contract, not as a sample: `k` 10 and `pool` 100 are the depths every surface starts from; `rrf_k` 10 is the shipped reciprocal-rank-fusion constant (the Python eval harness lineage uses 60 — a different system, not this contract); `jaccard_depth` 0 keeps Jaccard rescoring off; `planner_enabled` and `spell_enabled` false are not merely defaults but the only legal state — setting either is a validation error, as is a non-null `query_class_hint` or a `variant` other than `raw`. `wait` false is the protocol default — the one-shot CLI sets it unless `--no-wait` is given. Validation bounds that travel with these defaults: `parameters_version` must be 1, `shard_top_k` and `global_top_k` in 1–400, `rrf_k` in 1–10000, weights finite and non-negative with at least one positive, `jaccard_threshold` in [0, 1], `hydrate` at most 1000.

## Default directories (env-independent, HOME-rooted)

| Purpose                                    | Default path                                                                        |
| ------------------------------------------ | ----------------------------------------------------------------------------------- |
| Service config                             | `~/.local/state/atlas-search/service.json`                                          |
| Client config (token file)                 | `~/.config/atlas-search/client.json`                                                |
| Model snapshot                             | `~/.local/state/atlas-search/model/&lt;rev&gt;/`                                    |
| ORT runtime (staged)                       | `~/.local/state/atlas-search/ort/libonnxruntime.so`                                 |
| Trace sink                                 | `~/.local/state/atlas-search/traces` (one `atlas-traces-&lt;day&gt;.jsonl` per day) |
| Feedback sink                              | `~/.local/state/atlas-search/feedback`                                              |
| Turso token cache                          | `~/.cache/atlas-search/turso-token.json` (mode 0600)                                |
| Default local corpus (preferred candidate) | `~/atlas/turso-atlas/data/atlas.db`                                                 |

## Worked example

Provisioning a fresh seat, end to end, using only surfaces named in this chapter:

```bash
atlas-search init            # writes service.json (0600) with the v1 client table
atlas-search fetch-model     # stages the pinned model under the state dir
atlas-search serve           # reads service.json; binds 127.0.0.1:18760
```

On the client seat, place that identity's token in `~/.config/atlas-search/client.json` at mode 0600 (or export it as `ATLAS_SEARCH_TOKEN`), and searches route to the service with `--via-service`, `ATLAS_SEARCH_VIA=service`, or fleet.toml `[search] via = "service"` — with the requested ceiling clamped, server-side, to that identity's `ceiling_cap` whatever the client asks for.
