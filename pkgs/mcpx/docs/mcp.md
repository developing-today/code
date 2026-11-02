# MCP

mcpx speaks the Model Context Protocol in both directions, and in
both eras of it. As a **client** it connects to every MCP server in
its configuration, so their tools are reachable without their
schemas ever entering a model's context. As a **server** it offers
itself — a small gateway surface over all of those servers — to any
MCP host. The protocol reference is [protocol.md](./protocol.md);
this page is the map of the surface a host or a client connects to.

<a id="mcp"></a>

## MCP

### mcpx as an MCP server

A host reaches mcpx's MCP surface two ways:

- **Streamable HTTP**, at the daemon's `/mcp`. The daemon mounts
  `/mcp` on the same listeners as its [`/v1` API](./http-api.md#v1):
  the unix socket and the loopback TCP port. There is no separate
  MCP server process and no `serve --transport http` — that command
  was removed, and asking for it fails with the address to use
  instead ([protocol.md](./protocol.md) §5 tells the whole story).
  To put MCP on a network, move the daemon's listener:
  `mcpx daemon --port 8899 --address 0.0.0.0`, then connect to
  `http://host:8899/mcp`. The warning that applies to `/v1` applies
  here unchanged: the surface is unauthenticated, so whatever can
  route to the port can run tools as you.
- **stdio**, via `mcpx serve`. An MCP host starts its servers by
  spawning a process and talking down the pipe, so something has to
  be spawnable; `serve` is that something, and it is a thin shim —
  every tool it lists is answered by the daemon, which it starts if
  it is not running. A host configuration looks like the one every
  host already knows:

  ```json
  { "mcpServers": { "mcpx": { "command": "mcpx", "args": ["serve"] } } }
  ```

The surface is small on purpose. Exposing every upstream tool over
MCP would rebuild the problem mcpx was built to solve — hundreds of
schemas in the model's context — with extra steps
([story.md](./story.md), "mcpx speaks MCP"). What a host gets by
default (`serve.mode` = `code-mode`) is the gateway: the named
tools (`mcpx_exec` to run code next to the servers, `mcpx_call` for
one direct call, `mcpx_search` / `mcpx_types` / `mcpx_catalog` for
discovery) plus a generated `mcpx_<op>` tool for every operation in
the `/v1` table — every one of them a way to _reach_ the upstreams
through mcpx, never the upstreams' own schemas pasted into the
list. Prompts are offered as `<namespace>_<prompt>` and resources
as `mcpx://<namespace>/<uri>`.
Two more modes widen it: `reactive` keeps a bounded working set of
upstream tools, adapting as calls name new ones, and a
**pass-through** server (`mcp.passthrough`, `--passthrough <server>`
on `mcpx serve` or `mcpx daemon`) is offered as itself — its tools,
prompts and resources under their own names, its results verbatim.

A client can also ask for the raw surface per connection: a request
to `/mcp?codemode=false` gets every upstream tool, flat, under a
namespaced name (`<namespace>_<tool>`), and the gateway's own tools
are not on that list — the raw surface replaces the code-mode
surface, it does not add to it. Calls to a raw name go to its
upstream under the upstream's own name. This is the switch opencode
v2 appends to a remote server's URL when the server is not
configured `codemode: false`, so a v2 host's own code mode can
orchestrate mcpx's pooled servers directly. Any other value, and
the parameter's absence, change nothing; the switch is read from
each request's URL, so one connection can be answered raw and the
next through the gateway.

Questions flow backwards over the same connection. When an upstream
tool asks something mid-call — an elicitation, a sampling request —
mcpx puts the question to the client that made the call when that
client said it could answer, and to its broker otherwise
([elicitation.md](./elicitation.md)). And
`subscriptions/listen`, the modern subscription stream, is accepted
from a client of **any** era, each notification tagged with the
subscription id that asked for it.

Which methods are answered, per revision, and which result shapes
are downgraded for older clients, is [protocol.md](./protocol.md)
§2 — generated claims are not repeated here because that document
is where they are held to the code.

### mcpx as an MCP client

Upstream, mcpx is a dual-era client of every configured server. It
probes the modern era first (`server/discover`), falls back to the
legacy handshake, and caches the answer per server configuration,
so the probe costs one round trip rather than one per start. To a
legacy server it offers `2025-11-25` and accepts anything from
`2024-11-05` up; a per-server override exists (`upstream.protocol`)
for a server that needs to be met on specific terms. Transports are
stdio and Streamable HTTP, with the `2024-11-05` HTTP+SSE transport
as the last fallback of the HTTP probe. The per-revision client
rules are [protocol.md](./protocol.md) §4.

The server-initiated requests mcpx answers for its upstreams:

| request                  | how mcpx answers                                                                             |
| ------------------------ | -------------------------------------------------------------------------------------------- |
| `ping`                   | `{}`                                                                                         |
| `roots/list`             | the daemon's configured roots                                                                |
| `elicitation/create`     | through the broker — the question is routed to whoever can answer it, a host client included |
| `sampling/createMessage` | through the broker; a decline becomes an error, because sampling has no decline result       |
| anything else            | a `-32601` error — always answered, because silence is indistinguishable from a hung server  |

A modern server asks the same things by answering `input_required`
instead of sending a request; mcpx resolves those through the
identical function, so the two eras cannot answer the same question
differently.

<a id="mcp-sdk"></a>

## The MCP SDK question

"mcp sdk" names three different things. The honest answer is
different for each.

### 1. Which MCP SDK mcpx uses: none

mcpx is built on no MCP SDK. There is no MCP module in
`pkgs/mcpx/go.mod`; the client (`internal/mcpclient`), the server
(`internal/mcpserver`) and the JSON-RPC framing are mcpx's own code
— about one package each, which is the point.
[dependencies.md](./dependencies.md) says so in as many words, and
the dependency list stays at the handful of Go libraries there
because mcpx speaks the protocol itself instead of depending on
somebody else's reading of it.

### 2. Using mcpx from an MCP SDK: any of them

Any official SDK client connects the way it connects to any MCP
server: over Streamable HTTP to the daemon's `/mcp`, or by spawning
`mcpx serve` for stdio. With the official TypeScript SDK:

```ts
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const client = new Client({ name: "my-host", version: "1.0.0" });
await client.connect(new StreamableHTTPClientTransport(new URL("http://127.0.0.1:8899/mcp")));
const tools = await client.listTools();
const status = await client.callTool({ name: "mcpx_status", arguments: {} });
```

(The port is the one the daemon is actually on: with the default
`daemon.port` of 0 the daemon chooses one at start-up, and
`GET /v1/health` reports it — see [http-api.md](./http-api.md#v1).
For stdio, swap the transport for the SDK's
`StdioClientTransport` with `command: "mcpx"`, `args: ["serve"]`.)

### 3. An SDK mcpx provides for writing servers: there isn't one

mcpx publishes no SDK for writing MCP servers or mcpx extensions,
in any language. What exists for building on mcpx is the surfaces
themselves: a server is any program that speaks MCP and is named in
the configuration, and a program that wants to drive mcpx uses the
[`/v1` API and its generation story](./http-api.md#v1-sdk) or the
plugin's client described there. A page that implied a supported
server SDK would be promising a thing nobody maintains.

<a id="mcp-compatibility"></a>

## Compatibility

### The revisions

mcpx serves five revisions, in two eras, and speaks the same five
upstream:

| revision     | era    | mcpx serves                                                                      | mcpx speaks upstream                                           |
| ------------ | ------ | -------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `2024-11-05` | legacy | yes, over stdio and Streamable HTTP; its HTTP+SSE server transport is not hosted | when the server answers with it; HTTP+SSE as the last fallback |
| `2025-03-26` | legacy | yes                                                                              | when the server answers with it                                |
| `2025-06-18` | legacy | yes                                                                              | when the server answers with it                                |
| `2025-11-25` | legacy | yes                                                                              | offered on every `initialize`                                  |
| `2026-07-28` | modern | yes                                                                              | probed first                                                   |

The split that matters: a legacy implementation negotiates once and
can send its peer a request at any time afterwards; a modern one
has no handshake, restates its version on every request, and — as a
server — has no channel on which to send a request of its own.
Modern against legacy fails; only a dual-era implementation
bridges them, and mcpx is dual-era in both directions.

This table is a summary, and it knows it. The matrix lives as data
in `internal/mcpserver/revisions.go` and is served by
`GET /v1/protocol`; [protocol.md](./protocol.md) §1 defers to that
endpoint for the same reason this page does — a matrix in a document
and a matrix in code agree on the day they are written and never
again. A generated rendering of the matrix on this page is part of
the docs-generation work tracked alongside this page set; until it
lands, the endpoint and §1 are the sources, named here so a reader
is never left trusting a copy.

### Accept liberally, send conservatively

mcpx accepts any request a client's revision allows it to make
sense of — `subscriptions/listen` and core `tasks/*` are answered
for legacy clients that predate them — and sends nothing the
client's revision never defined: every result is built in the
newest shape and spelled down once, at the edge, so an older client
is never handed a field it was never told about. The rule stops at
removal: a method a client's revision _removed_ is answered
`-32601`, because the removal is the specification naming a
replacement, and answering it anyway would hide which era the
server is speaking.

### Known gaps

The gaps are written down in [protocol.md](./protocol.md) §6 rather
than copied here, so they cannot drift from the fixes: no OAuth on
`/mcp`, tool inputs and outputs not yet validated against their
JSON Schemas, a `tool` with `taskSupport: "required"` that cannot
be called, and the smaller conformance items listed there. A gap
named in one place is a gap somebody can close.

### Host compatibility

No MCP host has a recorded compatibility result yet, so this page
claims none. Any host that speaks one of the five revisions over
stdio or Streamable HTTP is expected to work — that is what the
conformance work in [protocol.md](./protocol.md) holds mcpx to —
but "expected" is not "tested against your host", and a host will
be named here only when somebody has actually pointed it at mcpx
and written down what happened.
