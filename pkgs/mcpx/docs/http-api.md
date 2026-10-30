# The /v1 HTTP API

`/v1` is the daemon's own API — the one surface everything else
sits on. The CLI talks to the daemon over it, the plugin's client
is an HTTP client for it, and the MCP tools are generated from the
same declarations it is. This page is the prose around the
specification: how to reach the API, the few conventions a caller
has to know, and how to get a client for it in your language. The
specification itself is `GET /v1/openapi.json`, and it is generated
— see below for why that sentence does real work.

<a id="v1"></a>

## /v1

### One table, one specification

Every `/v1` operation is declared once, as an `Op` in the table in
`internal/api/ops*.go` — 81 operations at the time of writing; the
generated table in [parity.md](./parity.md) is the live count and
the live list. From that declaration come the route, the
`mcpx_<op>` MCP tool, a CLI command and the operation's entry in
the OpenAPI document. `internal/api/openapi.go` builds the document
from the table, so `GET /v1/openapi.json` describes the daemon that
serves it, including one path per tool in the live catalogue.

Because the document is generated from declarations compiled into
the binary, it can be produced with **no daemon running**:
`mcpx openapi` prints it. The two documents overlap but neither is
a superset of the other. Every path they share is identical, byte
for byte, and `mcpx openapi` adds the MCP endpoint, a REST path
per mcpx tool (`POST /v1/tools/{tool}` is itself an operation in
the table) and the settings index. The daemon's document instead
adds one concrete path per tool in the **live catalogue**
(`POST /v1/call/<namespace>/<tool>`, with that tool's input
schema, generated per request by `handleOpenAPI` in
`internal/daemon/ops.go`), so a server that appears after startup
is described without a restart. A third form, `GET
/v1/openapi.json?static=1`, returns the daemon document *without*
the live catalogue merged in -- only the ops-table paths -- so a
docs build can publish the API surface without publishing the
builder's configured servers (#1883 item 4); the offline document reads no
configured servers and covers upstream tools only through the
declared `/v1/call/{server}/{tool}` template. The two documents
that used to share this name — one of them describing a server
that had been removed — are recorded in the header of
`internal/cli/openapi.go`, so the confusion cannot quietly return.

> **Which OpenAPI document do you want?** `GET /v1/openapi.json`
> describes **the daemon as it is running on this machine**: its
> operations, its routes, its settings, and the live catalogue's
> per-tool paths. The `/v1` half of `mcpx openapi` describes the
> same daemon surface as compiled into the binary, without the
> live catalogue. `mcpx schema
--format openapi` describes **the tools** of your configured MCP
> servers, as operations for a client generator. Feed the wrong one
> to a generator and you get a working client for the wrong thing.

The rendered reference page and the protocol revision matrix are
generated content, and generated content is never hand-copied into
a prose page — it would be wrong within two releases. The
generation that embeds them in this page set is still in progress;
until it lands, the interim sources are the documents themselves:
the operation table rendered in [parity.md](./parity.md) (itself
generated, and held current by a test) and the revision matrix in
[protocol.md](./protocol.md) §1, which defers to
`GET /v1/protocol`. Both are named here, on the page, so a reader
always knows which copy they are trusting.

### Reaching the API

The daemon listens in two places at once:

- **The unix socket** in its state directory (`paths.state`). This
  is the primary listener; the socket's file permissions are the
  access control.

  ```sh
  curl --unix-socket <sock> http://mcpx/v1/health
  ```

  The host name is a placeholder — over a unix socket it names
  nothing; the socket is the address. The OpenAPI document's first
  `servers` entry (`http://mcpx`) is written for exactly this call.

- **Loopback TCP**, on `daemon.port`. The default port is 0, which
  means the daemon chooses one at start-up; `GET /v1/health`
  reports the port this daemon chose, and the daemon writes its
  socket, endpoint and port into its info files in the state
  directory for clients to discover. A named endpoint
  (`MCPX_ENDPOINT`, as `unix:///path` or `http://host:port`) is how
  a program pins one daemon instead of discovering whichever is
  running.

Every endpoint is unauthenticated. That is a deliberate local-API
posture, not an oversight: binding the TCP listener wider than
loopback is a deliberate act, and the responsibility of whatever
does it, because whatever can route to the port can run tools as
you. The same listeners mount the [MCP surface](./mcp.md#mcp) at
`/mcp` — one listener, one access-control decision.

### Call-scoped settings: the `X-Mcpx-Settings` header

A daemon is started once and serves every later command, so a flag
on one command that governs something the daemon does would be
lost — accepted, validated, and ignored — if the client did not
send the effective value with the request. That is what the
`X-Mcpx-Settings` header is: a JSON object of setting path to
value, carried on one request:

```sh
curl --unix-socket <sock> 'http://mcpx/v1/search?q=screenshot' \
  -H 'X-Mcpx-Settings: {"search.limit": "5"}'
```

Only **call-scoped** settings are honoured; a path the registry
declares at any other scope is ignored, because a header that could
reconfigure a shared daemon for everybody is not a per-call
preference. A malformed header is also ignored rather than refused
— the client's bug should not cost it the call itself. The setting
names and scopes are the registry's, generated into
[configuration.md](./configuration.md).

### The event stream: `GET /v1/events`

`GET /v1/events` is a server-sent events stream over the daemon's
event bus — elicitations waiting for an answer, server state
changes, resource updates, progress and log messages. Query
parameters select what to hear: `kinds=` (prefixes; empty means
everything), `session=`, `server=`, and `uri=` for resource updates
on named resources (naming one subscribes to it upstream for as
long as the stream is open).

Reconnects resume. `?since=N` replays everything after sequence N,
and the `Last-Event-ID` header a browser's `EventSource` sends on
reconnect is honoured the same way; if the retained history no
longer reaches back that far, the stream says so in-band with a
`gap` event whose payload is `{"since": N, "latest": M}` — `since`
is the cursor the subscriber asked to resume from and `latest` is
the newest sequence the bus has published, **not** the oldest
sequence still retained — so a subscriber knows it missed
everything between its cursor and the retained history and can
resynchronise instead of trusting a stream with a hole in it. Over
MCP, the same stream is what `subscriptions/listen` carries — see
[mcp.md](./mcp.md#mcp).

### Admin operations

An operation whose declaration carries `Admin` changes the daemon
rather than reading from it — stopping it, restarting a server,
setting configuration. In the OpenAPI document these are marked
`x-mcpx-admin`, which is how a scoped token, when one exists, would
know what to grant separately. Today they are **offered rather
than hidden**, MCP annotations (`readOnlyHint`, `destructiveHint`)
included: a caller that can reach the socket can already do these
things, so withholding them would buy no safety. 20 of the 81
operations are admin at the time of writing; [parity.md](./parity.md)
lists which.

### Request body limits

Bodies are bounded, per surface, by settings: `http.bodyLimit`
(default 1 MiB) for ordinary requests, `http.callBodyLimit`
(default 64 MiB) for tool-call payloads, which legitimately carry
large arguments, and `http.controlBodyLimit` (default 64 KiB) for
the control endpoints. A body over its limit is refused by the
decoder rather than truncated into a half-parsed request.

<a id="v1-sdk"></a>

## A /v1 SDK

There is no published `/v1` SDK, in any language. What follows is
the recommended way to get one anyway, and the one client that does
exist.

### Recommended: generate one from the specification

The specification is generated from the ops table, so a client
generated from it cannot drift from the daemon the way a
hand-written SDK would — a hand-written SDK is a second copy of
the table, and keeping it correct is exactly the failure the table
exists to prevent. Generate with the standard tool for your
language:

| language    | tool                                                       |
| ----------- | ---------------------------------------------------------- |
| TypeScript  | `openapi-typescript` (types) with `openapi-fetch` (client) |
| Go          | `oapi-codegen`                                             |
| Python      | `openapi-python-client`                                    |
| most others | `openapi-generator`                                        |

The flow is the same everywhere:

1. Take the spec from the daemon you will talk to:
   `curl --unix-socket <sock> http://mcpx/v1/openapi.json -o openapi.json`
   (`mcpx openapi` works offline and every path it shares with the
   daemon document is the same bytes, but it has no live catalogue:
   a generated client that needs the concrete
   `/v1/call/<namespace>/<tool>` paths must be generated from the
   daemon's own document, not the offline one).
2. Run your generator over `openapi.json`.
3. Point the generated client at the daemon — which is the one
   mcpx-specific step, next.

**The unix-socket caveat.** Generated clients assume a TCP base
URL. The daemon's primary listener is a unix socket, so pick one:
run the daemon on a known loopback port (`daemon.port`, or start
it with `--port`) and use `http://127.0.0.1:<port>` as the base URL;
pin `MCPX_ENDPOINT` the same way for programs that read it; or give
the generated client a fetch/transport that can dial a unix socket
— the standard clients all accept a custom fetch or HTTP transport,
which is where a socket dialer belongs. What does not work is
handing a generated client the socket path as a URL and hoping.

### The client that exists: `DaemonClient`

For harness plugins and Bun/TypeScript programs there is the
plugin's `DaemonClient` (`plugin/opencode/mcpx/daemon.ts`): a
connection-reusing client that knows the discovery ladder (a named
endpoint first, then the daemon info files, then health checks),
speaks to the socket through Bun's `fetch` `unix` option, and
reaches every operation through its generated `ops` methods
(`plugin/opencode/mcpx/ops.gen.ts`, generated from the same table),
with named convenience methods beside them for the common calls.
It is tested against a real daemon. It is also **not published as
a package** — using it today means taking it from the plugin — and
it is Bun-first, which is why it is the harness answer rather than
the general one. The general answer is generation, above.

### What is not promised

No SDK-generation check runs in CI yet: no job generates a client
from the current spec and compiles it, so the generation flow above
is documented and follows the spec's own generation, but it is not
yet held by a test the way [parity.md](./parity.md) is. That check
belongs with the docs-generation work this page set leans on, and
it is recorded as the named remainder on the parent issue rather
than claimed here.
