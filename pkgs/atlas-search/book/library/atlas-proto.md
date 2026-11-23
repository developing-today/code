# atlas-proto: the wire contract

Source: [`crates/atlas-proto/src/lib.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-proto/src/lib.rs) and [`crates/atlas-proto/query-parameters-v1.json`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json).

`atlas-proto` is the shared wire contract between the Atlas search service and every client that talks to it over HTTP — the `atlas-search` CLI in its via-service mode, the `fleet search` client, and anything else that speaks the job protocol. It is serde types only (serde is Rust's serialization framework): no HTTP, no storage, no search policy. The server and the CLI's via-service client both depend on this one crate so their wire definitions cannot drift apart — a field renamed here is renamed on both sides in the same build. The engine behind the server is [atlas-core: the engine as a library](atlas-core.md); the endpoints that carry these types are [The service HTTP API](../reference/service-api.md).

One boundary to hold throughout: sensitivity clamping lives in the server, not here. This crate merely carries the ceiling a client *requests* and the effective ceiling the server *reports back*. A client that treats its request as the guarantee has misunderstood the contract.

## The defaults contract: `query-parameters-v1.json`

Every default in the query-parameter contract lives in one file, `crates/atlas-proto/query-parameters-v1.json`, embedded into the crate at compile time and parsed for every default value. Its complete contents:

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

Its role is to be the single source the defaults are derived from — the CLI's own defaults come from this file, not from a second table in the CLI's source. A missing or invalid default in the file panics at first use rather than falling back to a guessed value: a contract whose defaults are silently invented is worse than a crash. Field meanings, in brief: `k` is the final result count; `pool` is the per-branch candidate count; `expand` is neighbouring chunks included around a hit; `shard_top_k` / `global_top_k` bound the shard fan-out; `fts_weight` / `vector_weight` and `rrf_k` parameterize reciprocal rank fusion (the score-combining method for the full-text and vector branches); `jaccard_depth` / `jaccard_threshold` control an optional token-overlap rescore (off at depth 0); `hydrate` is top-N hydration to full content from the content store, distinct from `expand`; `raw_mode` selects pre-fusion output shapes. `wait: false` is the *wire* default — the one-shot CLI implies waiting and sets it unless told otherwise.

## The shared value types

| Type | Values (wire spelling) | Notes |
|---|---|---|
| `Sensitivity` | `shareable` &lt; `personal` &lt; `local-only` &lt; `secret-bearing` | kebab-case; the ordering *is* the security ordering; default `personal`; `clamped_to(cap)` is `min(self, cap)` |
| `SearchMode` | `fts`, `vector`, `hybrid` | lowercase; default `hybrid`; `branch_demand()` is 2 for hybrid, 1 otherwise — deliberately conservative, since a hybrid query whose full-text expression is empty may still use only the vector branch |
| `VectorBackend` | `scan`, `index-fixed`, `index-fill` | kebab-case; default `scan` (exact scan) |
| `RawMode` | `off`, `ungrouped`, `branches` | lowercase; default `off` |
| `QueryVariant` | `raw`, `rewritten`, `both` | lowercase; default `raw`; the other two are not implemented and are refused at validation |

These types are wire duplicates of atlas-core's own `Sensitivity`, `SearchMode`, and `VectorBackend` — same names, same spellings, same orderings, defined independently in both crates. The duplication is intentional: the wire crate must not force every client to depend on the engine.

`FilterSpec` carries the classification filters — `kind`, `topic`, `project`, `client` (each a list of strings), and `since` / `until` (optional date strings) — each field omitted from the JSON when empty or absent. The sensitivity ceiling is deliberately *not* a member of `FilterSpec`; it is a sibling field on `SearchRequest`, because it is clamped by a different rule than the filters are applied by.

## `SearchParametersV1` and `SearchRequest`

`SearchParametersV1` is flattened onto the wire — serialized inline into the request object, not nested — so each JSON name matches its CLI spelling exactly. Its fields are the contract file's fields: `parameters_version` (1), `shard_top_k` (100), `global_top_k` (100), `fts_weight` (1.0), `vector_weight` (1.0), `jaccard_depth` (0), `jaccard_threshold` (0.0), `rrf_k` (10), `planner_enabled` (false), `spell_enabled` (false), `query_class_hint` (null), `variant` (`raw`), `raw` (false), `no_cache` (false — bypass cache reads; the fresh result still lands in the cache), `wait` (false), `hydrate` (0), and `raw_mode` (`off`).

`validate(expand)` is part of the contract, and a conforming client runs the same checks before sending: `parameters_version` must be 1; `shard_top_k` and `global_top_k` in 1..=400; `rrf_k` in 1..=10000; both weights finite and at least 0, with at least one positive; `jaccard_threshold` finite in [0, 1]; `hydrate` at most 1000; `raw` combined with a non-zero `expand` is an error. Three settings are validation errors rather than quiet no-ops: `planner_enabled` or `spell_enabled` set (`planner/spell stages are not implemented yet`), any `query_class_hint` (`not implemented yet`), and a `variant` other than `raw` (`rewritten/both query variants are not implemented yet`).

`SearchRequest` is the request envelope for a search:

| Field | Default | Meaning |
|---|---|---|
| `query` | — (required) | the query text |
| `mode` | `hybrid` | a `SearchMode` |
| `filters` | empty | a `FilterSpec` |
| `ceiling` | `personal` | the *requested* `Sensitivity`; the server clamps it to the authenticated client's cap |
| `k` | `10` | final results (default read from the contract file) |
| `pool` | `100` | per-branch candidates; the server enforces the design maximum of 400 |
| `group_by_session` | `false` | roll chunk hits up to sessions |
| `expand` | `0` | neighbouring chunks per hit (contract-file default) |
| `parameters` | contract defaults | a `SearchParametersV1`, flattened |
| `with_message` | `false` | include parent message text |
| `model` | `Snowflake/snowflake-arctic-embed-m-v2.0` | the embedding model identity (see [atlas-embed: embeddings as a library](atlas-embed.md)) |
| `embedder` | `auto` | embedder selection spelling |
| `trunc` | `0` | truncate vectors to N dimensions (0 = full width) |
| `vector_backend` | `scan` | a `VectorBackend` |
| `page_limit` | none | one-shot endpoint only |
| `context` | none | the disclosure-context spelling — `fleet-internal`, `hosted-per-user`, `public-surface`, or `local-owner`; absent or unknown fails closed under the receiving surface's rule |

The crate also defines how a result is cache-keyed: `canonical_key_material(expand, group_by_session, with_message)` is a fixed-order labelled string covering every parameter plus those three request-level shaping controls, and `result_cache_key(query, snapshot_id, ceiling, expand, group_by_session, with_message)` is the hex SHA-256 of `atlas-result-cache/v1|query={query}|snapshot={snapshot_id}|ceiling={ceiling}|{canonical material}`. Note which ceiling goes into the key: the *effective* (clamped) one. Two clients requesting different ceilings above the same cap share a cache entry only at the ceiling both were actually served at.

## The job protocol types

Search over the service is a job protocol: submit, poll, page, delete. The types:

| Type | Carries |
|---|---|
| `JobCreated` | the 202 response to a job submission: `job_id`, `job_token`, `status`, `effective_ceiling`, `queue_depth`, `queue_position`, `submitted_at_ms`, `expires_at_ms` |
| `JobStatus` | `queued` (default), `running`, `done`, `failed`, `expired` — lowercase |
| `BranchState` / `BranchStates` | per-branch state for `fts` and `vector`: `pending` (default), `running`, `done`, `failed`, `skipped` |
| `JobStatusResponse` | `job_id`, `status`, `branch_states`, `effective_ceiling`, `pool`, `queue_depth`, `queue_position`, `notes`, `error`, `result_count`, `submitted_at_ms`, `started_at_ms`, `completed_at_ms`, `expires_at_ms` |
| `SearchPage` | one page of results: `job_id`, `status`, `effective_ceiling`, `envelope` (the search envelope with its `results` replaced by this page's slice), `results`, `next_cursor`, `total_results` |
| `JobSummary` / `JobListResponse` | the job-list rows, oldest first: metadata only — `job_id`, `status`, `query`, `mode`, `pool`, `effective_ceiling`, timestamps, `queue_position`. Never result content, never the capability token |
| `JobDeleted` | `job_id`, `status` |

The `job_token` in `JobCreated` is a per-job capability token: it authorizes only that job's status, results, chunk, and delete endpoints. It is not a client credential and must never appear in a job list — the `JobSummary` type makes that structural, by having no field for it.

Two smaller envelopes complete the surface:

- **Errors.** `ErrorResponse { error: ApiError }`, where `ApiError` is `{ code, message, retry_after_seconds, queue_depth }` (the last two optional). The constructor `ErrorResponse::rate_limited(message, retry_after_seconds, queue_depth)` fixes the code `rate_limited`. A client must branch on `code`, not on the message text.
- **Health and embedding.** `HealthResponse` answers the unauthenticated health endpoint: `{ status, embedder_state, queue_depth, active_branch_executions, active_jobs }`. `EmbedRequest { query, trunc }` (absent or 0 truncation = full width) and `EmbedResponse { model, dims, vector }` serve the embed endpoint: the response names its embedding space — model and dimensions — so a caller can refuse a vector from the wrong space. `EmbedResponse` is serialize-only; it is produced, never parsed, by the service side.

A toy submission and its created response, in shape only:

```json
{ "query": "toy sailboat ballast", "mode": "hybrid", "ceiling": "personal",
  "k": 10, "pool": 100, "parameters_version": 1, "rrf_k": 10 }
```

```json
{ "job_id": "toy-job-0001", "job_token": "&lt;per-job token&gt;",
  "status": "queued", "effective_ceiling": "personal",
  "queue_depth": 0, "queue_position": 0,
  "submitted_at_ms": 0, "expires_at_ms": 0 }
```

## Versioning posture

Versioning is by name, not by negotiation. The contract file is `query-parameters-v1.json`; the parameter type is `SearchParametersV1`; `parameters_version` must equal 1 and validation rejects anything else; the result-cache key carries its own `atlas-result-cache/v1` prefix. A future v2 is a new contract alongside this one, not a reinterpretation of it — a v1 client and a v1 server can never silently disagree about what a field means, because a mismatched version is an error at validation, before any search runs. The same posture appears across the wider system: stage, gate, trace, and feedback records all carry v1 format names of their own.

## What a conforming client must honour

1. **Build requests as `SearchRequest`.** Parameters flattened, JSON names matching the CLI spellings, defaults taken from the contract — not from a private copy that can drift.
2. **Validate before sending.** The `SearchParametersV1::validate` rules above; in particular, do not send planner, spell, hint, or rewritten-variant settings and expect them to be ignored — they are refused.
3. **Treat the ceiling as a request.** Read `effective_ceiling` from `JobCreated`, `JobStatusResponse`, and every `SearchPage`, and present results as served at that ceiling. The server clamps to the authenticated client's cap; neither the request nor the context widens the other.
4. **Keep the `job_token` scoped.** Present it only to its own job's endpoints; never log it, list it, or reuse it for another job.
5. **Page honestly.** Follow `next_cursor` until it is absent; `total_results` is the whole result set, `results` is only the page in hand, and the page's `envelope` has already had its `results` replaced by that slice.
6. **Branch on error `code`.** Honour `retry_after_seconds` and `queue_depth` on a `rate_limited` error instead of retrying immediately.
7. **Check the embedding space.** On an `EmbedResponse`, refuse a vector whose `model` or `dims` is not the space you asked in — vectors from different spaces are not comparable.
8. **Fail closed on context.** Send a recognized `context` spelling; an absent or unknown one is resolved by the receiving surface's fail-closed rule, which may serve you at a lower ceiling than you expected — by design.

The hosted Worker surface that consumes this same contract — including its MCP tool — is covered in [The Atlas MCP server](../reference/mcp.md), and a worked end-to-end use of all three client shapes is [Tutorial: search the corpus three ways](../tutorials/search-the-corpus.md).
