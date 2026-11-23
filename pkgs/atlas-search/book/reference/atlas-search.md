# atlas-search: search and get-chunk

Source: `pkgs/atlas-search/crates/atlas-search/src/main.rs`, `pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json`, `pkgs/atlas-search/crates/atlas-core/src/filters.rs` and `pkgs/atlas-search/crates/atlas-core/src/disclosure.rs` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/main.rs — provenance only; this chapter is complete without following it.

`atlas-search` is the Atlas access layer: hybrid search over the corpus. This chapter covers its two query verbs, `search` and `get-chunk`. The remaining verbs are in [atlas-search: serve, init, fetch-model, traces and feedback](atlas-search-admin.md).

Standalone is the default and is complete: the query is embedded in-process and the tool talks to Turso (the hosted corpus database) or a local file directly. `--via-service` *prefers* a running service and falls back to standalone **loudly** on any service failure — a warning on stderr plus a note spliced into the result envelope's `notes`. Routing details, including the requests that disqualify the service route (the `--via-service` help states them too), are under "Via-service vs standalone" below.

## Invocation

```
atlas-search search [OPTIONS] <QUERY>
atlas-search get-chunk [OPTIONS] <CHUNK_ID>
```

`search` takes one required positional, `&lt;QUERY&gt;`, a single string. `get-chunk` takes one required positional, `&lt;CHUNK_ID&gt;`, an integer: the id of one chunk (the corpus is sessions → messages → chunks, so a chunk is one searchable slice of one message).

There are no global flags before the subcommand on these verbs; backend and model flags live on the verbs that use them. The background vocabulary — the sensitivity ladder and disclosure contexts — is defined in [Sensitivity tiers and client ceilings](../concepts/sensitivity.md); the ranking machinery is in [Retrieval: hybrid full-text and vector search](../concepts/retrieval.md).

## `search`: modes, filters and the sensitivity request

### Modes

`--mode` selects which retrieval branches run:

| Mode | Meaning |
|---|---|
| `hybrid` (default) | Full-text (FTS) and vector branches both run; their candidate lists are fused with reciprocal-rank fusion (RRF). |
| `fts` | Full-text branch only. Never loads the embedding model. |
| `vector` | Vector branch only. |

### Filters

`--kind`, `--topic`, `--project` and `--client` are classification filters. Each takes a comma-separated (CSV) string: the value is split on commas, entries are trimmed, and empty entries are dropped. `--since` and `--until` take a `YYYY-MM-DD` date and bound `sessions.created_at`, inclusive at both ends.

### The sensitivity request: ceiling and context

Sensitivity is a caller ceiling, not a property you search *for*. The ladder is, in order:

```
shareable < personal < local-only < secret-bearing
```

`--max-sensitivity` names the highest tier a result may have; the default is `personal`. The top tier, `secret-bearing`, is never granted implicitly — it must be spelled out.

`--context` names the disclosure context of the destination surface (the surface the results will be shown on). Each context has its own ceiling:

| Context | Context ceiling | Default for |
|---|---|---|
| `fleet-internal` | `local-only` | `search` |
| `hosted-per-user` | `personal` | — |
| `public-surface` | `shareable` | — |
| `local-owner` | `secret-bearing` | `get-chunk` |

The effective ceiling is the **minimum** of the `--max-sensitivity` grant and the context ceiling; neither widens the other. Content marked `excluded_personal_life` is ineligible in every context. An unknown `--context` value is a usage error, exit 2:

```
atlas-search: invalid --context "bogus": expected one of fleet-internal, hosted-per-user, public-surface, local-owner
```

An unknown `--max-sensitivity` is likewise never silently clamped: on the standalone path it is a validation fatal, and the via-service attempt is deliberately skipped so that exact fatal is produced (see "Via-service vs standalone").

## `search`: flag table

All 44 flags of `search` are listed below, in three groups: the verb's own flags, the flattened backend group (shared with `get-chunk`), and the flattened vector group (`search` only). Defaults stated here are the literal defaults; the query-parameter defaults derive from the shared contract file `query-parameters-v1.json`.

### Search flags

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--k` | integer | `10` | Final results. Validated: at least 1, no CLI upper bound. |
| `--pool` | integer | `100` | Per-branch candidates. Validated: 1..=400. |
| `--mode` | `hybrid\|fts\|vector` | `hybrid` | Which retrieval branches run (see Modes). |
| `--kind` | CSV string | — (none) | Classification kind filter. |
| `--topic` | CSV string | — (none) | Topic filter. |
| `--project` | CSV string | — (none) | Project filter. |
| `--client` | CSV string | — (none) | Client filter. |
| `--since` | `YYYY-MM-DD` | — (none) | Lower bound on `sessions.created_at`, inclusive. |
| `--until` | `YYYY-MM-DD` | — (none) | Upper bound on `sessions.created_at`, inclusive. |
| `--max-sensitivity` | ladder name | `personal` | Sensitivity ceiling: `shareable<personal<local-only<secret-bearing`; `secret-bearing` must be spelled out. |
| `--context` | context name | `fleet-internal` | Disclosure context of the destination surface; its ceiling intersects `--max-sensitivity`. |
| `--group-by-session` | switch | off | Roll chunk hits up to sessions (score = best chunk, plus hit count). |
| `--expand` | integer | `0` | Include N neighbouring chunks (same message) around each hit. Validated: 0..=100. |
| `--parameters_version` | integer | `1` | Version of the shared query-parameter contract (currently 1). Validated: 1..=1. Note the underscore spelling. |
| `--shard_top_k` | integer | `100` | Candidates scanned per shard. Validated: 1..=400. Underscore spelling. |
| `--global_top_k` | integer | `100` | Candidates retained after shard fan-in. Validated: 1..=400. Underscore spelling. |
| `--fts_weight` | float | `1` | Reciprocal-rank-fusion multiplier for FTS. Must be finite and ≥ 0; the two weights must not both be zero. Underscore spelling. |
| `--vector_weight` | float | `1` | Reciprocal-rank-fusion multiplier for vectors. Same validation as `--fts_weight`. Underscore spelling. |
| `--jaccard_depth` | integer | `0` | Candidate depth for Jaccard rescoring over the fused union (0 = off). Validated: 0..=4000. Underscore spelling. |
| `--jaccard_threshold` | float | `0` | Near-exact Jaccard threshold in (0, 1]; 0 means the design default 0.60 when rescoring is on. Validated: finite, in [0, 1]. Underscore spelling. |
| `--rrf_k` | integer | `10` | Reciprocal-rank-fusion constant. Validated: 1..=10000. Underscore spelling. |
| `--planner_enabled` | switch | off | Enable the query planner/rewriter. **Not implemented**: setting it is a validation error. Underscore spelling. |
| `--spell_enabled` | switch | off | Enable spell correction. **Not implemented**: setting it is a validation error. Underscore spelling. |
| `--query_class_hint` | string | — (none) | Query-class hint. **Not implemented**: setting any value is a validation error. Underscore spelling. |
| `--variant` | `raw\|rewritten\|both` | `raw` | Query variant: `raw` is supported; `rewritten` or `both` is a validation error. |
| `--raw` | switch | off | Use the verbatim query with planner/spell and expansion disabled. Cannot be combined with a non-zero `--expand`. |
| `--with-message` | switch | off | Include the parent message text (truncated at 50000 chars). |
| `--no-cache` | switch | off | Bypass service cache reads; the fresh result still lands in the cache (a refresh). Service mode only: the local path has no result cache. |
| `--wait` | switch | off (the CLI implies wait) | Block for the full fused result; the one-shot CLI implies `--wait`, this spells it out. Conflicts with `--no-wait`. |
| `--no-wait` | switch | off | Do not block: print the service job handle plus an empty first page and return. |
| `--hydrate` | integer | `0` | Hydrate the top N results to full chunk + message content from the content store. This is **not** `--expand`, which keeps its neighbour-chunks meaning. Validated: at most 1000. |
| `--raw-rows` | optional value: `ungrouped\|branches` | — (off) | Ungrouped output rows: bare `--raw-rows` means `ungrouped` (flat fused rows); `--raw-rows=branches` means pre-fusion branch rows. Distinct from `--raw` (verbatim query), which is not repurposed. |
| `--text` | switch | off | Human-readable output instead of JSON. |
| `--shards` | string | — (off) | Shown in `--help` output. Environment variable: `ATLAS_SHARDS`. EXPERIMENTAL, off by default: serve the vector branch from the Atlas core shard fan-out — `dir:&lt;path&gt;` for local shard files, or `turso` / `turso:&lt;org&gt;` for hosted shards. FTS, details and `--expand` stay on the backend. Skips `--via-service` entirely. |

### Backend flags (flattened, shared with `get-chunk`)

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--backend` | `auto\|turso\|local` | `auto` | Data backend. `auto`: Turso when a token resolves, else local. |
| `--db` | path | canonical `atlas.db` | Local backend: SQLite DB path. Resolution is covered in [Configuration and environment](configuration.md) (the `ATLAS_DB` variable is the first step). |
| `--vecdb` | comma-separated paths | — (none) | Local backend: vector DB path(s), if the corpus DB has no vector table. |
| `--via-service` | switch | off | Prefer a running Atlas service; fall back to standalone, loudly. The help states the eligibility restrictions, detailed under "Via-service vs standalone". |
| `--service-url` | URL | `http://127.0.0.1:18760` | Service base URL for `--via-service`. Environment variable: `ATLAS_SEARCH_URL`. |

### Vector flags (flattened, `search` only)

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--model` | string | `Snowflake/snowflake-arctic-embed-m-v2.0` | Embedding model. Environment variable: `ATLAS_SEARCH_MODEL`. |
| `--vec-table` | string | `vec_snowflake_m_v2_768_f32` | Vector table name. Environment variable: `ATLAS_VEC_TABLE`. |
| `--embedder` | `auto\|fastembed\|st` | `auto` | Query embedder: `fastembed` (bge) or `st` = sentence-transformers with `prompt_name='query'` (Snowflake); `auto` picks by model name. See the embedder limits under Behaviour. |
| `--trunc` | integer | `0` | Truncate query+doc vectors to N dims (Matryoshka; 0 = full). Validated: ≥ 0. |
| `--vector-backend` | `scan\|index-fill\|index-fixed` | `scan` | Turso vector branch: `scan` = exact `vector_distance_cos` over the filtered set; `index-fixed` = one `vector_top_k` lookup at pool depth, filtered afterwards; `index-fill` = `vector_top_k` with escalating depth until the pool fills after filtering (cap 3200). |

## `get-chunk`: flag table

`get-chunk` fetches one chunk with its provenance (the session, client and sensitivity it came from). It has **no vector flags** and **no `--max-sensitivity`** — the sensitivity ceiling arrives through `--context`, whose default here is `local-owner` (context ceiling `secret-bearing`), because this is the owner's explicit-id tool.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--neighbors` | integer | `0` | Include N chunks either side in the same message. Validated: 0..=100. |
| `--context` | context name | `local-owner` | Disclosure context of the destination surface: `local-owner` (default), `fleet-internal`, `hosted-per-user`, or `public-surface`. |
| `--text` | switch | off | Human-readable output instead of JSON. |
| `--backend` | `auto\|turso\|local` | `auto` | Backend group, as in the `search` backend table above. |
| `--db` | path | canonical `atlas.db` | Backend group, as above. |
| `--vecdb` | comma-separated paths | — (none) | Backend group, as above. |
| `--via-service` | switch | off | Backend group, as above — but see the get-chunk routing note below. |
| `--service-url` | URL | `http://127.0.0.1:18760` | Backend group, as above. Environment variable: `ATLAS_SEARCH_URL`. |

There is no via-service path for `get-chunk` in the job protocol: chunk reads are scoped to an existing job. Requesting `--via-service` therefore produces the loud fallback note (of the shape `service at &lt;base&gt; unreachable; get-chunk served standalone`) and the chunk is served standalone.

## Behaviour

- **Validation happens first.** All count, range and contract checks run before any backend or service work. Violations are exit 2 with the message prefixed `atlas-search: `. The full rule set: `--k` ≥ 1; `--pool` 1..=400 (`--pool must be between 1 and 400 (design section 2)`); `--expand` 0..=100; `--trunc` ≥ 0; `--neighbors` (get-chunk) 0..=100; `--parameters_version` 1..=1; `--shard_top_k` 1..=400; `--global_top_k` 1..=400; `--jaccard_depth` 0..=4000; `--rrf_k` 1..=10000; `--fts_weight`/`--vector_weight` finite, ≥ 0, and not both zero; `--jaccard_threshold` finite, in [0, 1]; `--hydrate` at most 1000 (`hydrate must be at most 1000`).
- **Reserved contract surface.** `--planner_enabled`, `--spell_enabled`, any `--query_class_hint`, and `--variant rewritten|both` exist in the shared query-parameter contract but are not implemented; setting any of them is a validation error (`planner/spell stages are not implemented yet`, `query_class_hint is not implemented yet`, `rewritten/both query variants are not implemented yet`). `--raw` combined with a non-zero `--expand` is an error: `raw mode cannot be combined with non-zero expand`.
- **Traces.** Every executed standalone search writes an `atlas-query-trace/v1` record (to the trace sink's default directory; see [Configuration and environment](configuration.md)) and returns its `query_id` in the result envelope. A trace-write failure **fails the search** — it is not a warning.
- **The embedder is lazy.** FTS-only searches and `get-chunk` never load, or fail on, the embedding model.
- **Embedder and model limits in this binary.** In the Rust binary, `--embedder fastembed` with vector or hybrid mode is a loud error (`vector/hybrid modes with a fastembed embedder are not available in this binary … use --mode fts or the Python atlas-search`), and any `--model` other than the default Snowflake model is likewise refused for vector/hybrid modes. The bge/fastembed fallback and arbitrary models live only in the Python implementation; see [atlas_search.py: the Python reference implementation](python-implementation.md).

## Via-service vs standalone

Standalone is the default route and needs no service. With `--via-service`, the client prefers a running service at `--service-url` (default `http://127.0.0.1:18760`); on **any** service failure it falls back to standalone loudly — a stderr warning plus a note in the result envelope's `notes`. The service side of the protocol is described in [atlas-search: serve, init, fetch-model, traces and feedback](atlas-search-admin.md) and [The service HTTP API](service-api.md).

Three requests disqualify the service route. **None of them appears in any help text**; this is source behaviour:

| Request | Effect on the service route |
|---|---|
| `--backend local` | Cannot be served by the service; forces the loud standalone fallback with an explanatory note. |
| A non-default `--vec-table` | Cannot be served by the service; forces the loud standalone fallback with an explanatory note. |
| Any `--shards` spec (flag or `ATLAS_SHARDS`) | Skips the via-service attempt entirely — no attempt, no fallback note; the search runs standalone with shard fan-out. |

An unknown `--max-sensitivity` also skips the via-service attempt, for a different reason: so that the standalone path produces its exact validation fatal rather than a service-side clamp.

## Output shapes

The default output is JSON on stdout: a result envelope carrying the hits and, for an executed search, the `query_id` of the trace record it wrote, plus a `notes` array (where a loud fallback note lands when one happened). With `--no-wait` on the service route, the output is instead the service job handle plus an empty first page. `--raw-rows` changes the row shape (flat fused rows, or pre-fusion branch rows with `=branches`) without changing the envelope. `--group-by-session` rolls chunk hits up to one row per session.

`--text` renders the same results human-readably instead of JSON. No partial JSON is printed on failure paths: failures are a single stderr line (the via-service fallback is the exception by design — it *succeeds* standalone and records the fallback in `notes` and on stderr).

## Worked examples

Toy examples; the queries and ids are invented.

A plain hybrid search, five results, human-readable:

```
$ atlas-search search "toy example query" --k 5 --text
```

An FTS-only search over a date window — no model is loaded:

```
$ atlas-search search "release checklist" --mode fts --since 2026-11-01 --until 2026-11-30
```

A search that asks for the top tier explicitly, in the owner's context:

```
$ atlas-search search "toy secret-adjacent note" --max-sensitivity secret-bearing --context local-owner
```

Fetch one chunk and its two neighbours on either side:

```
$ atlas-search get-chunk 424242 --neighbors 2 --text
```

Prefer the service, with the loud fallback if it is down:

```
$ atlas-search search "toy example query" --via-service
```

## Failure shapes

- Unknown subcommand or flag: a clap error with usage, exit 2 — e.g. `atlas-search badverb` gives `error: unrecognized subcommand 'badverb'`.
- Validation failure: `atlas-search: &lt;msg&gt;` on stderr, exit 2 — e.g. `atlas-search search --k 0 foo` gives `atlas-search: --k must be at least 1`; `--pool 401` gives `atlas-search: --pool must be between 1 and 400 (design section 2)`.
- Backend, transport or embedding failure: `atlas-search: &lt;msg&gt;` on stderr, exit 1.
- `get-chunk` for a chunk that does not exist: `atlas-search: chunk 999999999 not found`, exit 1.
- Service failure under `--via-service`: not a failure of the command — a stderr warning, a note in the envelope's `notes`, standalone results, exit 0.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success (including a loud standalone fallback after a service failure). |
| 1 | Backend/transport/embed failures; `get-chunk` not-found. |
| 2 | Usage and validation failures: clap errors, count/parameter/context validation. |
