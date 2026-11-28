# Register a domain or capability

A domain exists in fleet because three things agree: the registry names it, a Cargo feature compiles it in, and the runtime reconstructs the same fact from the compiled binary. This chapter explains that machinery and lists everything a new domain must add. For the crate map, see [Repository layout](layout.md); for the build attributes that select feature sets, see [Building fleet](building.md).

## The registry: `crates/fleet-registry/src/lib.rs`

`fleet-registry` is a dependency-free crate of strings and tables — no I/O — compiled into every variant, including `fleet-connect`, so that `--help` is always complete. It owns three things.

### Domains

`pub const DOMAINS: &[Domain]` is the full registry, in help order, with 19 entries: `queue`, `serve`, `proxy`, `dispatch`, `dispatch-test`, `secrets`, `gh`, `bridge`, `orgs`, `agent`, `shell`, `search`, `cert`, `laptop-agent`, `commander`, `http`, `ssh`, `laptop`, `actions`. `pub fn domain(name)` is a linear find over it.

`struct Domain` is the unit that help lists, stubs name, and config disables:

| Field                                 | Meaning                                                                                                                                                                                                          |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name: &'static str`                  | The domain's invocation name.                                                                                                                                                                                    |
| `summary: &'static str`               | The one-line description help renders.                                                                                                                                                                           |
| `capabilities: &'static [Capability]` | Every capability the domain spans.                                                                                                                                                                               |
| `primary: Capability`                 | The headline capability availability keys on. Example: on `fleet-serve` the `search` domain is marked absent because its headline is the client, even though the `search-serve` capability exists under `serve`. |
| `aliases: &'static [&str]`            | The domain's argv0 aliases.                                                                                                                                                                                      |
| `subcommands: &'static [&str]`        | The top-level verb inventory, rendered by domain help and checked by the DOC-01 machine test.                                                                                                                    |

`struct AliasRoute { domain, implied_arg }` and `pub fn alias_route(argv0)` define the argv0 map. The shipped aliases are: `repo-queue` → `queue`; `queue-broker` → `serve` with implied arg `queue-broker`; `secrets-broker` → `serve` with implied arg `secrets-broker`; `laptop-agent-broker` → `proxy` with implied arg `serve`; `atlas-search` → `search`; `laptop-agent` → `laptop-agent`. An argv0 that is none of {`fleet`, a variant name, an alias} is a usage error at exit 2 naming the expected names. `is_known_invocation_name(name)` is `fleet` ∨ variant name ∨ alias.

### Capabilities: the 22 atoms

`enum Capability` has 22 variants with stable string ids (`Capability::id`, `Capability::ALL`):

`proxy-connect`, `proxy-serve`, `proxy-raw-splice`, `queue`, `queue-broker`, `secrets-broker`, `search`, `search-serve`, `dispatch`, `dispatch-test`, `laptop-agent`, `secrets`, `gh`, `bridge`, `agent`, `commander`, `laptop-http`, `laptop-ssh`, `laptop`, `cert`, `actions`, `orgs`.

Note there is no `shell` capability atom: the `shell` domain rides on the `agent` capability.

`CapabilitySet` is a `BTreeSet<Capability>`, built with `capability_set(&[…])`. Named sets: `full_capabilities()` (all 22), `serve_capabilities()` = {ProxyConnect, ProxyServe, ProxyRawSplice, QueueBroker, SecretsBroker, SearchServe, Secrets}, and `connect_capabilities()` = {ProxyConnect, ProxyRawSplice}.

### `CapState`: the four availability states

At dispatch, `Surface::cap_state(cap, domain_name)` checks in order and yields `enum CapState`:

| State              | Meaning                                                                |
| ------------------ | ---------------------------------------------------------------------- |
| `Available`        | Compiled in, in profile, not config-disabled. The domain handler runs. |
| `NotInBuild`       | The Cargo feature was not compiled into this build.                    |
| `NarrowedByName`   | Compiled in, but the argv0 profile (below) narrowed it out.            |
| `DisabledByConfig` | The `[domains]` table in `fleet.toml` disabled it on this host.        |

Any non-`Available` state falls through to the stub (exit 69 — see the stub wordings below). `Surface` itself is the invocation's whole effective surface: `compiled` (from Cargo features), `profile` (from the hyphenated invocation name), `effective` (= `compiled ∩ profile.capabilities()`, or `compiled` when there is no profile), and the loaded `fleet_config::Config`.

The registry's own tests are invariant oracles: `capability_ids_are_unique_and_stable`, `variant_profiles_nest` (connect ⊂ serve ⊂ full, and full == ALL), `every_domain_primary_is_in_its_surface`, `aliases_route_to_real_domains`, `domain_aliases_match_the_argv0_map`.

## How capabilities are compiled in: `compiled_capabilities()` and bin-crate features

Two layers, and both must agree.

**Layer 1 — Cargo features on the binary crate** (`crates/fleet/Cargo.toml`, `[features]`):

- `default = ["full"]`.
- `full = [proxy-serve, queue, queue-broker, secrets-broker, search, search-serve, dispatch, dispatch-test, laptop-agent, secrets, gh, bridge, agent, opencode-probe]`.
- `serve = [proxy-serve, queue-broker, secrets-broker, search-serve, secrets, opencode-probe]` — the broker-side subset, shipped as one feature for the Nix build.
- The connect build is `--no-default-features` with nothing optional on.
- `proxy-connect` is **not** a feature: it is compiled into every variant.
- Each feature wires optional dependencies, for example `queue = ["dep:fleet-secrets", "dep:fleet-queue", "fleet-queue/client"]` and `search-serve = ["dep:fleet-search", "fleet-search/serve"]`. `fleet-proxy` is a non-optional dependency with `default-features = false`: the proxy client is in every variant, only the broker half (`fleet-proxy/serve`) is gated.
- Two things are kept out of connect on purpose: `opencode-probe` (its `ureq` + rustls stack would break the D7 2 MiB gate — see [Building fleet](building.md)) and `fleet-secrets` (the connect variant carries nothing else).

Domain crates mirror the discipline one level down. In `fleet-queue`, the features are `client`, `queue-broker`, and `secrets-broker`, and `src/lib.rs` gates modules on them: `client`, `fallback`, `needs_drew`, `placement` under `client`; `queue_broker`, `turso_queue`, `validate` under `queue-broker`; `secrets_broker` under `secrets-broker`; `uds_http` under either broker. In `fleet-search`, `client` and `serve` split the client surface from the Atlas service role.

**Layer 2 — runtime reconstruction.** `fn compiled_capabilities()` in `crates/fleet/src/main.rs` starts from `{ProxyConnect, ProxyRawSplice}` unconditionally and inserts one `Capability` per `cfg!(feature = "…")`. `build_variant(compiled)` then re-derives which shipped `Variant` the set equals — Full, Serve, or Connect; an unrecognised set reports its build name as `fleet-custom`. The binary _reports_ its variant from its feature set; it does not trust a baked-in name. `fleet capabilities` prints both lists: `capabilities` (what this invocation can use) and `compiled_capabilities` (what the binary was built with) — their difference is exactly the profile and config narrowing.

## argv0 profiles narrow only

`enum Variant { Full, Serve, Connect }` is a property of the artefact whose names double as invocation profiles: `Variant::name()` is `fleet` / `fleet-serve` / `fleet-connect`. `Variant::from_invocation_name()` recognises only the two hyphenated names; plain `fleet` selects no profile.

The rule is **narrowing only**: a profile is a capability ceiling (`compiled ∩ profile.capabilities()`), never a widening. Invoking the full binary as `fleet-connect` gives you the connect surface; invoking the connect binary as `fleet` gives you nothing extra, because the capabilities were never compiled in (`NotInBuild`, not `NarrowedByName`). An alias route, by contrast, constructs the surface with no profile at all and prepends its implied argument.

## The stub wordings

When a domain is not `Available`, `fn stub(...)` in `main.rs` prints to stderr and returns exit 69 (`EXIT_UNAVAILABLE`, `EX_UNAVAILABLE` from sysexits.h). The first line names the domain and the reason class; the second line lists what this invocation _can_ do (`Enabled capabilities: <usable caps>.`); the third names the build. One form per `CapState`:

- `NotInBuild`: `fleet: this build does not include '<domain>'.` plus `This is the <build> build (<VERSION>).`, and ` The full build is 'fleet'.` when the build is not full.
- `NarrowedByName`: `fleet: invoked as '<invoked_as>', this build does not include '<domain>'.` plus the explanation that the invocation is name-scoped to the profile and that invoked as `fleet` it carries the full surface.
- `DisabledByConfig`: `fleet: '<domain>' is disabled by config on this host.` plus the instruction to re-enable the domain under `[domains]` in the named `fleet.toml` path.
- `Available` (for a surface compiled in with nothing behind it): `fleet: '<domain>' is included in this build but not implemented yet.` plus the line naming the build and the not-implemented state.

Help renders the same states without stubbing: domain help is registry data and is always available, even where the domain is not. Help marks each domain `included in this build`, `not in this build — invoking it prints the capability stub (exit 69)`, `narrowed out by this invocation name — … (exit 69)`, or `disabled by config on this host (exit 69 if invoked)`, and top-level help closes with: "A domain this invocation cannot use answers with a stub and exit code 69."

## Step by step: what a new domain must add

1. **Registry entry.** A `Domain` in `DOMAINS` in `crates/fleet-registry/src/lib.rs`, in help order, with `name`, `summary`, `capabilities`, `primary` (which must be a member of its own `capabilities` — the registry tests enforce this), `aliases`, and `subcommands`. If the domain needs a new capability atom, add the `Capability` variant with a stable string id and place it in `ALL` and in the named sets it belongs to.
2. **Crate.** A new library crate under `pkgs/fleet/crates/`, added to the workspace `members` in `pkgs/fleet/Cargo.toml` and to `[workspace.dependencies]` by path, exposing a `run` entry point with the domain's argument and config shape. Follow the existing crates' feature discipline: gate server roles and heavy dependencies behind features so the connect build stays clean.
3. **Bin-crate wiring.** An optional dependency plus a feature in `crates/fleet/Cargo.toml`; the feature added to `full` (and to `serve` if the domain has a broker-side role); one `cfg!(feature = "…")` insertion in `compiled_capabilities()` in `main.rs`.
4. **Dispatch arm.** In `run_domain` in `main.rs`, a `#[cfg(feature = …)]` arm under the `CapState::Available` path calling the crate's `run`. Subcommand-sensitive capability requirements (the way `serve` maps its role token to `QueueBroker` / `SecretsBroker` / `SearchServe` / `ProxyServe`) go in `required_capability(...)`. An argv0 alias, if the domain needs one, goes in `alias_route()` in the registry — and the registry test `domain_aliases_match_the_argv0_map` must still pass.
5. **Help.** The registry entry _is_ the top-level help. The domain's own `--help`, per-verb usage strings, and parser live in the domain crate, hand-rolled — see [Add a subcommand](add-a-subcommand.md).
6. **Capabilities JSON.** Nothing extra to write: `fleet capabilities` is generated from the registry plus `compiled_capabilities()`, so a correctly registered domain appears in it automatically, with its per-domain `state` one of `available`, `not-in-build`, `narrowed-by-name`, `disabled-by-config`. Verify it does, on both a full build and a narrowed build.
7. **Build and smoke coverage.** A Nix attribute only if the domain needs a new variant (rare — variants are profiles, not domains). Smoke coverage in `pkgs/fleet/smoke.sh` that the domain renders in help on every variant and stubs at 69 where it is unavailable.
8. **Docs in the same change (DOC-01).** `pkgs/fleet/docs/verbs.md` and this book in the same change — see [Keeping this book current](keeping-this-book-current.md). The DOC-01 machine test (`pmo_operator_docs_match_registered_help_inventory`) ties the README's operator contract to the registered inventory, so a registry entry without its contract docs fails the workspace suite.
