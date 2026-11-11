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
                            [secrets], [queue-broker], [secrets-broker],
                            [dispatch]; path/ports/socket constants.
                            Role sockets (#6215, design #5699):
                            env > [<role>] socket > $XDG_RUNTIME_DIR/
                            fleet/<role>.sock > the $HOME default;
                            [<role>] transport = "socket" | "tcp" is a
                            preference between working routes (the
                            search_via shape), never a disable lever.
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

## Versioning — derived per-app versions (#8566; policy #5790)

A fleet binary's version is **derived, never hand-maintained**. The
workspace `version = "0.1.0"` in `Cargo.toml` is only the release-line
base cargo requires; the version a deployed binary reports — the one
`fleet version`, `fleet capabilities` (JSON `build.derived_version`)
and every `--version` surface print as `Derived version:` — is baked
at build time by `crates/fleet/build.rs` from the derivation in
`crates/fleet/src/derived_version.rs`, through the same channel that
reports the source rev (`FLEET_SOURCE_REV`): a packaged build with no
git history receives it as `$FLEET_DERIVED_VERSION`, computed by the
release tooling where the history exists.

Derivation (nearest `<app>/vX.Y.Z` tag on the build commit's history;
the app is this workspace's short name, `fleet`):

| build state | reported version |
| --- | --- |
| exactly on the tag | `X.Y.Z` |
| on `main`, N commits after the tag | `X.Y.(Z+1)-dev.N+g<shorthash>` |
| on a feature branch, N commits after | `X.Y.(Z+1)-dev.N+g<shorthash>+<branch-slug>` |
| no `<app>/v*` tag yet | `0.1.0-dev.N+g<shorthash>` (bare `0.1.0-dev` if no history is available) |

The `+g<hash>` suffix is what guarantees no two distinct commits ever
report the same version string; the patch-bump + `-dev.N` form is what
makes the string sort after the tag it descends from and before its
own release.

### Tag and release procedure (ruled on #5790, 2026-11-06)

1. **Per-app tags.** A release is cut by tagging the exact tested
   commit `<short-app-name>/vX.Y.Z` — for this workspace,
   `fleet/vX.Y.Z`. The short app name is the tag prefix for ALL apps
   (one policy, no per-app mixing); app names used in tags and
   deploys must be unique across apps — if a short name would
   collide, fix the app name, not the policy.
2. **First tag and 0.x discipline.** Each app's first tag is
   `0.1.0`. Versions stay in 0.x: minor for feature releases, patch
   for fixes, and **no major-version bumps** until a separate 1.0
   decision.
3. **Releases are tag builds only.** Build the release artifact
   from the tagged commit and verify the artifact itself reports
   the clean version: `./result/bin/fleet version` must show
   `Derived version: X.Y.Z`. Builds deployed from `main` are
   **prereleases** (their `-dev.N` strings say so) — deploying one
   never makes it a release.
4. **Repo tags.** At every release point (not every commit, not
   every deploy), also cut `repo/v<YYYYMMDD>.<minor>.<patch>` —
   the date is the major segment; within a date, a release point
   including any app feature increments minor and resets patch, a
   fix-only or repeat point increments patch; minor/patch may
   reset to `0.0` on a new date. A repo **prerelease** tag on a
   deploy from main is also fine and does not make the deploy a
   release.
5. **Deploy truth.** What a target runs is read from the deployed
   binary's own `Derived version:` line, never from paperwork
   (that read is the #5760 deployed-state check's input).

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

## Deploy: the two live installs (read this before merging a new verb)

A verb is **not shipped when it merges** — it is shipped when BOTH
live installs report it. #6728 is the failure this section exists to
prevent: `fleet queue fallback` was merged as source + docs
(`crates/fleet-queue/src/fallback.rs`, wired in `lib.rs`/`client.rs`)
while the installed 0.1.0 binaries kept answering
`repo-queue: unknown command "fallback"` for a full day of
backend-down shifts.

Build ONE artifact from current `main` and deploy that same build to
both installs. If the build fails in the cargo vendor step with
`no matching package named '<crate>' found ... required by package
'atlas-core'` (or another atlas crate), `pkgs/fleet/Cargo.lock` has
drifted from the atlas-search path dependencies again — the fleet
lock covers their external dependencies too (see `default.nix`).
Regenerate it before building:

```sh
(cd pkgs/fleet && cargo update --workspace)
```

```sh
nix build .#fleet
./result/bin/fleet version   # record the source rev; both installs must match it
```

1. **Laptop install** — a plain binary copy at `~/.local/bin/fleet`
   (lane-writable). Copy the fresh artifact over it, then verify.
2. **Sandbox install** — `~/tools/fleet`, reached through the
   `~/tools/bin/fleet` and `~/tools/bin/repo-queue` symlinks. This
   file is **root-owned provisioning** (read-only to lanes): a lane
   cannot redeploy it. The sandbox redeploy is done by whoever
   provisions `~/tools`; a lane that verifies the sandbox install is
   stale files/updates the tracking issue with both source revs
   (installed vs current `main`) instead of improvising a replacement
   on `PATH` — the symlinks are the contract every lane resolves.

Verify BOTH installs after every deploy, with the verb surface as
the check (a version string alone proves nothing):

```sh
fleet queue --help            # on the laptop AND on the sandbox
fleet queue fallback scan --repo developing-today/code   # JSON items, exit 0
```

If the two installs ever disagree on source rev or verb list, the
stale one is a bug: file it against this section (the #6728 shape)
rather than working around it silently.

## Location-aware init: provision before the binary flip (#5687, impl #6770)

`fleet init` is location-aware: it resolves this machine's
location and role set from the `[host]` section of `fleet.toml`
(or `FLEET_HOST_CONFIG` / `FLEET_LOCATION`) and ensures exactly
those roles — sandbox = the session proxy under `init_mode =
"spawn"`; laptop/lab = the daemon roles under `init_mode =
"systemd"`, reported via their `fleet-<role>.service` units and
started only through `systemctl --user`. An unknown location
reports state, starts nothing, and exits 69. The full design is
issue #5687 §§1–6.

**The deploy sequence is part of the feature** (§6): the
location-aware binary has NO "no manifest ⇒ assume proxy"
fallback — that guess is the bug the design removes. Before a
location-aware build replaces either live install:

1. Write the `[host]` section into that machine's `fleet.toml`
   (sandbox: `location = "sandbox"`, `init_mode = "spawn"`,
   `[host.roles] proxy = {}`).
2. Pin the sandbox's `init-ensure.sh` call site to
   `FLEET_LOCATION=sandbox fleet init` (belt-and-braces: the env
   var makes the caller's assumption explicit and survives a
   lost/truncated file location).
3. Only then deploy the binary, and verify per the section
   above: `fleet init` prints `location sandbox (source: …)`, a
   second run is the strict no-op, exit 0 both times.

**Rollback gotcha:** `fleet-config` parses with
`deny_unknown_fields`, so a `fleet.toml` containing `[host]` is a
**parse error to the pre-#6770 binary**. Rolling the binary back
therefore requires removing the `[host]` section (or rolling
config and binary back together) — never discover this during
an incident.

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
