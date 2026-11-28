# Glossary

The words this book uses in exactly one sense. Where a word is
defined by a chapter, the entry points at it.

**adapter** — a command-line program declared as an MCP tool:
mcpx runs it as a namespace, from a declaration file named by
`paths.adapters`. See [The CLI](./cli.md) and
[Configuration](./configuration.md).

**agent twin** — the `.html.md` markdown copy of a rendered
page, emitted into the site by docs-twins so agents read the
book as text. See [Deploying this book](./deploying.md).

**API server** — an HTTP service described by an OpenAPI
document and exposed as tools, declared through `mcpx api`.
The counterpart of an adapter, over a document instead of a
program.

**artifact** — a file a script produced, kept by the daemon
under the `artifacts.*` settings and reachable through
`mcpx artifact`. See [exec and artifacts](./exec.md).

**catalogue** — the daemon's list of every tool on every
configured server; `mcpx catalog` prints it inside a token
budget.

**ceiling** — the most a caller may do: an autonomy level, a
write policy, a rate. Ceilings are set by the daemon's side,
never raised by a caller's request.

**compendium** — this book's level 3: everything that is
built, described in as much detail as wanted, with every
document placed by the manifest in `compendium.manifest.json`.
See [The compendium: structure and mapping](./compendium.md).

**daemon** — the long-lived mcpx process that owns every MCP
server process on the machine. Clients (the CLI, plugins,
other programs) talk to it over the `/v1` API; they never
spawn servers themselves.

**effect probe** — a named end-to-end test proving that a
setting changes behaviour. Every setting has one, or a
written exemption. See [Configuration](./configuration.md).

**elicitation** — a server asking the user a question in the
middle of a call. mcpx brokers the question; `mcpx elicit`
answers it, and a call that stopped to ask exits 75
(`EX_TEMPFAIL`) rather than failing. See
[Elicitation: a design](./elicitation.md).

**era** — one revision of the MCP protocol, as a set of
expectations a peer may hold. mcpx speaks every era in both
directions. See [Protocol: every revision, both
directions](./protocol.md).

**exemption** — a setting with no effect probe, carrying a
written reason instead. The count is pinned
(`EffectExemptionPin`) and can only shrink, and only in the
change that adds the replacing probe.

**inline start** — when no daemon is running, the CLI starts
one itself and polls for readiness (the poll interval is
`daemon.inlineStartPoll`) instead of failing.

**namespace** — the name a server's tools are grouped under:
`mcpx types <namespace>` loads its signatures, and a script
calls `<namespace>.<tool>(…)`.

**operation** — one entry in the daemon's `/v1` operation
table (`api.Ops` in `internal/api/ops.go`): a method, a path,
and where each parameter travels. The CLI commands, the MCP
tools, the OpenAPI document and the Go client are all derived
from this one table. See [Parity](./parity.md).

**parity** — the property that the four derived surfaces
agree, because they are generated from the same declarations;
also the chapter that prints the generated mapping.

**plumbing settings** — the internal half of the settings
registry: declared and enforced like any other setting, but
listed only when asked (`mcpx config --schema --plumbing`).
Mostly bounds and intervals a harness cannot vary.

**pool** — the daemon's set of live server instances. A
server's `scope` decides which process a call gets (one per
repo, worktree, session, …) and its `sharing` whether callers
share it or queue.

**preset / profile** — a named bundle of configuration: a
*profile* selects which servers are active; a *preset* is a
named settings bundle. `--profile` is a global flag; presets
live in the `presets` settings.

**recipe** — a saved script that declares its own holes:
`mcpx prompt` produces one from a request in words, and
`mcpx recipes` runs them. See [Scripts](./scripts.md).

**registry** — two senses, kept apart by context: the
*settings registry* (every setting declared once, in
`internal/settings`) and a *public registry* of MCP servers
that `mcpx registry` searches.

**scope** — who reads a setting (daemon, client, one call,
the plugin), or what decides which server process a call
gets. The word is used for both; the chapter says which.

**script** — TypeScript (or JavaScript) run by `mcpx exec` /
`mcpx run` on a real runtime — deno, else bun, else node —
with the generated client and every namespace preloaded.

**supervise / supervisor** — `mcpx supervise` runs one stdio
server child under a supervisor that owns the child's pipes
for its whole life, so a daemon upgrade need not orphan it.
The takeover handoff is the older mechanism it replaces.

**takeover** — the upgrade handoff: `mcpx upgrade` starts the
new binary as the old daemon's child and passes it the stdio
children's pipes over a unix socket. Streamable HTTP sessions
survive it; SSE sessions are closed and re-initialised.

**transport** — how a client reaches the daemon — a unix
socket, ssh, or a `tsnet://` tailnet identity — or how the
daemon reaches a server: stdio for a spawned process,
Streamable HTTP for a URL. See [Modes](./modes.md) and
[Reaching a daemon on another machine](./remote.md).
