# Plugins

"Plugin" means two different things in this project, and every page in this
set says which one it means:

| meaning | direction | exists today |
| --- | --- | --- |
| **harness plugin**: code that plugs mcpx *into* an agent harness | a consumer of mcpx | yes: the opencode plugin, under `plugin/opencode/` |
| **mcpx plugin**: an extension *of* mcpx itself | extends mcpx | no: there is no plugin concept in the daemon at all |

Nearly everything below is about the first meaning. The second gets a short
status section at the end of the first section, and stays a stub until the
extensibility work that designs it lands.

## Plugins

### Harness plugins: mcpx inside your agent

A harness plugin is **optional**. Any harness with a shell can use mcpx
through `AGENTS.md` alone: the agent runs `mcpx` commands, and mcpx needs
nothing from the harness to answer them. What the harness cannot give that
way is *identity* — and identity is what the plugin adds:

- **the session id**, so `session`-scoped servers persist across commands
  and two agents running at once lease separate server processes instead of
  corrupting one shared browser;
- **the socket fast path**: about 0.17 ms per call over the daemon's unix
  socket, against about 23 ms for spawning the binary — which is why the
  plugin also means **no `mcpx` on `PATH` is required** for any of it (the
  one exception is rung 4 of daemon discovery, below, a last fallback that
  runs the binary only when one exists);
- **optional extras**: tools offered to the model, a system-prompt line, tool
  timing records, and a picker for choosing between daemons.

Hosts that only speak MCP need no plugin at all: point them at
`mcpx serve --transport stdio` or the daemon's `/mcp` endpoint and mcpx is
just an MCP server to them. Session scoping still works there, but it is
mcpx's own transport session — a stable per-process key (`mcp-<pid>` for a
stdio server) — not the harness's session. What those hosts do not get is
*harness-session identity*: nothing ties the MCP connection to the agent
session that spawned it, so two agents sharing one MCP server process
share its session scope too. That identity is the trade the plugin exists
to provide.

### mcpx plugins: extending mcpx

Not built. There is no plugin concept in the daemon: no loading, no hooks,
no extension API. The design space — Go, binary, script or remote plugins,
hooks, variables — is tracked by the extensibility issues on the repository
(#80, #82, #83 in the original numbering; see the issue for this page,
#1855). Until that work lands, the ways to extend what mcpx can reach are
the built-in ones: a command-line program becomes tools through an adapter,
an OpenAPI document becomes tools through `mcpx api`, and a saved script or
recipe runs by name. This section stays a stub so the word "plugin" is
never left ambiguous: if a page does not say "mcpx plugin", it means the
harness kind.

## Plugin compatibility

Every cell cites a source or says "unknown". Nothing here is guessed, and
no support is claimed before an adapter exists.

| harness | plugin | how mcpx reaches it today | session id | source |
| --- | --- | --- | --- | --- |
| opencode v1 (1.18.31) | yes, `plugin/opencode/` | plugin, or shell + `AGENTS.md` | injected by the `shell.env` hook | `plugin/opencode/README.md`; `docs/opencode-plugin.md` §2, §6 |
| opencode v2 (2.0.3) | no: v1 plugins do not load | shell + `AGENTS.md`, or mcpx as an MCP server in its `mcp` config | not injected; needs the v2 session environment API | `docs/opencode-plugin.md` §4; see [opencode v2 plugin](#opencode-v2-plugin) below |
| codex | no | shell + `AGENTS.md`; MCP | unknown | adapter design: `docs/opencode-plugin.md` §5 |
| claude code | no | shell + `AGENTS.md`; MCP | unknown | adapter design: `docs/opencode-plugin.md` §5 |
| pi | no | unknown | unknown | adapter design: `docs/opencode-plugin.md` §5 |
| manus | no | unknown | unknown | adapter design: `docs/opencode-plugin.md` §5 |
| any MCP host | not needed | `/mcp` or stdio | no harness session; mcpx derives its own transport session (`mcp-<pid>` for stdio) | `internal/cli/serve.go` |

One runtime note: the portable core dials the socket with Bun's
`fetch(url, {unix})`. A Node-based harness cannot use that option, so it
uses `MCPX_DAEMON_ENDPOINT` and the daemon's TCP listener instead.
Everything else in the core is runtime-neutral.

## Plugin interface and config

There are two interfaces — plugin to harness, and plugin to daemon — and
one configuration surface.

### Plugin to harness

The opencode plugin uses four hooks (`docs/opencode-plugin.md` §2.2):

| hook | what it carries for mcpx | without it |
| --- | --- | --- |
| `shell.env` | `MCPX_SESSION_ID`, `MCPX_CALL_ID` and the rest of the injected environment, per shell command | no ids: calls are attributed to "unknown" and session-scoped leases fall back to **per-call isolation** — each invocation gets its own lease keyed by its call id, so state does not persist across commands and anything stateful must happen inside one invocation |
| `experimental.chat.system.transform` | the opt-in system-prompt line saying mcpx exists | the opt-in tools still work when named; the model may never reach for mcpx |
| the `tool` map | the opt-in tools `mcpx_discover`, `mcpx_exec`, `mcpx_observe` | mcpx is shell-only, which is the default anyway |
| `tool.execute.after` | one timing record per tool call, into mcpx's log | no harness timings; `mcpx stats` covers mcpx alone |

The names are the code's names: the plugin sets `MCPX_SESSION_ID`
(`plugin/opencode/mcpx-session.ts`), `MCPX_CALL_ID` and
`MCPX_PARENT_SESSION_ID`, and those are the names the `session` and
`parent-session` scopes read. (`MCPX_SESSION` without a suffix is a
different, live variable: mcpx sets it for each script it runs. It is not
the plugin's variable and must not be conflated with it.)

### Plugin to daemon

The portable core, `plugin/opencode/mcpx/daemon.ts`, imports nothing from
opencode. Its `DaemonClient` / `connect()` finds a daemon by walking a
ladder and stopping at the first answer (full design:
[plugin-discovery.md](plugin-discovery.md)):

| # | rung | applies when |
| --- | --- | --- |
| 0 | an `endpoint` or `socket` named outright, by option or environment | configured, or remote — and it never falls back to a local daemon |
| 1 | the choice remembered for this directory, still answering | every session after the first |
| 2 | the `daemon-*.json` info files the daemon publishes | **the usual case; no binary needed** |
| 3 | `GET /v1/resolve` against any live daemon | two or more candidates |
| 4 | spawn `mcpx --json status` | a binary exists and the daemons are too old for rung 3 — the only rung that runs the binary. `backend: v1` does **not** suppress this spawn: the backend guard governs the tool-execution fallback, and discovery still spawns when no daemon is discoverable. Only `bin: ""` forbids spawning outright |
| 5 | ask the user (toast, picker, or `mcpx_daemon_select`) | still ambiguous, and someone is there |
| 6 | the best candidate, with a warning | headless, or unanswered — the ladder never fails just because there are two |

`connect` returns `{ client: undefined }` rather than throwing: a plugin
that fails to load because mcpx is not running has broken the editor for a
tool the user may not even be using.

### For authors of plugins for other harnesses

The core is harness-free on purpose; an adapter per harness supplies the
rest. What an adapter must provide (`docs/opencode-plugin.md` §5):

1. a directory, to resolve which daemon;
2. a session identifier, to attribute and route;
3. somewhere to put environment variables, or a way to wrap tool calls;
4. a hook after a tool call, if timings are wanted;
5. a way to show a line of text, if ambiguity is to be reported;
6. optionally, a way to ask a question and a way to run a model.

Only (1) and (2) are required. Everything else degrades.

### Configuration

Precedence is **plugin option > environment variable > default**. Options
reach the plugin only when it is named in the opencode config as a
two-element array — a plugin merely dropped into the `plugin/` directory
gets no options, so the environment variables are the way to configure a
drop-in installation (see [opencode v1 plugin](#opencode-v1-plugin)).

The table below is derived from **one declared list**: the registry's
`plugin.*` settings, declared in `internal/settings/plugin.go`
(`pluginSettings()`), which is also what `mcpx settings` reports and what
the settings tests pin against the plugin's sources in both directions.
The two options that are not settings — `endpoint` and `socket`, which name
a daemon rather than tune the plugin, and are read before any settings
exist — are declared alongside the table, not in the registry. If an option
is missing from this table, that is a bug in the table's source list, not
a detail to fix in the table: `discoveryRetry` went missing from the
plugin README's hand-maintained table exactly that way.

| option | variable | default | effect |
| --- | --- | --- | --- |
| `endpoint` *(not a setting)* | `MCPX_DAEMON_ENDPOINT`, `MCPX_ENDPOINT` | — | a daemon named outright: `http://host:port` or `unix:///path`. Stops the ladder. |
| `socket` *(not a setting)* | `MCPX_SOCKET` | — | a socket named outright. Stops the ladder. |
| `bin` | `MCPX_PLUGIN_BIN` | `mcpx` | the binary for rung 4. `""` forbids spawning anything. |
| `binArgs` | `MCPX_PLUGIN_BIN_ARGS` | — | arguments put in front of every plugin invocation of mcpx (the case this exists for is `--config` or `--profile`). |
| `backend` | `MCPX_PLUGIN_BACKEND` | `auto` | `auto` prefers the socket and spawns only as a fallback; `v1` refuses the spawn fallback for tool execution (discovery may still spawn when no daemon is discoverable — `bin: ""` is the way to forbid spawning entirely); `cli` never uses the socket. |
| `discoveryRetry` *(environment only)* | `MCPX_PLUGIN_DISCOVERY_RETRY` | `60s` | how long the plugin waits before looking for mcpx again after a failed lookup, so an editor without mcpx does not pay for a failed spawn on every keystroke. Declared in the registry, but the adapter reads it from the environment only — passing it as a plugin option is silently ignored. |
| `toolTiming` | `MCPX_PLUGIN_TOOL_TIMING` | off | record opencode's tool timings into mcpx's log, so one `mcpx stats` covers both. |
| `env` | `MCPX_PLUGIN_ENV` | `full` | how much session context is injected into each command's environment: `minimal`, `standard`, `full` (the lists are under [opencode v1 plugin](#opencode-v1-plugin)). |
| `instructions` | `MCPX_PLUGIN_INSTRUCTIONS` | off | add mcpx usage guidance to the system prompt. |
| `tools` | `MCPX_PLUGIN_TOOLS` | off | offer `mcpx_discover`, `mcpx_exec` and `mcpx_observe` as tools the model can call. |
| `remember` | `MCPX_PLUGIN_REMEMBER` | `session` | how long a chosen daemon stays chosen: `session`, `until-gone`, `indefinite`. |
| `annotate` | `MCPX_PLUGIN_ANNOTATE` | on | add one line to each mcpx tool result naming the daemon that answered, when more than one matched. |
| `headless` | `MCPX_PLUGIN_HEADLESS` | `auto` | suppress toasts, because nobody is looking at a screen. `auto` decides from the process shape. |
| `daemonTools` | `MCPX_PLUGIN_DAEMON_TOOLS` | `auto` | offer `mcpx_daemon_status`, `mcpx_daemon_select` and `mcpx_daemon_forget`. `auto` offers them when `tools` is on, or when discovery found more than one daemon. |

`MCPX_STATE_DIR` and `XDG_STATE_HOME` are read too, because that is where
the daemon publishes its info files and where the remembered choice is
kept. They are mcpx's variables, not the plugin's. The full configuration
reference is [configuration.md](configuration.md).

## opencode v1 plugin

This section is about the **harness plugin** for opencode v1 (1.18.31) —
the one plugin that ships today. It lives in `plugin/opencode/`; the
install and reference material is `plugin/opencode/README.md`, and the
design is [opencode-plugin.md](opencode-plugin.md).

### What ships

| file | what it is for |
| --- | --- |
| `plugin/opencode/mcpx-session.ts` | the opencode adapter: hooks, tools, toasts, and the session id. The only file opencode loads as a plugin. |
| `plugin/opencode/mcpx/daemon.ts` | the portable core: the daemon HTTP client, the discovery ladder, and the remembered-choice file. Imports nothing from opencode. |
| `plugin/opencode/mcpx-tui.tsx` | optional TUI plugin: an arrow-key picker for the daemon. A different realm with a different API, installed separately. |
| `plugin/opencode/skills/` | four skills: `mcpx-basics`, `mcpx-observability`, `mcpx-browser`, `mcpx-daemon`. The same directories are embedded in the mcpx binary and served over MCP, so an MCP host can load them without copying anything. |

### Install

By hand, with `cp -R` — there is no packaged install, and the Nix
configuration does not install the plugin:

```sh
mkdir -p ~/.config/opencode/plugin
cp -R mcpx-session.ts mcpx ~/.config/opencode/plugin/

# optional, and a separate artifact: the arrow-key daemon picker
cp mcpx-tui.tsx ~/.config/opencode/plugin/

# skills are separate again
cp -R skills/mcpx-daemon ~/.config/opencode/skills/
```

(run from `plugin/opencode/`). `mcpx/` must stay a subdirectory: opencode
v1 loads every file directly in `plugin/` and calls each of its exports as
a plugin, so `daemon.ts` beside `mcpx-session.ts` would have its helpers
called as plugins. The glob is not recursive, so one level down the core is
only ever imported.

To pass options, name the plugin in opencode's config as a two-element
array; a drop-in file gets none:

```jsonc
{
  "plugin": [
    ["./.opencode/plugin/mcpx-session.ts", { "tools": true, "backend": "v1" }]
  ]
}
```

Every option also has the environment variable in the table under
[plugin interface and config](#plugin-interface-and-config), which is the
way to configure a drop-in installation.

### What gets injected

How much goes into each shell command's environment is the `env` option:

- `minimal`: `MCPX_SESSION_ID` only.
- `standard` adds what the hook is handed directly, what the plugin knew at
  startup, and the daemon this session settled on — `MCPX_CALL_ID`,
  `MCPX_PROJECT_ID`, the worktree and harness variables, and
  `MCPX_DAEMON_ENDPOINT`, so a shell command that runs `mcpx` talks to the
  same daemon the plugin does instead of walking its own ladder.
- `full` (the default) adds what needs a lookup, done once per session and
  cached: `MCPX_PARENT_SESSION_ID`, the session title, directory, version,
  depth, creation time and age, and the full trace ancestry in
  `MCPX_TRACE_IDS`.

Environment variables are not context: the model never sees them, so
anything cheap goes in. The full list, with who reads each variable, is in
[environment.md](environment.md).

### The tools

With `tools: true`, the plugin offers `mcpx_discover`, `mcpx_exec` and
`mcpx_observe`, each speaking the daemon API first and falling back to the
binary only when there is no daemon (never under `backend: v1`). They are
**off by default** because an agent with a shell can already run mcpx, and
a tool definition costs context on every request whether or not it is
used. Turn them on when the agent has no shell, when you want mcpx calls
visible as tool calls in the transcript, or when a model keeps forgetting
mcpx exists.

The daemon tools — `mcpx_daemon_status`, `mcpx_daemon_select`,
`mcpx_daemon_forget` — answer the which-daemon question when discovery
found more than one. They gate nothing; work proceeds on the best
candidate with a warning until somebody chooses.

Two more options, both off by default for the same context-cost reason:
`instructions` adds four lines of mcpx usage guidance to the system
prompt, paid on every request for the life of the session; `toolTiming`
records every opencode tool call into mcpx's log (a 0.09 ms POST over the
socket; with no daemon the record is dropped, never spawned — a timing is
not worth 23 ms).

### The TUI picker

`mcpx-tui.tsx` adds one palette command, **mcpx: choose daemon**: each
daemon's project, start time, server count and version, then how long the
choice should stick. It is a separate plugin because the TUI runs in a
different realm with a different API — a v1 module may export `server` or
`tui`, never both — and a choice made there reaches the server plugin
through the remembered-choice file.

### Interplay with opencode's own code mode

None, and none needed. opencode v1 ships its own code mode behind
`OPENCODE_EXPERIMENTAL_CODE_MODE`, which moves MCP tools out of the
model's tool list; the mcpx plugin does not conflict with it, and the
plugin needs nothing in opencode's `mcp` config — it reaches the daemon
itself, over the socket.

## opencode v2 plugin

**Status: not built.** This section is about the **harness plugin** for
opencode v2 (2.0.3), and it is a status section, not a feature description:
v1 plugins do not load in v2 at all, so the plugin needs a rewrite, not a
port. The work is tracked in the repository's issues (#47 in the original
numbering; see #1855), and the full design is in
[opencode-plugin.md](opencode-plugin.md) §4 and the compendium it links.

On opencode v2 today:

1. **Shell + `AGENTS.md` works**, with no session id — so `session`-scoped
   servers are isolated per call rather than per session.
2. **Or add mcpx as an MCP server**: `mcpx serve --transport stdio` in
   v2's `mcp` config. One caveat: v2 defaults MCP servers to code mode, so
   mcpx's own tools end up wrapped inside v2's code mode; whether v2's
   code-mode defaults recognise mcpx and turn that off for it is unknown.

What the rewrite changes, when it happens:

- **Unnecessary:** the `mcpx/` subdirectory (only `default` is read), hand-
  namespacing tool names, a state file for the remembered daemon
  (`ctx.storage` exists), and the per-command `shell.env` hook (the session
  environment API sets a session's shell environment once per session).
- **Possible:** answering MCP sampling with `ctx.generate.text`;
  registering MCP servers per session, so mcpx could hand opencode a
  filtered server set; putting a budgeted catalog into the session context
  hook instead of the system prompt; seeing tool *failures* in the
  after-hook, not just successes.
- **Breaks:** the plugin itself, Bun's `$` (gone from the v2 plugin API —
  the other reason the plugin must not need the binary), `sessionID` in
  the shell hook (its replacement carries no session id), and MCP SSE
  transport.
- **The same:** the TUI SDK surface, no isolation (plugins run in the
  host's realm), and Bun's `fetch(url, {unix})` — so the socket fast path
  survives the rewrite.

One v1 quirk worth knowing while both versions are in use: v1.18.31 runs
both plugin systems at once, so a v2 plugin file dropped into a v1
`plugin/` directory is loaded by the v2 host and rejected, harmlessly, by
the v1 loader.

## vibeMcp

Placeholder: what "vibeMcp" means is an open question for Drew (open
question 1 on #1855) — this anchor holds the name, and no meaning is
guessed here until he answers.
