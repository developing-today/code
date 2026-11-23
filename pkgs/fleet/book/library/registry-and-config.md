# Registry, config and capabilities

Two platform crates decide what a given `fleet` invocation _is_:
`fleet-registry` says what exists — every domain, capability and build
variant, in every build — and `fleet-config` says what this host allows
and where everything lives. On top of them, the binary computes an
invocation **surface**: compiled capabilities, narrowed by the name it
was invoked under and by the host's config, with every unusable domain
answering through the same stub. This chapter covers all three layers
and the gating rules an embedder must respect.

For the crate map, see [The crates](crates.md).

## `fleet-registry`: the inventory is the registry

`fleet-registry` is dependency-free — std only, strings and tables, no
I/O — and is compiled into **every** variant, including the size-gated
connect build, so `--help` is always complete. It owns four things.

### The 15 capabilities

`enum Capability`, with stable string ids in `Capability::ALL`, in
registry order:

```text
proxy-connect, proxy-serve, proxy-raw-splice, queue, queue-broker,
secrets-broker, search, search-serve, dispatch, dispatch-test,
laptop-agent, secrets, gh, bridge, agent
```

`CapabilitySet` is a `BTreeSet<Capability>`, built with
`capability_set(&[…])`; the crate provides `full_capabilities()`,
`serve_capabilities()` and `connect_capabilities()` for the three
variant sets below.

### The 12 domains

`pub const DOMAINS: &[Domain]` — the full registry in help order:
`queue`, `serve`, `proxy`, `dispatch`, `dispatch-test`, `secrets`,
`gh`, `bridge`, `agent`, `shell`, `search`, `laptop-agent`.

`struct Domain` is the unit that help lists, stubs name, and config
disables:

| Field                                 | Meaning                                              |
| ------------------------------------- | ---------------------------------------------------- |
| `name: &'static str`                  | The domain's invocation name                         |
| `summary: &'static str`               | The one-line description help prints                 |
| `capabilities: &'static [Capability]` | Every capability the domain spans                    |
| `primary: Capability`                 | The capability headline availability keys on         |
| `aliases: &'static [&str]`            | Its argv0 aliases                                    |
| `subcommands: &'static [&str]`        | The top-level verb inventory rendered by domain help |

The `primary` rule has a visible consequence: on the `fleet-serve`
build, the `search` domain is marked absent because its headline
capability is the `search` _client_, even though the `search-serve`
role exists under `serve`. And `shell`'s capability list is `["agent"]`
— there is no `shell` capability atom; shell rides on the agent
capability.

`pub fn domain(name)` looks a domain up by name (a linear find over
`DOMAINS`).

### The 3 variants

`enum Variant { Full, Serve, Connect }` — a property of the artifact,
whose names double as invocation profiles:

| Variant   | Installed name  | Compiled capabilities                                                                                                                                                  |
| --------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Full`    | `fleet`         | All 15                                                                                                                                                                 |
| `Serve`   | `fleet-serve`   | `proxy-connect`, `proxy-serve`, `proxy-raw-splice`, `queue-broker`, `secrets-broker`, `search-serve`, `secrets` (7) — daemon roles plus proxy, no working-seat clients |
| `Connect` | `fleet-connect` | `proxy-connect`, `proxy-raw-splice` (2)                                                                                                                                |

`Variant::name()` gives the installed side-by-side name; wherever a
variant is actually used, the binary is renamed plain `fleet` and the
hyphenated names survive as argv0 personalities (symlinks to the one
binary, created at install time). `Variant::capabilities()` is the
profile ceiling. `Variant::from_invocation_name()` recognises only
`fleet-serve` / `fleet-connect`; plain `fleet` selects no profile.
The profiles nest — connect ⊂ serve ⊂ full, and full equals
`Capability::ALL` — an invariant the crate's own tests assert, along
with unique/stable capability ids, every domain's primary being in its
own surface, aliases routing to real domains, and the domain alias
table matching the argv0 map.

### The argv0 alias map

`struct AliasRoute { domain, implied_arg }` and `pub fn alias_route(argv0)`
give the multicall names; anything not listed is not a multicall name:

| argv0                 | Routes to domain | Implied argument |
| --------------------- | ---------------- | ---------------- |
| `repo-queue`          | `queue`          | —                |
| `queue-broker`        | `serve`          | `queue-broker`   |
| `secrets-broker`      | `serve`          | `secrets-broker` |
| `laptop-agent-broker` | `proxy`          | `serve`          |
| `atlas-search`        | `search`         | —                |
| `laptop-agent`        | `laptop-agent`   | —                |

An alias route constructs the surface with **no profile** (aliases do
not narrow), prepends the implied argument, and dispatches straight to
the domain. `is_known_invocation_name(name)` is true for `fleet`, a
variant name, or an alias; any other invocation name is a usage error
(exit `2`) naming the expected names.

## `CapState`: the four answers a domain can give

The binary's `Surface` (in the bin crate) holds the invocation's whole
effective surface: the compiled set, the optional profile, their
intersection as the effective set, and the loaded config.
`Surface::cap_state(cap, domain_name)` checks in order and yields
`enum CapState`:

| `CapState`         | Meaning                                                                     | Stub wording (first line)                                             |
| ------------------ | --------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `Available`        | Compiled in, in profile, not config-disabled                                | — (the domain runs; the fourth form below is its skeleton variant)    |
| `NotInBuild`       | This build was compiled without it                                          | `fleet: this build does not include '<domain>'.`                      |
| `NarrowedByName`   | The invocation name (`fleet-serve` / `fleet-connect`) scoped it away        | `fleet: invoked as '<name>', this build does not include '<domain>'.` |
| `DisabledByConfig` | The `[domains]` table in this host's fleet config set the domain to `false` | `fleet: '<domain>' is disabled by config on this host.`               |

Every stub goes to stderr and returns exit **69** (`EX_UNAVAILABLE`).
Its second line is always `Enabled capabilities: <comma list>` — the
_usable_ set: the effective set minus every capability of every
config-disabled domain — and its third line names the build. There is
also a step-1 skeleton form for a capability that is `Available` but has
nothing behind it yet: "included in this build but not implemented
yet", in frozen epic-#5412 wording. Help renders the same four states
inline (`[not in this build]`, `[narrowed out by this name]`,
`[disabled by config]`), and renders them on _every_ variant — help
never stubs.

`fleet capabilities` prints the whole picture as JSON: the build block
(name, version, derived version, source revision, invoked-as, profile),
**two** capability lists — `capabilities` (what this _invocation_ can
use) and `compiled_capabilities` (what the _binary_ was built with);
their difference is exactly the profile/config narrowing — and one
entry per domain with its `state` (`available`, `not-in-build`,
`narrowed-by-name`, `disabled-by-config`) and `available` boolean.

## `compiled_capabilities()`: the binary reports its own variant

Capabilities are compiled in through Cargo features on the bin crate,
and reconstructed at runtime — the binary _derives_ its variant from
its feature set; it does not trust a baked-in name.

- The feature sets: `default = ["full"]`, where `full` enables every
  domain feature; `serve` enables the serve set as one feature for the
  Nix build; the connect build is `--no-default-features` with nothing
  optional on. Each feature wires its optional dependencies and the
  owning crate's matching feature (for example `queue =
["dep:fleet-secrets", "dep:fleet-queue", "fleet-queue/client"]`).
- `proxy-connect` is **not a feature**: it is compiled into every
  variant unconditionally. `fleet-proxy` itself is a non-optional
  dependency with default features off — the client is in every
  variant; only the broker half (`fleet-proxy/serve`) is gated.
  `fleet-secrets` is optional for the same reason in reverse: the
  connect variant must not carry it.
- `fn compiled_capabilities()` starts from `{ProxyConnect,
ProxyRawSplice}` unconditionally and inserts one `Capability` per
  `cfg!(feature = "…")`. `build_variant(compiled)` then matches the set
  against the three variant sets — `Full`, `Serve`, `Connect`, or
  `None`, which renders as the build name `fleet-custom` in
  `fleet version`.

Dispatch order, end to end: argv0 alias → domain route; argv0 variant
name → root dispatch with that profile as a ceiling; argv0 `fleet` →
root dispatch; then per domain — capability compiled in? in profile?
not config-disabled? → the domain handler, else the stub; an unknown
name is a usage error, exit `2`. The dispatch is hand-rolled,
explicitly not `clap`, because it must behave identically in every
variant — including the tiny connect build, where the full registry and
the stub machinery are the product.

## `fleet-config`: one place config resolves

`Config::load()` / `Config::file_path()` resolve, in order: environment
overrides (`FLEET_CONFIG`, `FLEET_BROKER_SOCKET`, …), then the config
file (`$FLEET_CONFIG`, else `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`),
then compiled-in defaults. The crate owns the `[domains]`
enable/disable table (`domain_enabled(name)` — the per-host shaping and
incident lever behind `DisabledByConfig`), the `[proxy]`, `[secrets]`,
`[queue-broker]`, `[secrets-broker]` and `[dispatch]` sections, and the
fleet's constants: ports (4799 laptop-agent, 4800 secrets-broker, 4801
queue-broker, 4802 gh cache, 4879 pm-dispatch, 3130 egress proxy) and
role sockets (environment beats `[<role>] socket` in the file beats the
runtime-dir/home defaults).

### The `[host]` manifest: `FLEET_HOST_CONFIG` and `FLEET_LOCATION`

`fleet init` — autostart this host's roles per the manifest — resolves
the manifest in this order:

1. **`FLEET_HOST_CONFIG`**: a complete inline `[host]` manifest. It wins
   outright. A manifest with no location is a loud error.
2. **`FLEET_LOCATION`**: names the location; roles come from the config
   file's `[host]` section only if the file's location matches, else
   from the built-in map.
3. **The config file's `[host]` section**: `location`, `init_mode`, and
   `[host.roles]` — the role set is exactly the key set of the roles
   table, and an unknown role name is a loud parse error, never silently
   ignored.

The built-in location map has two locations today (a future lab node
provisions its own `[host]` section instead of growing the table):

| Location  | `init_mode` | Roles                                                      |
| --------- | ----------- | ---------------------------------------------------------- |
| `sandbox` | `spawn`     | `proxy`, `secrets-broker`                                  |
| `laptop`  | `systemd`   | `laptop-agent`, `queue-broker`, `search`, `secrets-broker` |

The full `[host]` role vocabulary is five names — `proxy`,
`queue-broker`, `secrets-broker`, `search`, `laptop-agent` — a superset
of the serve roles (it includes `laptop-agent`, which is not a serve
role). Behaviour that follows from the manifest rules:

- An **unknown location** (no manifest from any source, or a location
  with no roles) makes `init` report state — every source consulted,
  plus detected hostname/OS/systemd for the report only, since detection
  never selects roles — start nothing, and exit `69`. Never guess-start
  a role.
- Roles the manifest itself disables (`enabled = false`) are reported as
  skipped with their reason; then the `[domains]` gate is applied and
  **wins over the manifest**.
- In `spawn` mode (sandbox), v1 spawn semantics exist for exactly the
  two sandbox roles; any other role in a spawn manifest is reported and
  skipped, never spawned. In `systemd` mode (laptop), init never
  detached-spawns a daemon systemd owns: it reports each unit's state,
  starts an inactive-but-existing unit, and reports a missing unit as
  skipped.

## Capability gating rules for embedders

These are the rules the narrowing-only design imposes on anyone
embedding or wrapping the binary:

1. **Profiles only narrow, never widen.** Invoking under `fleet-serve`
   or `fleet-connect` sets a capability _ceiling_: the effective surface
   is `compiled ∩ profile`. No invocation name, config file or
   environment variable can give an invocation a capability its binary
   was not compiled with.
2. **Config only disables.** The `[domains]` table can remove a domain
   on a host (the incident lever), and that gate wins even over the
   `[host]` manifest's role list — but it cannot enable what the build
   or profile lacks.
3. **Aliases do not narrow.** An argv0 alias (`repo-queue`,
   `queue-broker`, …) routes straight to its domain with no profile.
   Only the two hyphenated _variant_ names narrow.
4. **Check state, don't assume.** An embedder that depends on a domain
   must handle all four `CapState` outcomes — or, from outside the
   process, exit `69` with the stub's wording — and should read
   `fleet capabilities` (the JSON census) rather than hardcoding which
   build it hopes it is running. The two lists in that output exist
   precisely so a wrapper can tell "this binary can't" (compiled set)
   from "this invocation can't" (usable set).
5. **Help is not evidence of availability.** The full registry renders
   on every variant by design. Availability is the domain's `state` in
   the capabilities census, or the stub you get when you invoke it.

## Example: reading the registry and the config

This example is illustrative — it follows the researched API shapes
(`fleet_registry::DOMAINS`, `Capability::ALL`,
`fleet_config::Config::load`) but the exact signatures were not all
verified against source in this pass, and two spots are marked for the
coordinator rather than presented as checked signatures.

```rust
// List every registered domain with its primary capability, then ask
// the loaded config whether the queue domain is enabled on this host.
fn main() {
    for cap in fleet_registry::Capability::ALL {
        println!("capability: {}", cap.id());
    }
    for d in fleet_registry::DOMAINS {
        println!("domain {} (primary: {})", d.name, d.primary.id());
    }

    // Config::load() -> Result<Config, ConfigError>; a malformed
    // file is a loud error on every path, never a silent default.
    let config = fleet_config::Config::load().expect("fleet config must load");
    println!("queue enabled: {}", config.domain_enabled("queue"));
}
```

Signatures verified against the source: `Capability::id(self) -> &'static str`
is the accessor for the stable string ids used above, and
`fleet_registry::domain(name: &str) -> Option<&'static Domain>` is the
by-name lookup. `fleet_config` also offers
`Config::load_from(path: &Path) -> Result<Config, ConfigError>` for an
explicit manifest path.
