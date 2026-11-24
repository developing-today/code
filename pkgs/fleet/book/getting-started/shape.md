# The shape of the binary

Fleet is a _multicall binary_: one executable whose behaviour is selected
first by the name it is invoked under (argv0, the zeroth argument — the name
the operating system was asked to run), and then by its first real argument.
This chapter describes that shape: the personalities, the domain registry,
the capability system, the build variants, and what happens when an
invocation asks for something it cannot do.

## argv0 personalities and aliases

There are three classes of invocation name:

1. **`fleet`** — the full root dispatch: domains, plus the built-in commands.
2. **Variant names** — `fleet-serve` and `fleet-connect`. These select an
   invocation _profile_: a capability ceiling. A profile can only narrow the
   surface, never widen it. Invoking a serve build as `fleet` does not give
   it client capabilities it was not compiled with; invoking a full build as
   `fleet-connect` restricts it to the connect profile for that invocation.
3. **Alias names** — each routes straight into one domain, some with an
   implied argument prepended:

| argv0                 | Routes to             | Implied argument |
| --------------------- | --------------------- | ---------------- |
| `repo-queue`          | domain `queue`        | —                |
| `queue-broker`        | domain `serve`        | `queue-broker`   |
| `secrets-broker`      | domain `serve`        | `secrets-broker` |
| `laptop-agent-broker` | domain `proxy`        | `serve`          |
| `atlas-search`        | domain `search`       | —                |
| `laptop-agent`        | domain `laptop-agent` | —                |

An argv0 that is none of {`fleet`, a variant name, an alias} is a usage
error: the binary prints the expected names and exits 2. Aliases are how the
install works in practice — `~/tools/bin/repo-queue` is a symlink to the
fleet binary, not a second program (see
[Installation and bootstrap](installation.md)).

Dispatch order, exactly: argv0 alias → domain route; argv0 variant name →
root dispatch with that profile as the ceiling; argv0 `fleet` → root
dispatch; then, for a domain, the capability checks below decide between the
domain handler and the stub; an unknown name is a usage error, exit 2. The
dispatch is hand-rolled — deliberately not a command-line parsing library —
so that it behaves identically in every variant, including the tiny connect
build where the registry and the stub machinery are the product.

## The domain registry

The registry is compiled into _every_ build, including `fleet-connect`: every
build knows every domain's name, summary, aliases, and required capabilities,
whether or not it can run it. That is why `fleet --help` prints "the full
registry — every build knows every domain" on every variant, and why domain
help never stubs — help is registry data and renders identically everywhere,
with unavailable domains marked inline.

The twelve domains, in registry (help) order, are `queue`, `serve`, `proxy`,
`dispatch`, `dispatch-test`, `secrets`, `gh`, `bridge`, `agent`, `shell`,
`search`, and `laptop-agent`. Their purposes are tabulated in the
[introduction](../index.md#the-domain-map).

Each domain declares a _primary capability_ — the capability its headline
availability keys on. One consequence worth knowing: on the serve build the
`search` domain is marked absent, because its primary capability is the
search _client_, even though the `search-serve` capability exists in that
build under the `serve` domain.

## Capabilities: the 15

A _capability_ is an atom of function that a build can include or omit.
There are 15, in registry order:

```
proxy-connect, proxy-serve, proxy-raw-splice, queue, queue-broker,
secrets-broker, search, search-serve, dispatch, dispatch-test,
laptop-agent, secrets, gh, bridge, agent
```

Notes:

- `proxy-connect` is unconditional: it is compiled into every variant, and is
  not even a Cargo feature. `proxy-raw-splice` joins it in the runtime's
  always-present set.
- There is no `shell` capability atom: the `shell` domain rides on the
  `agent` capability.
- A domain can require several capabilities. `serve` requires four
  (`queue-broker`, `secrets-broker`, `search-serve`, `proxy-serve`) — one per
  role — and the capability checked for a `serve` invocation is the one for
  the role named. `proxy` requires three (`proxy-serve`, `proxy-connect`,
  `proxy-raw-splice`), checked per verb.

Three sets decide whether a domain runs for a given invocation: the
_compiled_ set (from the build's Cargo features), the _profile_ set (from a
variant invocation name, if any), and the host config's `[domains]` table,
which can disable a domain on one host. The _effective_ set is the compiled
set intersected with the profile; the _usable_ set further removes every
capability of every config-disabled domain. `fleet version` reports the
usable set; `fleet capabilities` reports both the usable set
(`capabilities`) and the compiled set (`compiled_capabilities`).

## Build variants

The variant is a property of the artifact. The same names double as the
invocation profiles above.

| Variant | Installed name  | Compiled capabilities                                                                                                                                                  |
| ------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Full    | `fleet`         | all 15                                                                                                                                                                 |
| Serve   | `fleet-serve`   | `proxy-connect`, `proxy-serve`, `proxy-raw-splice`, `queue-broker`, `secrets-broker`, `search-serve`, `secrets` (7) — daemon roles plus proxy, no working-seat clients |
| Connect | `fleet-connect` | `proxy-connect`, `proxy-raw-splice` (2)                                                                                                                                |

The profiles nest: connect ⊂ serve ⊂ full. The binary _reports_ its variant
by matching its compiled capability set against the variant sets; an
unrecognised compiled set is reported as a custom build.

Alongside the three variants, the Nix package set builds `fleet-brokers` —
the three standalone artifacts `repo-queue`, `queue-broker`, and
`secrets-broker` — and the two musl statics, `fleet-static` (full profile)
and `fleet-connect-static` (connect profile, size-gated at 2 MiB). See
[Installation and bootstrap](installation.md#nix-build-variants).

## The stub and exit 69

A domain this invocation cannot use answers with a _stub_: a block on stderr,
always exit code **69** (`EX_UNAVAILABLE`). There are four wordings, one per
reason:

1. **Not in build** — "this build does not include '&lt;domain&gt;'", followed by
   the enabled capabilities and the build name, plus "The full build is
   'fleet'" when the build is not the full one.
2. **Narrowed by name** — invoked as `fleet-serve` or `fleet-connect`, the
   name scoped the surface away: the stub says the invocation is name-scoped
   to that profile, and that invoked as `fleet` it carries the full surface.
3. **Disabled by config** — the `[domains]` table in this host's fleet config
   set the domain to `false`; the stub names the config file to re-enable it
   in. This is the per-host shaping / incident lever: one host's fleet can
   have a domain switched off without a rebuild.
4. **Available but not implemented** — "included in this build but not
   implemented yet", with the stub's third line naming the build and the
   not-implemented state.

Every stub's second line is `Enabled capabilities: <comma list>` (`none`
when empty), so a stub always tells you what the invocation _can_ do.

## Built-in commands overview

Five root built-ins are handled before domain dispatch. They take no
arguments: a trailing argument is a usage error naming it, exit 2 — and that
includes `--help`, so `fleet version --help` is an error, not help.

| Built-in        | Behaviour                                                                                                                                                                                                                                                                      |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `init`          | Autostart this host's roles per the `[host]` manifest; a no-op when they are already running. Byte-identical to `fleet proxy init` — one implementation, two spellings. With an unknown location it reports state, starts nothing, and exits 69: it never guess-starts a role. |
| `session bind`  | Validate and persist an explicit or inherited session identity (an OpenCode session id, `ses_` followed by ASCII letters/digits). The identity is stored in a `.fleet-session-id` marker at the git worktree root, and is used only when the session is verified live.         |
| `version`       | Five lines: version, build, derived version, source rev, enabled capabilities. `fleet --version` / `-V` print only the first two lines.                                                                                                                                        |
| `capabilities`  | The registry and this build's capabilities as JSON (see the Quickstart for how to read it).                                                                                                                                                                                    |
| `help [domain]` | `fleet help` ≡ `fleet --help`: the full registry, exit 0. `fleet help <domain>` prints one domain's help and rejects any second argument. An unknown name is a usage error, exit 2.                                                                                            |

Bare `fleet` with no arguments is a missing-subcommand error: the message
and the full help go to stderr, exit 2.

Two more root-level facts: every domain invocation first runs session
initialisation (the `session bind` machinery with no explicit id), and every
invocation — whatever its outcome — appends a payload-free record (invoked
as, arguments, exit code, elapsed milliseconds) to an invocation ledger;
failing to write the ledger never alters the result.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet/src/main.rs and https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-registry/src/lib.rs
