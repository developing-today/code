# Tutorial: give an agent the Atlas MCP server

Source: [pkgs/atlas-search/worker/src/index.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/index.ts), [pkgs/atlas-search/worker/src/mcp.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/mcp.ts), [pkgs/atlas-search/worker/src/space.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/space.ts), and [home/common/mcpx-settings.nix](https://github.com/developing-today/code/blob/main/home/common/mcpx-settings.nix) (issue #8625). Provenance only — this chapter is complete without following it.

This tutorial wires one agent to Atlas through MCP (the Model Context Protocol, the protocol agents use to call external tools). By the end the agent can call one tool, `search`, over the user's unified Atlas index. There is no per-agent account to create and no second search path to configure: the MCP endpoint and the Worker's REST search share one execution path, one cache, and one pinned ceiling. For the underlying surfaces see [The Atlas MCP server](../reference/mcp.md) and [The Worker HTTP API](../reference/worker-api.md).

## What you are connecting to

The endpoint is the Atlas Cloudflare Worker's MCP route:

```
POST https://atlas-search.developing-today.workers.dev/mcp
```

- Transport: stateless Streamable HTTP. There is no session to open or keep: no session id is issued, required, or read, and there is no streaming (SSE) response mode. Each HTTP request carries one JSON-RPC message, or a batch (a JSON array) of messages, and is answered with a plain JSON response.
- Methods: `POST` only. A `GET` to `/mcp` returns `405` with an `Allow: POST` header — and that method check runs before the gate, so an unauthenticated `GET` gets `405`, not `401`.
- Server identity: name `atlas-search`, version `0.1.0`. Supported protocol versions are `2024-11-05`, `2025-03-26`, and `2025-06-18`; an unsupported version in `initialize` falls back to `2025-03-26`.
- Tools: exactly one, `search`. Cache purge and cache inspect are deliberately not MCP tools — purge is a write, and inspect is an operator and audit surface. Both stay on the gated REST routes.

The Worker itself holds no corpus credentials. On a cache miss it forwards the search to the laptop service through its tunnel hostname, presenting its own service identity. If that service is unreachable the Worker returns `502` and caches nothing beyond a possibly stored embedding.

## Step 1 — Present the gate token

Every gated Worker route — `/mcp`, `/v1/search`, `/v1/cache/inspect`, and `/v1/cache/purge` — checks the same shared secret. Its name on the Worker side is `GATE_TOKEN`. No token value appears in this book, in the mcpx configuration, or in the Nix store.

Present it on every request as an exact bearer header:

```
Authorization: Bearer <GATE_TOKEN>
```

The prefix check is the exact, case-sensitive string `Bearer ` followed by a space. Anything else is treated as no token presented. The comparison is constant-time by construction: both the presented token and the configured token are SHA-256 digested and the two 32-byte digests are XOR-folded; they match only when the fold is zero. A missing token, an unset `GATE_TOKEN` on the Worker, or any mismatch returns `401` with code `unauthorized` and message `missing or invalid gate token`.

Where a client keeps the token depends on the client:

- A direct HTTP client keeps it in its own secret store or environment, never in a checked-in config file, a prompt, or a tool argument.
- The known production client, the laptop's mcpx daemon, keeps it in the daemon's environment. The mcpx configuration contains only an environment expansion (next step); the daemon expands it at runtime from the environment supplied through its environment file, so the value never lands in the rendered configuration or the Nix store. On the client side that expansion is named `ATLAS_SEARCH_GATE_TOKEN` in the mcpx settings — it is the client-side name for the same gate token, not a second credential.

There are no per-route, per-caller, or per-agent tokens on this endpoint. A valid gate check yields a boolean and nothing else, so every MCP caller is the same identity to the Worker (see Step 5 for what that identity can see).

## Step 2 — Register the server in mcpx as the remote `atlas` server

mcpx is the MCP aggregation daemon on the laptop: agents talk to mcpx, and mcpx talks to each registered server. The Atlas entry is a remote (URL) server, not a local process. Its shape in `home/common/mcpx-settings.nix` — a pure data file that the mcpx Nix module renders to the daemon's configuration — is:

```nix
"atlas" = {
  url = "https://atlas-search.developing-today.workers.dev/mcp";
  auth = {
    type = "bearer";
    token = "\${ATLAS_SEARCH_GATE_TOKEN}"; # shape only: env expansion
  };
};
```

Read the shape, not the spelling of any value:

- The server name is `atlas`. Agents address the tool through that namespace.
- `url` is the Worker `/mcp` endpoint above. There is no local command, working directory, or environment block for the server process, because there is no local process.
- `auth.type` is `bearer`, and `auth.token` is an expansion reference, not a literal. Do not replace the expansion with a pasted value, and do not edit the rendered configuration file by hand — it is a link to the file rendered from the Nix settings, so a hand edit does not reach the daemon durably.

After changing the settings, apply them the normal home-configuration way for the laptop and restart or reload mcpx as that configuration requires, then confirm the daemon started with the expansion present in its environment. If the expansion is missing, expect every Atlas call to fail at the gate with `401`; that is the gate working, not a search failure.

For a client that is not mcpx, register the same three facts in that client's own remote-server format: the `/mcp` URL, bearer auth, and the token supplied from the client's secret or environment store.

## Step 3 — Make a first `tools/list` call

Before searching, ask the server what it offers. Send an `initialize` first if your client does not do it for you, then `tools/list`:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "initialize",
  "params": { "protocolVersion": "2025-03-26" } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }
```

Both go as `POST` bodies to the `/mcp` URL, with the gate header from Step 1. A message with no `id` property at all is a notification: it is never executed and never answered, and a request made only of notifications returns `202` with an empty body. That rule includes `tools/call` — a search sent as a notification does nothing.

A correct `tools/list` result contains exactly one tool:

```json
{ "tools": [ {
  "name": "search",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "The query text." },
      "k": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10,
             "description": "Maximum number of results to return." }
    },
    "required": ["query"]
  },
  "annotations": { "readOnlyHint": true, "destructiveHint": false,
                   "idempotentHint": true, "openWorldHint": false }
} ] }
```

The description returned with the tool states the operating facts in one place: it searches the unified Atlas index, runs under the hosted surface's pinned disclosure context (`hosted-per-user`) and sensitivity ceiling (`personal`), returns the search envelope with per-hit provenance and the effective ceiling the service enforced, and serves a repeat query from cache. If `tools/list` returns more than this one tool, you are not talking to the Atlas Worker described here.

Common failures at this step: no gate header or a malformed bearer prefix gives `401`; a non-`POST` request gives `405`; a body over 64 KiB gives a JSON-RPC `-32600` error; an unknown method gives `-32601`.

## Step 4 — Make a first `tools/call`

Call the `search` tool with a toy query of your own — pick words you expect to occur in your own recent work, not text copied from anyone else's corpus:

```json
{ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
  "params": { "name": "search",
              "arguments": { "query": "queue broker lease", "k": 5 } } }
```

Behaviour to expect:

- `query` is required and must be a string. It is normalized before use (Unicode NFC, trimmed, every whitespace run collapsed to one space; case is preserved). A query that is empty after normalization fails with JSON-RPC `-32602` and message `arguments.query must not be empty`; a non-string query and an unknown tool name also fail with `-32602`.
- `k` is clamped, not rejected: only a JSON number is honoured, it is floored, and the result is clamped to 1–50. An absent, non-numeric, or non-finite `k` becomes the default, 10.
- On success the JSON-RPC result has `isError: false`, a `structuredContent` field carrying the search envelope verbatim, and a `content` array whose single text item is that same envelope as a JSON string. The envelope carries `cache` (`miss` on the first call, `hit` on an identical repeat), the cache `key`, the normalized `query`, the `result` list with per-hit provenance, and `meta` with the embedding model `Snowflake/snowflake-arctic-embed-m-v2.0`, `dims: 768`, timings, and hit count.
- On a search failure the HTTP status stays `200`; the failure is carried only by `isError: true` and the embedded error payload, with no `structuredContent`. Do not treat HTTP `200` alone as a successful search.

Run the identical call a second time. The second result should report `cache: "hit"` and return the same result list. That is not a coincidence of this tool: an MCP search and a REST `POST /v1/search` for the same normalized query and same clamped `k` share one Durable Object cache entry, because both call the same execution path.

## Step 5 — Understand the pinned `personal` ceiling

The ceiling is the highest sensitivity a search may return. The ladder, lowest to highest, is canonically `public`, `internal`,
`private`, `sensitive` — spelled in code `shareable`, `personal`,
`local-only`, `secret-bearing`. What this surface may ever search under is pinned in the Worker's source as a compile-time constant:

- Ceiling: `personal`.
- Disclosure context (the destination-surface half of the model, which intersects with the ceiling and can only narrow it): `hosted-per-user`, whose ceiling is also `personal`.

No environment variable, request field, or header can change either value. Neither the REST body nor the MCP tool schema has a field for a ceiling or a context, so an agent cannot ask for more and cannot be granted more through this door. In practical terms:

- The agent can see `shareable` and `personal` material, with sensitivity and disclosure filters applied inside the branch queries, before any ranking — not filtered out of a finished ranking afterwards.
- It cannot see `local-only` or `secret-bearing` material at all through this surface: such chunks are not returned, counted, or down-ranked into visibility. Content marked `excluded_personal_life` is ineligible in every context.
- The pin is enforced twice. The Worker states `personal` and `hosted-per-user` on every call to the laptop service, whose `cf-worker` client cap is also `personal`; then, before anything is cached, the Worker verifies the service reported `effective_ceiling` as `personal` and refuses with `502` if it did not. A result computed above `personal` can therefore never enter the cache. The ceiling and context are also fields of the cache key itself.
- All MCP callers share this one hosted identity. Per-agent identity stamping, per-agent ceilings, corpus scoping, and a query audit log are later slices of #8625 and are deliberately not improvised in the current endpoint. If an agent needs a higher ceiling, the answer is a different, named surface with its own cap — for example a local client identity on the laptop service — not a flag on this one. See [Sensitivity tiers and client ceilings](../concepts/sensitivity.md).

## Step 6 — Verify with `/v1/cache/inspect`

Verification uses the operator route, not an MCP tool. After the two identical searches in Step 4, inspect that query:

```
GET https://atlas-search.developing-today.workers.dev/v1/cache/inspect?query=queue%20broker%20lease&k=5
Authorization: Bearer <GATE_TOKEN>
```

The `query` parameter is required; `k` is optional, defaults to 10, and is clamped the same way as everywhere else, so pass the same `k` you searched with — a different `k` is a different cache entry. The route is gated: without a valid token it returns `401`. A missing `query` parameter, or one that is empty after normalization, returns `400`.

A healthy verification after one miss and one hit returns metadata like this (values are the shape, with a toy query):

```json
{ "key": "<64-hex-character cache key>", "query": "queue broker lease",
  "exists": true, "k": 5,
  "has_embedding": true, "embedding_dims": 768, "embedding_valid": true,
  "has_result": true, "result_bytes": 1234,
  "hit_count": 1, "embed_ms": 0, "search_ms": 0 }
```

Check four things:

1. `exists: true` and `has_result: true` — the fill completed and a result is stored.
2. `has_embedding: true`, `embedding_dims: 768`, and `embedding_valid: true` — the stored embedding is in the pinned Snowflake space (768 finite components). A wrong-space embedding is refused before storage, so a stored invalid embedding should never appear here.
3. `hit_count` matches the number of repeat calls you made — `1` after the second identical search above. It counts hits served from this entry, and it is the proof the repeat did not re-run the laptop fan-out.
4. The `key` is the same key the search envelope reported. Inspect returns metadata only: it never returns the result body or the embedding vector, only their presence, dimensions, validity, and byte size.

If `exists` is `false`, the query and `k` you inspected do not name the entry you searched — most often because the raw query normalized differently than you expected, or because `k` differed. Search again, copy the `key` and normalized `query` from that envelope, and inspect those exact values. If inspect shows an embedding but no result, the last fill stored the embedding and then failed at the search step; the next identical search will retry the search without re-embedding.

You are done when `tools/list` shows the single `search` tool, a `tools/call` returns `isError: false` with a `personal` effective ceiling in its envelope, a repeat returns a cache hit, and inspect confirms the entry as above. For how this surface fits the other consumers, see [Consumers: fleet, agents, memory and ID](../concepts/consumers.md) and [The service and the Worker](../concepts/service-and-worker.md).
