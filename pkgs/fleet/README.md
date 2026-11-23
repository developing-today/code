# fleet — the multicall fleet binary

One Rust binary for the whole fleet surface (epic
developing-today/code#5412; design note
`../../doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/2026-11-04T11-45-48Z_design_fleet_multicall_binary.md`). This
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
crates/fleet-bridge/        the bridge domain: fleet mailboxes on the
                            message bridge. Central sends messages or
                            commands to an agent, a group or all
                            (owner token); agents hello/inbox/post/ack
                            (fleet token, derived from the owner token
                            when its own secret is missing). Runbook:
                            doc/agents/fleet-command-channel.md.
crates/fleet-agent/         the agent domain and `fleet shell`: one
                            front door over the other verbs. `agent
                            enroll` registers any kind of agent and
                            reports which intake rungs its seat
                            reaches; `agent next` returns one task
                            from the highest rung that has one (a
                            pushed command, the queue, the host's
                            shortlist, GitHub priority); `shell` runs
                            a command or opens a shell on the laptop,
                            a codespace or here. Runbook:
                            doc/agents/onboarding-an-agent.md.
crates/fleet-queue/         the queue domain (step 2): the repo-queue
                            client, the queue-broker and secrets-broker
                            roles, and packet validation — the broker
                            family's first source home (code#5362).
                            Its src/bin/ builds the standalone
                            repo-queue / queue-broker / secrets-broker
                            artifacts (the `fleet-brokers` package):
                            the migration's rollback inventory. Remote
                            build/test dispatch runbook:
                            doc/agents/remote-build-and-test.md.
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
default.nix                 the six variant packages (see below)
smoke.sh                    per-variant smoke suite (post-build)
```

## Session identity in worktrees (#9075)

`fleet session bind [--session-id ID]` is the explicit, validated boundary for
binding an identity. It resolves the explicit value, then `FLEET_SESSION_ID`,
then `OPENCODE_SESSION_ID`, then the worktree's plain-text `.fleet-session-id`.
The OpenCode session plugin exports its session ID as `FLEET_SESSION_ID` to
shell commands. `fleet init` and supported Fleet domain invocations bind that
identity before starting work, persist it at the current worktree root, and
pass it to Fleet child processes through the inherited environment.
Every source must match the `ses_` plus ASCII alphanumeric format. Supplied
identities are persisted at the Git worktree root (not the common Git
directory), so linked worktrees can track different sessions; Git ignores this
runtime marker. A recovered marker is accepted only when the local OpenCode
server returns a matching, non-archived session record; the check uses
`OPENCODE_URL`, then `OPENCODE_SERVER_URL`, and the OpenCode Basic-auth password
from `OPENCODE_AUTH_FILE` or `~/.local/state/opencode/service-prod.json`. An
ended or unverifiable marker fails closed. A malformed nonempty session ID in
either environment variable also fails closed for supported Fleet invocations;
clear or correct that variable to resume unrelated commands.
`fleet init` validates help and extra arguments before session resolution, and
`fleet proxy init` resolves identity only once.
Dispatcher-handled help, dispatcher argument-shape errors, and unavailable
domains return before session resolution. Supported invocations resolve
identity before entering their domain handlers; domain-specific help and usage
errors can therefore be reported after marker resolution. A successfully
completed Fleet-owned `gh issue|pr close|reopen` also resolves the session
identity for its closure record, so a live marker is effective there even when
the parent environment did not carry the id. Create-path
`provenance.session_id` (tracked separately by #9073) remains outside this
checkpoint. A malformed inherited session ID causes supported domain
invocations and `fleet init` to exit with code 2 before starting work.

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

| build state                          | reported version                                                         |
| ------------------------------------ | ------------------------------------------------------------------------ |
| exactly on the tag                   | `X.Y.Z`                                                                  |
| on `main`, N commits after the tag   | `X.Y.(Z+1)-dev.N+g<shorthash>`                                           |
| on a feature branch, N commits after | `X.Y.(Z+1)-dev.N+g<shorthash>.<branch-slug>`                             |
| no `<app>/v*` tag yet                | `0.1.0-dev.N+g<shorthash>` (bare `0.1.0-dev` if no history is available) |

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
   `Derived version: X.Y.Z`. A packaged (nix) release build takes
   the version through the package's `derivedVersion` argument —
   the build source carries no `.git`, so the release tooling
   computes the version where the history exists and passes it in;
   a packaged build without one reports the rev-derived prerelease
   form `0.1.0-dev+g<rev>`, never a clean release string. Builds
   deployed from `main` are
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

| nix attr               | profile | capabilities                                                                    |
| ---------------------- | ------- | ------------------------------------------------------------------------------- |
| `fleet`                | full    | every capability                                                                |
| `fleet-serve`          | serve   | proxy-connect, proxy-serve, queue-broker, secrets-broker, search-serve, secrets |
| `fleet-static`         | full    | every capability (musl static; the sandbox's artifact)                          |
| `fleet-connect`        | connect | proxy-connect (dynamic)                                                         |
| `fleet-connect-static` | connect | proxy-connect (musl static, runs anywhere)                                      |
| `fleet-brokers`        | —       | standalone repo-queue / queue-broker / secrets-broker (rollback)                |

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

## Operator and contributor contract (DOC-01, #10728)

The queue surface's contract with the humans and agents who operate
it: lifecycle relations, authority, error posture and privacy. Source
designs: the lifecycle-subcommands (#9206), typed dependencies /
notes / journal / field-authority (#9208) and leases / fencing /
outbox / direct-completion (#9209) components of the pm-orchestration
design; this section is the reviewed operator/contributor runbook
their DOC-01 slice called for. A machine check ties it to the
registry: `pmo_operator_docs_match_registered_help_inventory`
(`crates/fleet-registry/tests/pmo_operator_docs.rs`).

### The inventory is the registry

`fleet --help` always renders the FULL registry on every variant and
`fleet capabilities` prints it as JSON. The registered inventory —
every domain's name, summary, argv0 aliases and the capability ids
behind them — lives in one place,
[`crates/fleet-registry/src/lib.rs`](crates/fleet-registry/src/lib.rs),
and every build compiles it, so no build can know a domain another
build forgot. The registered domains are `fleet queue`, `fleet serve`,
`fleet proxy`, `fleet dispatch`, `fleet dispatch-test`,
`fleet secrets`, `fleet gh`, `fleet bridge`, `fleet agent`,
`fleet shell`, `fleet search` and `fleet laptop-agent`, plus the
built-ins `fleet init`, `fleet session`, `fleet version`,
`fleet capabilities` and `fleet help`.

The per-domain `subcommands` arrays are the top-level command inventory rendered by `fleet <domain> --help`. The argument grammar and nested leaves live in the domain handlers and are documented in [`docs/verbs.md`](docs/verbs.md). That reference covers all shipped Fleet domains, top-level verbs, nested queue and agent commands, and where to ask a leaf for exact usage.

The queue client dispatches 32 top-level verbs, including `show`; grouped surfaces (`fallback`, `dispatch`, `session`, `surfaces`, `outbox`, and `stack`) document their nested commands and flags in the same reference. Before scripting an unfamiliar command, check `fleet capabilities` and the leaf's `--help`.
The same contract covers the book. [`book/`](book/) (mdBook sources,
`book.toml` at this package's root) is the complete, self-contained
reference for the binary — every domain, subcommand, flag, default and
operational procedure — and any change to a command, flag, default,
capability, surface or procedure updates the matching chapter in the
same change. The rule is recorded for agents in
[`doc/agents/documentation.md`](doc/agents/documentation.md) and in
the book's own "Keeping this book current" chapter. The book builds
with `nix build .#fleet-docs`; `create-missing = false` in
`book.toml` makes a missing chapter a build failure, never a stub.

Shipped examples — every verb below is dispatchable today:

```sh
fleet queue claim --worker <worker> --lane <lane>        # one atomic claim; identity is stamped at claim time
fleet queue checkpoint <task> --worker <worker> --stage <stage> --progress '{...}'
fleet queue park <task> --worker <worker> --reason "CI pending on PR <n>" --delay-seconds 1200
fleet queue heartbeat <task> --worker <worker> --extend-seconds 900
fleet queue dep-recheck                                 # re-evaluate recorded typed dependency edges
fleet queue complete <task> --worker <worker> --result '{...}'
```

### Designed spellings — not yet in the registry

Two command spellings from the #9206/#9208 designs are contract, not
surface. They are NOT in the registered inventory and not
dispatchable today — invoking either answers `unknown command`. They
appear here because the operator contract references them; do not
script them until they register:

- `fleet work complete --merge-pr … --close-issues … --close-queue
--verify --wait` — the designed one-operation completion saga
  (persist intent → verify each remote effect individually → direct
  queue close with receipt). Shipped equivalent: verify each remote
  effect yourself, then `fleet queue complete` with a structured
  result.
- `fleet dependency add` — the designed typed-edge verb for declaring
  a hard blocker on a work item. Shipped equivalent: record the edge
  with `fleet queue block` (`--blocked-by-issue`, `--blocked-by-pr`,
  `--blocked-by-task`, `--blocked-by-ci`) and let
  `fleet queue dep-recheck` re-evaluate it.

### Dependency semantics — typed edges only

What can block a work item is exactly the typed hard-blocker edges
recorded on it (blocked-by issue / PR / task / CI, each with its
recheck cadence and TTL). Referencing another item, or being filed as
its child, blocks nothing: dependency semantics are the typed edges
only. A cleared edge releases its block; a stale one is re-checked by
`fleet queue dep-recheck` on the recorded schedule.

### Authority and attribution

A GitHub author field is attribution, not authentication. A session's
authority over a work item comes from its authenticated queue claim —
worker, lane, lease, fence — and never from a name in a payload,
comment or label. The broker stamps claim identity at claim time;
progress written under a claim is the claimer's record, and message
text confers nothing.

### Privacy posture — private by default

Everything the broker holds about a task — checkpoints, park
reasons, heartbeat state, progress JSON — is private by default.
Nothing mirrors to GitHub automatically: an issue comment or a PR
reply is a separate, explicit act by the operating agent, and the
designed private-note mirror requires explicit scoped authorization
(not shipped). Secrets are by NAME only, resolved through the broker
([`crates/fleet-secrets`](crates/fleet-secrets)): values never appear
in code, tests, errors or logs — the strongest redaction is never
holding the value at all.

Reviewed redaction examples — apply before anything moves from the
broker's private store to a public surface (amend this table by PR,
never in place):

| private record (queue-side, fine)             | redacted form before it goes public                            |
| --------------------------------------------- | -------------------------------------------------------------- |
| `"progress": {"token": "ghp_abc123…"}`        | `"token": "<redacted — secret resolved by name, never value>"` |
| `--reason "blocked on https://host/x?auth=…"` | `--reason "blocked on internal service (URL redacted)"`        |
| a packet body naming a credential value       | the secret's NAME (e.g. `pm-dispatch.api-token`)               |

### Honesty contracts

- **No exactly-once guarantee.** The surface provides
  no exactly-once guarantee for completion or for the creation
  outbox: both are at-least-once with receipts, as designed in #9209 —
  every mutation records a stable operation identity, a result that
  comes back unknown after a send timeout requires a remote readback
  before any retry, and completion means a verified terminal state —
  never a promise that every effect happened exactly once. Operators
  and agents must not claim an exactly-once property anywhere on
  this surface.
- **Concurrency is leases and fencing.** The mechanism is the claim
  lease with generation/fence checks (#9209); no other concurrency
  mechanism is offered or implied here.
- **The Queue Table Improvements gate.** Persisted schema-coupled
  queue changes are gated on the Queue Table Improvements (QTI)
  crosswalk. QTI itself remains unlocated/unverified — #9241 is the
  provenance-recovery research issue — so this contract references
  it only as a gate and does not describe its content.

### Contributor contract

Keep the command contract together: update
[`crates/fleet-registry/src/lib.rs`](crates/fleet-registry/src/lib.rs)
when a top-level help inventory changes, and update
[`docs/verbs.md`](docs/verbs.md) for every changed or added command,
including nested leaves and examples, in the same change. Docs must
not present a designed spelling as shipped: the runtime dispatch is
the authority on what a binary can run. And a change
to any settled semantic this section records (claim/park/resume,
supervision and stop authority, cap policy, attribution/logging)
requires a decision record in the same body of work — see
[the fleet decision records index](../../doc/decisions/README.md).

## Operator reference

- [`docs/fleet-gh-credentials.md`](docs/fleet-gh-credentials.md) —
  the `fleet gh` credential lifecycle: secret names per leg,
  in-process minting, rotation order and overrides, retry-safety,
  unknown-org behavior, exit codes and raw mode (code#10744).
- [`doc/decisions/`](../../doc/decisions/README.md) — the **fleet
  decision records** (FDR) index: every durable ruling on queue
  claim/park/resume semantics, supervision and stop authority, cap
  policy, and attribution/logging, with its source ruling and
  implementation links. **A change to any of those semantics requires
  a decision record in the same body of work** — the same PR, or the
  same run for non-PR work. See
  [the rule](../../doc/decisions/README.md#5-same-body-of-work).

## Fleet program

The fleet program — full investigation of the fleet system, design completion,
and new designs toward a fully featured platform — is chartered by the
[Fleet program investigation & build-out prompt](../../doc/2026-11-23T12-45-06Z_design_fleet_program_investigation/2026-11-23T12-45-06Z_design_fleet_program_investigation.md),
tracked in [#13272](https://github.com/developing-today/code/issues/13272).
Its deliverables include a fleet charter, the fleet book extended in place
under `/code/fleet/`, and a prioritized map of all fleet issues with blockers
marked.

## Build and smoke

```sh
nix build .#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static
./pkgs/fleet/smoke.sh ./result ./result-1 ./result-2 ./result-3
# (the smoke suite also runs in CI: .github/workflows/fleet-smoke.yml, #6400)
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
`bash .github/scripts/check-fleet-lock.sh` names that drift in
seconds without building anything (fleet-smoke runs it on atlas-search
manifest changes too, #10363). Regenerate the lock before building:

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
