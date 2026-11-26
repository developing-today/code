# Design mill pass 4: fleet MCP through mcpx, remotes, fan-out and ID's MCP server

Issue: #10393, pass 4 of the fleet-surface design mill #10389. Read with the
supplement record for #10405
(`doc/2026-11-26T07-55-28Z_design_fleet_mcp_middle_road_10405/`), which this
record does not repeat. Status: design record. It names the five things the
issue's acceptance asks for:

1. the fleet and mcpx wiring;
2. the remote graph model;
3. the fan-out verb semantics;
4. ID's MCP tool surface and dynamic world attach;
5. the mcpx namespace and merge rules.

It writes no code; the build follows the slices in section 9.

One rule holds throughout: **do not build a fourth copy** of the
"connect to all the same things" machinery. fleet's MCP runs through mcpx;
fleet's remotes reuse pass 2's transport and `fleet-pool`'s walking
semantics; ID's MCP attach reuses ID's staged attach.

## 0. Governing decisions consumed

| Source                                           | What this record takes from it                                                                                                                  |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| #10405 record (`2026-11-26T07-55-28Z`)           | The standalone fleet MCP (D1), dynamic tool lists (D2), ID MCP attach (D3), naming (D6), views versus enforcement (D9), transitive union (D10). |
| Pass 1, #10390 and PR #14083 (open)              | Endpoint resolution order: flag, then setting, then environment, then the standard local endpoint. Raw mode is the escape hatch.                |
| Pass 2 record #10391 (`2026-11-24T05-55-51Z`)    | fleet's per-session iroh identity, the three modes (raw, relay-aware, initiator), fail closed on identity mismatch, no silent downgrade.        |
| #10394 research (`2026-11-25T02-10-00Z`)         | fleet reaches mcpx over its existing socket API; no cgo, no port; nothing new in `fleet-connect`.                                               |
| `fleet-pool` design (#13752, crate `fleet-pool`) | Walking, attempt semantics (`serve-failure` against `served-failure`), the decision log.                                                        |
| CLI-02 record (`2026-11-23T12-33-39Z`)           | Every fleet result is a `fleet.result/v1` envelope.                                                                                             |
| Drew's ruling of 2026-11-24 on #9196             | Gates are build gates only.                                                                                                                     |

## 1. Verified state on main `cb794c77`

| Fact                                                                                                                                                                                                                  | Evidence                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| mcpx serves MCP at the daemon's `/mcp` (unix socket and loopback TCP) and over stdio via `mcpx serve`, a shim that starts the daemon if needed.                                                                       | `pkgs/mcpx/docs/mcp.md`                                                                   |
| mcpx adds and removes upstream servers live over CLI, `/v1` and MCP; the entry is written to a config file and the daemon reloads.                                                                                    | `pkgs/mcpx/docs/configuration.md:235-238`; `docs/cli.md:215`                              |
| An mcpx namespace must be a TypeScript identifier, `[A-Za-z_$][A-Za-z0-9_$]*`. Dots are not allowed.                                                                                                                  | `pkgs/mcpx/internal/config/config.go:1017`; `SanitizeNamespace` at `:846`                 |
| mcpx already has aliases: an `aliasOf` entry gives another namespace, tool subset or description over the same server. Aliases of aliases are refused.                                                                | `pkgs/mcpx/docs/configuration.md:228-233`                                                 |
| mcpx's `tools` and `excludeTools` hide a tool from every listing and refuse a call to it with 400.                                                                                                                    | `pkgs/mcpx/docs/configuration.md` "Hiding tools"                                          |
| mcpx already chains to a remote mcpx: a client points `daemon.endpoint` at `ssh://` or `tsnet://`, and an MCP host can spawn `ssh <host> mcpx serve`.                                                                 | `pkgs/mcpx/docs/remote.md` sections 1, 4, 5                                               |
| The mcpx daemon API and MCP surface are unauthenticated; who can reach the socket or port can run tools.                                                                                                              | `pkgs/mcpx/docs/remote.md` "What a remote endpoint is not"                                |
| fleet serves no MCP and has no remote-fleet concept. Its multi-target machinery is `fleet-pool` (members, nested groups, ordered and tiered strategies, walls, decision log) and the laptop route walker built on it. | `pkgs/fleet/crates/fleet-pool/src/lib.rs` module doc; `fleet/Cargo.toml` `laptop` feature |
| fleet's result envelope is `fleet.result/v1`.                                                                                                                                                                         | `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs:45, 860`                             |
| ID has no general MCP server; the agent-in-worlds bridge has a fixed, session-pinned v1 tool list.                                                                                                                    | `pkgs/id/crates/id-caps/src/mcp_bridge.rs`                                                |

## 2. Intent

The issue asks for one fleet surface that reaches local MCP servers, remote
GitHub and Cloudflare MCP, ID worlds over iroh and other fleets, without
building a fourth connection layer. The parts it lists split cleanly by owner:
mcpx already does merging, code mode and live server changes; fleet needs
remotes and fan-out; ID needs an MCP server.

## 3. Ownership

| Part                                  | Owner                       | Decisions |
| ------------------------------------- | --------------------------- | --------- |
| fleet and mcpx wiring                 | fleet `mcp` domain          | D1, D2    |
| remote graph and fan-out              | fleet `remote` domain (new) | D3 to D6  |
| ID's MCP tool surface                 | ID                          | D7, D8    |
| mcpx namespace and merge rules        | mcpx                        | D9        |
| OpenCode fork's built-in `mcpx` tools | pass 6 (#10395) fork upkeep | D10       |
| cycle safety on chained calls         | later, not V1               | D11       |

## 4. Decisions

### D1. fleet registers into mcpx; fleet's MCP endpoint forwards to mcpx

When mcpx is reachable, `fleet mcp serve` (default mode) does three things:

1. **Find mcpx** by pass 1's order: `--mcpx-endpoint`, then the `[mcpx]`
   setting, then the environment, then mcpx's standard local socket.
2. **Register fleet** as an mcpx upstream through mcpx's own live
   `servers add` surface. The entry runs fleet's standalone server
   (#10405 D1) with **no attachments**, so mcpx's fleet upstream can never
   point back at mcpx. Registration is idempotent: an entry that already
   matches is left alone.
3. **Serve fleet's conventional MCP endpoint as a shim** that forwards to
   mcpx's `/mcp`, the same way `mcpx serve` is a shim for its daemon. The
   host sees mcpx's surface (code mode by default) with fleet inside it.

Registration targets, one config value, `mcp.register`:

| Value    | Effect                                                                 |
| -------- | ---------------------------------------------------------------------- |
| `stdio`  | mcpx spawns `fleet mcp serve --standalone` (default)                   |
| `socket` | fleet runs the standalone server on an alternate socket; mcpx connects |
| `url`    | mcpx connects to a remote fleet MCP URL                                |
| `off`    | fleet is not registered; the shim still forwards to mcpx               |

When mcpx is not reachable, D2 of the #10405 record applies: an error that
names `--standalone`, never a silent switch.

### D2. The fleet namespace inside mcpx

mcpx namespaces are TypeScript identifiers, so the issue's dotted names are
spelled with underscores:

- the first registration is `fleet`;
- on collision, `fleet_<hostname>`, with the hostname passed through mcpx's
  `SanitizeNamespace`;
- on a second collision, `fleet_<hostname>_1`, `fleet_<hostname>_2`, and so on.

Inside code mode a call reads `fleet_buildbox.kv_get(...)`. The registration
receipt names the namespace chosen. A namespace held by an entry fleet did
not register is never overwritten; fleet takes the next name.

### D3. Remotes are named connections, each its own unit

`fleet remote add <alias> <endpoint>` records one connection:

- **alias**: a friendly name, unique per fleet, matching
  `[A-Za-z_][A-Za-z0-9_-]*`;
- **endpoint**: an iroh ticket or node ID (pass 2), `ssh://`, or an HTTP
  URL;
- **auth**: a reference to a fleet secret route, never a value. The value is
  resolved per connection when it opens and is not written to the remotes
  file, logs or receipts;
- **expected identity**: the peer's iroh node ID or ssh host key. A mismatch
  fails closed (pass 2 section 4.1);
- **promotable**: whether this remote lets a caller promote its own
  secondary connections (D5). Default false.

Admin verbs: `fleet remote add`, `remove`, `list`, `show <alias>` (metadata
and details) and `stats <alias>` (calls, failures, last seen, latency). A
remote runs remote fleet with the same argv, so **every local subcommand is
available on a remote** with `fleet --on <selector> <subcommand> ...`. The
selector is an alias, a comma list, a glob, or `all`.

### D4. Fan-out verbs

`fleet --on <selector> --mode <mode> <subcommand>`. Every result line is a
`fleet.result/v1` envelope plus `remote` (the alias), `path` (aliases from
the caller to this remote) and `depth` (length of `path`).

| Mode      | Starts              | Returns                                   | On a failure                           |
| --------- | ------------------- | ----------------------------------------- | -------------------------------------- |
| `map`     | all targets at once | each result as it arrives (default)       | recorded on that line; others continue |
| `collect` | all targets at once | one array when every target has finished  | recorded in the array; others continue |
| `any`     | all targets at once | the first success; the rest are cancelled | ignored unless every target fails      |
| `first`   | all targets at once | each result as it arrives, like `map`     | cancels every unfinished target        |

`some` is an accepted spelling of `any`.

- **Per-target outcome**: `ok`, `failed` (the remote answered with a failure),
  `unreachable` (it never answered), `cancelled`. `unreachable` and `failed`
  follow `fleet-pool`'s `serve-failure` and `served-failure` split.
- **Exit code**: 0 when the mode's success condition holds (`map` and
  `collect`: every target ok; `any`: one ok; `first`: no failure). Otherwise
  the first non-zero code in arrival order.
- **Projection** is built in: `--select <expr>` applies a jq-style expression
  to each envelope's `data` before it prints, for example
  `--select '.name'`. A projection that fails on one line marks that line
  `failed` with the expression error; it does not stop the run.
- **Filters**: `--only ok`, `--only failed`. Drew's example is
  `fleet --on all kv get <key> --only failed`.
- **Sinks**: stdout is newline-delimited JSON. `--tee <file>` writes the same
  lines to a file as they arrive. Piping to a shell command works because
  lines are flushed as they arrive.
- The jq evaluator is a `full` build dependency only; `fleet-connect` stays
  under its 2 MiB gate and answers `--select` with the capability stub.

### D5. Tree walking and promotion

- **Walk.** `--walk` descends a remote's own remotes. A child connection
  starts only after its parent connection succeeded. Walking is breadth
  first by depth.
- **Stop rules.** `--max-depth`, default 3. A visited set keyed by
  expected identity (node ID or host key), not alias, so the same machine
  reached by two paths runs once and a loop ends. The second path is
  reported as `duplicate` with both paths.
- **Promotion.** `fleet remote promote <path>` makes a secondary connection
  addressable at the root. It is allowed only when the remote that owns the
  connection set `promotable` and the connection itself allows it. Drew's
  example: with A connected to B, B to C and B to D, where C refuses and D
  allows, the root exposes `B`, `D`, `B.C` and `B.D`.
- A promoted remote still reports its real `path` and `depth` in results,
  so promotion changes addressing, not provenance.

### D6. Remotes and MCP

A fleet remote is not an MCP upstream by itself. To reach a remote fleet's
tools through mcpx, register that remote's fleet MCP in mcpx with
`mcp.register = url` or mcpx's `ssh://` chaining, under the D2 names. fleet
remotes carry fleet commands; mcpx carries MCP. Neither re-implements the
other.

### D7. ID's MCP tool surface

ID's general MCP server (#10405 D3) starts with these tools, grouped:

- **Store**: list, search, get and put blobs; list and inspect namespaces.
- **Apps**: get an app by ID, hash or namespace with its name, details,
  server-side stats, links to comments and reviews, versions, releases and
  previous versions.
- **Worlds**: list a world's standard commands; list its MCP commands when it
  publishes them; hints for common shapes, schemas and tools; chat as the
  worked example.
- **Attach**: load all of a world's tools into the session, or attach the
  world so its tools update live (#10405 D3 and D2).

Exposure is behind a flag per deployment: off, admin only, named users, or
graph-wide. A deployment may expose it only inside the iroh environment.

### D8. Two code modes, kept apart

- **ID code mode** is an ID MCP tool that runs a script against the ID
  server's platform, as client or as admin, with that principal's
  permissions.
- **App code mode** runs inside a world app's own server code, with only the
  commands and capabilities that world exposes. Those capabilities are in
  the world's metadata and count toward its transitive union (#10405 D10).

The two never share a tool name. ID code mode is not granted by joining a
world.

**Forward-looking, not V1:** an mcpx server exposed through ID as a world,
for example a "my laptop" world whose published functions include an MCP
subset. Its path MCP, then mcpx, then the ID host, possibly through a relay,
is an external MCP edge under #10405 D10.

### D9. mcpx namespace and merge rules

- **One upstream, one namespace.** mcpx never merges two upstreams into one
  namespace, even when their command sets are identical. Aggregating across
  them is a code-mode script, not a namespace merge.
- **Caller-assigned names.** Five identical upstreams with random or absent
  names get namespaces the caller assigns: `server_a`, `server_b`,
  `server_c`. In code mode, `server_a.<cmd>` and `server_c.exec(...)`.
  Unassigned names fall back to `SanitizeNamespace` of the server name, then
  `_1`, `_2`.
- **Chains and clusters.** A remote mcpx is an upstream like any other, by
  `ssh://`, `tsnet://` or URL. A script fans out through it and reduces at
  each step.
- **Admin tools** to connect, disconnect, list and inspect upstreams are
  mcpx's existing live `servers` surface. This record adds none.
- **Coverage check.** mcpx's directive to describe itself fully in its own
  MCP server is checked by a test (S6) that compares its MCP tool list with
  its `/v1` operation table, before chaining work starts.

### D10. The `mcpx` tools built into the OpenCode fork

Default agent view: `list`, `get`, `inspect` and `exec` only, with no direct
upstream MCP commands. The admin and power-user scripts the issue lists
(add, remove and rename servers; aliases and namespace copies; per-user,
global, per-session and per-execution settings with backups written on
change; `shell_exec`, `run_exec` and an attachable `shell_session`) stay out
of every agent's default view and are opted in per agent. This lives in the
fork, so pass 6 (#10395) builds it.

### D11. Cycle safety on chained calls: shape only, not V1

Recorded so later work starts from a shape: a pass-through header on chained
calls carrying the originating request ID and a depth count that each hop
increments; a per-server maximum depth on received requests; refusal of a
request whose originating ID the server is already serving. Until then,
D5's visited set protects fleet's own walks, and mcpx chains rely on
configuration not forming loops.

## 5. Negative cases

| Case                                                                     | Required behaviour                                           |
| ------------------------------------------------------------------------ | ------------------------------------------------------------ |
| mcpx unreachable in default mode                                         | error naming `--standalone`; no silent switch                |
| `fleet` namespace taken by an entry fleet did not register               | fleet takes `fleet_<hostname>`; the other entry is untouched |
| The fleet upstream registered in mcpx configured with an mcpx attachment | refused at registration; it would loop                       |
| Remote identity does not match the expected node ID or host key          | that target fails closed as `unreachable`; no fallback route |
| Remote auth reference points at a missing secret                         | that target `failed` with the route named; no value printed  |
| `any` where every target fails                                           | non-zero exit; every failure listed                          |
| `first` with one failure while others run                                | the others are `cancelled`; exit non-zero                    |
| `--select` expression errors on one line                                 | that line `failed` with the error; the run continues         |
| Walk reaches the same machine by two paths                               | runs once; second path reported `duplicate`                  |
| Promote a connection whose owner is not `promotable`                     | refused, naming the owner                                    |
| `--select` on `fleet-connect`                                            | capability stub, exit 69                                     |
| Two identical upstreams in mcpx with no names assigned                   | two namespaces, `_1` suffix on the second; never merged      |

## 6. Oracles and tests

All builds and tests are dispatched remotely. Each slice first runs its test
binary with `-- --list`, then the tests.

| Slice | Test                                                 | Oracle                                                                       |
| ----- | ---------------------------------------------------- | ---------------------------------------------------------------------------- |
| S1    | `register_into_mcpx_is_idempotent`                   | two runs against a test mcpx daemon leave one `fleet` entry                  |
| S1    | `registration_takes_hostname_namespace_on_collision` | with `fleet` held by another entry, the receipt names `fleet_<host>`         |
| S1    | `shim_forwards_tools_list_to_mcpx`                   | the shim's `tools/list` equals the test daemon's `/mcp` list                 |
| S2    | `remote_identity_mismatch_fails_closed`              | target outcome `unreachable`; no second dial                                 |
| S2    | `remote_file_never_holds_secret_value`               | the remotes file and receipts contain the route name and not the value       |
| S3    | `fanout_modes_match_table`                           | one table-driven test over fake remotes for `map`, `collect`, `any`, `first` |
| S3    | `select_error_marks_one_line_failed`                 | one bad line, the rest printed                                               |
| S4    | `walk_dedupes_by_identity`                           | diamond topology; the shared node runs once; `duplicate` reported            |
| S4    | `promotion_respects_owner_flag`                      | the C refuses and D allows case from D5                                      |
| S5    | `id_mcp_tools_list_matches_d7`                       | the ID MCP server lists the D7 groups                                        |
| S6    | `mcpx_mcp_surface_covers_v1_table`                   | every `/v1` operation has its `mcpx_<op>` tool                               |
| S6    | `identical_upstreams_never_merge`                    | two identical fake upstreams give two namespaces                             |

## 7. Compatibility and reversal

- mcpx is unchanged by S1 to S4; fleet uses mcpx's existing live surfaces.
  S6 is a test in mcpx plus any gap it finds.
- `fleet --on` is new; without it every command runs locally as today.
- Registration is reversible with `mcpx servers remove <namespace>`, and the
  receipt names the namespace.
- `fleet-connect` gains nothing but capability stubs.

## 8. Requirement trace

| Issue item                                                | Decision      |
| --------------------------------------------------------- | ------------- |
| Convergence rule: fleet MCP through mcpx                  | D1            |
| Namespacing `fleet`, `fleet.<hostname>`, on collision     | D2            |
| Named remotes, auth per connection, admin verbs           | D3            |
| Every subcommand across remotes                           | D3            |
| Fan-out `map`, collect, `any`/`some`, `first`             | D4            |
| jq-style projection, streaming, tee                       | D4            |
| Tree walking, promotion, depth and path metadata          | D5            |
| ID's MCP server: commands, app inspection, world commands | D7            |
| Dynamic attach with live tool updates                     | D7, #10405 D3 |
| ID code mode against app code mode                        | D8            |
| ID MCP exposed behind a flag, iroh-only option            | D7            |
| mcpx through ID as a world                                | D8            |
| mcpx chaining, clustering, caller-assigned namespaces     | D9            |
| OpenCode fork `mcpx` default tools and admin scripts      | D10           |
| Fleet tools to list, extract, promote or add servers      | D1, D5, D9    |
| Cycle safety                                              | D11           |

## 9. Build slices and gates

| Slice | Scope                                                                | Gate                                  |
| ----- | -------------------------------------------------------------------- | ------------------------------------- |
| S1    | fleet: mcpx registration, namespace choice, shim                     | #10405 S1 (standalone server)         |
| S2    | fleet: `fleet remote` add, remove, list, show, stats; identity, auth | pass 2 identity code for iroh remotes |
| S3    | fleet: `--on` and the four modes, projection, filters, tee           | S2                                    |
| S4    | fleet: walking, visited set, promotion                               | S3                                    |
| S5    | ID: general MCP server with the D7 tools                             | #10405 S3                             |
| S6    | mcpx: surface coverage test and the no-merge namespace test          | none                                  |

S2 can run over `ssh://` before pass 2's iroh code lands; iroh remotes join
when it does. S6 can start at once.

## 10. Non-goals

- A fourth connection layer, or any copy of mcpx's merging in fleet.
- Authentication on mcpx's MCP surface; this record inherits mcpx's
  documented warning.
- Cycle safety on chained calls in V1 (D11).
- The OpenCode fork changes themselves (pass 6).
- Embedding ID or mcpx in fleet (pass 5, #10394).
