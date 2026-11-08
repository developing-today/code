# Modes

"Mode" is three questions wearing one word, and most confusion about
mcpx is two of them being answered at once. How does a _client_ — the
CLI, a script, the plugin — reach a daemon? How does the daemon reach
an upstream _server_? And in what form does mcpx itself run: a daemon
in the background, a process a host started, or no daemon at all?
This page takes them in that order. The knobs themselves live on the
[configuration page](./configuration.md); this page is about the
choices they add up to.

<a id="client-modes"></a>

## Client modes

Every command walks the same ladder, cheapest first:

1. **socket** — an existing local daemon, over its unix socket.
2. **url** — a named remote daemon, over HTTP.
3. **spawn** — start a daemon, then use it. About 135× the cost of a
   warm socket, paid once; everything after it is cheap.
4. **inline** — run the servers inside this command's own process.

The first two are found, not chosen per command. A daemon is keyed by
the set of configuration files it was started from, so the socket a
command looks for is the one belonging to its own configuration —
two projects with different configs get different daemons, and a
command never answers from a neighbour's. A remote daemon is named
with the `daemon.endpoint` setting (the `MCPX_DAEMON_ENDPOINT`
variable, or `--daemon-endpoint`): an `http://` URL, or
`unix:///path` for a non-default local socket. Naming an endpoint
stops the ladder at that rung: if the remote daemon is unreachable
the command fails rather than quietly starting a local one and
answering from the wrong machine.

**Spawn** is the default life of most installs: `daemon.autostart`
is on, so the first command that finds no daemon starts one
detached, waits for it to answer, and proceeds; the daemon then
outlives the command and serves the rest. **Inline** is the last
rung and off by default (`daemon.inline`). It hosts the same API
inside the command, on a private socket, for exactly as long as the
command runs — which means every server starts again on every
command and nothing, a browser session included, survives between
them. It exists for sandboxes with no fork, read-only filesystems,
and containers whose init will not reap: places where the
alternative is not working at all.

Where a script runs follows from the same choice. `exec.where`
(`--local` / `--remote`, default `auto`) runs a script locally when
the daemon is local and on the daemon when it is not; asking for
`--local` against a remote endpoint is refused, because the script
would succeed and still write its files on the wrong side. See
[Where a script runs](./exec.md#where-a-script-runs).

Two controls shape what a client sees once connected. **Profiles**
select which servers exist for a command at all — `mcpx ls` shows
the default set, `--profile web` adds a group, `--skip-default`
narrows to exactly it, `--all-profiles` ignores profiles — and the
selection applies to everything derived from the list, the
generated client included. **Output** is client- and call-scoped
configuration: how results are rendered and truncated is decided by
the process asking, per call, not by the daemon serving everyone.

<a id="transport-options"></a>

## Transport options

There are two directions, and they have different answers. Keeping
them apart is most of this section.

**Upstream — the daemon to a configured server.** A server block
with a `command` is spoken to over **stdio**: mcpx starts the
process and talks JSON-RPC down its pipes; there is nothing else a
spawned process could offer. A server block with a `url` is spoken
to over **Streamable HTTP** (the setting is `transport: "http"`,
and it is filled in from the `url` when left blank). Replies from a
Streamable HTTP server may _arrive_ as a server-sent-events stream —
that is a property of the response, not a second transport.

The `/v1` API also accepts `transport: "sse"` when adding a server.
That value does not conjure an SSE-only client. What actually
happens with a legacy server: if a remote server turns out to speak
only the deprecated 2024-11-05 HTTP+SSE transport, the pool falls
back to a dedicated legacy client for it and forces the legacy
protocol era for that connection. So old servers still work, and
the fallback is a property of the server you pointed at, not a
mode you select.

**Downstream — a client to the daemon.** The daemon serves its
whole API, `/v1` and `/mcp` together, on the listeners it already
has: the unix socket, and a TCP port that is loopback unless you
bind it wider (`mcpx daemon --address 0.0.0.0 --port 8899`). There
is no separate MCP listener to configure; one listener, one
access-control decision. Binding beyond loopback prints a warning,
because the API is unauthenticated: whatever can route to the port
can run tools as you.

Events travel as **server-sent events**, deliberately not
WebSockets: SSE is plain HTTP, passes the proxies HTTP passes, and
a browser's `EventSource` reconnects it for free, while answers
never needed the return channel — they go back as ordinary POSTs.
Every event carries a sequence number, and a reconnect with
`Last-Event-ID` (or `?since=N` on `GET /v1/events`) replays
everything after it, so a dropped connection is a pause, not a
hole; a reconnect older than the retained history gets an in-band
`gap` event saying so.

<a id="server-modes"></a>

## Server modes

"Server" is overloaded too, so here are its three meanings, kept
apart.

**An upstream server being pooled.** This is the sense in a config
file's `mcpServers` block, and its modes are the `mcpx` block's two
axes: `sharing` — `shared` (the default; any number of callers use
one process at once) or `exclusive` (one caller at a time, others
queue) — and `scope`, what a live process is keyed by: `global`
(the default), `repo`, `worktree`, `cwd`, `session`,
`parent-session`, `pid`, or `call`. One live process exists per
distinct key, so scope is the answer to "how many of this server am
I running, and who gets a separate one". A stateful server — a
browser — typically wants `exclusive` + `session`; a stateless one
wants the defaults and no thought at all. The full treatment is
[Sharing and scope](./story.md#sharing-and-scope) and the
[server configuration section](./configuration.md#server-config).

**mcpx as an MCP server.** `mcpx serve` offers mcpx itself to a
host that speaks only MCP — another editor, a hosted agent. The
surface is ten tools reaching every server mcpx knows, `mcpx_exec`
chief among them: it runs TypeScript next to the servers and only
what the script prints crosses back, so schemas stay on this side
of the wire instead of filling the host's context. On stdio, serve
is a thin shim a host spawns, which is inherent to how MCP hosts
start servers. On HTTP there is nothing separate to start: the
daemon mounts `/mcp` on its own listeners, so serving MCP to the
network is the daemon command above, not a second process.

**The daemon's own process modes.** A daemon is either started for
you — detached, by the first command that needs one, exiting after
`autostart.idleExit` with nothing to do — or run in the foreground
with `mcpx daemon`, which is the same daemon with its log in front
of you, and the form a service manager should supervise. Inline
mode, from the client ladder, is the third form: a daemon's API
with no daemon process at all. Nothing about a server's pooling or
an MCP host's view changes between the three; what changes is
lifetime, and who owns it.
