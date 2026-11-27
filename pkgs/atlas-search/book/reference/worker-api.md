# The Worker HTTP API

Source: [worker/src/index.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/index.ts), [worker/src/search.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/search.ts), [worker/src/cache_do.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/cache_do.ts), [worker/src/body.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/body.ts), [worker/src/key.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/key.ts) — provenance only; this chapter is complete without following it.

The Worker is the hosted half of Atlas: a Cloudflare Worker named `atlas-search`, reached at `https://atlas-search.developing-today.workers.dev`, that fronts the laptop service described in [The service HTTP API](service-api.md) with a per-query cache and a pinned disclosure context. Routing is a hand-written `fetch` handler — exact method plus exact path matches, no router library, no trailing-slash handling, and no CORS handling anywhere.

```http
GET  /health
POST /v1/search
GET  /v1/cache/inspect?query=<query>&k=<k>
POST /v1/cache/purge
POST /mcp
POST /v1/admin/retained/audit
GET  /v1/admin/retained/compare?object=<id>&from=<version>&to=<version>
POST /v1/admin/retained/report
POST /v1/admin/retained/promote
```

That is the whole public surface: **9 public routes**, plus a catch-all 404 for everything else. Three further routes exist inside the Worker's cache Durable Object; they are internal and marked as such below. The four retained-object admin routes are gated like the others, answer 503 `retained_off` when no retained-object store is configured, and are covered in [Retained objects and retention classes](retained-objects.md).

## The shared gate

Every route except `GET /health` sits behind one shared gate:

| Aspect       | Rule                                                                                                                                                                                                                                      |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Secret name  | `GATE_TOKEN` (named only; never valued in this book).                                                                                                                                                                                     |
| Presentation | Header `Authorization: Bearer &lt;GATE_TOKEN&gt;`. The prefix check is the exact, case-sensitive string `Bearer `; anything else presents an empty token and fails.                                                                       |
| Comparison   | Both the presented and the configured token are SHA-256 digested, and the 32 digest bytes are XOR-folded; equal if and only if the fold is 0. Digests are fixed-size, so the comparison is length-safe and constant-time by construction. |
| Failure      | Missing presented token, or `GATE_TOKEN` unset → the gate fails → 401.                                                                                                                                                                    |

The gate failure shape is the shared error envelope (below) with code `unauthorized` and message `missing or invalid gate token`.

One token serves all gated routes — `/v1/search`, `/v1/cache/inspect`, `/v1/cache/purge`, `/mcp` and the four `/v1/admin/retained/*` routes. There are no per-route, per-caller or per-agent tokens in the Worker, and no caller identity is extracted from the token: a passed gate yields a boolean, nothing else. Every gated caller is therefore the same identity to the Worker. What that identity is — disclosure context `hosted-per-user`, ceiling `personal`, both pinned at compile time — is covered in [The Atlas MCP server](mcp.md) and [Sensitivity tiers and client ceilings](../concepts/sensitivity.md); no env var, request field or header can change either value.

Where the gate sits in each route's validation order matters and differs by route; each route below states it.

## The shared error envelope and the body limit

Every Worker-generated error uses one envelope:

```json
{ "error": { "code": "<code>", "message": "<message>" } }
```

Two exceptions: the MCP endpoint answers with JSON-RPC error objects (see [The Atlas MCP server](mcp.md)), and upstream failures on the search path pass through the Durable Object's `query_failed` envelope (see "Upstream failures" below).

Request bodies are capped at 64 KiB (`MAX_SEARCH_BODY_BYTES = 64 * 1024`). A declared `Content-Length` over the cap is refused without reading; an undeclared body is read incrementally and cancelled at the cap. Over-cap → 413, code `payload_too_large`, message `body is larger than the 64 KiB limit`.

## `GET /health`

Open — no gate, checked before it. No body, no parameters read. The only response this route produces:

```json
{ "status": "ok", "service": "atlas-search-worker" }
```

with status 200. It is liveness only; it says nothing about the laptop service behind the Worker.

## `POST /v1/search`

The gated search route. Validation order is fixed — the gate is checked **before the body is read**, then the body chain runs:

| Order | Check                                                          | Failure                                                          |
| ----- | -------------------------------------------------------------- | ---------------------------------------------------------------- |
| 1     | Gate token                                                     | 401, `unauthorized`, `missing or invalid gate token`             |
| 2     | Body ≤ 64 KiB                                                  | 413, `payload_too_large`, `body is larger than the 64 KiB limit` |
| 3     | Body parses as JSON                                            | 400, `bad_request`, `body must be JSON`                          |
| 4     | Parsed body is an object (JSON `null` parses, then fails here) | 400, `bad_request`, `body must be a JSON object`                 |
| 5     | `query` is a string                                            | 400, `bad_request`, `query must be a string`                     |
| 6     | `query` non-empty **after normalization**                      | 400, `bad_request`, `query must not be empty`                    |

Normalization is NFC, trim, and collapse of every whitespace run to one space — so a whitespace-only query passes check 5 and fails check 6. Case is preserved; the fan-out is case-sensitive. `Content-Type` is not checked: the body is read as text and parsed regardless of header.

Request:

```json
{ "query": "queue broker lease expiry", "k": 10 }
```

`k` is optional and clamped by `clampK` (below). Success is 200 with the search envelope:

```json
{
  "cache": "miss",
  "key": "<64-hex-char SHA-256 cache key>",
  "query": "queue broker lease expiry",
  "result": { "<the service's search result, passed through verbatim>" },
  "meta": {
    "created_at": "<ISO-8601>",
    "searched_at": "<ISO-8601>",
    "hit_count": 0,
    "embed_ms": 41,
    "search_ms": 6123,
    "dims": 768,
    "model": "Snowflake/snowflake-arctic-embed-m-v2.0"
  }
}
```

`cache` is `miss` when this call ran the fan-out and `hit` when the Durable Object cache served it. `result` is whatever the laptop service returned — the Worker treats it as opaque and does not validate its inner shape beyond the `effective_ceiling` check described in [The service and the Worker](../concepts/service-and-worker.md). (The service's own result shape carries fields such as `status`, `effective_ceiling`, `total_results`, `job_id` and `next_cursor`; those are the service's fields, passed through inside `result`, not the Worker's — the Worker has no job protocol of its own.)

## `GET /v1/cache/inspect`

The operator and audit surface: metadata readback for one cache entry, without running a search. Gated (failure → the same 401 as above). Parameters arrive in the query string:

| Parameter | Required | Rule                                                                                                                                                        |
| --------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `query`   | yes      | Missing or empty parameter → 400, `bad_request`, `query parameter is required`. After normalization, empty → 400, `bad_request`, `query must not be empty`. |
| `k`       | no       | Read as a number, then `clampK`. Absent → 10; non-numeric → 10 (see `clampK`).                                                                              |

The route computes the cache key, opens the cache object for that key, and merges its metadata into the response. When no entry exists, the whole answer is:

```json
{ "key": "<cache key>", "query": "<normalized query>", "exists": false }
```

When an entry exists, the forwarded metadata is:

```json
{
  "exists": true,
  "query": "<stored query>",
  "k": 10,
  "has_embedding": true,
  "embedding_dims": 768,
  "embedding_valid": true,
  "has_result": true,
  "result_bytes": 12345,
  "created_at": "<ISO-8601>",
  "searched_at": "<ISO-8601 or null>",
  "embed_ms": 41,
  "search_ms": 6123,
  "hit_count": 3
}
```

`embedding_dims` is 0 when there is no embedding or it does not parse; `embedding_valid` means exactly: dimensions equal 768 and every component is finite. Metadata only — inspect never returns the result body or the embedding vector, only presence, dimensions, validity and byte size.

## `POST /v1/cache/purge`

Gated, with the gate checked before the body is read, and a validation chain identical to `POST /v1/search` (413, then the four 400s, in the same order with the same codes and messages). Body:

```json
{ "query": "queue broker lease expiry", "k": 10 }
```

The route computes the cache key and deletes that one entry. Response, 200:

```json
{ "key": "<cache key>", "query": "<normalized query>", "purged": true }
```

`purged` is true if and only if an entry existed. Purge is exact-key only: there is no prefix or enumeration purge, deliberately — the key is a SHA-256 digest, and enumeration would need a separate index.

## `POST /mcp`

The path `/mcp` matches regardless of method, and the method check runs **before** the gate: a non-POST request gets 405 — code `method_not_allowed`, message `the MCP endpoint accepts POST only`, plus the response header `Allow: POST` — even without a token. A POST without a valid gate token gets the shared 401. A gated POST is delegated to the MCP handler, whose full protocol is [The Atlas MCP server](mcp.md).

## `clampK`: how `k` is read on every surface

One function clamps the result depth on every Worker surface — REST search body, REST purge body, inspect query parameter, and MCP tool arguments:

| Input                                                                                                     | Result                                                                             |
| --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| A JSON number                                                                                             | Floored (`10.9` → 10), then clamped to [1, 50]: 0 and negatives → 1, over 50 → 50. |
| A non-finite number (`NaN`, `Infinity` — reachable through the inspect route's numeric coercion)          | 10                                                                                 |
| Anything that is not a JSON number — a numeric string such as `"20"`, a boolean, null, or an absent field | 10                                                                                 |

The defaults behind the table: `DEFAULT_K = 10`, `MAX_K = 50`. Because the clamped `k` is a field of the cache key, different clamped values are different cache entries.

## Upstream failures: the `query_failed` passthrough

Search execution runs inside the cache Durable Object. Any exception inside it is caught and returned as:

```json
{ "error": { "code": "query_failed", "message": "<the error's message>" } }
```

with HTTP status taken from the error when it carries one, else 500. The two error kinds the service client raises both carry 502 — an unreachable or misbehaving laptop service, and an embed response that fails validation against the pinned Snowflake space — so in practice a REST search failure is **502 `query_failed`**, and only a genuinely unexpected internal error surfaces as **500**. The status and payload pass through the search execution path unchanged to the REST caller. (MCP callers see the same payload differently: wrapped in an `isError: true` result over HTTP 200 — see [The Atlas MCP server](mcp.md).) One failure detail worth knowing: a failed search caches nothing beyond a possibly stored embedding, so a retry after the service recovers does not re-embed.

## Status codes, all of them

| Status | Code                 | Produced by                                                                                                                                                                       |
| ------ | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 200    | —                    | Health, search success, inspect, purge.                                                                                                                                           |
| 400    | `bad_request`        | Unparseable JSON body; non-object body; `query` not a string; `query` empty after normalization; missing or empty inspect `query` parameter.                                      |
| 401    | `unauthorized`       | Gate failure on any gated route.                                                                                                                                                  |
| 404    | `not_found`          | The catch-all: any other method/path combination, including `GET /v1/search` and `POST /health` — method is part of the match for every route except `/mcp`. Message: `no route`. |
| 405    | `method_not_allowed` | Non-POST to `/mcp`, with `Allow: POST`.                                                                                                                                           |
| 413    | `payload_too_large`  | Body over the 64 KiB cap, on any route that reads a body.                                                                                                                         |
| 500    | `query_failed`       | Unexpected error inside the cache object, with no status of its own.                                                                                                              |
| 502    | `query_failed`       | Upstream service or embedding-space failure, passed through from the cache object.                                                                                                |

## Internal routes: the `QueryCache` Durable Object

**Internal — not reachable from the public internet.** These three routes exist only on synthetic `https://do/…` URLs, invoked by the Worker itself through the `QUERY_CACHE` binding, one object instance per cache key. No gate token is checked inside the object; the public routes above are the only gate.

| Method and path | Caller                    | Behaviour                                                                                                                                                    | Response                                                               |
| --------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- |
| `POST /query`   | The search execution path | Body `{ "query": …, "k": … }`. Runs the check-and-fill: serve a complete entry as a hit, otherwise embed, persist the embedding, search, persist the result. | 200 with the outcome, or the `query_failed` envelope.                  |
| `GET /inspect`  | `GET /v1/cache/inspect`   | Metadata readback for the entry.                                                                                                                             | 200 with the metadata object.                                          |
| `POST /purge`   | `POST /v1/cache/purge`    | Deletes the object's single entry row.                                                                                                                       | 200 with `{ "purged": &lt;bool&gt; }`.                                 |
| anything else   | —                         | —                                                                                                                                                            | 404 with `{ "error": { "code": "not_found", "message": "no route" } }` |

## Worked example

A gated search, then the audit readback for the same entry:

```http
POST /v1/search
Authorization: Bearer <gate token>
Content-Type: application/json

{ "query": "  queue   broker lease  " }
```

The query normalizes to `queue broker lease`, `k` defaults to 10, and the answer carries `cache: "miss"` the first time and `cache: "hit"` on every repeat — including a repeat that arrives through the MCP `search` tool, because both surfaces share this execution path and this cache. Then:

```http
GET /v1/cache/inspect?query=queue%20broker%20lease
Authorization: Bearer <gate token>
```

returns the entry's metadata with `hit_count` counting the hits so far.

## Exit codes

None: the Worker is a hosted HTTP surface with no process interface. Its deploy and test commands (`wrangler dev`, `vitest run`, the `cf` versions flow) belong to [Configuration and environment](configuration.md) and the operations chapters.
