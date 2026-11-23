# The service HTTP API

Source: [crates/atlas-search/src/server.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/server.rs) and its `server/` submodules, [crates/atlas-proto/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-proto/src/lib.rs), [crates/atlas-core/src/feedback.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/feedback.rs) — provenance only; this chapter is complete without following it.

The service is the laptop half of Atlas: a loopback HTTP service, run by `atlas-search serve` or by its rehomed twin `fleet serve search` (see [fleet search and fleet serve search](fleet-search.md)), that holds one shared Snowflake embedder and a read-only backend and serves search as a *job protocol* — submit a search, then pull its immutable fused ranking in pages. It binds `127.0.0.1:18760` and nothing else; wider reach is a `tailscale serve` mapping in front of the loopback port, never a public listener of its own. The Cloudflare Worker described in [The Worker HTTP API](worker-api.md) is a separate, hosted surface that calls this service; the two are not the same API.

```http
GET    /health
POST   /v1/search
POST   /v1/search/jobs
GET    /v1/search/jobs
GET    /v1/search/jobs/<job_id>
GET    /v1/search/jobs/<job_id>/results?cursor=<cursor>&limit=<n>
DELETE /v1/search/jobs/<job_id>
GET    /v1/search/jobs/<job_id>/chunk/<chunk_id>
POST   /v1/embed
POST   /v1/feedback
GET    /v1/feedback
```

Every route except `GET /health` requires a per-identity bearer token (see "Identity, tokens and the ceiling clamp" below). Request and response bodies are JSON. The wire types are shared by the service and its clients through the `atlas-proto` crate, so the definitions cannot drift between them.

## Route summary

| Method and path | Auth | Success | Meaning |
|---|---|---|---|
| `GET /health` | none | 200 | Liveness, embedder state and queue depth. No corpus data. |
| `POST /v1/search` | client token | 200 | One-shot search: submit, wait, and return the first page in one call. |
| `POST /v1/search/jobs` | client token | 202 | Submit a search job; returns the job handle. |
| `GET /v1/search/jobs` | client token | 200 | List the calling identity's own jobs, metadata only. |
| `GET /v1/search/jobs/&lt;job_id&gt;` | client token or job token | 200 | One job's status. |
| `GET /v1/search/jobs/&lt;job_id&gt;/results` | client token or job token | 200 | One page of a job's ranking. |
| `DELETE /v1/search/jobs/&lt;job_id&gt;` | client token or job token | 200 | Delete a job and its stored ranking. |
| `GET /v1/search/jobs/&lt;job_id&gt;/chunk/&lt;chunk_id&gt;` | client token or job token | 200 | One chunk, scoped to an existing job. |
| `POST /v1/embed` | client token | 200 | Embed one query text in the pinned Snowflake space. |
| `POST /v1/feedback` | client token | 201 | Cast a feedback vote (`atlas-feedback/v1`); returns the stored record. |
| `GET /v1/feedback` | client token | 200 | Retrieve feedback by query (and optionally chunk). |
| any other route | — | 404 | `not_found` in the shared error envelope. |

`/v1/feedback` also answers at the unversioned alias `/feedback`, for both `POST` and `GET`.

## `GET /health`

No authentication, no parameters, no corpus data:

```json
{
  "status": "ok",
  "embedder_state": "ready",
  "queue_depth": 0,
  "active_branch_executions": 0,
  "active_jobs": 0
}
```

`embedder_state` is the shared embedder's load state (design vocabulary: `cold`, `warming`, `ready`). `queue_depth`, `active_branch_executions` and `active_jobs` describe the job machinery at the moment of the call.

## Identity, tokens and the ceiling clamp

Two kinds of token exist, and they are not interchangeable:

| Token kind | Where it comes from | What it authorises |
|---|---|---|
| Client token | One per named identity in `service.json` (see [Configuration and environment](configuration.md)); the client presents the matching value from its own `client.json`. | Every authenticated route, for that identity only. |
| Job token | Minted by the service and returned once, in the 202 response that creates a job. | Only that one job's status, results, chunk and delete routes. A job token cannot list jobs and cannot touch any other job. |

Presentation is the same for both: the header `Authorization: Bearer &lt;token&gt;`. Token values are never printed or logged by any tool; this book names tokens only.

Each client identity carries a *ceiling cap* — the highest sensitivity ceiling that identity may ever search under. Sensitivity is a ladder, lowest to highest: `shareable` &lt; `personal` &lt; `local-only` &lt; `secret-bearing`. A search request names a requested ceiling; the service clamps it:

```
effective_ceiling = min(requested_ceiling, calling identity's ceiling_cap)
```

The clamp is the security property: no request field, header or client-side flag can widen it. The effective ceiling is recorded on the job and reported in every status, page and envelope response, so a caller can always see the ceiling its results were actually computed under. See [Sensitivity tiers and client ceilings](../concepts/sensitivity.md) for the ladder and what each rung admits.

The v1 client table written by `atlas-search init` is:

| Identity | Ceiling cap | Intended caller |
|---|---|---|
| `drew-cli` | `secret-bearing` | The user's own CLI on the laptop. |
| `openchamber` | `personal` | OpenChamber sessions. |
| `fork-server` | `personal` | Fork / OpenCode agents. |
| `sandbox-scripts` | `personal` | Sandbox scripts and Muse lanes, over the tailnet. |

One later identity exists beyond the `init` table: `cf-worker`, cap `personal`, whose token the Cloudflare Worker presents when it calls this service (see [The Worker HTTP API](worker-api.md)). Raising any cap is a local edit to `service.json` by the user; there is no API for it.

A missing or invalid bearer token fails with an unauthorised error in the shared error envelope (see "Errors"), on every authenticated route, in the same shape.

## The search request

Both search submissions — one-shot and job — take the same request body, the `atlas-proto` `SearchRequest`:

```json
{
  "query": "queue broker lease expiry",
  "mode": "hybrid",
  "filters": { "kind": [], "topic": [], "project": [], "client": [] },
  "ceiling": "personal",
  "k": 10,
  "pool": 100,
  "group_by_session": false,
  "expand": 0,
  "with_message": false,
  "model": "Snowflake/snowflake-arctic-embed-m-v2.0",
  "embedder": "auto",
  "trunc": 0,
  "vector_backend": "scan",
  "context": "fleet-internal"
}
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `query` | string | — (required) | The query text. |
| `mode` | `hybrid` \| `fts` \| `vector` | `hybrid` | Which branches run. |
| `filters` | object | empty | Classification filters: `kind`, `topic`, `project`, `client` (string lists) and `since`, `until` (session dates, inclusive). |
| `ceiling` | ladder name | `personal` | The *requested* ceiling; the service clamps it to the caller's cap. |
| `k` | integer | 10 | Final results wanted. |
| `pool` | integer | 100 | Per-branch candidates; the service enforces the design maximum of 400. |
| `group_by_session` | boolean | `false` | Roll chunk hits up to sessions. |
| `expand` | integer | 0 | Neighbouring chunks to include around each hit (0–100). |
| `with_message` | boolean | `false` | Include the parent message text. |
| `model` | string | `Snowflake/snowflake-arctic-embed-m-v2.0` | Embedding model name; the pinned space (see below). |
| `embedder` | string | `auto` | Embedder selection. |
| `trunc` | integer | 0 | Truncate vectors to N dimensions; 0 = full width. |
| `vector_backend` | `scan` \| `index-fixed` \| `index-fill` | `scan` | Vector branch strategy. |
| `page_limit` | integer or null | null | One-shot endpoint only: page size for the first page. |
| `context` | string or null | null | Disclosure context spelling: `fleet-internal`, `hosted-per-user`, `public-surface` or `local-owner`. Absent or unknown values fail closed under the receiving surface's rule; the context intersects the ceiling and neither widens the other. |

Flattened onto the same object are the versioned query parameters (`parameters_version`, `shard_top_k`, `global_top_k`, `fts_weight`, `vector_weight`, `jaccard_depth`, `jaccard_threshold`, `rrf_k`, `planner_enabled`, `spell_enabled`, `query_class_hint`, `variant`, `raw`, `no_cache`, `wait`, `hydrate`, `raw_mode`), whose defaults are the contract in `query-parameters-v1.json` — see [Configuration and environment](configuration.md). Two of them steer the protocol itself: `wait` (protocol default `false`; the one-shot CLI sets it unless told not to) and `no_cache` (bypass the service's result-cache reads; the fresh result still lands in the cache).

## `POST /v1/search` — the one-shot

The one-shot submits the search, waits for the job to complete, and returns the first page in a single response, with status 200. Its body is a `SearchPage` (below). This is the form the Worker uses, with `page_limit` unset. A caller that wants the handle instead of the wait — or wants to page at its own pace — uses the job routes.

## The job protocol

### Submit: `POST /v1/search/jobs` → 202

```json
{
  "job_id": "<job id>",
  "job_token": "<per-job capability token>",
  "status": "queued",
  "effective_ceiling": "personal",
  "queue_depth": 1,
  "queue_position": 0,
  "submitted_at_ms": 1763832000000,
  "expires_at_ms": 1763832600000
}
```

Keep both halves of the handle: `job_id` names the job in every later path, and `job_token` is the capability that authorises those calls without exposing the client token. `expires_at_ms` is when the job's stored ranking leaves the temp store (see "The temp store and job lifetime").

### Status: `GET /v1/search/jobs/&lt;job_id&gt;`

Job statuses are `queued`, `running`, `done`, `failed` and `expired`. The status response reports both branches separately — `branch_states` holds an `fts` and a `vector` state, each `pending`, `running`, `done`, `failed` or `skipped` — because a filtered vector scan is seconds of server-side work and the branches do not finish together.

| Field | Meaning |
|---|---|
| `job_id`, `status` | The job and its lifecycle state. |
| `branch_states` | `{ "fts": &lt;state&gt;, "vector": &lt;state&gt; }`. |
| `effective_ceiling` | The clamped ceiling the job runs under. |
| `pool` | The per-branch candidate depth in force. |
| `queue_depth`, `queue_position` | Backlog at the time of the call; `queue_position` is null once running. |
| `notes` | Strings the service attached while running the job (fallbacks, cache notes, degradations). |
| `error` | The failure message when `status` is `failed`; otherwise null. |
| `result_count` | Total results in the fused ranking once known; otherwise null. |
| `submitted_at_ms`, `started_at_ms`, `completed_at_ms`, `expires_at_ms` | Millisecond timestamps; start and completion are null until they happen. |

### Results: `GET /v1/search/jobs/&lt;job_id&gt;/results?cursor=&lt;cursor&gt;&amp;limit=&lt;n&gt;`

Results are pulled, not pushed — *pull-until-satisfied*. The ranking is computed once, fused once, and is immutable for the life of the job; paging never re-runs ranking, and fetch-to-fill depth escalation lives inside the job's vector branch, never in client paging.

| Parameter | Default | Meaning |
|---|---|---|
| `cursor` | — (first page) | The previous page's `next_cursor`, passed back verbatim. Cursors are opaque: do not parse, construct or store them beyond the life of the job. |
| `limit` | 100 | Page size. |

```json
{
  "job_id": "<job id>",
  "status": "done",
  "effective_ceiling": "personal",
  "envelope": { "results": ["<this page's slice of the search envelope>"] },
  "results": ["<hit objects, in fused rank order>"],
  "next_cursor": "<opaque cursor>",
  "total_results": 37
}
```

`next_cursor` is the whole continuation contract: pass it back as `cursor` to get the next page, and stop when it comes back null — null means the ranking is exhausted, not that a page was empty by accident. `envelope` is the ordinary search envelope with its `results` replaced by this page's slice, so a client that renders envelopes needs no second code path. `total_results` is the full ranking's size, the same on every page.

### List: `GET /v1/search/jobs`

Returns `{ "jobs": [ &lt;summaries&gt; ] }`, oldest first, containing only the calling identity's own jobs. A summary is metadata only — `job_id`, `status`, `query`, `mode`, `pool`, `effective_ceiling`, `submitted_at_ms`, `expires_at_ms`, `queue_position` — never result content and never a job token. Job capability tokens cannot call this route at all.

### Chunk: `GET /v1/search/jobs/&lt;job_id&gt;/chunk/&lt;chunk_id&gt;`

Chunk reads are scoped to an existing job: there is no free-standing chunk route, because a chunk's disclosure is judged against the job that surfaced it. The query parameter `neighbors` asks for that many chunks either side in the same message, at most 100.

### Delete: `DELETE /v1/search/jobs/&lt;job_id&gt;`

Deletes the job and its stored ranking immediately, ahead of expiry. Response: `{ "job_id": "&lt;job id&gt;", "status": "&lt;status string&gt;" }`.

## `POST /v1/embed`

Embeds one query text and returns the vector, for callers — in practice the Worker, on its cache-miss path — that need the embedding itself:

```json
{ "query": "queue broker lease expiry", "trunc": 0 }
```

```json
{ "model": "Snowflake/snowflake-arctic-embed-m-v2.0", "dims": 768, "vector": [0.012, -0.034] }
```

(The vector above is a toy two-number stand-in; a real response carries 768 numbers.) `trunc` is optional; absent or 0 means full width. The response names its space — model and dimensions — so a caller can refuse a vector from the wrong space, and callers do: the Worker validates every embed response against the pinned space (model `Snowflake/snowflake-arctic-embed-m-v2.0`, 768 dimensions, every component finite) and refuses anything else before storing it. See [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md).

Any valid client token may embed. Ceilings govern search, not embedding: an embedding of a query text discloses nothing from the corpus, so no cap applies to this route.

## `POST` and `GET /v1/feedback`

Feedback is a thumbs vote on a served result, captured under the schema string `atlas-feedback/v1`. A vote attaches to the `(query_id, chunk_id)` result identity — never to a rank position — and a vote with `chunk_id` null is a per-query vote.

Submission (`POST`, returns 201 with the stored record):

```json
{
  "query_id": "<the query_id the search envelope carried>",
  "chunk_id": 112081,
  "vote": "up",
  "comment": "right session, wrong chunk",
  "requester": { "kind": "agent", "id": "lane-3" },
  "opened": true
}
```

| Field | Rule |
|---|---|
| `query_id` | Required, non-empty. |
| `chunk_id` | A chunk id, or null for a per-query vote. |
| `vote` | Exactly `up` or `down`. |
| `comment` | Optional, stored verbatim, at most 256 KiB. |
| `requester.kind` | One of `drew`, `agent`, `user`, `anonymous`; empty means `anonymous`. |
| `requester.id` | Stable requester id; may be empty for anonymous. |
| `opened` | Whether the result was opened or expanded before the vote. |

The stored record adds `schema` (`atlas-feedback/v1`), a minted `feedback_id`, and `ts` (RFC 3339, UTC). Records are append-only and never mutated; a later vote from the same requester on the same `(query_id, chunk_id)` supersedes the earlier one in the derived view, never by rewriting it.

The client token is this surface's credential — the requester named in the body may be anonymous, because the token already identifies the calling surface. `GET /v1/feedback` retrieves the records for a query (and, when given, a chunk), each with its supersede state.

## The temp store and job lifetime

Job results live in an **in-memory** temp store inside the service process. Two lifetimes bound every job, both overridable at serve time (see [Configuration and environment](configuration.md)):

| Lifetime | Default | Meaning |
|---|---|---|
| Completed-job TTL | 600000 ms (10 minutes) | How long a completed job's ranking stays pageable after completion. `expires_at_ms` on the handle and status is this deadline. |
| Submit hard cap | 3600000 ms (60 minutes) | The outer bound from submission, however the job's run goes. |

Nothing in the temp store survives a service restart, by requirement: job results hold ceiling-gated snippets, and persistence would be a liability, not a feature. A durable-store design (a scratch database) was rejected on exactly that ground, and on the evidence of the corpus writer's stall behaviour. When a job passes its deadline its status reads `expired` and its pages are gone; re-running the search is the recovery, and the service's result cache may make that cheap.

Backpressure is part of the protocol shape: the service bounds concurrent branch executions and per-client queued jobs, and saturation is answered with the `rate_limited` error (below) carrying `retry_after_seconds` and `queue_depth`, rather than with unbounded queueing.

## Errors

Every error uses the shared envelope — the same shape the Worker uses, defined in `atlas-proto`:

```json
{ "error": { "code": "<code>", "message": "<message>" } }
```

The envelope may also carry `retry_after_seconds` and `queue_depth` when the code is `rate_limited`; both are absent otherwise.

| Status | Code | When |
|---|---|---|
| 200 / 201 / 202 | — | Success: reads and one-shot search (200), feedback submission (201), job submission (202). |
| 401 | `unauthorized` | Missing or invalid bearer token, client or job, on any authenticated route. One shape for every route, so a caller cannot probe which token kind a route expected. (Status and code verified in `crates/atlas-search/src/server.rs`: `ServerError::unauthorized` is HTTP 401 with code `unauthorized`.) |
| 404 | `not_found` | Unknown route. A chunk the caller may not see — or that does not exist — is also 404, with code `chunk_not_found`; an expired or unknown job is 404 with code `job_expired_or_unknown`, and a job that expired before its results were read is 410 with code `job_expired`. |
| 429 | `rate_limited` | Saturation: the error carries `retry_after_seconds` and `queue_depth`; wait the stated interval rather than retrying immediately. |

## Worked example

Submit without waiting, poll, then page — the toy session a client library performs on a caller's behalf:

```http
POST /v1/search/jobs
Authorization: Bearer <client token>
Content-Type: application/json

{ "query": "onnx build failure", "ceiling": "secret-bearing", "k": 10 }
```

A `drew-cli` caller (cap `secret-bearing`) gets 202 with `effective_ceiling: "secret-bearing"`; the same body from `sandbox-scripts` (cap `personal`) gets 202 with `effective_ceiling: "personal"` — the clamp, visible on the handle, is the only difference. Then:

```http
GET /v1/search/jobs/<job_id>
Authorization: Bearer <job token>
```

until `status` is `done`, then:

```http
GET /v1/search/jobs/<job_id>/results?limit=10
Authorization: Bearer <job token>
```

and repeat with each returned `next_cursor` as `cursor` until `next_cursor` is null.

## Exit codes

This is an HTTP surface; it has no process exit codes of its own. The serving processes do: config-load failures under `serve` (missing config, config not mode 0600, empty client table, duplicate client names, a bad `ceiling_cap`) exit 2 with `run atlas-search init first` guidance where applicable, and runtime serve failures exit 1 — see [atlas-search: serve, init, fetch-model, traces and feedback](atlas-search-admin.md).
