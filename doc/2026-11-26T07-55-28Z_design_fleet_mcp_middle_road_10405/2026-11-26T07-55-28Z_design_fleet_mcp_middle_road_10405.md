# The middle-road fleet MCP, ID graph publication and transitive capability purity

Issue: #10405, a supplement to the fleet-surface design mill #10389, closest
to pass 4 (#10393). Status: design record. It turns Drew's dictated intent of
2026-11-14 into decisions, negative cases, oracles and build slices, and it
says which pass or crate owns each section. It writes no code.

Three rules hold throughout. They are the issue's own non-negotiables:

- **The middle road is a designed option.** A basic fleet MCP that is fleet
  itself, with no code mode, has its own decision (D1), tests and slice.
- **A filtered view is never enforcement.** A shadow or filtered view is a
  convenience. Only the serving MCP server can refuse a call (D9).
- **Every exposure states its transitive capability union.** What a world
  or MCP can reach through others is its real capability, and it is
  published next to the direct set (D10).

## 0. Governing decisions consumed

| Source                                         | What this record takes from it                                                                                                                                                        |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #10393 pass 4 convergence rule                 | Where mcpx exists, fleet's MCP runs through mcpx. The middle road is the explicit exception for setups without it.                                                                    |
| #10394 embedding research (`2026-11-25T02-10`) | fleet reaches mcpx over its existing socket API; fleet reaches ID by exec for occasional calls and a kept-alive child or socket for chatty ones; nothing new goes in `fleet-connect`. |
| #10391 pass 2 record (`2026-11-24T05-55-51Z`)  | fleet's iroh identity and transport. This record adds no second transport.                                                                                                            |
| #10392 pass 3 record (`2026-11-24T08-40-00Z`)  | ID's REST session and presentation model. Discovery output (D7) is presented through it.                                                                                              |
| #5994 section 5.5 and #5956 section 7          | Purity is the empty requirement set, never a self-asserted label. The transitive resolver is the planned consumer of requirement sets. D10 extends that set; it adds no second judge. |
| Drew's ruling of 2026-11-24 on #9196           | Gates are build gates only. No slice here waits on a design-only gate.                                                                                                                |

## 1. Verified state on main `4ec43d77`

| Fact                                                                                                                                                                                                                         | Evidence                                                                                                          |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| fleet serves no MCP. Its only MCP code is a client: the `commander` feature reaches the Desktop Commander endpoint over ureq.                                                                                                | `pkgs/fleet/crates/fleet/Cargo.toml` (`commander` feature comment); no `tools/list` handler under `pkgs/fleet`    |
| fleet already knows its own effective command surface. `fleet-registry` keeps the compiled set, the profile ceiling and their intersection, and `Domain::available_in` answers per domain. It is compiled into every build.  | `pkgs/fleet/crates/fleet-registry/src/lib.rs` (module doc; `available_in`, `full_capabilities`)                   |
| mcpx serves MCP over streamable HTTP at the daemon's `/mcp` and over stdio via `mcpx serve`. The default surface is the code-mode gateway; `?codemode=false` gives the raw `<namespace>_<tool>` surface.                     | `pkgs/mcpx/docs/mcp.md` ("mcpx as an MCP server")                                                                 |
| mcpx already does dynamic tool lists in both directions. As a server it broadcasts `notifications/tools/list_changed` (reactive mode). As a client it handles the tools, resources and prompts `list_changed` notifications. | `pkgs/mcpx/internal/mcpserver/reactive.go:174`, `listen.go:180`; `pkgs/mcpx/internal/mcpclient/client.go:918-981` |
| mcpx changes its configured servers live: `mcpx servers add` and `remove` need no restart.                                                                                                                                   | `pkgs/mcpx/docs/cli.md:215`                                                                                       |
| mcpx's MCP surface is unauthenticated: whatever can route to the port can run tools.                                                                                                                                         | `pkgs/mcpx/docs/mcp.md` (network warning)                                                                         |
| ID serves no general MCP server. It has the agent-in-worlds bridge, a translator with a fixed v1 tool list pinned at session start, and a host-side policy point (ceiling, quota, audit).                                    | `pkgs/id/crates/id-caps/src/mcp_bridge.rs`, `mcp_policy.rs`; `pkgs/id/tests/mcp_bridge_e2e.rs`                    |
| ID has staged attach and detach machinery: checks run at request time, the change lands at the next tick boundary, refusals name the failed check, and an in-flight call keeps its snapshot.                                 | `pkgs/id/crates/id-worlds/src/attachment_dispatch.rs:46-80`; `id-caps/src/library_attach.rs:529-624`              |
| ID's purity rule exists in parse form only: an absent platform block is a pure library. The transitive resolver is planned, not built.                                                                                       | `pkgs/id/crates/id-caps/src/library_manifest.rs:10-20, 714-724`                                                   |

## 2. Intent

Drew wants four things settled before the pass owners build:

1. A fleet MCP that works without mcpx's code mode.
2. A clear answer on dynamic tool lists, with a fallback ladder.
3. A publication model for MCPs inside the ID graph: where they live, what
   they are called, how fleet discovers and installs them, and how
   conflicts resolve.
4. A capability model that cannot be fooled by views, and that reports
   what a world can really reach.

Several pieces are "exists at some point". This record gives each piece a
slice, so it is buildable when a pass owner takes it, and marks the low
priority ones.

## 3. Ownership

| Section | Surface                                | Owner                                       |
| ------- | -------------------------------------- | ------------------------------------------- |
| D1, D2  | `fleet mcp serve` (standalone)         | pass 4 (#10393), fleet `mcp` domain         |
| D3      | ID's MCP server with attach and detach | ID, under pass 4                            |
| D4      | fleet boot orchestration               | pass 1 (#10390) boot path, pass 4           |
| D5, D6  | publication location and naming        | pass 3 (#10392) presentation, ID namespaces |
| D7      | discovery and inspection               | ID (endpoints), fleet (`fleet mcp status`)  |
| D8      | install flow and conflict policy       | fleet `mcp` domain                          |
| D9      | shadow worlds and aggregates           | ID worlds                                   |
| D10     | transitive capability union            | `id-caps` resolver                          |

## 4. Decisions

### D1. The middle road is `fleet mcp serve --standalone`

A basic MCP server that is fleet itself.

- **Tool list.** Generated from `fleet-registry`'s effective set (compiled
  set intersected with the profile ceiling). A command outside the
  effective set is never listed, so the server cannot advertise what this
  build cannot run. Tool names are `fleet_<domain>_<verb>`.
- **Optional attach at start.** `--attach-id <endpoint>` and
  `--attach-mcpx <endpoint>` each connect once at start, read `tools/list`,
  register fleet where the far side accepts registration, and add the far
  side's tools under `<alias>_<tool>` (D6). Neither is required.
- **What it does not do.** It never adds or removes providers at runtime and
  has no exec runtime. Those are mcpx's job.
- **Selection is explicit.** The convergence rule of pass 4 is the default:
  when mcpx is reachable, `fleet mcp serve` registers into mcpx and forwards.
  The middle road runs only with `--standalone` or `mcp.mode = "standalone"`.
  There is no silent fallback from one to the other, matching mcpx's own
  no-fallback transport rule; an unreachable mcpx with the default mode is
  an error that names `--standalone`.
- **Transport.** stdio first. Streamable HTTP on loopback is a later slice
  and carries the same unauthenticated-surface warning mcpx documents.
- **Build.** A `mcp-serve` cargo feature in `full` only, never in
  `fleet-connect` (2 MiB gate).
- **Priority.** Low. Drew: "I don't know that I would use it, but it might
  exist." It is slice S1 so that it exists, not because it is urgent.

### D2. Dynamic tool lists: the protocol supports them, so use it

The open question in the issue has an answer on main. MCP servers declare
`tools.listChanged` in their capabilities and send
`notifications/tools/list_changed`; clients that honour it call `tools/list`
again. mcpx already does both sides (§1).

- The standalone server declares `tools.listChanged: true`.
- **Refresh triggers.** An attached upstream sends `list_changed`; a client
  calls the `fleet_mcp_refresh` tool; the process gets SIGHUP. Each trigger
  re-pulls `tools/list` from the attached upstreams, diffs the merged list,
  and sends one notification only when the set changed.
- Refresh re-reads already attached upstreams. It never attaches a new one,
  so it does not break D1's no-runtime-providers rule.
- **Fallback ladder**, in order:
  1. A client that honours `list_changed` sees the change at once.
  2. A client that does not sees it on its next connection.
  3. On HTTP transport, `fleet_mcp_restart` drops all sessions so clients
     reconnect. On stdio a server cannot respawn itself, so the reconnect is
     the host's affordance; an OpenCode "reconnect MCP servers" action
     belongs to fork upkeep, pass 6 (#10395).
  4. Always true: code mode lists commands by calling a command, so it is
     dynamic by construction.

### D3. ID's MCP server attaches worlds by ID or URI

ID gets a general MCP server, separate from the agent-in-worlds bridge.

- **Input.** A list of IDs or URIs: hashes or namespace paths. Each is
  resolved; only those that really publish an MCP are exposed. One that
  publishes none is reported as `not-mcp` and skipped, never an error for
  the whole list.
- **Machinery.** Attach and detach reuse the `attachment_dispatch` pattern:
  checks at request time, change at the next tick boundary, named refusals,
  in-flight calls keep their snapshot. After a change the server sends
  `list_changed` (D2).
- **Origin and removability.** Every entry records its origin: `config`,
  `flag`, `admin-runtime` or `fleet-boot`. Entries from `config` are not
  removable at runtime by default; the rest are. Both defaults are
  configurable per origin.
- **The bridge stays pinned.** The agent-in-worlds bridge keeps its fixed v1
  tool list pinned at session start. The general server is a different
  surface and does not inherit that pin, and the bridge does not inherit
  dynamic attach.
- ID can expose the fleet MCP and mcpx over iroh and the graph through the
  same attach path, once the endpoint's auth format is one ID accepts.

### D4. Fleet boot orchestration is a list of optional, idempotent steps

`fleet` boot runs these steps in order. Each is off unless configured, each
is idempotent, and each writes one line to the boot receipt with its stage
(`skipped`, `applied`, `already-present`, `conflict`, `failed`):

1. Ensure mcpx is running and register fleet on it (pass 4).
2. Ask ID to expose its MCP over the graph (D3).
3. Expose mcpx over the graph through ID.
4. Inject configured extra MCPs into mcpx, into ID worlds, or into the
   standalone server.
5. Optionally swap fleet's conventional MCP port to forward to mcpx.

A failed step does not undo earlier ones. Whether a failure stops boot is
the conflict policy of D8. Pass-through of an unrelated remote MCP happens
only when its auth format is one ID accepts, and it enters the transitive
union (D10) as `external`.

### D5. Publication lives in the admin's private namespace by default

- **Default.** ID uses the server's default admin credentials and publishes
  into the admin's private namespace.
- **Alternatives**, each a config value:
  - another location the admin can write;
  - a key on disk that the server points at;
  - a **drop-box namespace**: it accepts puts from listed users or groups
    and allows no read and no list to them;
  - **delegated admin**: an admin or moderator group's public key is
    attached as delegate, for the whole namespace or for named MCP servers.
- **Filtering.** Command filtering that refuses calls lives at the world
  and server level only (D9). Graph-level partial visibility stays an open
  question, and if it is ever built it is visibility, never enforcement.

### D6. Naming

- **Inside ID namespaces.** One configurable root key, default `mcp`, with
  entries `<root>.<name>`. Default names: `fleet`, `mcpx`, `id`. So the
  defaults are `mcp.fleet`, `mcp.mcpx` and `mcp.id`. Changing the root
  (for example to `mcps`) moves all three.
- **On a flat MCP surface** (the standalone server, or the core ID MCP
  server), tools from another source are `<alias>_<tool>`, with an
  underscore. Many hosts reject dots in tool names, and mcpx's raw surface
  already uses `<namespace>_<tool>`. The dotted `alias.command` form is used
  only inside worlds and namespaces, where it is a path.
- **Collision.** A second registration of an alias in use gets `_1`, `_2`,
  matching pass 4's `fleet.<hostname>.1` rule, and the receipt says so.

### D7. Discovery and inspection

One query surface with three outputs: human, JSON and TSV. It is the same
data over shell, REST, MCP and the graph.

- **Listings:**
  - system ports and what runs on each;
  - runtime ports (custom, user or graph-driven) with the ID and namespace
    served on each, marked as dynamic;
  - active worlds serving over iroh and localhost apps, with an MCP-capable
    view and the command list of each MCP location.
- **Inspector for ID's MCP server.** For each attached MCP: origin (D3),
  removable or not, which iroh IDs have connected, and a link to the logs of
  the commands that ran.
- **Access.** Whoever holds the permission set for the admin's local
  storage, or for wherever the admin publishes (D5).
- **fleet's path, in order:** ID's REST endpoint; if not found, exec the
  `id` binary; ask `id` for its location or primary port. This is #10394's
  recommendation: exec for occasional calls, a kept-alive child or socket
  when chatty.
- **Bar.** Static and slowly changing cases are the bar. Native tracking of
  Roc-driven changes is the ideal, not a gate.

### D8. Install flow and conflict policy

fleet asks ID for MCP endpoints filtered by tag (`fleet`, `mcpx`, both, or
an address filter). It lists its own URL and port, the mcpx URL and port,
and a `fleet-direct` entry for the alternate port.

**Install rule.** fleet installs all of its entries in one call only when
all three hold: every standard alias is free, none of the addresses is
already published, and no published MCP carries any of the three tags. The
input is a generated file, a configured path, or passed directly.

**Conflict policy**, one config value:

| Value      | Effect                                                   |
| ---------- | -------------------------------------------------------- |
| `fail`     | stop fleet start with an error that names the conflict   |
| `fill`     | add only what is missing; report the rest (default)      |
| `force`    | rewrite until everything matches; needs explicit config  |
| `suffix`   | add under `_1`, `_2`, and so on                          |
| `archive`  | rename the old alias to `.archived.<date>`, keep it live |
| `push-all` | push everything and remove nothing                       |

`fill` is the default because it never overwrites someone else's entry and
never blocks boot over an optional feature. Every applied or skipped entry is
in the boot receipt.

**Same target twice.** Two entries pointing at the same namespace and path
are decided by the server admin's `duplicate_target` setting (`block`,
`replace`, `keep-both`), default `block`: the new entry is refused and
reported.

### D9. Shadow worlds and aggregates are views; servers enforce

- Every filtered view, shadow world or aggregate carries `enforced: true` or
  `enforced: false`. It is `true` only when the serving MCP server itself
  refuses calls outside the filter, for example a separate URL whose filter
  parameters the server applies on `tools/call`. Otherwise it is `false`,
  and docs, listings and the inspector print it as "view only".
- A view with `enforced: false` must never be granted to a group as a
  narrower permission. The grant path refuses it.
- An **aggregate** of read-only commands from many worlds counts as
  read-only only if each source server enforces read-only for that grant.
  A tool's name or description never makes it read-only.
- A world may expose its MCPs as standalone MCPs on dedicated random ports,
  tagged by purpose, and may add `<alias>_<command>` entries to the server's
  core ID MCP, when its permissions allow. Each such exposure is an
  exposure for D10.

### D10. The transitive union is the published capability

Every MCP exposure, world and aggregate publishes two sets:

- `capabilities_direct`: what it declares itself.
- `capabilities_transitive`: the union over everything it can reach through
  other worlds and MCPs.

Markers on the transitive set:

| Marker          | When                                                                                         |
| --------------- | -------------------------------------------------------------------------------------------- |
| `external`      | any external MCP is reachable; all MCPs in this record are external                          |
| `unscoped`      | it can reach any world rather than a listed set; the set is then "all", and it is never pure |
| `impure-pinned` | a call stored data at a location whose ID now replaces later calls until reset               |
| `pure`          | only when the transitive requirement set is empty                                            |

- Purity stays the empty requirement set (#5994 section 5.5). An MCP
  exposure adds a requirement entry, so exposure through an external MCP
  makes the world impure by the existing rule, with no new label.
- `impure-pinned` names the pinned location ID. Under dynamic conditions the
  honest answer is impure, so a world that pins and can be re-pinned at
  runtime reports impure, not `impure-pinned`.
- The computation is the planned transitive resolver of #5956 section 7
  step 2, fed MCP exposure edges. It is not a second resolver.

## 5. Negative cases

| Case                                                                       | Required behaviour                                                       |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Standalone server asked to list a command outside the effective set        | not listed; a direct call gets the registry's capability stub (exit 69)  |
| Default mode with mcpx unreachable                                         | error naming `--standalone`; no silent switch                            |
| Refresh with no change in the merged list                                  | no `list_changed` sent                                                   |
| `fleet_mcp_refresh` asked to add a provider                                | refused; refresh only re-reads attached upstreams                        |
| ID attach list holds a hash that publishes no MCP                          | that entry reported `not-mcp`; the others attach                         |
| Runtime detach of a `config` entry with default settings                   | refused, naming the origin                                               |
| Detach while a call is in flight                                           | the call finishes on its snapshot; the detach lands at the tick boundary |
| Install with one alias taken and policy `fill`                             | the free ones install; the taken one is reported; boot continues         |
| Two entries for the same namespace and path, `duplicate_target` unset      | the new one is refused and reported                                      |
| A hidden tool in a `enforced: false` view called through code mode or exec | the call reaches the server; the view is labelled view only, not a fault |
| A hidden tool in a `enforced: true` view called by any route               | the server refuses it                                                    |
| Grant of an `enforced: false` view to a group                              | refused at the grant path                                                |
| A world reaching an external MCP declares itself pure                      | the transitive set shows `external`; purity is false                     |
| A world with access to any world                                           | transitive set is `unscoped`                                             |

## 6. Oracles and tests

All builds and tests are dispatched remotely. Each slice first runs its
test binary with `-- --list` to confirm the names exist, then runs them.

| Slice | Test                                                   | Oracle                                                                                |
| ----- | ------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| S1    | `standalone_mcp_lists_only_effective_set`              | `tools/list` equals the registry's effective set for the build and profile under test |
| S1    | `default_mode_without_mcpx_names_standalone`           | exit is non-zero; stderr names `--standalone`; no server started                      |
| S2    | `upstream_list_changed_relays_once`                    | a fake upstream sends two notifications with one real change; the client sees one     |
| S2    | `refresh_never_attaches_new_provider`                  | refresh with a new endpoint in config leaves the attached set unchanged               |
| S3    | `id_mcp_attach_skips_non_mcp_entries`                  | mixed list; only MCP-publishing entries listed; others reported `not-mcp`             |
| S3    | `id_mcp_config_entry_not_removable_by_default`         | runtime detach refused with the origin named                                          |
| S4    | `mcp_status_outputs_agree`                             | human, JSON and TSV rows carry the same entries                                       |
| S5    | `install_fill_skips_taken_alias`                       | receipt lists installed and skipped entries; boot exits 0                             |
| S5    | `duplicate_target_blocked_by_default`                  | second entry refused; first unchanged                                                 |
| S6    | `filtered_view_unenforced_is_labelled_and_ungrantable` | listing prints view only; grant refused                                               |
| S6    | `external_mcp_makes_transitive_set_impure`             | resolver output carries `external`; purity false                                      |

## 7. Compatibility and reversal

- Nothing here changes mcpx. fleet uses mcpx's existing socket, HTTP and
  MCP surfaces.
- The agent-in-worlds bridge is untouched; its pinned v1 list stays.
- `fleet-connect` is untouched; `mcp-serve` is `full` only.
- Each slice is behind config that is off by default, so reversal is turning
  the config off. Install entries carry their origin, so `fleet-boot`
  entries can be listed and removed together.

## 8. Requirement trace

| Issue section                              | Decision    |
| ------------------------------------------ | ----------- |
| 1. middle road basic fleet MCP             | D1          |
| 2. dynamic tool-list updates               | D2          |
| 3. ID's MCP server, dynamic attach         | D3          |
| 4. fleet boot orchestration                | D4          |
| 5. ID serving worlds, publication location | D5, D9      |
| 6. naming conventions                      | D6          |
| 7. discovery and inspection tooling        | D7          |
| 8. install flow and conflict policy        | D8          |
| 9. world MCPs, aggregates, shadow worlds   | D9          |
| 10. transitive capability purity           | D10         |
| Acceptance (a), (b), (c)                   | D1, D9, D10 |

## 9. Build slices and gates

| Slice | Scope                                                                 | Gate                          |
| ----- | --------------------------------------------------------------------- | ----------------------------- |
| S1    | fleet: `fleet mcp serve --standalone`, stdio, registry-generated list | none                          |
| S2    | fleet: start-time attach of ID and mcpx, refresh, list_changed relay  | S1                            |
| S3    | ID: general MCP server with attach and detach by ID or URI            | none                          |
| S4    | ID endpoints and `fleet mcp status` listings and inspector            | S3 for the inspector part     |
| S5    | fleet: install flow, tags, conflict policy, boot receipt lines        | S3, S4                        |
| S6    | id-caps: transitive union markers, `enforced` flag, grant refusal     | the #5956 transitive resolver |

S6 must land before any aggregate or shadow world is granted to a group.
S3 and S6 can start in parallel with S1.

## 10. Non-goals

- Rebuilding mcpx in fleet: no runtime provider changes, no exec runtime,
  no code mode in the standalone server.
- Graph-level enforcement of filtered views.
- cgo or a Rust port of mcpx (#10394 section 4).
- Any MCP surface in `fleet-connect`.
- Authentication for loopback HTTP in this record. The standalone HTTP
  slice inherits mcpx's documented warning until pass 4 settles auth.
