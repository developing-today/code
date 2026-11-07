# fleet — the multicall fleet binary

One Rust binary for the whole fleet surface (epic
developing-today/code#5412; design note
`doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/`). This
workspace began as the step-1 (#5429) skeleton: the registry,
config, secrets-by-name and proxy-protocol platform crates. Step 2
(#5430) landed the queue domain — the repo-queue client and the
queue-broker / secrets-broker serve roles are real below. Step 3
(#5431) landed the proxy domain — the session broker, its client,
the hello negotiation and the PATH shims. Step 4 (#5432) landed
the dispatch domain — a thin client over pm-stack dispatch. Step 5
(#5433) landed the search domain — the atlas-search client surface
over the live Atlas service, gated on the #5350 step-6 cutover.
Step 6 (#5554) landed the secrets verb. #5604 landed the search
_serve role_ (`fleet serve search`): the Atlas service itself,
rehomed from the atlas-search bin crate. With it, every domain and
every serve role in the registry runs for real in the full build.

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
                            [secrets], [dispatch]; path/ports/socket
                            constants.
crates/fleet-secrets/       secrets by NAME only (broker first, then
                            `bws` fallback). Values never appear in
                            code, tests, errors or logs.
crates/fleet-proxy-proto/   session-broker wire protocol: 8-byte BE
                            length + JSON frames (the live wire's
                            size — the note's "4-byte" was a census
                            error), hello/capability handshake,
                            major-version window (N, N-1, plus the
                            legacy proto 0 the Python clients speak).
crates/fleet-proxy/         the proxy domain (step 3): `proxy serve`
                            (the broker — per-port CONNECT-tunnel
                            pools on the historical socket path,
                            idle exit, nothing persisted) and the
                            client (`proxy connect`, `fleet init`,
                            the --stdio ProxyCommand relay, and PATH
                            shim regeneration from the negotiated
                            services map). The `serve` half is the
                            fleet-proxy/serve cargo feature, present
                            only in builds carrying proxy-serve.
crates/fleet-queue/         the queue domain (step 2): the repo-queue
                            client, the queue-broker and secrets-broker
                            roles, and packet validation — the broker
                            family's first source home (code#5362).
                            Its src/bin/ builds the standalone
                            repo-queue / queue-broker / secrets-broker
                            artifacts (the `fleet-brokers` package):
                            the migration's rollback inventory.
crates/fleet-search/        the search domain (step 5): the
                            atlas-search CLIENT surface over the
                            live Atlas service's job protocol —
                            `search` (hybrid FTS+vector with the
                            classification filters) and `get-chunk`.
                            Routing is the Atlas Q3 ruling: sandbox
                            seat via-service by default (through the
                            laptop-ssh loopback forward, the
                            deployed wrapper's pattern; loud
                            standalone fallback), laptop seat
                            standalone by default. Standalone means
                            the kept `atlas-search` artifact on
                            PATH — this workspace carries no second
                            embedder/backend. Ceilings are enforced
                            server-side against the host's atlas
                            client identity (client.json); the wire
                            types mirror atlas-proto (the contract
                            of record, untouched), and the output
                            writers are ported verbatim from the
                            atlas-search bin so service envelopes
                            print byte-identically.
                            The `serve` cargo feature (#5604) adds
                            the service role: `fleet serve search`
                            (also spelled `atlas-search serve` via
                            argv0) — server.rs is a parity-guarded
                            verbatim copy of atlas-search's service,
                            serve.rs the role glue (arg surface,
                            lazy shared embedder, read-only Turso
                            backend). atlas-core / atlas-embed /
                            atlas-proto are consumed by relative
                            path from pkgs/atlas-search, untouched,
                            which is why default.nix builds from the
                            pkgs/ tree. The standalone atlas-search
                            artifact stays buildable: it is the
                            single-step rollback of the unit
                            re-point (home/common: atlasSearchServeViaFleet).
crates/fleet-dispatch/      the dispatch domain (step 4): a THIN
                            client over pm-stack's pm-dispatch —
                            `dispatch status` (GET /health),
                            `dispatch launch` (POST /v1/sessions),
                            `dispatch enqueue` (POST /v1/tasks).
                            pm-stack is not rewritten (D6); the
                            bearer token resolves by name through
                            fleet-secrets, and the client reaches
                            the service directly (laptop seat) or
                            through its own CONNECT on the proxy
                            egress (sandbox seat).
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

| nix attr               | profile | capabilities                                                           |
| ---------------------- | ------- | ---------------------------------------------------------------------- |
| `fleet`                | full    | all ten                                                                |
| `fleet-serve`          | serve   | proxy-connect, proxy-serve, queue-broker, secrets-broker, search-serve, secrets |
| `fleet-static`         | full    | all nine (musl static; the sandbox's artifact)                         |
| `fleet-connect`        | connect | proxy-connect (dynamic)                                                |
| `fleet-connect-static` | connect | proxy-connect (musl static, runs anywhere)                             |
| `fleet-brokers`        | —       | standalone repo-queue / queue-broker / secrets-broker (rollback)       |

Every package installs its binary as `bin/fleet`; the hyphen
variants also get a `bin/fleet-serve` / `bin/fleet-connect` symlink
for side-by-side installs.

## The stub contract

A domain that cannot run answers on stderr and exits **69**
(EX_UNAVAILABLE), with wording that says which layer said no:

| case                         | wording                                                        |
| ---------------------------- | -------------------------------------------------------------- |
| not compiled into this build | `this build does not include '<domain>'` + enabled list        |
| narrowed out by argv0 name   | `invoked as 'fleet-connect', this build does not include …`    |
| disabled by config           | `'<domain>' is disabled by config on this host`                |
| compiled in, not yet built   | `'<domain>' is included in this build but not implemented yet` |

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

Step 3 flips the sandbox session proxy the same way: the new
binary deploys beside the old, the client proves itself against
the still-running Python broker (legacy dialect), then the broker
role flips on the same socket path and every consumer re-proves
(the evidence lives on #5431). The Python broker and
laptop-proxy-connect.py stay in place, untouched — SSH's
ProxyCommand re-point is gated on days of proof, a separate
reversible flip.

Step 1 installed nothing live. Step 2 performs the queue pilot's
flips — the laptop's broker units re-point at `fleet serve
<role>`, then the sandbox `~/tools/bin/repo-queue` symlink at the
fleet artifact — each verified by health plus a real
enqueue→claim→complete round-trip before the next begins (the
evidence lives on #5430). The standalone artifacts above are the
rollback for every flip.

Step 4 flips nothing live: the dispatch domain is a client, and
pm-stack keeps running exactly as it did (the step's gate is a
zero pm-stack diff). What changes is the caller's side — agents
hold `fleet dispatch status|launch|enqueue` instead of
hand-rolled curl against pm-dispatch, with the bearer token
resolved by name (`pm-dispatch.api-token`) through fleet-secrets
and the transport chosen by seat: direct on the laptop, a CONNECT
tunnel on the proxy egress from the sandbox. The steward cron's
call sites move to these verbs only if they want them; no cron
is rewritten by this epic (the evidence lives on #5432).
