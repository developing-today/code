# fleet — the multicall fleet binary

One Rust binary for the whole fleet surface (epic
developing-today/code#5412; design note
`doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/`). This
workspace is step 1 (#5429): the skeleton. The registry, config,
secrets-by-name and proxy-protocol platform crates are real; the
domain commands are capability stubs until their own steps land
(queue #5430, proxy #5431, search #5432, dispatch #5433).

## Layout

```
Cargo.toml                  workspace (release profile is size-tuned:
                            opt-level z, LTO, one codegen unit, stripped)
rust-toolchain.toml         pinned toolchain (atlas pattern)
crates/fleet-registry/      the FULL domain registry — every domain's
                            name, summary, aliases, subcommands and the
                            capability ids behind them. Compiled into
                            every variant, so --help is always complete.
crates/fleet-config/        fleet.toml: [domains] switches, [proxy],
                            [secrets]; path/ports/socket constants.
crates/fleet-secrets/       secrets by NAME only (broker first, then
                            `bws` fallback). Values never appear in
                            code, tests, errors or logs.
crates/fleet-proxy-proto/   session-broker wire protocol: 4-byte BE
                            length + JSON frames, hello/capability
                            handshake, major-version window (N, N-1).
crates/fleet/               the binary: multicall dispatch, capability
                            stubs, `version`, `capabilities`, help.
default.nix                 the four variant packages (see below)
smoke.sh                    per-variant smoke suite (post-build)
```

## Variants

Capability surface = compiled features ∩ invocation-name profile ∩
config. Invoking under a hyphenated name (`fleet-serve`,
`fleet-connect`) NARROWS the surface to that profile — it never
widens past the compiled features (refinement A). Domain alias
names (`repo-queue`, `queue-broker`, `secrets-broker`,
`laptop-agent`, `atlas-search`, `laptop-agent-broker`) route to
their domain through argv0.

| nix attr              | profile | capabilities                                                        |
| --------------------- | ------- | ------------------------------------------------------------------- |
| `fleet`               | full    | all nine                                                            |
| `fleet-serve`         | serve   | proxy-connect, proxy-serve, queue-broker, secrets-broker, search-serve |
| `fleet-connect`       | connect | proxy-connect (dynamic)                                             |
| `fleet-connect-static`| connect | proxy-connect (musl static, runs anywhere)                          |

Every package installs its binary as `bin/fleet`; the hyphen
variants also get a `bin/fleet-serve` / `bin/fleet-connect` symlink
for side-by-side installs.

## The stub contract

A domain that cannot run answers on stderr and exits **69**
(EX_UNAVAILABLE), with wording that says which layer said no:

| case                        | wording                                                    |
| --------------------------- | ---------------------------------------------------------- |
| not compiled into this build| `this build does not include '<domain>'` + enabled list    |
| narrowed out by argv0 name  | `invoked as 'fleet-connect', this build does not include …`|
| disabled by config          | `'<domain>' is disabled by config on this host`            |
| compiled in, not yet built  | `'<domain>' is included in this build but not implemented yet` |

Usage errors exit 2. `fleet --help` always renders the full
registry, marking each unavailable domain with the reason
(`[not in this build]`, `[narrowed out by this name]`,
`[disabled by config]`). `fleet version` prints the version, the
build name, the source rev and the exact enabled-capability list;
`fleet capabilities` prints the same surface as JSON.

## Build and smoke

```sh
nix build .#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static
./pkgs/fleet/smoke.sh ./result ./result-1 ./result-2 ./result-3
cargo test --workspace   # unit tests (registry, config, proto, secrets)
```

The connect variants carry the D7 size contract: **≤ 2 MiB hard
gate** (target ≤ 1.5 MiB), enforced in `default.nix` postFixup
against the final stripped artifact and re-asserted by smoke.sh.

No package here installs anything live: no units, no PATH changes,
no flips. The first live flip is step 2.
