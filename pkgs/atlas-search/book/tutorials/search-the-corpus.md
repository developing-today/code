# Tutorial: search the corpus three ways

This tutorial runs one toy question through the three client surfaces that search the Atlas corpus (the searchable body of classified, embedded chunks): the standalone `atlas-search` binary, the `fleet search` domain surface, and the hosted MCP server (MCP is the Model Context Protocol; the hosted surface is a Cloudflare Worker in front of the search service). All three end at the same engine and the same result envelope — what differs is routing, identity, and ceiling. The worked reference for the verbs is [atlas-search: search and get-chunk](../reference/atlas-search.md); the wire types underneath are [atlas-proto: the wire contract](../library/atlas-proto.md).

The toy query, used verbatim in every step so the results are comparable:

```
how does a toy sailboat stay upright
```

All output below is schematic, with invented toy hits — field names and shapes are real, chunk ids and text are made up for this tutorial.

## Way 1: standalone `atlas-search`

Standalone is the default and is complete: the query is embedded in-process and the backend (hosted Turso, or a local file) is talked to directly.

1. Run the search, asking for five results in human-readable form first if you want to skim, or as JSON (the default) to work with:

   ```sh
   atlas-search search 'how does a toy sailboat stay upright' --k 5
   ```

2. Read the envelope. Its shape (trimmed):

   ```json
   {
     "query_id": "&lt;uuidv7 minted for this search&gt;",
     "notes": [],
     "results": [{ "chunk_id": 900001, "score": 0.181818, "text": "toy note: a model sailboat stays upright because its ballast sits low…" }]
   }
   ```

   Two things to notice: the envelope carries a `query_id` — every executed search writes a trace record under that id, and a trace-write failure fails the search — and `notes` is where the surface confesses anything non-standard (a fallback, a missing vector store, a rescore it applied).

3. Open the hit you care about with `get-chunk`, asking for one neighbouring chunk (a chunk from the same message, either side of the hit) for context:

   ```sh
   atlas-search get-chunk 900001 --neighbors 1
   ```

   `get-chunk` returns the one chunk with its provenance — session, client, sensitivity — plus the neighbours. It never loads the embedding model (there is no query to embed), it has no via-service path in the job protocol (chunk reads are scoped to an existing job), and a chunk id that does not exist exits 1. Its default disclosure context is `local-owner` — it is the owner's explicit-id tool — where `search` defaults to `fleet-internal`.

## Way 2: the same search via `fleet search`

`fleet search` is the fleet domain surface over the same client: the same flags for the common subset, plus routing.

1. Run the identical query:

   ```sh
   fleet search 'how does a toy sailboat stay upright' --k 5
   ```

2. Compare the output. It is the same envelope from the same engine — same `query_id` discipline, same result shape, the same hit for the same query at the same ceiling. If your seat routed via the service, the result was produced by the loopback service's job protocol ([atlas-proto: the wire contract](../library/atlas-proto.md)); if it ran standalone, the fleet client delegated your arguments to the `atlas-search` artifact.

3. Note the routing rule, because it decides which of those happened: the sandbox seat prefers the service, with a loud standalone fallback (a warning on stderr plus a note in the envelope's `notes`) on any service failure; the laptop seat — the serving host — defaults to standalone. `--via-service` and `--standalone` force a route and are mutually exclusive. Requesting a ceiling above your client identity's cap does not fail: the server clamps `--max-sensitivity` to the identity's ceiling (`drew-cli` may reach `secret-bearing`; `sandbox-scripts` is capped at `personal`).

4. Fetch the same hit through the fleet surface:

   ```sh
   fleet search get-chunk 900001 --neighbors 1
   ```

   This always runs standalone, even on a service-preferring seat: get-chunk is job-scoped in the protocol, so the fleet client delegates the verb verbatim to the `atlas-search` artifact, with a loud note when via-service was in effect. Full detail: [fleet search and fleet serve search](../reference/fleet-search.md).

## Way 3: the same question through the MCP `search` tool

The hosted Worker exposes exactly one MCP tool, `search`, over stateless Streamable HTTP at `POST /mcp`. There are no sessions: each HTTP request is one JSON-RPC message (or a batch), answered with plain JSON. The endpoint is gated by a bearer gate token (named, never valued, here).

1. (Optional, once) Confirm the tool list with a raw JSON-RPC call:

   ```sh
   curl -s -X POST '&lt;worker-base&gt;/mcp' \
     -H 'Authorization: Bearer &lt;gate-token&gt;' \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
   ```

   The result lists exactly one tool, `search`, with required argument `query` and optional `k` (integer, 1–50, default 10). Cache purge and cache inspect are deliberately not MCP tools — one is a write, the other an operator surface.

2. Call the tool with the toy query:

   ```sh
   curl -s -X POST '&lt;worker-base&gt;/mcp' \
     -H 'Authorization: Bearer &lt;gate-token&gt;' \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","id":2,"method":"tools/call",
          "params":{"name":"search",
                    "arguments":{"query":"how does a toy sailboat stay upright","k":5}}}'
   ```

3. Read the result. On success the JSON-RPC result carries the search envelope twice — as JSON text in `content[0].text` and as the object itself in `structuredContent` — with `isError: false`:

   ```json
   {
     "jsonrpc": "2.0",
     "id": 2,
     "result": {
       "content": [{ "type": "text", "text": "{ …the same envelope, as JSON text… }" }],
       "structuredContent": {
         "cache": "miss",
         "key": "&lt;64-hex-char cache key&gt;",
         "query": "how does a toy sailboat stay upright",
         "result": { "…": "the service's search result, passed through" },
         "meta": { "hit_count": 1, "dims": 768, "model": "Snowflake/snowflake-arctic-embed-m-v2.0" }
       },
       "isError": false
     }
   }
   ```

   Run it again and `cache` flips to `hit`: an MCP search and a REST search for the same normalized query and `k` share one cache entry, because both go through the single `executeSearch` path. On failure the HTTP status stays 200 and the failure is carried by `isError: true` with the error payload as the text content — check `isError`, not the HTTP status.

4. Note what you could not ask for. An optional `AGENT_TOKENS` map can authenticate distinct MCP callers for audit, but there is still no argument for a per-agent ceiling, context or corpus scope: the Worker sends the shared `hosted-per-user` context and `personal` ceiling and verifies the service's reported effective ceiling before anything is cached. The toy hit above is returned only because it is visible at `personal`; a hit classified above that ceiling would simply not be in the results. Full surface: [The Atlas MCP server](../reference/mcp.md).

## How the three relate

All three ways run the same hybrid search — full-text and vector branches, filters and ceiling applied inside the branch queries before ranking, fused with the versioned parameter contract — and all three return the same envelope at the core. The differences are only in the wrapper: standalone returns the bare envelope with its `query_id`; the fleet surface adds routing (service-preferred or standalone, decided per seat) and per-identity ceiling clamping; the MCP surface wraps the envelope in a cache-reporting Worker envelope (`cache`, `key`, `meta`) and a JSON-RPC result, at a pinned ceiling you cannot raise.

## Which surface when

- **Standalone `atlas-search`** — you are on a machine with the binary and the model, you want the full flag surface (the query-parameter contract knobs, `--shards`, local backends, traces and feedback verbs), or you are debugging and want no intermediary. Also the only route for `get-chunk` at full fidelity.
- **`fleet search`** — you are working inside fleet tooling and want the seat's routing decided for you: service when it is up, loud standalone fallback when it is not, your identity's ceiling applied by the server. The common search flags only — engine-only knobs are deliberately not on this surface.
- **The MCP `search` tool** — an agent or a hosted client needs search with no local install and no corpus credentials: one tool, one pinned `personal` ceiling, cached results. Accept its limits as features: no ceiling above `personal`, no filters, no `get-chunk` — if you need any of those, drop to one of the other two.
