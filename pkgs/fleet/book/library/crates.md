# The crates

The fleet binary is one Rust workspace, `pkgs/fleet/`, with 16 crates:
15 library crates and the one binary crate that assembles them. The
workspace is self-contained — its own `Cargo.toml`, `Cargo.lock` and
`rust-toolchain.toml`, no workspace relationship to the repository root
— at version `0.1.0`, edition 2024, rust-version 1.85, licensed
`MIT OR Apache-2.0`. Shared externals are pinned once at the workspace
level (`serde 1` with derive, `serde_json 1`, `sha2 0.11`, `toml 1.2`);
the fleet crates depend on each other by path only. The release profile
is a size contract, not a speed profile: `opt-level = "z"`, `lto =
true`, `codegen-units = 1`, `strip = true`, sized for the connect
variant's budget (target ≤ 1.5 MiB, hard ≤ 2 MiB, stripped).

This chapter names every crate by its exact package name, gives its
purpose in the crate's own words (the `description` line of its
`Cargo.toml`, quoted verbatim), and maps which crate owns which domain.
The chapters after this one go deep on the three library surfaces an
embedder touches most: [The queue client](queue-client.md),
[Placement and surfaces as data](placement.md), and
[Registry, config and capabilities](registry-and-config.md).

## The 16 crates

### `fleet`

> "One multicall binary for the fleet's tools: search, queue, dispatch, proxy — with compile-time capability-selected variants (epic developing-today/code#5412)."

The bin crate: `[[bin]] name = "fleet"`, and **no `lib.rs`**. Its
`src/main.rs` is the multicall entrypoint — argv0 routing, the
capability surface, the stub machinery, root built-ins — and it also
carries two domains as bin-crate modules rather than library crates:
`secrets` (the `fleet secrets` verbs, over `fleet-secrets`) and `gh`
(the `fleet gh` passthrough, with its `gh_cache`, `gh_closure`,
`gh_guard` and `gh_metadata` modules). Unconditional modules include
`invocation_ledger`, `session_state` and `derived_version` (the last
also included by path from `build.rs`, which bakes the source revision
and the derived version into the binary).

### `fleet-agent`

> "The fleet agent domain: one front door for any agent. Enrol a named agent of any kind, check what this seat can reach, ask for the next piece of work down the intake ladder (pushed command, queue claim, local shortlist, highest priority), and open a shell where builds belong."

Owns the `agent` domain (`kinds`, `enroll`, `doctor`, `next`,
`shortlist`) and the `shell` domain (`run_shell` entry): one crate, two
registry domains, both riding the single `agent` capability. It links
only `fleet-config` among the fleet crates: it runs the other domains'
verbs through the same binary rather than linking their crates.

### `fleet-bridge`

> "The fleet bridge domain: send and receive through the message bridge's fleet mailboxes — central commands one agent, a group or all; agents read, answer and ack."

Owns the `bridge` domain. Essentially a single `src/lib.rs` plus its
tests: the mailbox client for the bridge service, with the two token
classes (owner token for central verbs, fleet token for agent verbs)
resolved by name through `fleet-secrets`.

### `fleet-cert`

> "The `fleet cert` domain: PKI/X.509 roots, issuers, leaves, chains and bundles over a library-first Rust API (developing-today/code#9157, spec doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets)."

Owns the `cert` domain (`fleet cert`, alias `fleet pki`), entry point
`fleet_cert::run`, with the domain's own usage text in `fleet_cert::cli`.
The verb surface covers roots, issuers, leaves, chains, bundles and
profiles; the OCSP verb group (sign, inspect, test) and the
`ocsp-responder` profile landed in #13722. The broker TLS profiles
`broker-server` and `broker-client` (CSR-only, Base issuer set,
`.internal` DNS + tailnet IP SANs, 90 days) and CSR issuance
(`leaf issue --csr`, `--ip`) landed in #14399.

### `fleet-config`

> "fleet configuration: ~/.config/fleet/fleet.toml + env resolution, the [domains] enable/disable table, and state-dir paths."

A platform crate, not a domain: the one place configuration resolves.
Environment overrides, then the config file
(`${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml` unless `$FLEET_CONFIG`
names another), then compiled-in defaults. A missing file is not an
error; a malformed file — including unknown sections or keys — is a loud
error on every path, never a silent fallback. It also owns the fleet's
path, port and socket constants. Covered in depth in
[Registry, config and capabilities](registry-and-config.md).

### `fleet-dispatch`

> "The fleet dispatch domain: a thin client over pm-stack's pm-dispatch service — status, launch, and enqueue verbs (epic developing-today/code#5412, step 4 #5432)."

Owns the `dispatch` domain: a single-file client over pm-dispatch's
existing HTTP routes — exactly three (`GET /health`,
`POST /v1/sessions`, `POST /v1/tasks`). pm-stack itself is a working
TypeScript/Bun service suite that is deliberately not rewritten; per-task
status is not here, because that state is the repo queue itself.

### `fleet-dispatch-test`

> "The fleet dispatch-test domain: explicit-dispatch CI test — scan fleet repos for free GitHub Actions capacity, submit a job to the Turso-backed jobs store, dispatch the pilot workflow, and track the job's stored status."

Owns the `dispatch-test` domain (`scan`, `submit`, `status`). Nothing in
it is trigger-driven: every verb runs only when a caller invokes it, and
`submit` dispatches a `workflow_dispatch` run explicitly.

### `fleet-kv`

> "Identity-scoped local SQLite storage for fleet-kv."

A platform crate, not a domain: the identity-scoped SQLite storage core
(#13362) that other fleet surfaces build on.

### `fleet-laptop-agent`

> "The fleet laptop-agent domain: the transport ladder (broker, Cloudflare, Tailscale, SSH) and the verbs over the laptop-agent daemon (epic developing-today/code#5412, issue #10796)."

Owns the `laptop-agent` domain (`health`, `jobs`, `kill`, `exec`,
`run`, `poll`, `fetch`): the client for the laptop-agent daemon on port
4799, with a four-leg transport ladder — broker, Cloudflare, Tailscale,
and an SSH leg that exists in the ordering as a stub and never carries
traffic in this build.

### `fleet-orgs`

> "The `fleet orgs` domain: the Turso-backed org/App registry, synced from each GitHub App's own JWT (developing-today/code#8884)."

Owns the `orgs` domain, entry point `fleet_orgs::run`. The registry
tables are `sync_runs`, `apps`, `orgs` and `installations`, with
idempotent upserts; sync uses each App's JWT against `GET /app` and
`GET /app/installations`. `fleet orgs list` prints the table or
`--json`, with `--app`, `--org` and `--permission` filters, and shows
the last successful sync time. The Turso URL comes from
`FLEET_ORGS_TURSO_URL`; its token is the secret `turso.fleet-orgs-token`.

### `fleet-proxy`

> "The fleet session proxy domain: `proxy serve` (the broker) and `proxy connect`/`init` (the client) — design note §4, epic developing-today/code#5412."

Owns the `proxy` domain: the session broker (`proxy serve`), the client
verbs (`connect`, `init`, `ssh`, `splice`, `get`, `put`), warm tunnel
pools to the laptop's services, and file transfer over the agent
channel. The crate is a non-optional dependency of the bin crate with
its own features split: the client is in every variant; only the broker
half (`serve`) is feature-gated. It also carries two extra binaries
(`fleet-connect-udp`, `fleet-udp-forward`).

### `fleet-proxy-proto`

> "The fleet proxy wire contract: length-prefixed JSON framing plus the hello/capability frame types. The contract lives here and nowhere else."

A platform crate: the session-broker wire protocol and nothing else —
8-byte big-endian length-prefixed JSON frames, the hello/capability
handshake, and the major-version window (N / N−1, plus the legacy proto
0 dialect). (The design note's "4-byte" framing was a census error; the
live wire is 8-byte, and the crate README records the erratum.)

### `fleet-queue`

> "The fleet queue domain: the repo-queue client surface plus the queue-broker and secrets-broker server roles (epic developing-today/code#5412, step 2 #5430)."

By far the largest crate, and the owner of three registry surfaces at
once: the `queue` domain (the client, including the GitHub fallback
queue and the placement module), and — under the `serve` domain — the
`queue-broker` and `secrets-broker` roles. It also builds three
standalone binaries from `src/bin/` (`repo-queue`, `queue-broker`,
`secrets-broker`), the migration's rollback inventory: they retire only
at the epic's close, not before. Its internal split — client surface,
broker surface, and the feature gates between them — is the subject of
[The queue client](queue-client.md) and
[Placement and surfaces as data](placement.md).

### `fleet-registry`

> "The full fleet domain registry: names, one-line descriptions, argv0 aliases, capabilities and variant profiles. Std-only; compiled into every fleet variant."

A platform crate with a hard constraint: **dependency-free** (std only,
strings and tables, no I/O), so it can be compiled into _every_ variant
— including the size-gated connect build — and `--help` is always
complete. It owns the 22 capability atoms, the 19 domains, the three
build variants and the argv0 alias map. Covered in depth in
[Registry, config and capabilities](registry-and-config.md).

### `fleet-search`

> "The fleet search domain: the atlas-search client surface over the live Atlas service (job protocol), with the Q3 host defaults — via-service with loud standalone fallback on the sandbox seat, standalone on the laptop seat (epic developing-today/code#5412, step 5 #5433) — and the `serve search` role, the Atlas service itself rehomed from the atlas-search bin crate (#5604)."

Owns the `search` domain (the Atlas client) and the `search` serve role
(the Atlas service itself, behind the crate's `serve` feature). It is
the one workspace member that consumes crates **outside** the workspace:
`atlas-core` / `atlas-embed` / `atlas-proto` by relative path from
`pkgs/atlas-search`. The serve role's engine is a verbatim copy of
atlas-search's service, held byte-for-byte by a parity-guard workflow.

### `fleet-secrets`

> "The one secrets-by-name path for fleet: secrets-broker protocol first, bws CLI fallback. Values are never stored, never logged, and redacted in Debug."

A platform crate, not a domain: the resolver every domain uses to turn a
secret _name_ into a value. The resolution chain is broker first (direct
socket where the broker is local, framed through the session broker
elsewhere), then the `bws` CLI fallback, then a 1Password source where a
vault is named. Resolved values live in a type whose `Debug` is
redacted; chain errors count failed sources without ever carrying a
value.

## Dependency shape

Three rules describe the whole graph:

1. **`fleet-registry` depends on nothing** — not even on the workspace's
   shared externals. That is what lets every variant, however small,
   know every domain name, summary, alias and required capability,
   whether or not it can run it.
2. **Platform crates sit at the bottom**: `fleet-registry`,
   `fleet-config`, `fleet-secrets` and `fleet-proxy-proto` own the
   contracts (registry, configuration, secret resolution, wire framing)
   that domain crates consume. Domain crates do not shell out around
   them — no domain resolves its own config file, fetches its own
   Bitwarden secret, or invents its own framing.
3. **The bin crate assembles.** `crates/fleet` wires the domain crates
   behind Cargo features, reconstructs its compiled capability set from
   those features at runtime (see
   [Registry, config and capabilities](registry-and-config.md)), and
   routes argv0 → domain → capability check → handler or stub. Domain
   crates generally do not depend on each other; where one domain needs
   another's verbs (`fleet-agent` running queue or bridge verbs), it
   goes through the assembled binary, not a crate edge.

Feature wiring mirrors the capability set one level down. On the bin
crate, for example, `queue = ["dep:fleet-secrets", "dep:fleet-queue",
"fleet-queue/client"]` and `queue-broker = ["dep:fleet-secrets",
"dep:fleet-queue", "fleet-queue/queue-broker"]`: enabling a capability
enables exactly the matching feature surface of the owning crate.
`proxy-connect` is not a feature at all — it is compiled into every
variant unconditionally.

## Which crate owns which domain

| Registry domain                                  | Primary capability | Owning crate                                    | Entry point                                                          |
| ------------------------------------------------ | ------------------ | ----------------------------------------------- | -------------------------------------------------------------------- |
| `queue`                                          | `queue`            | `fleet-queue`                                   | `fleet_queue::client::run`                                           |
| `serve` (roles `queue-broker`, `secrets-broker`) | `queue-broker`     | `fleet-queue` (bin-crate glue routes the roles) | `fleet_queue::queue_broker::run`, `fleet_queue::secrets_broker::run` |
| `serve` (role `search`)                          | `search-serve`     | `fleet-search`                                  | `fleet_search::serve::run`                                           |
| `serve` (role `proxy`)                           | `proxy-serve`      | `fleet-proxy`                                   | the proxy serve loop, via bin-crate glue                             |
| `proxy`                                          | `proxy-connect`    | `fleet-proxy` (+ `fleet-proxy-proto`)           | bin-crate glue over `fleet-proxy` client/serve                       |
| `dispatch`                                       | `dispatch`         | `fleet-dispatch`                                | `fleet_dispatch::run`                                                |
| `dispatch-test`                                  | `dispatch-test`    | `fleet-dispatch-test`                           | `fleet_dispatch_test::run`                                           |
| `secrets`                                        | `secrets`          | bin crate, over `fleet-secrets`                 | `crates/fleet/src/secrets.rs`                                        |
| `gh`                                             | `gh`               | bin crate                                       | `crates/fleet/src/gh.rs` and siblings                                |
| `bridge`                                         | `bridge`           | `fleet-bridge`                                  | `fleet_bridge::run`                                                  |
| `agent`                                          | `agent`            | `fleet-agent`                                   | `fleet_agent::run`                                                   |
| `shell`                                          | `agent`            | `fleet-agent`                                   | `fleet_agent::run_shell`                                             |
| `search`                                         | `search`           | `fleet-search`                                  | `fleet_search::run`                                                  |
| `laptop-agent`                                   | `laptop-agent`     | `fleet-laptop-agent`                            | `fleet_laptop_agent::cli::run`                                       |
| `orgs`                                           | `orgs`             | `fleet-orgs`                                    | `fleet_orgs::run`                                                    |
| `cert`                                           | `cert`             | `fleet-cert`                                    | `fleet_cert::run`                                                    |

Note the two irregular rows, because embedders trip on both: `shell`
has **no capability atom of its own** — it rides the `agent` capability
— and `secrets` and `gh` are domains without library crates, living as
modules in the bin crate (over `fleet-secrets`, in the `secrets` case).

## Example: depending on the platform layer

The smallest useful embedding is the dependency-free registry: any
crate, tool or test can ask what the fleet's surface _is_ without
linking a single domain. This example is illustrative — it follows the
researched API (`fleet_registry::DOMAINS`, `fleet_registry::domain`)
but the exact return type of the lookup helper was not verified against
the source signature in this pass.

```toml
# Cargo.toml
[dependencies]
fleet-registry = { path = "../fleet-registry" }
```

```rust
// Print the registered summary for the queue domain, if it exists.
fn main() {
    for d in fleet_registry::DOMAINS {
        println!("{} — {}", d.name, d.summary);
    }
    // fleet_registry::domain(name: &str) -> Option<&'static Domain>
    if let Some(d) = fleet_registry::domain("queue") {
        println!("queue aliases: {:?}", d.aliases);
    }
}
```
