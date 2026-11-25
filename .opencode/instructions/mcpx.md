# MCP scripts and the two Code Mode runtimes

Use mcpx for MCP servers. Discover what this machine actually has; do not copy a
namespace inventory from another host. `mcpx ls`, then `mcpx search <words>`, then
`mcpx types <namespace>.<tool>` loads only the signatures you need. The native
`mcpx_discover` tool does the same. Names in the generated signatures win over
spelling guessed from a server's display name.

## Choose the runtime before writing the script

- **`mcpx_exec`** (native OpenCode tool) sends TypeScript to the mcpx daemon
  (`POST /v1/exec`). It reaches every configured MCP server, files, subprocesses
  and network. Only what you print returns to the conversation.
- **`mcpx exec '<source>'` / `mcpx run <name-or-file.ts>`** runs the script locally,
  with the shell caller's environment and working directory. MCP calls still go
  through the daemon. A saved script's arguments are in `Deno.args`.
- **`execute`** (OpenCode Code Mode) runs confined JavaScript, not a local Deno
  program. It reaches the built-in `tools.comms`, `tools.opencode`, any tools
  discovered by `search`, and MCP tools currently projected into its catalog.
  It has `fetch`, but no direct filesystem or subprocess API. `tools.openchamber`
  exists only when that integration supplied it; discover it, do not assume it.

**Comms is not an MCP server.** The installed OpenCode fork's `mcpx_exec` description
and `mcpx_discover({ namespace: "comms" })` advertise `tools.comms` in mcpx scripts,
but the daemon does not implement that namespace. Reproduced on 2026-11-24:

```ts
// In mcpx_exec (or mcpx exec): both fail; no message is sent.
await tools.comms.sessions({ mode: "chain", depth: "self" }); // not a function
await call("comms", "msg", { mode: "read", topic: "private" }); // unknown namespace
```

Use `execute` instead:

```js
return await tools.comms.sessions({ mode: "chain", depth: "self" });
```

The same `execute` runtime provides `tools.comms.msg` (including direct wake-up
messages), topics and session logs. See [subagents.md](subagents.md#cross-session-messaging).
This is a runtime boundary, not a missing secret or a server to install.

## Chain, filter, save

Prefer one script for independent queries, pagination, joins or a multi-step MCP
process. Run independent reads together with `Promise.allSettled`, await every
call you need, and return a small answer that keeps each error visible. Never
turn a failed source into an empty array and then report "none found".

In a mcpx script, `Object.keys(tools)` lists the real server namespaces.
`typeof tools.some_name` is not an existence check: the proxy returns a function
for invented names too. Likewise, a successful script exit is not proof every
`Promise.allSettled` entry succeeded; inspect each result.

When a process will run again, save it in `<repo>/.mcpx/scripts/<name>.ts` or
`~/.config/mcpx/scripts/<name>.ts`, then `mcpx run <name>`. The first comment line is
its description in `mcpx scripts`. Ignore the generated `mcpx-client.ts`,
`mcpx-globals.d.ts` and `.*.mcpx-entry.ts` written beside a script by the launcher.
Use `mcpx --json run <name>` when another program consumes the result.

A CLI is still fine. In a script that mixes MCP calls with `gh`, `az`, `kubectl` or
`git`, use `Deno.Command(program, { args })`, check the process exit code and throw
on error. It executes directly; `$SHELL` does not mean an interactive/login shell
is sourced. Choose an absolute binary path and explicit `cwd` when they matter.

## Environment and failures

On the darwin-nix Mac, daemon-run scripts start as the signed-in user in `$HOME`
(the launchd agent's WorkingDirectory), whereas CLI scripts use the caller's
working directory (`script.cwd=caller`). `configuration.nix` appends the usual
system/user profile and Homebrew directories to the daemon PATH; a `just switch`
is needed to activate that setting. `just mcpx-exec-env` in darwin-nix prints the
actual user, directory, shell and command resolution. Elsewhere inspect the
service definition; do not assume a daemon inherited your login shell's PATH.
The HTTP API supports per-run `options.cwd` and `options.env` (tested against the
running daemon), but the native `mcpx_exec` tool accepts only `source` and does
not supply the caller's directory. Use CLI execution or explicit subprocess
`cwd` when a task needs its worktree rather than the daemon's home directory.

- A 45-second wait ending in **"may be destructive and was not confirmed"** is a
  write-policy refusal, not slow inference. An unannotated tool is treated as a
  possible write under `policy.write=ask`. For a genuinely read-only tool,
  configure exact `readOnlyTools` names in that server's mcpx settings. Do not
  disable the global gate to get one search through.
- `mcpx log --since 5m` shows calls and errors; `mcpx status` shows server stderr.
  Restart only the affected namespace with `mcpx restart <namespace>` if needed.
- `mcpx config --sources` shows the config files. All clients on a machine must
  name the daemon's exact config path; an equal copy or a symlink can select a
  different daemon. Use the already-configured `MCPX_CONFIG`.
- Missing secrets go through the private secret UI or `mcpx secrets set <NAME>`,
  never a prompt transcript or a literal config value.

For this Mac's servers, credentials, Azure/Kubernetes read-only settings and
build/activation details: [doc/fleet/mcp.md](../../doc/fleet/mcp.md). For the
step-by-step local scripting procedure, load the `mcpx-scripts` skill when it is
available; its source is darwin-nix's `.agents/skills/mcpx-scripts/`.
