# Interfaces

Every way into mcpx is one of two kinds. The **abstract interfaces**
are the declarations everything else is generated from: one operation
table, one settings registry, one command list. The **client
interfaces** are the surfaces code actually drives: a generated
TypeScript client, the CLI, the daemon's `/v1` HTTP API, the plugin's
`DaemonClient`, and MCP itself.

This page is the map. The detail lives in the pages it links to: the
[CLI](./cli.md), the [binary](./binary.md), [scripts](./scripts.md),
and the generated [parity table](./parity.md).

<a id="abstract-interfaces"></a>

## Abstract interfaces

mcpx declares each thing it can do **once**, as an operation — an
`Op` in the table spread across `internal/api/ops.go` and its sibling
`ops_*.go` files (81 operations at the time of writing; the generated
table in [parity.md](./parity.md) is the live count). From that one
declaration come:

- the `/v1` route on the daemon,
- the `mcpx_<op>` MCP tool,
- a generated CLI command, for operations that have one,
- a typed plugin method on `DaemonClient.ops`
  (`plugin/opencode/mcpx/ops.gen.ts`, generated the same way),
- the operation's entry in `/v1/openapi.json` and in the document
  `mcpx openapi` prints — one document now, generated from the table;
  the stale hand-written serve-surface document it replaced is
  recorded in the header of `internal/cli/openapi.go`.

```
              internal/api/ops*.go
              one Op per operation
                        |
       +----------------+----------------+
       |                |                |
  /v1 route        mcpx_<op>        generated CLI
  (registered      MCP tool         command, and
   by hand,                         DaemonClient.ops
   held by test)                    method
```

One part of that diagram is deliberately not an arrow: the routes
are **not** generated. They are registered by hand in the daemon,
and a test (`internal/api/parity_test.go`) reads them back out and
checks them against the table. Generation where it is safe, a test
where it is not — the effect is the same: no surface can quietly
drift from the declaration.

The claim the design makes is quoted word for word at the top of
[parity.md](./parity.md):

> Everything the CLI can do can be done from a plugin, can be done
> over `/v1`, can be done from MCP.

What enforces it today is also listed there, because the parity
document is generated from the same declarations and cannot
describe a surface that does not exist. Each direction is held by a
named test — every route has an operation and every operation a
route; every non-streaming operation is reachable as an MCP tool;
every operation is reachable from the CLI; every command the binary
dispatches is declared in `Commands()` and every declared one runs
(`TestEveryDispatchedCommandIsDeclared`); and the plugin's operation
methods are generated and exercised against a real daemon. Where an
operation is privileged — stopping the daemon, for example — it is
offered rather than hidden, and carries MCP annotations
(`readOnlyHint`, `destructiveHint`) for a client that wants to scope
on them; a caller who can reach the socket can already do these
things, so hiding them would buy no safety.

The CLI and the plugin both sit **on top of** `/v1`: the CLI talks
to the daemon over its unix socket, and the plugin's client is an
HTTP client for the same API. There is no second, private interface
for insiders.

Two sibling declarations work the same way, and the interface pages
lean on both:

- **Settings.** Each setting is declared once in the registry with
  its type, default, scope and aliases, and is then reachable from
  the config file, an environment variable and a per-command flag
  without being restated. The inventory is generated into
  [configuration.md](./configuration.md); the environment names are
  generated into [environment.md](./environment.md).
- **Commands.** `Commands()` in `internal/cli/manual.go` declares
  every subcommand once, so help, the man page, shell completions
  and the generated command table in [parity.md](./parity.md)
  cannot disagree about what exists. See
  [cli-commands](./cli.md#cli-commands).

<a id="client-interfaces"></a>

## Client interfaces

Every way code can drive mcpx, with the smallest useful snippet for
each. Pick by who is calling: a script wants the generated client, a
shell wants the CLI, a harness wants `DaemonClient` or MCP, and
anything else wants raw `/v1`.

### The generated TypeScript client — for scripts

`mcpx client` prints the client generated from your configured
servers' schemas; inside `mcpx run` and `mcpx exec` it is already
installed as the globals `tools`, `log` and `emit`, with each
namespace also a bare identifier. `mcpx types` shows the signatures.

```ts
const pages = await tools.chrome_devtools.list_pages({});
```

In a file of your own, import it instead:

```ts
import tools from "./mcpx-client.ts";
```

Discovery is in the same surface: scripts can call `search()` and
`describe()` to find a tool and read its signature at run time
rather than having every schema loaded up front. The contract a
script runs under — stdout is the result, `emit()` streams, a
default export is the entry point — is the
[scripts page](./scripts.md#script).

### The CLI — for people, shells and CI

```sh
mcpx ls
mcpx search screenshot
mcpx call chrome_devtools.take_screenshot '{}'
```

The CLI is a client interface, not just a person's interface:
`--json` makes its output one machine-readable document, and
`mcpx call <ns>.<tool> [json]` makes a single tool call with no
JavaScript runtime involved at all. The whole surface is the
[CLI page](./cli.md#cli).

### `DaemonClient` and `connect()` — for harnesses and Bun/TypeScript programs

The opencode plugin's client (`plugin/opencode/mcpx/daemon.ts`)
talks to the daemon over its unix socket — measured at about
0.17 ms per call against about 23 ms for spawning the binary. It is
not published as a package; harness plugins vendor it, and any
Bun/TypeScript program can do the same.

```ts
import { connect } from "./mcpx/daemon.ts";

const { client, discovery } = await connect({ directory });
if (client) await client.call("fff", "grep", { query: "x" }, sessionID);
```

`connect()` runs the discovery ladder (which daemon, on which
socket, for this directory). The client itself offers generic
`raw` / `get` / `text` / `post` / `postRaw` / `putRaw` methods,
named typed conveniences (`status`, `exec`, `call`, `logQuery`,
`putSetting`, …), and `client.ops` — one generated method per
operation, so nothing is reachable only through a hand-built URL.
The plugin's own documentation is
`plugin/opencode/README.md`, "Talking to the daemon directly".

### Any MCP client — for MCP hosts

A host that only speaks MCP needs no plugin and no client library:
point it at the daemon's `/mcp` endpoint, or have it spawn
`mcpx serve --transport stdio`. mcpx is then simply an MCP server
with a deliberately small tool surface — the operations above,
reachable as `mcpx_<op>` tools. The MCP pages (overview, SDK,
compatibility) are the sibling half of this page set; the protocol
itself is specified in [protocol.md](./protocol.md).

### Raw HTTP on `/v1` — for curl and other languages

```sh
curl --unix-socket <sock> http://mcpx/v1/health
```

Every operation in the table is an HTTP route, and
`GET /v1/openapi.json` describes them all — generated from the
table, so a client generator fed with it produces a client for the
daemon that actually exists. Do not confuse it with
`mcpx schema --format openapi`, which describes the **tools** of
your configured servers, not the daemon: feeding the wrong document
to a generator produces a working client for the wrong thing.
