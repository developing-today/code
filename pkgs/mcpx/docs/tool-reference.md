# MCP tool reference

API-level reference for the tools mcpx exposes to MCP clients. For the
route and CLI mapping of every operation, see [Parity](parity.md). For the
protocol, see [MCP](mcp.md).

This page describes the surface as implemented in
`internal/mcpserver/server.go` (hand-written tools) and
`internal/mcpserver/ops.go` (generated tools). It documents behaviour; it
does not change it.

## Surfaces

| Surface | Tools | Source |
|---------|-------|--------|
| Hand-written | 12 | `internal/mcpserver/server.go`, `Tools()` |
| Generated | one `mcpx_<op>` per operation | `internal/mcpserver/ops.go`, `OpTools` |
| Dynamic | adapter, OpenAPI, recipe, reactive, passthrough, raw | `internal/cli/serve.go` |

Dynamic tools exist only when their upstream or configuration is present.
Their names are derived from the upstream: `<adapter>_<tool>`, OpenAPI
`operationId` (with `_2`-style suffixes for collisions), `recipe_<name>`,
and `<namespace>_<tool>` on the raw surface (`?codemode=false`).

## Hand-written tools

| Tool | Parameters | Purpose |
|------|------------|---------|
| `mcpx_namespaces` | none | List configured namespaces. |
| `mcpx_catalog` | `budget`, `bias` | Catalog of available tools, sized to a budget. |
| `mcpx_types` | `namespaces` | TypeScript signatures for namespaces. |
| `mcpx_search` | `query`, `limit` | Search tools by name or description. |
| `mcpx_call` | `namespace`, `tool`, `arguments` | Call one upstream tool. |
| `mcpx_exec` | `source`, `timeoutSec` | Run a Code Mode script. |
| `mcpx_log` | `since`, `level`, `event`, `limit` | Read daemon log entries. |
| `mcpx_stats` | `dimension` (enum) | Usage and timing statistics. |
| `mcpx_registry` | `query`, `limit` | Search the server registry. |
| `mcpx_status` | none | Daemon status. |
| `mcpx_batch` | `calls`, `stopOnError`, `parallel` | Run several calls in one request. |
| `batch_call` | as `mcpx_batch` | Alias of `mcpx_batch`. The reactive-surface version has a different schema. |

## Generated tools

Each operation in `internal/api/ops.go` `Ops()` produces one
`mcpx_<op>` tool, except:

- operations covered by a hand-written tool (`status`, `namespaces`,
  `search`, `types`, `catalog`, `call`, `call_path`, `batch`, `exec`);
- streaming operations (`events`);
- operations marked `NoTool` (`secrets_set`).

Input schemas come from each operation's `Params` through `InputSchema()`.
Annotations are derived from the operation:

| Annotation | Value |
|------------|-------|
| `readOnlyHint` | `!Mutating` |
| `idempotentHint` | `!Mutating \|\| Idempotent` |
| `destructiveHint` | from the operation |
| `openWorldHint` | `true` |

The per-operation name, method, path and CLI mapping is in
[Parity](parity.md). Because that table is generated from `Ops()`, prefer
it over any hand-maintained list.

## Return values

- Operation tools return pretty-printed JSON text, or raw text when the
  operation sets `Text`.
- An empty upstream body returns `(the daemon answered with an empty body)`.
- Tool failures return a result with `isError: true` and the error text.
  Text the tool produced before failing is kept ahead of the error.

## Errors

| Condition | Result |
|-----------|--------|
| Invalid parameters | JSON-RPC error `-32602`. |
| Upstream JSON-RPC error | Relayed as `UpstreamError`. |
| Unknown name on the raw surface | `no tool named "<name>" on the raw surface (codemode=false)`. |

## Known gaps

- Tool inputs and outputs are not yet validated against their JSON Schemas
  (see [Protocol](protocol.md), known gaps).
- Operation counts in [interfaces](interfaces.md) and [HTTP API](http-api.md)
  predate the current operation set; the live count comes from `Ops()`.
