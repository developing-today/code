# The Atlas MCP server

Source: [worker/src/mcp.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/mcp.ts), [worker/src/index.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/index.ts) (the gate and the method branch), [home/common/mcpx-settings.nix](https://github.com/developing-today/code/blob/main/home/common/mcpx-settings.nix) (the client wiring) — provenance only; this chapter is complete without following it.

The Atlas MCP server is the Model Context Protocol surface of the Cloudflare Worker described in [The Worker HTTP API](worker-api.md): a stateless Streamable HTTP MCP server at `POST /mcp` on the Worker, exposing exactly **one tool, `search`**, over the same execution path, the same pinned `hosted-per-user` disclosure context and the same Durable Object cache as the Worker's REST search, but under a **per-agent ceiling** (`AGENT_POLICY`, defaulting to the surface pin `personal`). An MCP search and a REST search for the same normalized query and `k` share one cache entry only when the ceilings match; the cache key carries the ceiling so a result computed above one ceiling is never served under another.

```http
POST /mcp
Authorization: Bearer <gate token>
Content-Type: application/json

{ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }
```

## Transport shape

| Property            | Behaviour                                                                                                                                                                                                                                                                                                                                          |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Session handling    | **None.** No session id is issued, required or read; there is no `Mcp-Session-Id` header handling in the code.                                                                                                                                                                                                                                     |
| Message framing     | Each HTTP request is one JSON-RPC message, or a **batch**: a JSON array of messages. The response is a plain JSON body (`Content-Type: application/json`); batch responses preserve request order and contain one response per non-notification item.                                                                                              |
| Streaming           | None. There is no SSE or streaming response mode, and no GET endpoint: `GET /mcp` is 405 (see [The Worker HTTP API](worker-api.md)).                                                                                                                                                                                                               |
| Notifications       | A message with **no `id` property at all** is a notification: it is never executed and never answered — this explicitly includes a `tools/call` sent as a notification. A single notification, or a batch of only notifications, gets HTTP **202** with an empty body. In a mixed batch, notifications are simply omitted from the response array. |
| The `id: null` edge | A message whose `id` property is present but null is a request, not a notification — the test is whether the property exists, not whether it is null. It is executed and answered with `id: null`.                                                                                                                                                 |
| Body limit          | The same 64 KiB cap as every Worker route (see the error table below for its JSON-RPC shape).                                                                                                                                                                                                                                                      |
| Server identity     | `name: "atlas-search"`, `version: "0.1.0"`.                                                                                                                                                                                                                                                                                                        |
| Protocol versions   | Supported (`SUPPORTED_VERSIONS`): `2024-11-05`, `2025-03-26`, `2025-06-18`. Fallback when negotiation cannot echo the client's version: `2025-03-26`. The server identity (`SERVER_INFO`) is name `atlas-search`, version `0.1.0`.                                                                                                                 |

## The gate and the pinned identity — read this plainly

The endpoint checks credentials before dispatch. If the optional `AGENT_TOKENS` Worker secret is configured, it must be a non-empty JSON object mapping stable ids (letters, digits, `.`, `_`, `:`, `-`) to distinct bearer tokens at least 16 characters long. The matching token determines `agent_id`; request headers and MCP parameters cannot override it. In this mode `GATE_TOKEN` is not accepted on `/mcp`. Without `AGENT_TOKENS`, the legacy `GATE_TOKEN` resolves to `hosted-per-user`. Method is checked before authentication, so `GET /mcp` gets 405; missing or invalid POST credentials return HTTP 401 before JSON-RPC dispatch.

Each valid MCP search emits a structured Worker log event (`event: atlas_mcp_query`) containing the authenticated `agent_id`, the enforced per-agent `ceiling`, SHA-256 of the normalized query (not its raw text), k, status, cache result, and timestamp. The backend request states that ceiling and the shared disclosure context `hosted-per-user`; the worker verifies the service-reported `effective_ceiling` equals the requested ceiling before the result may be cached, and refuses the answer otherwise. Corpus scoping is the named #8625 follow-on (no corpus dimension exists in the branch queries yet). The four caller surfaces are not configured with distinct credentials by this code change alone.

## Methods

Exactly four methods are handled; everything else gets JSON-RPC error `-32601`.

| Method       | Behaviour and result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `initialize` | Negotiates the protocol version: echoes `params.protocolVersion` if it is a string in the supported list, otherwise returns `2025-03-26`. Result: `{ protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "atlas-search", version: "0.1.0" }, instructions: "Atlas search: one tool, \`search\`, over Drew's unified Atlas index under your agent ceiling (at most personal). Results are cached per query and ceiling; a repeat query is served from cache." }`. No client identity or capabilities from `params` are stored or used. |
| `ping`       | Result `{}`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `tools/list` | Result `{ tools: [ &lt;the search tool&gt; ] }` — exactly one tool, below.                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `tools/call` | Executes the `search` tool, below.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |

`params` is used only when it is a non-null object, and otherwise treated as `{}`; the same rule applies to a `tools/call` `arguments` value.

## The one tool: `search`

The base definition returned by `tools/list`:

```json
{
  "name": "search",
  "description": "Search the unified Atlas index — all of Drew's Atlas databases — for a natural-language query. Runs under your agent ceiling (at most personal) in the hosted-per-user disclosure context. Returns the search envelope: cache status, the result list with per-hit provenance, and the effective ceiling the service enforced.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "The query text." },
      "k": {
        "type": "integer",
        "minimum": 1,
        "maximum": 50,
        "default": 10,
        "description": "Maximum number of results to return."
      }
    },
    "required": ["query"]
  },
  "annotations": {
    "readOnlyHint": true,
    "destructiveHint": false,
    "idempotentHint": true,
    "openWorldHint": false
  }
}
```

After negotiating MCP `2025-06-18`, send `MCP-Protocol-Version: 2025-06-18` with `tools/list`. The search definition then includes an `outputSchema` for successful structured results. The schema requires `cache` (either `hit` or `miss`), `key` and `query` (strings), and `result` and `meta` (objects with service-defined contents). A tool error uses `isError: true` and has no structured payload. Requests carrying an earlier revision or omitting the header receive the base definition. The Worker reads the version from each request because the transport is stateless.

Input rules: `query` is required and must be a string, non-empty after normalization (NFC, trim, collapse whitespace runs — the same normalization as the REST surface). `k` is an integer in the schema, 1–50, default 10; at execution it passes through the Worker's one `clampK`, so a non-number becomes 10, a number is floored, and the result is clamped to [1, 50] — see [The Worker HTTP API](worker-api.md).

Execution validates the tool name, validates and normalizes the query, then calls the same search execution path as `POST /v1/search` under the caller's policy ceiling. Output, on success:

```json
{
  "content": [{ "type": "text", "text": "<the search envelope, as a JSON string>" }],
  "structuredContent": { "cache": "miss", "key": "<cache key>", "query": "<normalized query>", "result": {}, "meta": {} },
  "isError": false
}
```

The text content and `structuredContent` carry the identical payload — parsing the text yields the structured object. The envelope itself is the REST search envelope (`cache`, `key`, `query`, `result`, `meta`) specified in [The Worker HTTP API](worker-api.md).

On search failure (the underlying search returns a non-200 status, in practice the 502 `query_failed` passthrough), the result is `{ content: [{ type: "text", text: "&lt;the error payload, as a JSON string&gt;" }], isError: true }` — **no** `structuredContent`, and the HTTP status stays 200; the failure is carried only by `isError` and the embedded payload.

Deliberately **not** MCP tools: cache purge (a write) and cache inspect (an operator and audit surface). Both exist only as gated REST routes.

## JSON-RPC error codes

Framing and dispatch errors are returned as `{ "jsonrpc": "2.0", "id": &lt;id or null&gt;, "error": { "code": …, "message": … } }`.

| Condition                                                       | HTTP                    | Code     | Message                                                                          |
| --------------------------------------------------------------- | ----------------------- | -------- | -------------------------------------------------------------------------------- |
| Body over 64 KiB                                                | 413                     | `-32600` | `body is larger than the 64 KiB limit` (id null)                                 |
| Body not parseable JSON                                         | 400                     | `-32700` | `parse error: body must be JSON` (id null)                                       |
| Body is a JSON array of length 0                                | 400                     | `-32600` | `invalid request: empty batch`                                                   |
| Batch item not an object                                        | 200, in the batch array | `-32600` | `invalid request: batch items must be objects` (that item's response id is null) |
| Body neither object nor array (JSON `null`, a string, a number) | 400                     | `-32600` | `invalid request: body must be a JSON object or batch`                           |
| Message `method` not a string                                   | 200                     | `-32600` | `invalid request: method must be a string`                                       |
| Unknown method                                                  | 200                     | `-32601` | `method not found: &lt;method&gt;`                                               |
| `tools/call` with an unknown tool name                          | 200                     | `-32602` | `unknown tool: &lt;name&gt;`                                                     |
| `tools/call` `arguments.query` not a string                     | 200                     | `-32602` | `arguments.query must be a string`                                               |
| `tools/call` `arguments.query` empty after normalization        | 200                     | `-32602` | `arguments.query must not be empty`                                              |

In code terms: `-32700` is parse error, `-32600` is invalid request, `-32601` is method not found, and `-32602` is invalid params — the last covering both the unknown-tool and bad-`query` cases on `tools/call`.

## How mcpx wires it: the remote `atlas` server

The known production client is the laptop's mcpx daemon, which connects to this endpoint as its remote `atlas` server. The wiring is data, not code: an entry under `"mcpServers"` in `home/common/mcpx-settings.nix` — a pure data file rendered by the mcpx Nix module, to which `~/.config/mcpx/config.json` is now a link, so editing a JSON file in a checkout no longer reaches the daemon.

```nix
"atlas" = {
  url = "https://atlas-search.developing-today.workers.dev/mcp";
  auth = {
    type = "bearer";
    token = "\${ATLAS_SEARCH_GATE_TOKEN}";  # environment expansion, expanded by the daemon at runtime
  };
};
```

The shape is: remote URL plus bearer auth whose token is expanded from the daemon's environment at runtime (the daemon's environment file is wired through `services.mcpx.environmentFile` in the home configuration). The token value never lands in the settings file or in the Nix store; this book names the expansion variable, `ATLAS_SEARCH_GATE_TOKEN`, and nothing more. The token it expands to is the Worker's shared gate token — the same credential the REST routes use, because there is only one gate.

## Worked example

A minimal session — initialize, list, call — as three separate POSTs (or one batch):

```json
{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "search", "arguments": { "query": "queue broker lease expiry", "k": 5 } } }
```

The first answers with `protocolVersion: "2025-06-18"` echoed, the server identity and the instructions string. The second answers with `isError: false` and the search envelope in both text and structured form — `cache: "miss"` the first time this query runs at `k` 5 under the caller's ceiling, `cache: "hit"` thereafter for the same ceiling, through whichever Worker surface. A batch of `[ping, a notification, tools/list]` answers 200 with exactly two responses, for the ping and the list, in request order; the notification is silently dropped from the array.

## Exit codes

None: this is a hosted HTTP protocol surface. Client-side failures surface as the HTTP and JSON-RPC shapes tabulated above.
