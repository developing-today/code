# Architecture

Fleet is a self-contained Cargo workspace at `pkgs/fleet/` in
developing-today/code: its own `Cargo.toml`, `Cargo.lock`, and
`rust-toolchain.toml` (channel 1.99.1), with no workspace relationship to the
repo root. The workspace package is version 0.1.0, edition 2024, Rust 1.85
minimum, licensed MIT OR Apache-2.0. The release profile is a size contract,
not a speed profile (`opt-level = "z"`, LTO on, one codegen unit, stripped):
the connect variant's budget is a target of at most 1.5 MiB and a hard limit
of 2 MiB, stripped.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/Cargo.toml

## The crates map: 16 crates

| Crate                 | Owns                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet` (the bin)     | The multicall entrypoint (`src/main.rs`), root built-ins, dispatch and the stub machinery, plus the domains that live in the bin crate itself: `secrets` verbs (over `fleet-secrets`), and `gh` with its cache, closure, guard, and metadata modules.                                                                                                                                                                                                                                                                               |
| `fleet-registry`      | The domain registry: the 22 capability atoms, the three variants and their capability sets, the 19 domain entries (name, summary, capabilities, primary capability, aliases, subcommand inventory), and the argv0 alias map. Dependency-free; compiled into every variant so help is always complete.                                                                                                                                                                                                                               |
| `fleet-config`        | The one place config resolves: `fleet.toml` (default `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`, overridable with `FLEET_CONFIG`), the `[domains]` switches, `[proxy]`, `[secrets]`, `[queue-broker]`, `[secrets-broker]`, `[dispatch]`, `[host]` sections, and the fleet's path/port/socket constants. Resolution order is environment → config file → compiled-in defaults. A missing file is not an error; a malformed file — including unknown sections or keys — is a loud error on every path, never a silent fallback. |
| `fleet-secrets`       | The secret resolver library: the broker-first, Bitwarden-fallback chain, by name only. Every domain that needs a secret resolves it through this crate; no domain shells out to `bws` or reads a token file on its own.                                                                                                                                                                                                                                                                                                             |
| `fleet-proxy-proto`   | The session-broker wire protocol: 8-byte big-endian length-prefixed JSON frames, the hello/capability handshake, and the protocol version window (current and previous major, plus legacy proto 0).                                                                                                                                                                                                                                                                                                                                 |
| `fleet-proxy`         | The session proxy: the broker (`serve`), the client (`connect`, `init`, `ssh`, `get`/`put`), raw splice, tunnels, and shims. The client half is in every variant; only the broker half is feature-gated.                                                                                                                                                                                                                                                                                                                            |
| `fleet-queue`         | The queue domain end to end: the client (32 verbs), the queue broker, the secrets broker, the Turso queue store, packet validation, the GitHub fallback queue, placement, the creation outbox, and merge-intent stacks. Its features split the surfaces: `client`, `queue-broker`, `secrets-broker`.                                                                                                                                                                                                                                |
| `fleet-dispatch`      | The `dispatch` domain: a single-file thin client over the pm-dispatch HTTP service.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `fleet-dispatch-test` | The `dispatch-test` domain: scan/submit/status over GitHub and its Turso jobs store.                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `fleet-laptop-agent`  | The `laptop-agent` client: verbs, transport legs, and the connection ladder for the laptop-agent daemon.                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `fleet-search`        | The `search` domain: the Atlas client surface, and — behind its `serve` feature — the Atlas search service itself, consuming the `atlas-core`/`atlas-embed`/`atlas-proto` crates from `pkgs/atlas-search` by relative path.                                                                                                                                                                                                                                                                                                         |
| `fleet-bridge`        | The `bridge` domain: the mailbox client for the message bridge (send/agents/log for central; hello/inbox/post/ack/health for agents).                                                                                                                                                                                                                                                                                                                                                                                               |
| `fleet-agent`         | The `agent` and `shell` domains: kinds, enrolment, doctor, the intake ladder (`next`), the shortlist, and the shell target planner. It links only `fleet-config`: it runs the other domains' verbs through this same binary rather than linking them.                                                                                                                                                                                                                                                                               |

| `fleet-kv`          | Identity-scoped local SQLite storage: the local store other fleet surfaces build on. |
| `fleet-cert`        | The `cert` domain: PKI/X.509 roots, issuers, leaves, chains and bundles over a library-first Rust API, plus the OCSP verb group. |
| `fleet-orgs`        | The `orgs` domain: the Turso-backed org/App registry, synced from each GitHub App's own JWT. |

Domain ownership, one line each: `queue` → `fleet-queue`; `serve` → bin-crate
glue over `fleet-queue` / `fleet-search` / `fleet-proxy`; `proxy` →
`fleet-proxy` (+ `fleet-proxy-proto`); `dispatch` → `fleet-dispatch`;
`dispatch-test` → `fleet-dispatch-test`; `secrets` → the bin crate over
`fleet-secrets`; `gh` → the bin crate; `bridge` → `fleet-bridge`; `agent` and
`shell` → `fleet-agent`; `search` → `fleet-search`; `orgs` → `fleet-orgs`; `cert` → `fleet-cert`; `laptop-agent` →
`fleet-laptop-agent`.

## The brokers and their ports and sockets

A _broker_ in fleet is a resident process that other processes ask for a
service, as distinct from a library they link. There are three fleet brokers,
plus the laptop-agent daemon and two auxiliary services:

| Service                | Port            | Unix socket (default)                  | Notes                                                                                                                                                    |
| ---------------------- | --------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| laptop-agent daemon    | 4799            | —                                      | The Python daemon on the laptop that runs jobs. Not a fleet binary, but the proxy's routing table names it.                                              |
| secrets-broker         | 4800            | `~/.config/secrets-broker/broker.sock` | The Bitwarden cache. Dual-binds TCP and the Unix socket with identical routes and auth. On the sandbox its TCP bind is forced to `127.0.0.1:4800`.       |
| queue-broker           | 4801            | `~/.config/queue-broker/broker.sock`   | The work queue service. Dual-binds TCP (`100.99.113.103:4801`, the laptop tailnet address) and the Unix socket.                                          |
| gh cache daemon        | 4802            | —                                      | Loopback token/secret cache for the `gh` passthrough. See [The gh passthrough](gh.md).                                                                   |
| pm-dispatch            | 4879            | —                                      | The TypeScript/Bun dispatch service the `dispatch` domain is a client of.                                                                                |
| session broker (proxy) | — (socket only) | `~/.cache/laptop-agent/broker.sock`    | The sandbox-side broker every local client talks to; it tunnels framed requests to the laptop's services. Socket overridable with `FLEET_BROKER_SOCKET`. |
| RustDesk direct access | 21118           | —                                      | Also the loopback listen port of `fleet proxy splice`.                                                                                                   |
| tailnet egress proxy   | 3130            | —                                      | The host session's CONNECT proxy. Not a fleet listener; fleet is its client.                                                                             |

The session broker's routing table maps port → service name — 4799
laptop-agent, 4800 secrets-broker, 4801 queue-broker, 21118 rustdesk — and
that map is what its hello frame advertises to clients.

Daemon-side token files live in each broker's own config directory
(`~/.config/queue-broker/token`, `~/.config/secrets-broker/token`). They are
different files from the client-side token files under `~/tools/`; see
[Secrets and the secrets broker](secrets.md).

The brokers are run by `fleet serve`: one role in the foreground, or a list
of roles in one process, one thread each, fail-fast — the first role thread
to return or panic takes the whole process down and the external supervisor
(systemd) restarts the unit. A role that exits cleanly is converted to exit
1, because a daemon role returning at all is abnormal. See
[Proxy, sessions and recovery](proxy.md) for the proxy's own serve
behaviour.

## The Turso store

The queue's only backend is a dedicated Turso (libSQL) database named
`fleet-queue`. The queue broker is the only process that talks to it; clients
never touch the database. Because libSQL is single-writer, a claim is one
conditional UPDATE whose WHERE clause re-checks claimability against the
picked row — the equivalent of `FOR UPDATE SKIP LOCKED`; a competing claim
that loses matches no row and is answered as "no claim". The schema (tasks,
leases, lanes, workers, checkpoints, the lane ledger, events, the creation
outbox, merge intents) is detailed in
[The queue: tasks, lanes and leases](queue.md#the-turso-store).

`dispatch-test` keeps a second, separate Turso database for its job rows;
see [Dispatch and dispatch-test](dispatch.md).

## The session proxy

Sandbox seats cannot dial the laptop's services directly. The session proxy
is the bridge: a broker process on the sandbox side, owning the session
broker Unix socket, which tunnels framed requests (8-byte big-endian
length-prefixed JSON) to the laptop. Token files are read per request and
never stored or logged; the broker persists nothing — its routing table comes
from config and its tunnels live in memory. The full mechanics — warm tunnel
pools, the Cloudflare leg versus the tailnet fallback, recovery — are in
[Proxy, sessions and recovery](proxy.md).

## Data flows

### The claim path (an agent takes queue work)

1. The agent runs `fleet agent next`; the queue rung runs `fleet queue claim`
   through this same binary, with a 4-hour lease.
2. The queue client chooses a route to the queue broker: a configured direct
   URL (TCP or the broker's Unix socket) on the laptop, or — the sandbox
   default — a framed exchange through the session broker socket to port
   4801, naming the client token file `~/tools/queue-broker-token`. The
   session broker reads that file on the laptop side and forwards the token
   as the `X-Agent-Token` header.
3. The broker authenticates from headers alone (a wrong or missing token is
   401, an oversized body 413, before any body byte is read), then runs the
   claim against the Turso store: expire lapsed leases, pick the
   highest-priority due row, stamp worker/lane/session/model, set the lease,
   write the `claimed` event — in one conditional update.
4. The client checks the claimed task's issue against the GitHub fallback
   queue; if the fallback currently holds that issue under a different
   worker, the client blocks the task back with reason class `fallback_held`
   and reports no claim (exit 5). If the GitHub check itself fails, the claim
   is served with a warning — the guard fails open.
5. The client prints the task row as JSON, plus a `mode_line` describing the
   route the claim took. No matching row is `{"task": null}`, exit 5.

### The secret path (a caller resolves a name)

1. A caller — a domain inside fleet, or `fleet secrets get <name>` — asks
   `fleet-secrets` to resolve a name. Nothing else in the system resolves
   secrets.
2. The chain tries the broker leg first: directly over the secrets broker's
   Unix socket where that socket answers (the laptop), else by a framed
   exchange through the session broker to port 4800, naming the token file
   `~/tools/secrets-broker-token`. The session broker reads the file per
   request; the value never transits fleet's own memory on this leg.
3. A broker answer of 404 is definitive about the name and is not retried. A
   broker that cannot answer is retried with a bounded backoff budget sized
   to real restart windows, then the chain moves on.
4. The fallback leg runs `bws secret list` + `bws secret get`, with the
   machine-account credential from the `BWS_ACCESS_TOKEN` environment
   variable or the on-disk token file.
5. The value is returned in a type whose debug output is redacted. For
   `fleet secrets get`, it goes to stdout and nowhere else. Broker-side cache
   semantics — ages, read-through, refresh — are in
   [Secrets and the secrets broker](secrets.md).

### The dispatch path (a build runs remotely)

1. A caller runs `fleet queue dispatch --repo <owner/name> --command '<cmd>'
--dispatch` (or `fleet shell --target actions`, which drives the same
   verb).
2. Placement loads the surface registry — data, not code: `--registry` or
   `FLEET_PLACE_REGISTRY`, else a home copy, else the source-tree path, else
   the copy embedded in the binary — and measures live load: for each owner,
   the in-progress plus queued run counts on that owner's fleet runner-host
   repos, read from the GitHub API.
3. The tier cascade picks a surface: a hard `--surface` pin first, then
   follow-up affinity, then free GitHub-hosted Actions (least-loaded owner,
   least-recently-used rotation), then Blacksmith within its minute budget,
   then codespaces as the last resort. Every step appends a reasoning line.
4. The adapter mints the token the surface dispatches as (a GitHub App
   installation token, or the signed-in user's token), snapshots the host
   repo's existing workflow runs, and POSTs the `fleet-command.yml` workflow
   dispatch. Anything but HTTP 204 is fatal.
5. It polls the run list (up to 12 polls, 3 s apart) for the new run —
   matched by a fresh dispatch id in the run title when the surface declares
   one — and writes a session record under a handle `fp_` plus 32 hex
   characters in `~/.local/state/fleet-place/sessions.json`.
6. The caller follows the run with `fleet queue session status|result
<handle>`; the portable form `OWNER/REPO#RUN_ID` resolves the same run
   without the local store. Full detail: [Placement and remote dispatch](placement.md).
