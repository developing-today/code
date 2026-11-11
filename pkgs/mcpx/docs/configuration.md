# Configuration

Every number in mcpx is a setting, including the ones nobody will ever
change.

That sounds like an overreaction until you have hit the alternative. A
constant written next to the code that needs it is fine right up to the day
somebody's proxy kills an idle event stream at thirty seconds, or their
sandbox refuses to exec the running binary, or a tool legitimately takes a
five-megabyte argument. At that point the only remedies are a patched binary
or a fork, and both are absurd outcomes for a number that was chosen in five
seconds and never revisited. Declaring it costs one line and removes that
outcome entirely.

So the rule is: nothing is hardcoded. A value lives in
`internal/defaults/defaults.json` as data, and -- wherever a user could
conceivably want it different -- it is declared once in the settings registry,
from which its configuration key, its environment variable, its command-line
flag, its `/v1` representation and its MCP tool are all derived. There is no
second place to update, which is the point: the arrangement this replaced had
four, and they drifted.

The handful of exceptions are permission bits. `0700` on the state directory
and `0600` on the socket are not preferences. The socket grants the power to
run tools as you; a configuration key that widens it would be a footgun with
no legitimate use. They are in `defaults.json` as data so there is still one
place to read them, and they are deliberately absent from the registry.

A guard test enforces this. It matches inline durations in both spellings
gofmt produces, byte sizes written as shifts, bounds assigned to variables
whose names say they are bounds, and permission literals, with an allowlist
that carries a reason for each exemption. The version it replaced matched
fixed strings and missed every duration gofmt had compacted into a function
argument -- nine of them, including the daemon's shutdown grace and the event
stream's keepalive.

## Where a value can come from

Lowest to highest:

1. **default** -- `internal/defaults/defaults.json`, embedded in the binary.
2. **file** -- every configuration file on the search path, ranked by
   distance, nearest highest. `mcpx config --sources` lists them.
3. **env** -- `MCPX_*`. Every setting has one, derived from its path, whether
   or not anybody uses it.
4. **flag** -- the command line.
5. **runtime** -- an override set through `PUT /v1/settings/{path}` or
   `mcpx settings set`. Highest because it is the most recent statement of
   intent: somebody changed their mind while the thing was running, and having
   a flag from ten minutes ago win would be inexplicable.

`mcpx settings get <path>` prints the winning value, where it came from, and
what it displaced. "Why is this not what my config says" is the most common
configuration question there is, and the answer is always in that list.

The search path itself is decided before any of those layers exist, in two
passes. The discovery pass walks the default locations, seeded by
`MCPX_PATHS_CONFIG` / `--paths-config`, and reads only `paths.config`. The
load pass resolves that list and loads everything on it, nearest last: a
file's `paths.config` governs the rest of the chain from that point, a null
entry stands for the locations the default walk provided, and a list with
no null replaces the farther remainder. A `paths.config` found only in the
load pass is reported by `mcpx config --sources`, not applied, and a cycle
is refused naming the path that closed it. `MCPX_CONFIG` bypasses both
passes and loads exactly one file.

Two spellings of one setting at the same level is an error rather than a race:
`--log-level` and `--logging-level` together, or two variables that mean the
same thing with different values. The environment has no order, so there is
genuinely nothing to prefer, and picking one silently is how a configuration
becomes unexplainable.

## Scope: who reads it

Every setting says which process acts on it. This exists because of a failure
with no symptom.

A daemon is started once, in whatever environment it happened to have, and
then serves every command for hours. A flag on one of those commands that
governs something the _daemon_ does was simply lost: accepted, validated,
printed back, and then ignored, because the process that would act on it had
read its configuration an hour earlier. Nothing anywhere said so.

- **daemon** -- the pooled servers, the listener, the log the daemon writes.
  A client changes one by persisting it and restarting, or through
  `mcpx settings set` if the setting is hot.
- **client** -- the CLI process itself: how it renders, how it finds a daemon,
  whether it may start one. Setting it on the daemon does nothing.
- **call** -- meaningful per request. The CLI sends its effective value with
  every request in an `X-Mcpx-Settings` header, and the daemon honours it for
  that request only. This is what makes `mcpx search --limit 3` work against
  a daemon that was started without it. Only call-scoped settings are honoured
  from that header: widening it would make a header a way for any client to
  reconfigure a shared daemon.
- **plugin** -- read by the opencode plugin, which is neither. The plugin
  reads `process.env` directly, so the variable names are the contract between
  the two and a test pins them.

## Hot: does it take effect now

A **hot** setting is read afresh every time it is used, so changing it at
runtime works. A cold one was consumed once at startup -- a listener address,
a log file that is already open -- and recording a change to it would do
nothing. `PUT /v1/settings/{path}` says so rather than accepting a value that
would be silently inert.

## Reaching them

| surface     | read                                             | change                                                                                              |
| ----------- | ------------------------------------------------ | --------------------------------------------------------------------------------------------------- |
| config file | any `mcpx` config on the search path             | edit it                                                                                             |
| environment | `MCPX_*`                                         | export it                                                                                           |
| CLI         | `mcpx settings list`, `mcpx settings get <path>` | `mcpx settings set <path> <value> [--persist runtime\|project\|user]`, `mcpx settings unset <path>` |
| `/v1`       | `GET /v1/settings`, `GET /v1/settings/{path}`    | `PUT /v1/settings/{path}`, `DELETE /v1/settings/{path}`                                             |
| MCP         | `mcpx_settings_list`, `mcpx_settings_get`        | `mcpx_settings_set`, `mcpx_settings_unset`                                                          |

`mcpx settings list` resolves locally, which is what the _client_ will use.
`--daemon` asks the daemon what it resolved, which is what a daemon-scoped
setting actually is. The two differing is information, not a bug: it is
exactly the case that used to be invisible.

`--persist runtime` changes the running daemon and nothing on disk, which is
what an experiment wants. `project` and `user` write the corresponding
configuration file _and_ apply at once where the setting allows it -- a change
written to a file but not applied would leave `mcpx settings get` answering
with the old value, which reads as the write having failed.

<a id="common-config"></a>

## Common config

Start with `mcpx init`: it writes a starter configuration whose
every key is one the loader reads, comments included, and
`mcpx ls` will tell you immediately whether it took. The file is
JSON. Its two halves are the `mcpServers` object — the servers, next
section — and settings, carried in nested blocks named for each
setting's first segment (a `logging` block, a `pool` block), the
same grouping the inventory at the bottom of this page uses.

There is rarely one file. mcpx merges every configuration file on
its search path, so a project can add its servers in a file of its
own without anybody copying the user-level file to edit it. The
search path, in the order the files are consulted:

1. If `MCPX_CONFIG` is set, that one file, alone.
2. Otherwise, at each directory from the working directory up to
   the root: .config/mcpx/config.json, then .mcpx.json, then
   .mcpx/config.json, then .mcp.json.
3. Then the user level: $XDG_CONFIG_HOME/mcpx/config.json,
   ~/.config/mcpx/config.json, ~/.mcpx.json.
4. Finally /etc/mcpx/config.json.

Files nearer the working directory win: merging is per key and per
server name, so a nearer file overrides what it names and inherits
the rest, and a server it does not want it can drop with
`"disabled": true`. One honest surprise in that list: .mcp.json is
Claude Code's project file, and mcpx reads it on purpose — a
project already configured for Claude Code is already configured
here, its `mcpServers` included. An explicit `--config <file>`
stands outside all of this and is used alone, because naming a file
means meaning it.

(`paths.config` can redirect the search itself, in two passes —
discovery, then load. That machinery is [Where a value can come
from](#where-a-value-can-come-from), above; the list here is the
default it starts from.)

Whatever the file, a value in it is one layer of five — defaults,
file, environment, flag, runtime override, in that order — and the
file layer is the only one most people ever touch. When a value is
not what the file says, do not guess: `mcpx config --sources` lists
the files that were actually read, and `mcpx settings get <path>`
names the winning value, where it came from, and what it displaced.

<a id="server-config"></a>

## Server config

A server is one entry in `mcpServers`:

```json
{
  "mcpServers": {
    "chrome-devtools": {
      "command": "chrome-devtools-mcp",
      "args": ["--headless", "--isolated"],
      "mcpx": { "sharing": "exclusive", "scope": "session", "max": 4 }
    },
    "internal-docs": { "url": "http://docs.internal:8080/mcp" }
  }
}
```

Two names for this, and they are not interchangeable: the file key
is `mcpServers` — the same object other MCP hosts use, so an
existing configuration pastes in unchanged — while `servers` is the
CLI command (`mcpx servers list`) and the `/v1` resource
(`GET /v1/servers`). A file that says `servers` configures nothing.

A block with a `command` is a stdio server: the command and its
`args`, an `env` map for the child, nothing more needed. A block
with a `url` is a remote server, reached over Streamable HTTP; it
may carry `headers` and an `auth` block, and `transport` is filled
in as `http` when left blank. What the transports actually mean —
including the legacy-SSE fallback — is
[Transport options](./modes.md#transport-options).

The `mcpx` block is mcpx's own, ignored by other hosts, and it is
where a server stops being a line in a file and starts being a
process policy:

- `sharing`: `shared` (default) lets any number of callers use one
  process at once; `exclusive` gives one caller at a time and
  queues the rest. Stateful servers want exclusive.
- `scope`: what a live process is keyed by — `global` (default),
  `repo`, `worktree`, `cwd`, `session`, `parent-session`, `pid`,
  `call`. One process per distinct key. `session` and
  `parent-session` are resolved by the caller, through
  `MCPX_SESSION_ID` / `MCPX_PARENT_SESSION_ID`; a scope that cannot
  resolve degrades to per-call isolation and says so in the log.
- `profiles` and `default`: which profile selections see the
  server at all; a server is default-on unless it says otherwise.
- `tools` / `excludeTools`: the allowlist and denylist of tool
  names callers may see — [Hiding tools](#hiding-tools), below.
- `max` and `idleTimeout`: how many processes a scope may hold,
  and how long an idle one is kept.

An **alias** is a second entry pointing at the first:
`"aliasOf": "<name>"`, with its own `mcpx` block giving a different
namespace, tool subset, or description over the same server —
aliases of aliases are refused. Whether an alias shares its
target's *process* depends on whether its pooling fields match;
change its `scope` and it is a separate process.

Adding and removing servers does not require editing by hand — the
CLI, `/v1`, and MCP surfaces for it are the [Servers](#servers)
table above, the entry is written to a configuration file, and the
daemon reloads and makes it callable immediately.

<a id="client-config"></a>

## Client config

Client configuration is the settings whose reader is the process
you ran — or the single call you are making — as opposed to the
daemon's settings, which were fixed when the daemon started. The
distinction is the [Scope](#scope-who-reads-it) section above; this
is what it means when you go to set something.



**Client-scoped** settings govern how the CLI finds and treats a
daemon: `daemon.endpoint` (a daemon somewhere else instead of the
local socket), `daemon.autostart` and the `autostart.*` family
(how a started daemon is launched and waited for), `daemon.inline`
(the last-resort in-process mode). Setting one of these *on the
daemon* — recording it as a value the daemon holds — does nothing
at all: the process that acts on it is the next client, not the
running daemon. Set them in a file, the environment, or a flag,
where a client will read them.

**Call-scoped** settings travel with a request, so they work
against any daemon however it was started: the CLI sends its
effective value in a header and the daemon honours it for that
request only. `mcpx search --limit 3` is `search.limit` arriving
this way; `catalog.budget`, the `artifacts.*` delivery settings,
and `diagnose.preflight` behave the same. Only call-scoped settings
are honoured from that header — widening it would let any client
reconfigure a shared daemon.

A **host** configuring mcpx for its users — an editor plugin, a
harness, a script's launcher — mostly sets client configuration:
the endpoint if it runs its own daemon, the profile if it exposes a
subset, the session identifiers if it multiplexes agents. The
opencode plugin is the worked example: it reads its settings from
the environment directly (the plugin scope), which is why the
variable names it uses are a contract a test pins. What a host does
*not* configure per user is the daemon's behaviour — that is
[Common config](#common-config) and the daemon's own settings, set
once, on purpose.

<a id="env-vars"></a>

## Env vars

Every setting has an environment variable, derived from its path —
`daemon.endpoint` is `MCPX_DAEMON_ENDPOINT`, `search.limit` is
`MCPX_SEARCH_LIMIT` — and the inventory below lists each one beside
its setting, so there is no separate list to keep current. As a
layer, the environment sits above files and below flags: export a
variable to change one value for one shell without editing any
file, and expect a flag on the command line to beat it.

Two spellings of one setting at the same level — two variables
that mean the same setting, carrying different values — are an
error, not a race. The environment has no order, so there is
nothing to prefer, and picking one silently is how a configuration
becomes unexplainable.

Three cautions about what the variables are *for*:

- **Setting a variable is not the same as the script environment.**
  Everything above is input: a person telling mcpx something. The
  names mcpx and the plugin hand *to* a script — its session, its
  endpoint, its socket — are output, a different list with a
  different owner, generated on [The script
  environment](./environment.md) page. Names appear on both sides
  (`MCPX_SESSION_ID` is how a caller tells mcpx its session *and*
  how a script learns it); that page is the authority for the
  script-facing direction.
- **A few names act before any setting exists.** `MCPX_CONFIG`
  selects the configuration file itself, and `MCPX_PATHS_CONFIG`
  (or `--paths-config`) seeds the discovery pass; neither could be
  a setting, because a setting's value is read from the very files
  these choose. They are the whole hand-written remainder: if a
  name is not a setting's derived variable, not in the script
  environment, and not one of these, mcpx does not read it.
- **A variable only reaches the process that reads it.** Exporting
  a daemon-scoped variable after the daemon has started changes
  nothing until it restarts — scope again, not a special property
  of the environment. `mcpx settings list --daemon` shows what the
  daemon actually resolved.

## Local-model controls by surface

These are independent controls, not one fleet-wide Qwen switch. In particular,
parking hosted chat-model listings does not disable embeddings or an interpreter.
The checked-in state below is from this repository's main configuration, not a
guarantee about a running process: nearer files, environment, flags and runtime
overrides can change it. See [#2218](https://github.com/developing-today/code/issues/2218)
for the remaining single-switch work.

| Surface | Control and checked-in state | What it does not control |
| --- | --- | --- |
| mcpx catalog embeddings | `embeddings.backend` defaults to `auto`; `.mcpx.json` sets no `embeddings` block. With no URL or WASM path configured there, the cascade reaches the built-in pure-Go subword engine, not a Qwen model. `local` forces that engine. | Prompt generation, OpenCode messaging and provider listings. `auto` can use a configured remote or WASM model; it is not a global local-model off switch. |
| mcpx prompt generation | `prompt.sample` defaults to `never`; `model.provider` defaults to `none`. Neither is set in `.mcpx.json`. The file does contain a legacy `repair` block naming `opencode` and `claude-code/claude-haiku-5-5`, not Qwen. | Embeddings or arbitrary models chosen by a caller or external OpenCode. |
| OpenCode messaging embeddings | `.opencode/opencode.jsonc` sets `messaging.embedding` to `"off"`. Removing that setting restores the OpenCode default, which starts its Qwen3-Embedding local server. | Chat provider listings or mcpx embeddings. |
| OpenCode chat provider listings | Qwen definitions for `cline-pass` and `commandcode` are parked as comments; other Qwen/Llama listings are under disabled providers or commented blocks. | Messaging embeddings. Restoring the parked blocks requires editing them: there is not yet a single switch restoring all listings. |
| OpenChamber route interpreter | The separate local route-interpreter stage is controlled in OpenChamber Settings; its Qwen2.5 model requires a download and was reported absent on the laptop in the #2218 audit. | OpenCode provider listings and mcpx. This is a separate cross-repository remainder, not inherited state. |

For catalog embeddings, `auto` tries the configured `embeddings.url` remote
endpoint, then a successfully loaded `embeddings.wasmPath`, then the subword
engine. `remote` and `wasm` each fall back to subword on failure; `local` uses
subword directly. A remote endpoint may itself be local (for example Ollama):
the backend name does not say where the model runs. `embeddings.model` names
the remote model, not a local download. These settings select an embedder; they
do not switch semantic search itself off.

For prompt generation, `prompt.sample: never` prevents an unmatched-recipe
request from asking a model by default; `ask` allows it. An explicit request
provider or model, or a non-`none` `model.provider`, also opts in even when
`prompt.sample` is `never`. `model.provider` selects the newer
model layer (`none`, `auto`, `sampling`, `plugin`, `openai`, `anthropic`, or
`google`), with `model.baseURL` and `model.default` for a configured endpoint
and model. `model.providers` is a JSON object keyed by provider name, with
per-provider `baseURL` and explicitly named `apiKeyEnv` entries;
`model.profiles` is a JSON object of named, ordered `provider/model` fallback
chains, optionally bounded by a token budget and per-attempt timeout. Either
`model.default` or a per-operation model override may name a profile or a
provider/model id. mcpx never scans the environment for key names. An
OpenAI-compatible local endpoint can serve Qwen when explicitly configured.
The singular `model.baseURL` and `model.apiKeyEnv` remain the fallback for
`model.provider` and keep single-provider configurations working.
`model.provider: none` alone is not a universal veto:
the legacy scripting provider can resolve an explicit request, environment or
installed CLI when generation is allowed. The legacy `repair.provider` and
`repair.model` keys in the fleet file are not registered settings in this
version, so do not assume that block selects the effective provider. Per-request
`mcpx prompt --provider` / `--model` can also select a provider. None of these
controls constrain independent callers of external services.

Use `mcpx settings get embeddings.backend --daemon`, `mcpx settings get
prompt.sample --daemon`, and `mcpx settings get model.provider --daemon` to
inspect the daemon's resolved settings; check `mcpx config --sources` and the
daemon environment as well. Embedding backends are constructed when settings
are installed, so verify the running daemon after changing configuration rather
than assuming an existing index or embedder has switched. Model provider and
prompt sampling settings are daemon-scoped and cold.

## Servers

Servers are configuration too, and until now adding one meant an editor and a
restart.

| surface | list                | add                                                                    | remove                       |
| ------- | ------------------- | ---------------------------------------------------------------------- | ---------------------------- |
| CLI     | `mcpx servers list` | `mcpx servers add <name> -- <cmd>...`, `mcpx registry add <n> --write` | `mcpx servers remove <name>` |
| `/v1`   | `GET /v1/servers`   | `POST /v1/servers`                                                     | `DELETE /v1/servers/{name}`  |
| MCP     | `mcpx_servers_list` | `mcpx_servers_add`                                                     | `mcpx_servers_remove`        |

The entry is written to a configuration file and the daemon reloads, so the
server is callable immediately and is still there tomorrow. A server whose
process definition did not change keeps its running child: adding one server
must not restart the rest, or a stateful server -- a browser holding a session
-- loses it because a neighbour appeared.

The daemon also re-reads its configuration files when they change on disk,
checked on the same tick that reaps idle instances (`daemon.watchConfig`). An
edit by hand therefore takes effect within `daemon.reapInterval` without
anything being restarted.

Making that work required changing what identifies a daemon. The socket used
to be keyed to the _contents_ of the config files, so that editing one got a
fresh daemon rather than a stale one. A daemon that reloads changes the
contents of the files it is keyed by, which moves its own key, which makes it
unreachable by the client that just edited it -- and hands the next command a
different daemon that happens to match the new key with old servers loaded.
That was observed: `mcpx servers remove` succeeded, the file was correct, and
the next `mcpx servers list` showed the removed server. The key is now the
file _set_, and staleness is handled where it belongs.

### Hiding tools

`tools` (an allowlist) and `excludeTools` (a denylist) go in a server's `mcpx`
block, or in the top-level `pool` block to apply to every server. A hidden
tool is left out of every listing -- `mcpx tools`, generated types, search,
the MCP pass-through -- and a call to it by name is refused with 400.

```json
{
  "pool": { "excludeTools": ["delete_*"] },
  "mcpServers": {
    "datadog": { "command": "mcp-remote", "args": ["..."], "mcpx": { "excludeTools": ["execute_*", "/^(update|upsert)_.*dashboard/"] } }
  }
}
```

Each entry is one of:

| entry                    | means                                                                                  | example                   |
| ------------------------ | -------------------------------------------------------------------------------------- | ------------------------- |
| `/.../`                  | a Go regular expression, unanchored as regexps are                                     | `/delete/`, `/^delete_/`  |
| contains `*`, `?` or `[` | a glob over the whole name: `*` any run, `?` one character, `[a-z]` / `[!a-z]` a class | `delete_*`, `*_monitor`   |
| anything else            | the exact tool name                                                                    | `delete_datadog_workflow` |

A pattern that cannot compile is a configuration error naming the field, not
a rule that silently matches nothing.

### Constraining calls by their arguments

Hiding decides by tool name: a tool is callable with any arguments or not at
all. `argumentRules` decides by the call: a per-server map in the `mcpx`
block -- also allowed in the top-level `pool` block -- whose keys are tool
patterns matched exactly as `tools`/`excludeTools` match (exact name, glob,
or `/regexp/`), and whose values are JSON Schema objects. A call is allowed
only if its arguments validate against **every** rule whose key matches the
tool. Pool rules and server rules add up, and so do rules from layered config
files: rules only tighten, and a nearer file cannot loosen a farther one.

```json
{
  "mcpServers": {
    "github": {
      "command": "...",
      "mcpx": {
        "argumentRules": {
          "create_repository": {
            "required": ["private", "organization"],
            "properties": {
              "private": { "const": true },
              "organization": { "pattern": "^(my-org|my-org-sandbox)$" }
            }
          },
          "/^(push_files|create_or_update_file|delete_file)$/": {
            "required": ["branch"],
            "properties": { "branch": { "not": { "enum": ["main", "master"] } } }
          },
          "*": {
            "properties": { "owner": { "pattern": "^(my-org|my-user)$" } }
          }
        }
      }
    }
  }
}
```

Per-argument rules are the same mechanism: a `"*"` key whose schema is a
`properties` object constrains that argument on every tool that carries it,
and leaves tools without the argument untouched.

A refused call is a 400, as a hidden tool is, before the server is started:
the message names the argument path, the value sent, the constraint it
broke, and where the rule came from (config file, server, key). Over the MCP
pass-through the same refusal arrives as a tool result with `isError: true`.
Listings advertise the tightened schema: the `inputSchema` shown by
`mcpx types`, the generated client, search, and the pass-through's
`tools/list` is the upstream schema `allOf` every matching rule, so an agent
sees `private: true` before it ever calls.

A rule whose key matches no tool on its server warns at load and on catalog
changes -- a server that renames a tool must not silently disable the rule
that guarded it. A value that is not a valid schema, or a `pattern` the
validator cannot compile (JSON Schema patterns are ECMA-262; lookahead and
backreferences are unsupported), is a configuration error naming the server
and key, like a bad tool pattern.

Two cautions, plainly:

- **Server defaults are invisible.** A rule constrains what is *sent*, not
  what the server fills in. If a constraint matters, list the argument in
  `required` -- never rely on a server-side default to satisfy it.
- **Free-form arguments cannot truly be constrained.** A `pattern` over a
  `query`/`jql`/`sql` string matches text, not meaning. A tool whose danger
  lives in a free-form argument belongs in `excludeTools`, not in a rule.

`extraIncludeTools` takes the same entries and adds them to the allowlist
instead of replacing it, so a server can widen a pool-wide `tools` by one tool
without restating the rest:

```json
{
  "pool": { "tools": ["get_*", "list_*", "search_*"] },
  "mcpServers": {
    "datadog": { "mcpx": { "extraIncludeTools": ["create_datadog_monitor"] } }
  }
}
```

With no `tools` anywhere, `extraIncludeTools` is the allowlist -- it includes,
as `tools` does, so everything it does not name is hidden.

`excludeTools` wins over both. A server's own `tools` replaces the pool's;
`extraIncludeTools` and `excludeTools` from the pool and the server all apply,
so a server that hides one more tool does not undo a pool-wide rule. A nearer
config file's `pool.extraIncludeTools` likewise adds to a farther one's. An alias (`aliasOf`) filters
independently of the server it shares a process with.

Names are the server's own tool names, as `mcpx tools` prints them in its
`tool` column, not the generated function names.

Some servers can filter on their side too -- Datadog's `omit_tools` URL
parameter, GitHub's `GITHUB_READ_ONLY` -- which keeps the tool from ever
reaching mcpx. Those are each server's own convention; this one works for any
server.

## Write policy (`policy.write`)

A tool call is a **write** when its tool is not annotated `readOnlyHint`
true, read with the specification's defaults: an unannotated tool may
write, and may be destructive. The daemon-wide setting `policy.write`
(default `ask`) gates every such call, between resolving an instance
and using it:

| mode | what happens to a write call |
| --- | --- |
| `ask` | a confirmation is raised showing the exact call -- the tool and its arguments in canonical (key-sorted) JSON -- and the call proceeds only if somebody answers true |
| `allow` | the call proceeds with no confirmation from the policy |
| `deny` | the call is refused, with an error naming the tool, the mode, and the settings that can change it |

A server's own `writePolicy` in its `mcpx` block overrides the daemon
mode for that server alone:

```json
{ "mcpServers": { "datadog": { "command": "...", "mcpx": { "writePolicy": "deny" } } } }
```

Under `ask`, a caller that cannot answer questions is **refused**, not
waited out and not let through: the error says the policy is ask, that
there is no question broker to answer it, and that setting
`policy.write` to `allow` (or `deny`) is the escape. An unanswered or
refused confirmation refuses the call too.

`elicit.confirmDestructive` is a **floor** on top of this, not an
alternative to it: with it on, a destructive call asks even under
`allow` -- raised by the destructive confirmation itself, with its
annotation-trust rules unchanged -- and the floor never weakens a
`deny`. Under `ask` the policy has already asked once, so no call is
asked about twice.

### Operator-declared read-only tools (`readOnlyTools`)

The gate reads a tool's classification from its annotations, and many
real servers publish none -- fff, whose tools are all searches, is
one. The specification's defaults then make every call a possibly
destructive write, so under `ask` a headless caller (an API session,
a script -- the normal fleet mode) can never run them at all: the
confirmation question has no answerer, and every call comes back as
the gate's error (#6116).

The operator can declare specific tools read-only in a server's
`mcpx` block, by exact name:

```json
{ "mcpServers": { "fff": { "command": "...", "mcpx": { "readOnlyTools": ["find_files", "grep", "multi_grep"] } } } }
```

A declared tool is classified read-only whatever its annotations say
(or do not say): the write policy does not gate it in any mode, and
the destructive floor does not ask about it. The declaration is the
operator's own word in their own configuration, which is why it is
trusted -- a server cannot declare its own tools read-only this way,
and a remote server's annotations still clear nothing (#2191). Names
are exact, never patterns, and the declaration covers only the tools
listed: a tool the server adds later, under a new name, is gated by
the defaults until the operator lists it too. Listing a tool that
actually writes is the operator's mistake to make -- and theirs
alone; nothing infers the list.

When the gate does refuse a call -- an unanswered confirmation, a
denied write -- the refusal is typed, not a generic execution
failure: HTTP 403 with a `code` in the error body
(`confirmation_required` for an unconfirmed call,
`write_policy_denied` for a policy denial), so a caller can tell
"the gate refused this" apart from "the tool ran and failed".

This gate is an operator consent gate, not the autonomy dial
(`docs/decisions/0002-autonomy-dial.md`): the dial bounds what mcpx
does on its own, the write policy bounds what a caller may do to the
outside world through it, and neither clamps the other.

## Every setting

Everything between the markers below is generated from `settings.Registry()` by
`TestTheDocumentationListsEverySetting`, which fails when it is stale:

```
go test ./internal/settings -run TestTheDocumentationListsEverySetting -update
```

It used to say it was generated while being written by hand, and it had
drifted: two settings were missing an alias their flag really answers to.
`mcpx config --schema --plumbing` prints the same inventory, and
`mcpx settings list` prints it with the values that are actually in force.

Entries marked _(plumbing)_ are internals. They work and they are supported,
but there is no ordinary reason to change one, so they are kept out of
ordinary help rather than hidden -- a flag a user can find in the source and
that `--help` denies exists is worse than a long list.

`paths.configFile` is the one setting a configuration file may not set: it
decides which file is read, so it is accepted from the environment and from
the global `--config`, which goes before the command.

`plumbing.strictUnknownKeys` governs keys **no setting claims**, and keys a
server entry declares that mcpx does not know — `type`, `alwaysAllow` and the
rest, which belong to the other hosts that read the same file. Those are
recorded, reported by `mcpx doctor`, and refused only when this is on.

A key inside a server's `"mcpx"` block is not governed by it and is always
refused. That block is mcpx's alone — nothing else writes into it — so a key
it does not know is a typo or a name that no longer exists, not another host's
field. This is deliberately not a setting: it is how `mcpx init` wrote
`"mode": "session"` into every new config for months without anyone noticing,
and a switch to turn the report off would have kept it quiet.

A script whose first line names its interpreter (`#!`) runs under that
interpreter, ahead of every setting below: a `-S`/`--split-string`
payload is re-split with shell quoting rules (so `-S sh -c "printf
hello"` keeps `printf hello` one argument; an unterminated quote fails
the run), `VAR=value` assignments reach the child even when the value
contains `/`, `env -u`/`env -i` apply (the child starts from the parent's
environment minus what they remove, or from an empty one), and
`env -C`/`env --chdir` moves the run, beating `script.cwd` the way the
interpreter choice beats `script.runtime`. An env option mcpx does not
implement fails the run rather than being dropped. A dispatched run
never inherits `MCPX_CLIENT`, `MCPX_RUNTIME`, `MCPX_ENTRY` or
`MCPX_DISPATCH` from an outer run, and its notice names anything else it
goes without (no generated client, no enforced permission profiles,
skipped launcher phases).

<!-- BEGIN GENERATED: settings.Registry(); `go test ./internal/settings -run TestTheDocumentationListsEverySetting -update` -->

### artifacts

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `artifacts.chunkBytes` | bytes | `256KiB` | daemon | no | `--artifacts-chunk-bytes` | `MCPX_ARTIFACTS_CHUNK_BYTES` | *(plumbing)* how much of a body one streamed frame carries |
| `artifacts.delivery` | enum | `reference` | call | yes | `--artifacts-delivery` | `MCPX_ARTIFACTS_DELIVERY` | how artifact bodies reach the caller |
| `artifacts.dir` | string | *(empty)* | call | yes | `--artifacts-dir` | `MCPX_ARTIFACTS_DIR` | write this run's artifacts here |
| `artifacts.enabled` | bool | `true` | daemon | no | `--artifacts-enabled` | `MCPX_ARTIFACTS_ENABLED` | whether the daemon keeps files scripts produce |
| `artifacts.gcInterval` | duration | `10m` | daemon | no | `--artifacts-gc-interval` | `MCPX_ARTIFACTS_GC_INTERVAL` | *(plumbing)* how often expired artifacts are swept |
| `artifacts.idleTtl` | duration | `0s` | daemon | no | `--artifacts-idle-ttl` | `MCPX_ARTIFACTS_IDLE_TTL` | when set, reading an artifact renews it for this long; off keeps absolute expiry |
| `artifacts.inlineMaxBytes` | bytes | `1MiB` | call | yes | `--artifacts-inline-max-bytes` | `MCPX_ARTIFACTS_INLINE_MAX_BYTES` | the largest artifact that may be base64'd into a result |
| `artifacts.interceptImages` | bool | `true` | call | yes | `--artifacts-intercept-images` | `MCPX_ARTIFACTS_INTERCEPT_IMAGES` | turn image and audio content in a result into artifact references |
| `artifacts.maxAge` | duration | `168h` | daemon | no | `--artifacts-max-age` | `MCPX_ARTIFACTS_MAX_AGE` | the absolute ceiling on a renewed artifact's life, however often it is read |
| `artifacts.maxBytes` | bytes | `64MiB` | daemon | no | `--artifacts-max-bytes` | `MCPX_ARTIFACTS_MAX_BYTES` | the largest single artifact that may be stored |
| `artifacts.quota` | bytes | `1GiB` | daemon | no | `--artifacts-quota` | `MCPX_ARTIFACTS_QUOTA` | the total the artifact store may hold |
| `artifacts.ttl` | duration | `24h` | daemon | no | `--artifacts-ttl` | `MCPX_ARTIFACTS_TTL` | how long an artifact is kept before it is collected |

### autonomy

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `autonomy.max` | enum | `run` | daemon | no | `--autonomy-max` | `MCPX_AUTONOMY_MAX` | the most autonomy this daemon exercises, whatever a caller asks for |

### autostart

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `autostart.args` | list | `daemon,--detached` | client | no | `--autostart-args` | `MCPX_AUTOSTART_ARGS` | *(plumbing)* the arguments an auto-started daemon is given |
| `autostart.bin` | string | *(empty)* | client | no | `--autostart-bin` | `MCPX_AUTOSTART_BIN` | which executable is started as the daemon |
| `autostart.connectTimeout` | duration | `20s` | client | no | `--autostart-connect-timeout` | `MCPX_AUTOSTART_CONNECT_TIMEOUT` | how long to wait for a started daemon to answer |
| `autostart.idleExit` | duration | `4h0m0s` | client | no | `--autostart-idle-exit` | `MCPX_AUTOSTART_IDLE_EXIT` | how long an auto-started daemon survives with nothing to do |
| `autostart.logTail` | bytes | `2000` | client | no | `--autostart-log-tail` | `MCPX_AUTOSTART_LOG_TAIL` | *(plumbing)* how much of the daemon log is shown when it will not start |
| `autostart.pingTimeout` | duration | `2s` | client | no | `--autostart-ping-timeout` | `MCPX_AUTOSTART_PING_TIMEOUT` | *(plumbing)* how long a health check waits before calling it dead |
| `autostart.pollInterval` | duration | `50ms` | client | no | `--autostart-poll-interval` | `MCPX_AUTOSTART_POLL_INTERVAL` | *(plumbing)* how often a starting daemon is probed |

### catalog

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `catalog.bias` | list | *(empty)* | call | yes | `--catalog-bias`, `--bias` | `MCPX_CATALOG_BIAS` | words that pull matching tools toward the front |
| `catalog.budget` | int | `2000` | call | yes | `--catalog-budget`, `--budget` | `MCPX_CATALOG_BUDGET` | token ceiling for the catalog listing |
| `catalog.instructions` | bool | `true` | call | yes | `--catalog-instructions` | `MCPX_CATALOG_INSTRUCTIONS` | include each server's own instructions |
| `catalog.pinnedTools` | list | *(empty)* | daemon | no | `--catalog-pinned-tools` | `MCPX_CATALOG_PINNED_TOOLS` | tools always exposed in reactive mode |

### completion

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `completion.maxValues` | int | `100` | daemon | yes | `--completion-max-values` | `MCPX_COMPLETION_MAX_VALUES` | *(plumbing)* how many completions one reply carries |
| `completion.perMinute` | int | `120` | daemon | yes | `--completion-per-minute` | `MCPX_COMPLETION_PER_MINUTE` | *(plumbing)* how many completion requests one caller may make against one server per minute; 0 disables |
| `completion.serverPerMinute` | int | `600` | daemon | yes | `--completion-server-per-minute` | `MCPX_COMPLETION_SERVER_PER_MINUTE` | *(plumbing)* how many completion requests one server may receive per minute across all callers; 0 disables |

### daemon

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `daemon.address` | string | `127.0.0.1` | daemon | no | `--daemon-address`, `--address` | `MCPX_DAEMON_ADDRESS` | which interface the daemon listens on |
| `daemon.autostart` | bool | `true` | client | no | `--daemon-autostart` | `MCPX_DAEMON_AUTOSTART` | start the daemon on demand when it is not running |
| `daemon.endpoint` | string | *(empty)* | client | no | `--daemon-endpoint` | `MCPX_DAEMON_ENDPOINT` | a daemon somewhere else, instead of the local socket |
| `daemon.idleExit` | duration | `0s` | daemon | no | `--daemon-idle-exit`, `--idle-exit` | `MCPX_DAEMON_IDLE_EXIT` | stop the daemon after this long with nothing to do |
| `daemon.inline` | bool | `false` | client | no | `--daemon-inline` | `MCPX_DAEMON_INLINE` | as a last resort, run servers inside this process |
| `daemon.inlineStartPoll` | duration | `20ms` | client | no | `--daemon-inline-start-poll` | `MCPX_DAEMON_INLINE_START_POLL` | *(plumbing)* how often a starting in-process daemon is probed |
| `daemon.inlineStartTimeout` | duration | `5s` | client | no | `--daemon-inline-start-timeout` | `MCPX_DAEMON_INLINE_START_TIMEOUT` | *(plumbing)* how long an in-process daemon has to become reachable |
| `daemon.leaseTTL` | duration | `30m0s` | daemon | yes | `--daemon-lease-ttl` | `MCPX_DAEMON_LEASE_TTL` | *(plumbing)* how long a silent caller's instances are remembered |
| `daemon.port` | int | `0` | daemon | no | `--daemon-port`, `--port` | `MCPX_DAEMON_PORT` | listen on a TCP port instead of choosing one |
| `daemon.probeTimeout` | duration | `200ms` | client | no | `--daemon-probe-timeout` | `MCPX_DAEMON_PROBE_TIMEOUT` | *(plumbing)* how long another daemon's socket is given to answer |
| `daemon.reapInterval` | duration | `30s` | daemon | no | `--daemon-reap-interval` | `MCPX_DAEMON_REAP_INTERVAL` | *(plumbing)* how often idle instances are swept |
| `daemon.refreshTimeout` | duration | `3m0s` | daemon | yes | `--daemon-refresh-timeout` | `MCPX_DAEMON_REFRESH_TIMEOUT` | how long POST /v1/refresh may take |
| `daemon.roots` | paths | *(empty)* | daemon | no | `--daemon-roots` | `MCPX_DAEMON_ROOTS` | filesystem paths exposed to upstream servers through roots/list |
| `daemon.saveInterval` | duration | `5m` | daemon | no | `--daemon-save-interval` | `MCPX_DAEMON_SAVE_INTERVAL` | *(plumbing)* how often daemon state is written to disk |
| `daemon.socketProbeTimeout` | duration | `500ms` | daemon | no | `--daemon-socket-probe-timeout` | `MCPX_DAEMON_SOCKET_PROBE_TIMEOUT` | *(plumbing)* how long a socket left by a crashed daemon is given to answer |
| `daemon.takeoverTimeout` | duration | `15s` | daemon | no | `--daemon-takeover-timeout` | `MCPX_DAEMON_TAKEOVER_TIMEOUT` | *(plumbing)* how long a handoff may take, and how long the old daemon waits for its successor to hang up |
| `daemon.tsnet.hostname` | string | *(empty)* | daemon | no | `--daemon-tsnet-hostname` | `MCPX_DAEMON_TSNET_HOSTNAME` | also serve the daemon on a tailnet, under this name |
| `daemon.tsnetUpTimeout` | duration | `30s` | daemon | no | `--daemon-tsnet-up-timeout` | `MCPX_DAEMON_TSNET_UP_TIMEOUT` | *(plumbing)* how long a node may take to authenticate |
| `daemon.ui` | bool | `true` | daemon | yes | `--daemon-ui`, `--ui` | `MCPX_DAEMON_UI` | serve the dashboard page at /dashboard |
| `daemon.uiDir` | string | *(empty)* | daemon | yes | `--daemon-ui-dir`, `--ui-dir` | `MCPX_DAEMON_UI_DIR` | serve your own pages instead of the embedded dashboard |
| `daemon.warm` | bool | `true` | daemon | no | `--daemon-warm`, `--warm` | `MCPX_DAEMON_WARM` | read every server's schemas in the background at startup |
| `daemon.warmTimeout` | duration | `3m0s` | daemon | no | `--daemon-warm-timeout` | `MCPX_DAEMON_WARM_TIMEOUT` | *(plumbing)* how long the background schema fetch may take |
| `daemon.watchConfig` | bool | `true` | daemon | yes | `--daemon-watch-config` | `MCPX_DAEMON_WATCH_CONFIG` | re-read the configuration files when they change on disk |

### deps

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `deps.autoInstall` | enum | `off` | client | no | `--deps-auto-install` | `MCPX_DEPS_AUTO_INSTALL` | whether the runtime may fetch a missing package on its own |
| `deps.manifest` | source | *(empty)* | client | no | `--deps-manifest` | `MCPX_DEPS_MANIFEST` | the package.json or package.jsonc naming a script's packages |
| `deps.packages` | list | *(empty)* | client | no | `--deps-packages`, `--package` | `MCPX_DEPS_PACKAGES` | packages a script may import, as name or name@version |
| `deps.source` | enum | `host` | client | no | `--deps-source` | `MCPX_DEPS_SOURCE` | where a script's packages come from when nothing more specific says |

### diagnose

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `diagnose.history` | int | `8` | daemon | no | `--diagnose-history` | `MCPX_DIAGNOSE_HISTORY` | *(plumbing)* how many changes are kept per tool |
| `diagnose.preflight` | bool | `true` | call | yes | `--diagnose-preflight` | `MCPX_DIAGNOSE_PREFLIGHT` | check a script's tool calls against the live schemas first |

### doctor

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `doctor.timeout` | duration | `20s` | client | no | `--doctor-timeout` | `MCPX_DOCTOR_TIMEOUT` | how long `mcpx doctor` gives the daemon to answer |

### elicit

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `elicit.askTimeout` | duration | `45s` | daemon | no | `--elicit-ask-timeout` | `MCPX_ELICIT_ASK_TIMEOUT` | how long mcpx waits for an answer to a question it raised itself |
| `elicit.confirmDestructive` | bool | `false` | daemon | no | `--elicit-confirm-destructive` | `MCPX_ELICIT_CONFIRM_DESTRUCTIVE` | ask before a call to a tool that may be destructive |
| `elicit.disambiguate` | enum | `never` | daemon | no | `--elicit-disambiguate` | `MCPX_ELICIT_DISAMBIGUATE` | ask which instance of a stateful server to use when several are live and the script did not choose |
| `elicit.disambiguateDefault` | enum | `new` | daemon | no | `--elicit-disambiguate-default` | `MCPX_ELICIT_DISAMBIGUATE_DEFAULT` | which instance is used when nobody answers in time: the one the scope would have chosen, or the most recently used |
| `elicit.maxAge` | duration | `30m` | daemon | no | `--elicit-max-age` | `MCPX_ELICIT_MAX_AGE` | the absolute ceiling on a question's life, however often it is read |
| `elicit.pendingLimit` | int | `100` | daemon | yes | `--elicit-pending-limit` | `MCPX_ELICIT_PENDING_LIMIT` | *(plumbing)* how many unanswered questions one listing returns |
| `elicit.ttl` | duration | `120s` | daemon | no | `--elicit-ttl` | `MCPX_ELICIT_TTL` | how long a question waits for an answer, renewed by use |

### embeddings

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `embeddings.apiKey` | string | *(empty)* | daemon | yes | `--embeddings-api-key` | `MCPX_EMBEDDINGS_API_KEY` | API key for remote embeddings endpoint |
| `embeddings.backend` | string | `auto` | daemon | yes | `--embeddings-backend` | `MCPX_EMBEDDINGS_BACKEND` | which embedding engine to use (auto, local, remote, wasm) |
| `embeddings.model` | string | `text-embedding-3-small` | daemon | yes | `--embeddings-model` | `MCPX_EMBEDDINGS_MODEL` | model name for remote embeddings endpoint |
| `embeddings.url` | string | *(empty)* | daemon | yes | `--embeddings-url` | `MCPX_EMBEDDINGS_URL` | OpenAI-compatible embeddings endpoint URL |
| `embeddings.wasmPath` | string | *(empty)* | daemon | yes | `--embeddings-wasm-path` | `MCPX_EMBEDDINGS_WASM_PATH` | path to compiled WebAssembly neural embedding model |

### events

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `events.history` | int | `1024` | daemon | no | `--events-history` | `MCPX_EVENTS_HISTORY` | how many past events a late subscriber can replay |
| `events.reconnect` | duration | `2s` | client | no | `--events-reconnect` | `MCPX_EVENTS_RECONNECT` | how long a client waits before resuming a dropped stream |

### exec

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `exec.maxOutputBytes` | bytes | `64KiB` | call | yes | `--exec-max-output-bytes` | `MCPX_EXEC_MAX_OUTPUT_BYTES` | cap the stdout a run returns, with a truncation marker |
| `exec.output` | enum | `text` | call | yes | `--exec-output`, `--output` | `MCPX_EXEC_OUTPUT` | text for a terminal, structured for one document, stream for frames |
| `exec.timeout` | duration | `120s` | call | yes | `--exec-timeout` | `MCPX_EXEC_TIMEOUT` | kill a script after this long |
| `exec.where` | enum | `auto` | call | yes | `--exec-where` | `MCPX_EXEC_WHERE` | in this process, or on the daemon |

### hooks

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `hooks` | string | *(empty)* | daemon | no | `--hooks` | `MCPX_HOOKS` | config-declared hooks over the event bus: a JSON array of hook objects |
| `hooks.autonomy` | enum | `run` | call | yes | `--hooks-autonomy` | `MCPX_HOOKS_AUTONOMY` | whether configured script hooks (before, onSuccess, onError) run |
| `hooks.timeout` | duration | `2s` | daemon | no | `--hooks-timeout` | `MCPX_HOOKS_TIMEOUT` | the hard timeout for a declared hook that does not set its own |

### http

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `http.bodyLimit` | bytes | `1048576` | daemon | yes | `--http-body-limit` | `MCPX_HTTP_BODY_LIMIT` | *(plumbing)* the ceiling on an ordinary /v1 request body |
| `http.callBodyLimit` | bytes | `67108864` | daemon | yes | `--http-call-body-limit` | `MCPX_HTTP_CALL_BODY_LIMIT` | the ceiling on a POST /v1/call body |
| `http.controlBodyLimit` | bytes | `65536` | daemon | yes | `--http-control-body-limit` | `MCPX_HTTP_CONTROL_BODY_LIMIT` | *(plumbing)* the ceiling on a body that should hold one field |
| `http.idleConnTimeout` | duration | `30s` | client | no | `--http-idle-conn-timeout` | `MCPX_HTTP_IDLE_CONN_TIMEOUT` | *(plumbing)* how long an unused connection is kept |
| `http.idleConns` | int | `8` | client | no | `--http-idle-conns` | `MCPX_HTTP_IDLE_CONNS` | *(plumbing)* idle connections kept to a local daemon |
| `http.readHeaderTimeout` | duration | `10s` | daemon | no | `--http-read-header-timeout` | `MCPX_HTTP_READ_HEADER_TIMEOUT` | *(plumbing)* how long a client has to finish sending its headers |
| `http.remoteIdleConns` | int | `16` | client | no | `--http-remote-idle-conns` | `MCPX_HTTP_REMOTE_IDLE_CONNS` | *(plumbing)* idle connections kept to a daemon over the network |
| `http.requestTimeout` | duration | `10m0s` | client | no | `--http-request-timeout` | `MCPX_HTTP_REQUEST_TIMEOUT` | how long a CLI request to the daemon may take |
| `http.shutdownGrace` | duration | `5s` | daemon | yes | `--http-shutdown-grace` | `MCPX_HTTP_SHUTDOWN_GRACE` | how long in-flight requests have when the daemon stops |
| `http.ssePing` | duration | `15s` | daemon | yes | `--http-sse-ping` | `MCPX_HTTP_SSE_PING` | *(plumbing)* how often a comment is sent on an idle event stream |
| `http.sseRetry` | duration | `2s` | daemon | yes | `--http-sse-retry` | `MCPX_HTTP_SSE_RETRY` | *(plumbing)* how long a dropped subscriber is told to wait |
| `http.streamBufferInit` | bytes | `65536` | client | no | `--http-stream-buffer-init` | `MCPX_HTTP_STREAM_BUFFER_INIT` | *(plumbing)* the initial line buffer when reading an event stream |
| `http.streamBufferMax` | bytes | `8388608` | client | no | `--http-stream-buffer-max` | `MCPX_HTTP_STREAM_BUFFER_MAX` | *(plumbing)* the largest single event line that will be read |

### logging

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `logging.dir` | string | *(empty)* | daemon | no | `--logging-dir`, `--log-dir` | `MCPX_LOGGING_DIR` | where the JSONL files are written |
| `logging.file` | bool | `true` | daemon | no | `--logging-file` | `MCPX_LOGGING_FILE` | whether the durable JSONL log is written at all |
| `logging.format` | enum | `text` | client | no | `--logging-format`, `--format` | `MCPX_LOGGING_FORMAT` | how records are rendered |
| `logging.include` | list | `host,user,process,version` | daemon | no | `--logging-include`, `--include` | `MCPX_LOGGING_INCLUDE` | which context blocks are attached to lifecycle records |
| `logging.keep` | int | `8` | daemon | no | `--logging-keep`, `--keep` | `MCPX_LOGGING_KEEP` | how many rolled files to keep |
| `logging.level` | enum | `info` | daemon | yes | `--logging-level`, `--log-level` | `MCPX_LOGGING_LEVEL`, `MCPX_LOG_LEVEL` | the lowest level that is kept |
| `logging.maxAge` | duration | `24h` | daemon | no | `--logging-max-age` | `MCPX_LOGGING_MAX_AGE` | roll the log file once it is this old |
| `logging.maxBytes` | bytes | `16MB` | daemon | no | `--logging-max-bytes` | `MCPX_LOGGING_MAX_BYTES` | roll the log file once it reaches this size |
| `logging.maxLines` | int | `0` | daemon | no | `--logging-max-lines` | `MCPX_LOGGING_MAX_LINES` | roll the log file once it holds this many records |
| `logging.source` | enum | `warn` | daemon | yes | `--logging-source`, `--log-source` | `MCPX_LOGGING_SOURCE` | from which level upward to record the calling file and line |
| `logging.trace` | bool | `false` | daemon | no | `--logging-trace` | `MCPX_TRACE` | log which instance served every tool call |

### logstore

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `logstore.followBacklog` | int | `1000` | client | no | `--logstore-follow-backlog` | `MCPX_LOGSTORE_FOLLOW_BACKLOG` | *(plumbing)* how many records one poll of `mcpx log --follow` may emit |
| `logstore.queryLimit` | int | `100` | call | yes | `--logstore-query-limit` | `MCPX_LOGSTORE_QUERY_LIMIT` | how many records a log query returns by default |

### mcp

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mcp.pageSize` | int | `100` | client | no | `--mcp-page-size` | `MCPX_MCP_PAGE_SIZE` | how many items one tools/list reply carries |
| `mcp.passthrough` | string | *(empty)* | client | no | `--mcp-passthrough`, `--passthrough` | `MCPX_MCP_PASSTHROUGH` | serve upstreams' tools, prompts and resources under their own names |

### model

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `model.apiKeyEnv` | string | *(empty)* | daemon | no | `--model-api-key-env` | `MCPX_MODEL_API_KEY_ENV` | the environment variable holding the configured provider's API key |
| `model.baseURL` | string | *(empty)* | daemon | no | `--model-base-url` | `MCPX_MODEL_BASE_URL` | the OpenAI-compatible endpoint the configured provider calls |
| `model.budget` | int | `0` | daemon | no | `--model-budget` | `MCPX_MODEL_BUDGET` | hard ceiling on the tokens one generation through the layer may ask for; 0 is no ceiling |
| `model.default` | string | *(empty)* | daemon | no | `--model-default` | `MCPX_MODEL_DEFAULT` | the model id used when an operation names none, as provider/model |
| `model.profiles` | string | `{}` | daemon | no | `--model-profiles` | `MCPX_MODEL_PROFILES` | JSON object of named ordered provider/model fallback chains |
| `model.provider` | enum | `none` | daemon | no | `--model-provider` | `MCPX_MODEL_PROVIDER` | where mcpx's own generation gets a model: a configured provider, sampling through the caller, or none |
| `model.providers` | string | `{}` | daemon | no | `--model-providers` | `MCPX_MODEL_PROVIDERS` | JSON object mapping provider names to baseURL and apiKeyEnv |
| `model.timeout` | duration | `1m30s` | daemon | no | `--model-timeout` | `MCPX_MODEL_TIMEOUT` | how long one generation through the layer may take |

### output

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `output.color` | enum | `auto` | client | no | `--output-color` | `MCPX_OUTPUT_COLOR` | whether to colourise the browser |
| `output.json` | bool | `false` | client | no | `--output-json`, `--json` | `MCPX_OUTPUT_JSON` | emit one machine-readable document |

### paths

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `paths.adapters` | paths | *(empty)* | client | no | `--paths-adapters` | `MCPX_PATHS_ADAPTERS` | files declaring command-line programs as MCP servers |
| `paths.apis` | paths | *(empty)* | client | no | `--paths-apis` | `MCPX_PATHS_APIS` | files naming OpenAPI documents to expose as tools |
| `paths.cache` | string | *(empty)* | client | no | `--paths-cache` | `MCPX_PATHS_CACHE`, `MCPX_CACHE_DIR` | where generated clients and schemas are kept |
| `paths.config` | paths | *(empty)* | client | no | `--paths-config` | `MCPX_PATHS_CONFIG` | where configuration files are looked for |
| `paths.configFile` | string | *(empty)* | client | no | `--config` | `MCPX_CONFIG` | read exactly this configuration file instead of searching |
| `paths.deps` | paths | *(empty)* | client | no | `--paths-deps` | `MCPX_PATHS_DEPS` | where a script's package.json / package.jsonc is looked for |
| `paths.placeholders` | paths | *(empty)* | client | no | `--paths-placeholders` | `MCPX_PATHS_PLACEHOLDERS` | directories of files declaring launcher placeholders |
| `paths.plugins` | paths | *(empty)* | client | no | `--paths-plugins` | `MCPX_PATHS_PLUGINS` | directories of plugins: an mcpx-plugin.json manifest, or executables loaded as zero-manifest plugins |
| `paths.scripts` | paths | *(empty)* | client | no | `--paths-scripts` | `MCPX_PATHS_SCRIPTS` | where named scripts are looked for |
| `paths.state` | string | *(empty)* | client | no | `--paths-state` | `MCPX_PATHS_STATE`, `MCPX_STATE_DIR` | where the daemon socket, logs and index live |

### plugin

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `plugin.annotate` | bool | `true` | plugin | no | `--plugin-annotate` | `MCPX_PLUGIN_ANNOTATE` | add mcpx's own notes to a tool result |
| `plugin.backend` | enum | `auto` | plugin | no | `--plugin-backend` | `MCPX_PLUGIN_BACKEND` | whether the plugin talks to the daemon or spawns the binary |
| `plugin.bin` | string | `mcpx` | plugin | no | `--plugin-bin` | `MCPX_PLUGIN_BIN` | the executable the plugin invokes |
| `plugin.binArgs` | list | *(empty)* | plugin | no | `--plugin-bin-args` | `MCPX_PLUGIN_BIN_ARGS` | arguments put in front of every plugin invocation of mcpx |
| `plugin.daemonTools` | enum | `auto` | plugin | no | `--plugin-daemon-tools` | `MCPX_PLUGIN_DAEMON_TOOLS` | offer the tools that list and choose between daemons |
| `plugin.disabledTools` | list | *(empty)* | plugin | no | `--plugin-disabled-tools` | `MCPX_PLUGIN_DISABLED_TOOLS` | model-facing mcpx tools hidden from every session |
| `plugin.discoveryRetry` | duration | `1m0s` | plugin | no | `--plugin-discovery-retry` | `MCPX_PLUGIN_DISCOVERY_RETRY` | how long the plugin waits before looking for mcpx again |
| `plugin.env` | enum | `full` | plugin | no | `--plugin-env` | `MCPX_PLUGIN_ENV` | how much session context is put into each command's environment |
| `plugin.headless` | enum | `auto` | plugin | no | `--plugin-headless` | `MCPX_PLUGIN_HEADLESS` | suppress toasts, because nobody is looking at a screen |
| `plugin.instructions` | bool | `false` | plugin | no | `--plugin-instructions` | `MCPX_PLUGIN_INSTRUCTIONS` | add mcpx usage guidance to the system prompt |
| `plugin.remember` | enum | `session` | plugin | no | `--plugin-remember` | `MCPX_PLUGIN_REMEMBER` | how long a chosen daemon stays chosen |
| `plugin.toolTiming` | bool | `false` | plugin | no | `--plugin-tool-timing` | `MCPX_PLUGIN_TOOL_TIMING` | write every tool outcome into mcpx's log |
| `plugin.tools` | bool | `false` | plugin | no | `--plugin-tools` | `MCPX_PLUGIN_TOOLS` | expose mcpx itself as tools the model can call |

### plumbing

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `plumbing.allowTsJsOverlap` | bool | `false` | client | no | `--plumbing-allow-ts-js-overlap` | `MCPX_PLUMBING_ALLOW_TS_JS_OVERLAP` | *(plumbing)* permit a script directory holding both foo.ts and foo.js |
| `plumbing.indexOnQuery` | bool | `true` | daemon | no | `--plumbing-index-on-query` | `MCPX_PLUMBING_INDEX_ON_QUERY` | *(plumbing)* bring the log index up to date before answering a query |
| `plumbing.launcherPlaceholderRepeat` | list | *(empty)* | client | no | `--plumbing-launcher-placeholder-repeat` | `MCPX_PLUMBING_LAUNCHER_PLACEHOLDER_REPEAT` | *(plumbing)* launcher placeholders permitted to resolve more than once |
| `plumbing.sourceDirAllowed` | bool | `true` | client | no | `--plumbing-source-dir-allowed` | `MCPX_PLUMBING_SOURCE_DIR_ALLOWED` | *(plumbing)* whether a directory may stand in for a source string at all |
| `plumbing.sourceDirRecursive` | bool | `false` | client | no | `--plumbing-source-dir-recursive` | `MCPX_PLUMBING_SOURCE_DIR_RECURSIVE` | *(plumbing)* when a directory is given as source, descend into subdirectories |
| `plumbing.sourceProbePaths` | bool | `true` | client | no | `--plumbing-source-probe-paths` | `MCPX_PLUMBING_SOURCE_PROBE_PATHS` | *(plumbing)* treat a source argument that names an existing file as a file |
| `plumbing.strictUnknownKeys` | bool | `false` | client | no | `--plumbing-strict-unknown-keys` | `MCPX_PLUMBING_STRICT_UNKNOWN_KEYS` | *(plumbing)* fail on a configuration key no setting claims |
| `plumbing.validatePaths` | bool | `true` | client | no | `--plumbing-validate-paths` | `MCPX_PLUMBING_VALIDATE_PATHS` | *(plumbing)* resolve and verify every referenced path before doing any work |

### policy

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `policy.write` | enum | `ask` | daemon | no | `--policy-write` | `MCPX_POLICY_WRITE` | what to do before a call that may write: ask for confirmation, allow it, or deny it |

### pool

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `pool.callTimeout` | duration | `120s` | daemon | yes | `--pool-call-timeout` | `MCPX_POOL_CALL_TIMEOUT` | how long one tool call may take |
| `pool.idleTimeout` | duration | `5m` | daemon | yes | `--pool-idle-timeout` | `MCPX_POOL_IDLE_TIMEOUT` | how long an unused server lingers before it is stopped |
| `pool.max` | int | `4` | daemon | yes | `--pool-max` | `MCPX_POOL_MAX` | how many copies of one server may run at once |
| `pool.min` | int | `0` | daemon | yes | `--pool-min` | `MCPX_POOL_MIN` | how many copies to keep started even when idle |
| `pool.scope` | enum | `global` | daemon | no | `--pool-scope` | `MCPX_POOL_SCOPE` | what counts as the same caller for sharing purposes |
| `pool.sharing` | enum | `shared` | daemon | no | `--pool-sharing` | `MCPX_POOL_SHARING` | whether callers reuse one instance or each get their own |
| `pool.startTimeout` | duration | `60s` | daemon | yes | `--pool-start-timeout` | `MCPX_POOL_START_TIMEOUT` | how long a server has to become ready |

### preset

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `preset` | list | *(empty)* | client | no | `--preset` | `MCPX_PRESET` | which presets to apply, in order |

### presets

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `presets` | string | *(empty)* | client | no | `--presets` | `MCPX_PRESETS` | named flag bundles: {name: ["--json", "--timeout=30s"]} |

### prompt

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `prompt.autonomy` | enum | `propose` | call | yes | `--prompt-autonomy` | `MCPX_PROMPT_AUTONOMY` | whether a request returns the script (propose) or runs it (run) |
| `prompt.catalogBudget` | int | `1200` | call | yes | `--prompt-catalog-budget` | `MCPX_PROMPT_CATALOG_BUDGET` | approximate token ceiling for the slice of catalog sent with a sampling request |
| `prompt.maxTokens` | int | `1500` | call | yes | `--prompt-max-tokens` | `MCPX_PROMPT_MAX_TOKENS` | *(plumbing)* maxTokens on the sampling request |
| `prompt.runTimeout` | duration | `5m0s` | call | yes | `--prompt-run-timeout` | `MCPX_PROMPT_RUN_TIMEOUT` | how long a recipe or generated script may run |
| `prompt.sample` | enum | `never` | daemon | no | `--prompt-sample` | `MCPX_PROMPT_SAMPLE` | whether a request with no matching recipe may ask the caller's model to write one |
| `prompt.sampleTimeout` | duration | `1m30s` | daemon | no | `--prompt-sample-timeout` | `MCPX_PROMPT_SAMPLE_TIMEOUT` | how long a generation request waits for a model |

### proto

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `proto.askPoll` | duration | `500ms` | daemon | no | `--proto-ask-poll` | `MCPX_PROTO_ASK_POLL` | *(plumbing)* how long one wait for a question may block |
| `proto.askRounds` | int | `8` | daemon | no | `--proto-ask-rounds` | `MCPX_PROTO_ASK_ROUNDS` | how many times one request may come back asking for more |
| `proto.askTTL` | duration | `15m` | daemon | no | `--proto-ask-ttl` | `MCPX_PROTO_ASK_TTL` | *(plumbing)* how long the daemon keeps a call waiting for an answer |
| `proto.askTimeout` | duration | `10m` | daemon | no | `--proto-ask-timeout` | `MCPX_PROTO_ASK_TIMEOUT` | how long one request may be held while a question goes unanswered |
| `proto.elicitationPerMinute` | int | `30` | daemon | no | `--proto-elicitation-per-minute` | `MCPX_PROTO_ELICITATION_PER_MINUTE` | max elicitation requests one upstream server may make per minute, across all its sessions |
| `proto.mcpPath` | string | `/mcp` | daemon | no | `--proto-mcp-path` | `MCPX_PROTO_MCP_PATH` | where the daemon serves MCP |
| `proto.native` | bool | `true` | daemon | no | `--proto-native` | `MCPX_PROTO_NATIVE` | put an upstream server's questions to mcpx's own MCP client |
| `proto.samplingPerMinute` | int | `60` | daemon | no | `--proto-sampling-per-minute` | `MCPX_PROTO_SAMPLING_PER_MINUTE` | max sampling requests one upstream server may make per minute, across all its sessions |
| `proto.samplingSessionPerMinute` | int | `20` | daemon | no | `--proto-sampling-session-per-minute` | `MCPX_PROTO_SAMPLING_SESSION_PER_MINUTE` | max sampling requests one upstream server may make within a single session per minute |
| `proto.serveMCP` | bool | `true` | daemon | no | `--proto-serve-mcp` | `MCPX_PROTO_SERVE_MCP` | mount mcpx's own MCP server on the daemon's listeners |
| `proto.sessionIdle` | duration | `30m` | daemon | no | `--proto-session-idle` | `MCPX_PROTO_SESSION_IDLE` | *(plumbing)* how long an unused Streamable HTTP session is kept |
| `proto.stateTTL` | duration | `30m` | daemon | no | `--proto-state-ttl` | `MCPX_PROTO_STATE_TTL` | *(plumbing)* how long a client may resume an interrupted request with |

### protoMessages

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `protoMessages.listMaxAge` | duration | `1m0s` | daemon | no | `--proto-messages-list-max-age` | `MCPX_PROTO_MESSAGES_LIST_MAX_AGE` | ttlMs on server/discover and every list result sent to a 2026-07-28 client |
| `protoMessages.readMaxAge` | duration | `0s` | daemon | no | `--proto-messages-read-max-age` | `MCPX_PROTO_MESSAGES_READ_MAX_AGE` | ttlMs on resources/read results sent to a 2026-07-28 client |
| `protoMessages.taskAfter` | duration | `2s` | daemon | no | `--proto-messages-task-after` | `MCPX_PROTO_MESSAGES_TASK_AFTER` | how long a tools/call runs in line before a tasks-extension client is handed a task |

### protoTasks

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `protoTasks.cancelGrace` | duration | `5s` | daemon | no | `--proto-tasks-cancel-grace` | `MCPX_PROTO_TASKS_CANCEL_GRACE` | how long cooperative task work may finish after cancellation |
| `protoTasks.maxPerRequestor` | int | `32` | daemon | no | `--proto-tasks-max-per-requestor` | `MCPX_PROTO_TASKS_MAX_PER_REQUESTOR` | how many non-terminal tasks one requestor may hold at once |
| `protoTasks.maxTTL` | duration | `1h0m0s` | daemon | no | `--proto-tasks-max-ttl` | `MCPX_PROTO_TASKS_MAX_TTL` | the longest TTL a task may ask for; longer requests are clamped |
| `protoTasks.perMinute` | int | `120` | daemon | no | `--proto-tasks-per-minute` | `MCPX_PROTO_TASKS_PER_MINUTE` | the tasks/* admission window per requestor; 0 disables it |
| `protoTasks.pollInterval` | duration | `1s` | daemon | no | `--proto-tasks-poll-interval` | `MCPX_PROTO_TASKS_POLL_INTERVAL` | the pollInterval every task carries |

### proxy

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `proxy.timeout` | duration | `10m` | daemon | no | `--proxy-timeout` | `MCPX_PROXY_TIMEOUT` | how long the daemon waits for a chat-completions upstream |

### recipes

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `recipes.limit` | int | `5` | call | yes | `--recipes-limit` | `MCPX_RECIPES_LIMIT` | how many ranked recipes are returned |
| `recipes.matchMargin` | int | `150` | call | yes | `--recipes-match-margin` | `MCPX_RECIPES_MATCH_MARGIN` | how far ahead the best recipe must be, as a percentage of the next one |
| `recipes.minScore` | int | `30` | call | yes | `--recipes-min-score` | `MCPX_RECIPES_MIN_SCORE` | the score below which a request is not considered a match for any recipe |

### registry

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `registry.limit` | int | `20` | call | yes | `--registry-limit` | `MCPX_REGISTRY_LIMIT` | how many registry results are returned |
| `registry.maxPages` | int | `10` | client | no | `--registry-max-pages` | `MCPX_REGISTRY_MAX_PAGES` | *(plumbing)* how many requests one registry search may make |
| `registry.pageSize` | int | `30` | client | no | `--registry-page-size` | `MCPX_REGISTRY_PAGE_SIZE` | *(plumbing)* how many entries are fetched per registry request |
| `registry.timeout` | duration | `30s` | client | no | `--registry-timeout` | `MCPX_REGISTRY_TIMEOUT` | how long a registry request may take |
| `registry.url` | string | `https://registry.modelcontextprotocol.io` | client | no | `--registry-url` | `MCPX_REGISTRY_URL` | where `mcpx registry` looks for servers |

### repair

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `repair.autonomy` | enum | `advise` | call | yes | `--repair-autonomy` | `MCPX_REPAIR_AUTONOMY` | how far mcpx goes about a failed call: off attaches nothing, advise attaches diagnostics |

### retention

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `retention.maxTools` | int | `24` | daemon | no | `--retention-max-tools` | `MCPX_RETENTION_MAX_TOOLS` | maximum active tools in reactive working set |
| `retention.stickyWindow` | duration | `2h` | daemon | no | `--retention-sticky-window` | `MCPX_RETENTION_STICKY_WINDOW` | duration explicitly requested tools remain retained |

### sampling

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `sampling.answerLocally` | bool | `false` | daemon | no | `--sampling-answer-locally` | `MCPX_SAMPLING_ANSWER_LOCALLY` | answer upstream sampling requests with mcpx's configured model provider instead of relaying them to the caller |

### script

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `script.before` | source | *(empty)* | client | no | `--script-before` | `MCPX_SCRIPT_BEFORE` | runs first, ahead of the globals being installed |
| `script.captureConsole` | bool | `true` | client | no | `--script-capture-console` | `MCPX_SCRIPT_CAPTURE_CONSOLE` | route console calls into the log |
| `script.check` | enum | `off` | client | no | `--script-check`, `--script-typecheck`, `--typecheck` | `MCPX_SCRIPT_CHECK`, `MCPX_SCRIPT_TYPECHECK` | check the generated program before running it; path alias: script.typecheck; one of off, types (also on), strict |
| `script.checkOnError` | enum | `fail` | client | no | `--script-check-on-error` | `MCPX_SCRIPT_CHECK_ON_ERROR` | what to do when script checking finds an error |
| `script.checkOnUnsupported` | enum | `warn` | client | no | `--script-check-on-unsupported` | `MCPX_SCRIPT_CHECK_ON_UNSUPPORTED` | what to do when the selected runtime cannot check the script |
| `script.cwd` | enum | `caller` | client | no | `--script-cwd` | `MCPX_SCRIPT_CWD` | where a script runs: caller, script, config or workdir |
| `script.env` | list | *(empty)* | client | no | `--script-env`, `--env` | `MCPX_SCRIPT_ENV` | KEY=VALUE pairs added to the script's environment |
| `script.imports` | enum | `report` | client | no | `--script-imports` | `MCPX_SCRIPT_IMPORTS` | resolve the script's import graph before running it |
| `script.importsCrossProject` | enum | `isolate` | client | no | `--script-imports-cross-project` | `MCPX_SCRIPT_IMPORTS_CROSS_PROJECT` | how to handle a file edge into another project's manifest |
| `script.importsNetwork` | enum | `on` | client | no | `--script-imports-network` | `MCPX_SCRIPT_IMPORTS_NETWORK` | allow registry lookups when a package has no declared version |
| `script.importsOnDynamic` | enum | `warn` | client | no | `--script-imports-on-dynamic` | `MCPX_SCRIPT_IMPORTS_ON_DYNAMIC` | what to do about an import() whose specifier is computed |
| `script.importsOnMissing` | enum | `fail` | client | no | `--script-imports-on-missing` | `MCPX_SCRIPT_IMPORTS_ON_MISSING` | how to handle a package absent from the dependency set |
| `script.interpreter` | string | *(empty)* | client | no | `--script-interpreter`, `--interpreter`, `--shell` | `MCPX_SCRIPT_INTERPRETER` | command line that runs scripts instead of a JavaScript runtime |
| `script.launcher` | source | *(empty)* | client | no | `--script-launcher` | `MCPX_SCRIPT_LAUNCHER` | replace the generated launcher entirely |
| `script.onError` | source | *(empty)* | client | no | `--script-on-error` | `MCPX_SCRIPT_ON_ERROR` | runs when the script throws; the error still propagates |
| `script.onSuccess` | source | *(empty)* | client | no | `--script-on-success` | `MCPX_SCRIPT_ON_SUCCESS` | runs when the script returns without throwing |
| `script.permissions` | string | `all` | client | no | `--script-permissions`, `--permissions` | `MCPX_SCRIPT_PERMISSIONS`, `MCPX_PERMISSIONS` | permission profiles, composed in order, or raw: flags |
| `script.prefix` | source | *(empty)* | client | no | `--script-prefix` | `MCPX_SCRIPT_PREFIX` | runs after globals are installed, before the script |
| `script.profiles` | string | *(empty)* | client | no | `--script-profiles` | `MCPX_SCRIPT_PROFILES` | user profiles: {name: {deno: [flags], node: [flags]}} or {name: "raw flags"} |
| `script.runtime` | string | `auto` | client | no | `--script-runtime`, `--runtime` | `MCPX_SCRIPT_RUNTIME` | which JavaScript runtime executes the script |
| `script.runtimeOrder` | list | `deno,bun,node` | client | no | `--script-runtime-order` | `MCPX_SCRIPT_RUNTIME_ORDER` | the runtimes auto tries, in order |
| `script.runtimes` | string | *(empty)* | client | no | `--script-runtimes` | `MCPX_SCRIPT_RUNTIMES` | named runtime binaries: {name: {kind, bin, args}} |
| `script.suffix` | source | *(empty)* | client | no | `--script-suffix` | `MCPX_SCRIPT_SUFFIX` | runs last on both paths, like a finally |

### search

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `search.limit` | int | `20` | call | yes | `--search-limit`, `--limit`, `--n` | `MCPX_SEARCH_LIMIT` | how many tools a search returns |
| `search.semantic` | bool | `false` | call | yes | `--search-semantic` | `MCPX_SEARCH_SEMANTIC` | rank search results using local dense vector embeddings |

### serve

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `serve.mode` | string | `code-mode` | daemon | no | `--serve-mode`, `--mode` | `MCPX_SERVE_MODE` | server mode: code-mode, reactive, or full |

### session

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `session.releaseTimeout` | duration | `10s` | client | no | `--session-release-timeout` | `MCPX_SESSION_RELEASE_TIMEOUT` | *(plumbing)* how long releasing a finished session may take |

### spec

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `spec.first` | string | *(empty)* | daemon | no | `--mcp-spec` | `MCPX_MCP_SPEC` | move one MCP revision to the front of spec.precedence |
| `spec.lenient` | list | *(empty)* | daemon | no | `--spec-lenient` | `MCPX_SPEC_LENIENT` | MCP revisions not held strictly; every revision not listed is strict |
| `spec.precedence` | list | `2026-07-28,2025-11-25,2025-06-18,2025-03-26,2024-11-05` | daemon | no | `--spec-precedence` | `MCPX_SPEC_PRECEDENCE` | MCP revisions in the order their rules win a conflict, first wins |

### stats

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `stats.top` | int | `20` | call | yes | `--stats-top` | `MCPX_STATS_TOP` | how many rows a ranked statistic shows |

### tasks

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tasks.maxAge` | duration | `1h0m0s` | daemon | yes | `--tasks-max-age` | `MCPX_TASKS_MAX_AGE` | the maximum retention period for a terminal task result, even when repeatedly read |
| `tasks.resultWait` | duration | `2m0s` | call | yes | `--tasks-result-wait` | `MCPX_TASKS_RESULT_WAIT` | how long collecting a task result blocks before giving up |
| `tasks.ttl` | duration | `10m0s` | daemon | yes | `--tasks-ttl` | `MCPX_TASKS_TTL` | how long a finished task's result is kept |

### transport

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `transport.allowedOrigins` | list | *(empty)* | daemon | no | `--transport-allowed-origins` | `MCPX_TRANSPORT_ALLOWED_ORIGINS` | browser Origins, beyond loopback, that may reach /mcp |
| `transport.bearerToken` | string | *(empty)* | daemon | no | `--transport-bearer-token` | `MCPX_TRANSPORT_BEARER_TOKEN` | when set, every /v1 and /mcp request must carry this token as a Bearer credential |
| `transport.sessionMetaKeys` | list | `dev.mcpx/session,ai.opencode/sessionID` | daemon | yes | `--transport-session-meta-keys` | `MCPX_TRANSPORT_SESSION_META_KEYS` | the _meta keys a client may name its session with, first present wins |
| `transport.sseKeepAlive` | duration | `15s` | daemon | no | `--transport-sse-keep-alive` | `MCPX_TRANSPORT_SSE_KEEP_ALIVE` | how often a quiet MCP event stream carries a comment line |
| `transport.stdioDrain` | duration | `30s` | client | no | `--transport-stdio-drain` | `MCPX_TRANSPORT_STDIO_DRAIN` | how long `mcpx serve` finishes in-flight requests after its input closes |

### upstream

| setting | kind | default | scope | hot | flag | variable | governs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `upstream.eraCache` | bool | `true` | daemon | no | `--upstream-era-cache` | `MCPX_UPSTREAM_ERA_CACHE` | remember which era each server configuration speaks, across restarts |
| `upstream.probeTimeout` | duration | `2s` | daemon | no | `--upstream-probe-timeout` | `MCPX_UPSTREAM_PROBE_TIMEOUT` | how long a stdio server/discover may go unanswered before initialize is also sent |
| `upstream.protocol` | enum | `prefer-discover` | daemon | no | `--upstream-protocol` | `MCPX_UPSTREAM_PROTOCOL` | which request to send first to a server that names no protocol of its own; one of prefer-discover (also modern, prefer-modern, prefer-stateless, prefer-newest), prefer-initialize (also legacy, prefer-legacy, prefer-session, prefer-oldest), force-discover (also force-modern, force-stateless), force-initialize (also force-legacy, force-session), follow |

<!-- END GENERATED -->

## Adapter declarations

An entry in a `paths.adapters` file names a program and the subcommands worth
exposing. `command` is normally a binary on `PATH`; `runtime` makes it a
script instead, and resolves the interpreter for you:

```jsonc
{ "adapters": [{
  "name": "reports", "command": "./tools/report.py", "runtime": "py",
  "tools": [{ "name": "daily", "params": [{ "name": "day", "required": true }] }]
}] }
```

`runtime` is `bash`, `js`, `ts` or `py`. bash resolves `bash` (then `sh`), py
resolves `python3` (then `python`), and js/ts use the same deno/bun/node order
`script.runtimeOrder` chooses for `mcpx exec`. This is a separate resolver on
purpose: `script.runtime` names the runtime mcpx itself executes generated
scripts on, not the one a declaration asks for. A relative `command` resolves
against the declaration's `dir`, then `PATH`. Without `runtime`, nothing
changes: `command` is looked up on `PATH` and exec'd directly. `mcpx adapter
check` reports a missing interpreter or script.

### Example: tailscale, no credential

The shipped declaration at `examples/tailscale.json` wraps the local
`tailscale` CLI (#1936). It declares no credential and needs none: the CLI
is already logged in on the machine, and that session is the
authentication. Only two tools are exposed, both over
`tailscale status --json`:

```jsonc
{ "adapters": [{
  "name": "tailscale", "command": "tailscale",
  "tools": [
    { "name": "listTailnet",   "args": ["status", "--json"], "json": true },
    { "name": "listHostnames", "args": ["status", "--json"], "json": true }
  ] }] }
```

Point `paths.adapters` at a copy of that file and the tailnet appears in
`mcpx ls` like any other server. The shipped copy is pinned by
`TestShippedTailscaleAdapterDeclaration`, so it cannot drift from the
shape above.

`mcpx catalog` also ends with the recipe tools in `tools/list`: how many are
exposed against the recipe budget, and which recipes were kept out by a name
collision or the cap.
