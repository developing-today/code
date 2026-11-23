# Built-in commands

The `fleet` binary has five root built-in commands that belong to no domain: `init`, `session bind`, `version`, `capabilities`, and `help`. They are available in every build variant (`fleet`, `fleet-serve`, `fleet-connect`) because they describe the binary and its registry rather than any domain's work.

Two rules govern all five, without exception:

- **Built-ins take no arguments.** A trailing argument is a usage error naming it, exit 2 — never silently discarded. That rule includes `--help`: `fleet version --help` and `fleet capabilities --help` are errors, not help, and `fleet help --help` answers `error: no domain named '--help'`. This is the opposite of the leaf-verb convention elsewhere in the binary, where `<verb> --help` prints that verb's usage and exits 0.
- **Bare `fleet` is an error.** With no subcommand at all, the binary prints a missing-subcommand message and the full help to stderr and exits 2. `fleet --help`, `fleet -h`, and `fleet help` (exactly one word) print the registry help and exit 0. `fleet --version` and `fleet -V` print the short version form (see `version` below).

A domain an invocation cannot use answers with a stub on stderr and exit 69 (`EX_UNAVAILABLE`). There are four stub wordings, one per reason class: the domain is not in this build, the invocation name (`fleet-serve` / `fleet-connect`) narrowed the surface away, the `[domains]` table in fleet config disabled the domain on this host, or the domain is included but not yet implemented. Every stub's second line is `Enabled capabilities: <comma list>` (`none` when empty). Help marks unavailable domains inline — `[not in this build]`, `[narrowed out by this name]`, `[disabled by config]` — but prints the full registry on every variant: every build knows every domain.

Config resolution lives in one place (`fleet-config`): environment overrides (`FLEET_CONFIG`, `FLEET_BROKER_SOCKET`) beat the config file (`$FLEET_CONFIG`, else `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`), which beats compiled-in defaults. A missing file is not an error; a malformed file — including unknown sections or keys — is a loud error on every path, never a silent fallback.

Sources: [fleet main.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet/src/main.rs), [fleet-registry](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-registry/src/lib.rs), [fleet-config](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-config/src/lib.rs), [fleet-proxy client](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-proxy/src/client.rs), [session state](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet/src/session_state.rs).

## fleet init

```text
Usage: fleet init
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| _(none)_ | — | — | `init` takes no arguments and no flags. Any trailing argument is a usage error, exit 2. The identical command is also spelled `fleet proxy init`; both spellings resolve to one implementation. |

**Behaviour.** `fleet init` autostarts this host's fleet roles if they are not running, per the `[host]` manifest. It first runs session initialisation (see `session bind` below); a session error is exit 2 before any role work. The capability ladder applies before anything else: a build whose usable surface lacks `proxy-serve` can only stub `init`, exit 69.

The manifest resolution order is:

1. **`FLEET_HOST_CONFIG`** — a complete inline `[host]` manifest; wins outright. A manifest with no location is a loud error.
2. **`FLEET_LOCATION`** — names the location; roles come from the config file's `[host]` section only if the file's location matches, else from the built-in map.
3. **The config file's `[host]` section** — `location`, `init_mode`, and `[host.roles]`; the role set is exactly the key set of the roles table, and an unknown role name is a loud parse error, never silently ignored.

Only two built-in locations exist today; a future lab node provisions its own `[host]` section instead of growing the table:

| Location | init_mode | Roles |
|---|---|---|
| `sandbox` | spawn | `proxy`, `secrets-broker` |
| `laptop` | systemd | `laptop-agent`, `queue-broker`, `search`, `secrets-broker` |

The full `[host]` role vocabulary is five names — `proxy`, `queue-broker`, `secrets-broker`, `search`, `laptop-agent` — a superset of the serve roles (it includes `laptop-agent`, which is not a serve role).

- **Unknown location** (no manifest from any source, or a location with no roles): init reports state — every source consulted, plus detected hostname/OS/systemd for the report only (detection never selects roles) — starts nothing, and exits 69. It never guess-starts a role.
- **Manifest skips and the kill switch:** roles the manifest itself disables (`enabled = false`) are reported as skipped with their reason; then the `[domains]` gate is applied and wins over the manifest (the proxy role keys on the `proxy` domain, the daemon roles on `serve`).
- **`spawn` mode (sandbox):** spawn semantics exist for exactly two roles; any other role in a spawn manifest is reported and skipped, never spawned.
  - *proxy:* probe the broker socket and say hello. A live broker (either protocol dialect) means status plus PATH-shim sync and out — a strict no-op: never a spawn, never a restart; init never fights an incumbent broker. Only an unanswered socket spawns `fleet serve proxy` detached, then reports what it started and syncs shims from the new broker's services map.
  - *secrets-broker:* probe its Unix socket; a live answer means status and out. Otherwise spawn `fleet serve secrets-broker` detached (output appended to `secrets-broker.log` beside the socket) with `SECRETS_BROKER_BIND=127.0.0.1:4800` set explicitly — and the broker forces loopback itself when `FLEET_LOCATION=sandbox`, so the sandbox broker is loopback-only twice over. Init then waits (50 × 100 ms probes) for the socket to answer before reporting success.
- **`systemd` mode (laptop):** init never detached-spawns a daemon systemd owns. For each role it reports `fleet-<role>.service` state via `systemctl --user is-active`; an active unit is a no-op; an inactive but existing unit is started via `systemctl --user start`; a missing unit is reported and skipped, not an error; unavailable systemctl is a report, not a spawn.

Exit codes: 0 ok (including the all-roles-already-running no-op) · 2 usage error or session-initialisation error · 69 capability stub, or unknown location (starts nothing).

```text
$ fleet init
proxy: running (broker socket answers; shims synced)
secrets-broker: running (socket answers)
$ echo $?
0
```

## fleet session bind

```text
Usage: fleet session bind [--session-id ID]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--session-id` | session id | resolved (see Behaviour) | The explicit session identity to validate and persist. Both `--session-id ID` and `--session-id=ID` spellings are accepted; at most one value — a second value is the usage error shown in the help dump. |

**Behaviour.** `fleet session bind` validates and persists this worktree's session identity. The identity is an OpenCode session id of the form `ses_` followed by ASCII letters and digits — anything else fails closed.

Resolution precedence: the explicit `--session-id` → `$FLEET_SESSION_ID` → the OpenCode environment → the worktree's marker file. The marker is a plain one-line file `.fleet-session-id` at the git worktree root, so each worktree retains its own active session association. A marker is used only when the session is **live** — checked against the OpenCode server's session endpoint; a stale marker (ended or unverifiable session) is an explicit error naming the file and the id, never a silent fall-through, and an unavailable server is not proof a recorded session is live.

A successful bind sets `FLEET_SESSION_ID` for fleet's child processes and persists the identity to the marker, so a later bare invocation in the same worktree resolves the same session. Every domain invocation runs this initialisation (with no explicit id) before dispatch; `fleet init` runs it explicitly.

Exit codes: 0 ok · 2 usage error, malformed session id, or stale/unverifiable marker.

```text
$ fleet session bind --session-id ses_01J9EXAMPLE0000000000000
session bound: ses_01J9EXAMPLE0000000000000 (worktree marker .fleet-session-id)
$ fleet session bind --session-id not-a-session
fleet session: invalid session id 'not-a-session' (expected ses_ followed by letters/digits)
```

## fleet version

```text
Usage: fleet version
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| _(none)_ | — | — | `version` takes no arguments. `fleet version --help` is a usage error, exit 2. The short form is spelled `fleet --version` or `fleet -V`. |

**Behaviour.** `fleet version` prints five lines; the short `--version` / `-V` form prints only the first two:

```text
fleet 0.1.0
Build: fleet [(invoked as <name>; surface narrowed to the <variant> profile)]
Derived version: 0.1.0-dev+gb8bbcc0
Source rev: b8bbcc0c49564ab31db8d58b026b67532db9491e
Enabled capabilities: proxy-connect, proxy-serve, … (usable set, registry order)
```

The fields are: the workspace **version**; the **build** name — `fleet`, `fleet-serve`, or `fleet-connect`, derived by matching the *compiled* capability set against the variant sets (an unrecognised compiled set falls back to the plain build name) — with the profile annotation when the invocation name narrowed the surface; the **derived version** (the `0.1.0-dev+g<rev>` form); the **source rev** (the full commit SHA the build was cut from); and the **enabled capabilities** — the *usable* set (compiled ∩ profile ∩ `[domains]` enables), comma-joined in registry order, or `none`. The values above are the reference build's, as captured in the capabilities dump.

There are 15 capability atoms, in registry order: `proxy-connect, proxy-serve, proxy-raw-splice, queue, queue-broker, secrets-broker, search, search-serve, dispatch, dispatch-test, laptop-agent, secrets, gh, bridge, agent`. `proxy-connect` is unconditional — present in every variant. The three build variants are: Full (`fleet`, all 15), Serve (`fleet-serve`: `proxy-connect, proxy-serve, proxy-raw-splice, queue-broker, secrets-broker, search-serve, secrets` — daemon roles plus proxy, no working-seat clients), and Connect (`fleet-connect`: `proxy-connect, proxy-raw-splice` only). Invoking the binary under a hyphenated name selects that profile as a ceiling; profiles can only narrow, never widen. An unknown invocation name is a usage error, exit 2, naming the expected names.

Exit codes: 0 ok · 2 usage error (any argument, including `--help`).

```text
$ fleet version
fleet 0.1.0
Build: fleet
Derived version: 0.1.0-dev+gb8bbcc0
Source rev: b8bbcc0c49564ab31db8d58b026b67532db9491e
Enabled capabilities: proxy-connect, proxy-serve, proxy-raw-splice, queue, queue-broker, secrets-broker, search, search-serve, dispatch, dispatch-test, laptop-agent, secrets, gh, bridge, agent
```

## fleet capabilities

```text
Usage: fleet capabilities
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| _(none)_ | — | — | `capabilities` takes no arguments. `fleet capabilities --help` is a usage error, exit 2. |

**Behaviour.** `fleet capabilities` prints the registry for agents as pretty-printed JSON:

```json
{
  "build": {
    "name": "fleet", "version": "0.1.0",
    "derived_version": "0.1.0-dev+gb8bbcc0",
    "source_rev": "b8bbcc0c49564ab31db8d58b026b67532db9491e",
    "invoked_as": "fleet", "profile": null
  },
  "capabilities": [ "…" ],
  "compiled_capabilities": [ "…" ],
  "domains": [
    { "name": "queue", "summary": "…", "aliases": ["repo-queue"],
      "capabilities": ["queue"], "available": true, "state": "available" }
  ]
}
```

Per-domain `state` is one of `available`, `not-in-build`, `narrowed-by-name`, `disabled-by-config` (the four reason classes behind the exit-69 stub), and `available` is exactly `state == "available"`. Note the two distinct capability lists: `capabilities` is what this *invocation* can use; `compiled_capabilities` is what the *binary* was built with — their difference is the profile/config narrowing. In the reference build both are the full 15 and every domain is `available`. One registry quirk: the `shell` domain's capability list is `["agent"]` — there is no `shell` capability atom; shell rides on the agent capability.

Exit codes: 0 ok · 2 usage error (any argument, including `--help`).

```text
$ fleet capabilities | head -20
{
  "build": {
    "name": "fleet",
    "version": "0.1.0",
    "derived_version": "0.1.0-dev+gb8bbcc0",
    "source_rev": "b8bbcc0c49564ab31db8d58b026b67532db9491e",
    "invoked_as": "fleet",
    "profile": null
  },
  "capabilities": [
    "proxy-connect",
    "proxy-serve",
    "proxy-raw-splice",
    "queue",
    …
```

## fleet help

```text
Usage: fleet help [domain]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<domain>` (positional) | domain name | full registry | With no argument, prints the full registry help (identical to `fleet --help`). With one domain name, prints that domain's help. A second argument is rejected. `fleet help --help` is an error: there is no domain named `--help`. |

**Behaviour.** `fleet help` with no argument prints the full registry: `fleet 0.1.0 — one multicall binary for the fleet's tools`, the usage lines, all 12 domains with summaries and argv0 aliases, the five built-in commands, and the note that a domain this invocation cannot use answers with a stub and exit code 69. `fleet help <domain>` prints the domain's registry help: summary line, `Usage: fleet <domain> <command> [args...]`, argv0 aliases, the command list, and a status line whose wording is per-domain and capability-aware. An unknown name is a usage error: `error: no domain named '<name>' — run 'fleet --help' for the registry`, exit 2. Domain help is registry data and renders identically on every build, with unavailable domains marked — help never stubs.

Exit codes: 0 ok · 2 unknown domain name or extra argument.

```text
$ fleet help queue
fleet queue — Repo work queue: enqueue, claim, and complete tasks through the queue broker.

Usage: fleet queue <command> [args...]
Aliases (argv0): repo-queue
Commands: validate, enqueue, list, claim, release, lane, bind, heartbeat, checkpoint, park, wake, block, unblock, dep-recheck, quarantine-recheck, cancel, revive, fail, complete, verify-disposition, timeline, history, show, models, health, signal, fallback, dispatch, session, surfaces, outbox, stack
Status: included in this build
```

Next: the queue domain itself, starting with [fleet queue: core verbs](queue-core.md).
