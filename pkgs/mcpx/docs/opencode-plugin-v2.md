# The opencode v2 plugin — verified API facts and implementation map

Companion to [opencode-plugin.md](opencode-plugin.md), which sketches the v1
design and the v1→v2 mapping. That document's v2 claims came from a reading of
opencode's `v2` branch; this one re-checks every fact the rewrite depends on
against the **pinned** v2 source (reported version `2.0.3`) and records where
the pin differs. Work item: [#1858](https://github.com/developing-today/code/issues/1858),
queue task 3068.

```
created:      2026-10-29T23:05:00-05:00
status:       verified facts + implementation map (core rewrite not started)
scope:        CORE rewrite only (#1858 SET-REMAINDER is out of scope)
```

Verified against the local opencode fork checkout (`~/git/opencode`, main at
`c9edb37fda`), paths below are inside `packages/`.

---

## 1. Loading: `{id, setup}` is the whole contract

`core/src/plugin/module.ts:107-134` — the module schema accepts exactly two
default-export shapes:

```ts
{ id: string, setup: (context: Context) => Promise<Cleanup | void> | Cleanup | void }
{ id: string, effect: (context: Context) => Effect<void> }
```

Only `default` is read (`Module = Schema.Struct({ default: ... })`). Failure to
match aborts the module with *"Plugin must export a default definition with an
id and an effect or setup function."* Consequences for the port:

- the `plugin/opencode/mcpx/` subdirectory workaround is unnecessary in v2,
  but the v1 tree keeps its layout until v2 is what runs;
- helpers can be plain sibling modules in the v2 tree, and only the default
  export is inspected — so `export const TUNING`, `export class DaemonClient`
  etc. are safe to import from a shared file;
- the v2 loader accepts a plugin **directory** as well as a file, so the v2
  tree can be one directory whose `index.ts` holds the default export.

## 2. The context surface the port actually uses

`promise/plugin.ts:26-73` (the `setup` variant of `Context`):

| port need | v2 surface |
| --- | --- |
| daemon choice remembered across sessions | `ctx.storage.get/set/remove/scan` (Promise, `schema.Json` values) |
| session id (the thing `shell.env` used to carry) | `ctx.session.hook(...)` inputs carry `sessionID`; `ctx.session.context({sessionID})` reads one |
| system-prompt line | `ctx.session.hook("context", input => …)` mutating `input.system` |
| opt-in tools | `ctx.tool.transform(editor => …)`, `editor.namespace(...)` + `editor.add(...)` |
| tool timing | `ctx.tool.hook("execute.after", …)` — a **union on `status`** |
| a line to the user | `ctx.tui`? no — server plugins get the SDK `client.tui.*` only (unchanged from v1) |
| headless detection | unchanged problem; no `isMainThread` trick in v2's two-process model |

`ctx.generate` (one-shot completion), `ctx.mcp.transform`, `ctx.permission.hook("evaluate")`
and the TUI `select()` picker are the **SET-REMAINDER** — deliberately not used
by this core.

### 2.1 `execute.after` really is a status union

`promise/tool.ts:41-70`: the hook input is
`{ tool, sessionID, agent, messageID, id, input }` intersected with either
`{ status: "completed", result }` or `{ status: "error", … }`. So the timing
record gets a real failure signal; the v1 tree only ever saw success.

### 2.2 `session.hook("context")` is the system-prompt path

`promise/session.ts:25-37` — `SessionRequest` is
`{ sessionID, model, system: Array<SystemPart>, messages, options }`, and
`SessionContext` adds `{ agent, tools }`. Pushing a part onto `input.system` is
the direct replacement for `experimental.chat.system.transform`; no per-command
hook is involved. Note the hook fires per turn, so the injected paragraph must
be idempotent (append once, not accumulate).

## 3. The session environment — two corrections to the mapping table

The mapping table on #1858 says
`PUT /api/experimental/session/:id/environment`. Against the pin:

1. **The path is `/api/session/:sessionID/environment`** —
   `protocol/src/groups/session.ts:879-891`. Payload is
   `{ variables: Record<string, string> }`, success is `204`.
2. **It replaces, it does not merge.** `core/src/session/environment.ts:18-31`
   keeps a `Map<SessionID, Variables>` and `set()` overwrites the entry;
   `core/src/shell.ts:262` then reads
   `environments.get(sessionID) ?? process.env`.

That second point is load-bearing: a plugin that sends only `MCPX_*` variables
would hand every shell command in that session an environment **without `PATH`,
`HOME`, or anything else**. The v2 adapter must send the full environment it
wants the session to have — `{ ...process.env, ...MCPX_* }`, stringified — or
must not set one at all. Verified by reading the consumer, not the doc comment.

The plugin reaches this route either through
`ctx.serverUrl ?? http://…` + `fetch`, or (preferred, when the SDK exposes it)
through the session client. The promised-domains list (`promise/session.ts:153-172`)
does not include an `environment` method, so the raw HTTP call is the route the
core rewrite should plan for, with a `ctx.serverUrl === undefined` fallback
(embedded hosts: skip, degrade to "unknown session", exactly as v1 degrades).

## 4. `ctx.storage` replaces the remembered-daemon file

`promise/storage.ts` — `get(key) => Promise<Json | undefined>`,
`set(key, value)`, `remove(key)`, `scan({ prefix })`. Plugins are namespaced per
plugin id by the host, so the v1 `rememberPath()` / `readRemembered()` /
`writeRemembered()` trio (and the `TUNING.rememberFile` knob) map onto one key
per daemon target. `until-gone` lifetime is preserved by remembering
`{ target, socket, seenAt }` and dropping it when the health check fails — the
same rule as today, with the file gone.

## 5. What the port does **not** need to rewrite

`plugin/opencode/mcpx/daemon.ts` is already harness-free: `discover()`,
`DaemonClient`, `rank()`, `summarise()`, `readInfoFiles()`, `stateDir()` and the
health checks take an `env` map and a directory and speak `/v1` over the unix
socket with Bun's `fetch(url, { unix })`. v2 keeps Bun and keeps the socket, so
the v2 tree imports this file unchanged. Only the rung-4 spawn (`runBin`,
`askCLI`) is harness-specific: `node:child_process` still works in v2's server
process, but the core rewrite should make rung 4 optional (off unless
`MCPX_PLUGIN_BIN` names a binary) rather than assume it.

## 6. Implementation map (first slice)

Target tree, beside the v1 files, all under `pkgs/mcpx/plugin/opencode/v2/`:

| file | contents | acceptance it satisfies |
| --- | --- | --- |
| `index.ts` | `export default { id: "mcpx", setup }` | loads via `{id, setup}` |
| `options.ts` | the v1 `Options` type + `pluginSettings()`-declared variable names, ported verbatim | settings guard |
| `session-env.ts` | first-sighting → `PUT /api/session/:id/environment` with the **full** env | once per session, no per-command `shell.env` |
| `context.ts` | `session.hook("context")` appending one idempotent paragraph | system line |
| `tools.ts` | `tool.transform` with namespace + `discover`/`exec`/`observe`/`daemon_*` | namespaced registration |
| `timing.ts` | `tool.hook("execute.after")` recording both statuses | failures recorded |
| `remember.ts` | `ctx.storage` remembered daemon | no state file |
| `daemon.ts` | re-export of `../../mcpx/daemon.ts` | core reuse |

Tests follow the existing `bun test` style in `plugin/opencode/mcpx/*.test.ts`
(`daemon.test.ts`, `ops.test.ts`, `ceiling.test.ts`), stubbing the context
object: hooks, `storage` map, and a fake `fetch` for the environment route.

## 7. Open questions for the implementer

- **Namespace spelling** — `tool.transform(e => e.namespace("mcpx"))`: confirm
  whether the namespace prefixes tool ids in the model's tool list or only
  groups them, since the v1 tools are named `mcpx_exec` by hand and the
  remainder work (`mcp.transform`) will care.
- **Environment route reachability** — with `ctx.serverUrl` undefined (embedded
  SDK host) there is no HTTP route; decide degrade quietly vs. first-tick retry
  when a later hook reveals a URL.
- **`context` hook idempotency** — whether the host hands the same `system`
  array across turns or a fresh one per request decides between "append once,
  guarded" and "append each turn".
- **Settings guard scope** — `TestThePluginReadsNoUndeclaredSetting` scans
  `plugin/**/*.ts(x)`; the v2 tree must therefore read only the names in
  `internal/settings/plugin.go`, including `plugin.discoveryRetry`,
  `plugin.remember` and `plugin.annotate`.
