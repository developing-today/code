# fleet verb reference

Contract for each verb of the `fleet` multicall binary. The code is the
source of truth: the dispatch in `crates/fleet/src/main.rs` and the domain
handlers. The `subcommands` arrays in `crates/fleet-registry/src/lib.rs` are
stale (the queue domain lists about 16 verbs, the code dispatches 31); do not
generate docs from them until they are fixed.

Invocation: `fleet <domain> <verb> [args]`, or the domain's alias as argv0
(`repo-queue`, `queue-broker`, `secrets-broker`, `laptop-agent-broker`,
`atlas-search`, `laptop-agent`).

## Root

| Verb | Contract |
|------|----------|
| `init` | Autostart this host's fleet roles (named by the `[host]` manifest in fleet config) if they are not running. |
| `version`, `--version`, `-V` | Print the version. |
| `capabilities` | List available domains and their state. |
| `help [domain]` | Print help for the binary or one domain. |

Exit codes: `0` success; `2` usage error; `69` (`EX_UNAVAILABLE`) when a
domain is unavailable (capability stub).

## queue

Client for the repo-queue broker (`crates/fleet-queue/src/client.rs`).

| Verb | Contract |
|------|----------|
| `validate` | Validate a packet without writing it. |
| `enqueue` | Add a packet to the queue. |
| `list` | List items, with filters. |
| `claim` | Claim the highest-priority eligible item for a worker and lane. |
| `release` | Release a claim (requires `--reason`). |
| `lane` | Inspect or set lane state. |
| `bind` | Bind an item to a task reference. |
| `heartbeat` | Extend a claim's lease. |
| `checkpoint` | Record progress and the next action on a claimed item. |
| `park` | Park an item until its delay or a wake. |
| `wake` | Return a parked item to eligible. |
| `block`, `unblock` | Mark an item blocked with a reason class, or clear it. |
| `dep-recheck` | Re-evaluate dependency edges. |
| `quarantine-recheck` | Re-evaluate quarantined items. |
| `cancel` | Cancel an item. |
| `revive` | Reopen a failed or cancelled item. |
| `fail` | Mark a claimed item failed, with a reason class. |
| `complete` | Mark a claimed item done with a structured result. |
| `verify-disposition` | Verify a recorded disposition. |
| `timeline`, `history` | Event timeline for one item (same handler). |
| `models` | Model catalog and pool information. |
| `health` | Broker health and per-queue counts. |
| `signal` | Send a signal to a worker or item. |
| `fallback` | `scan`, `status`, `claim`, `heartbeat`, `release` (fallback mode). |
| `dispatch` | `status`, `launch`, `enqueue` (`placement.rs`). |
| `session` | Placement session commands. |
| `surfaces` | Print the placement surface registry (`pkgs/fleet/fleet-surfaces.json`) as JSON. Read-only; takes `[--registry PATH]`. |
| `outbox` | Creation outbox group. |
| `stack` | Stack group. |

## Other domains

| Domain and verb | Contract |
|-----------------|----------|
| `serve` | Serve role (`fleet-serve`). |
| `proxy serve`, `connect`, `init`, `ssh`, `splice`, `get`, `put` | Proxy leaves. `connect` takes connection flags; `put` takes mode and no-resume flags. |
| `dispatch status`, `launch`, `enqueue` | Dispatch status, launch a run, enqueue work (`crates/fleet-dispatch`). |
| `dispatch-test scan`, `submit`, `status` | Test-dispatch scan, submit and status (`crates/fleet-dispatch-test`). |
| `secrets get`, `list` | Secret lookup and listing. |
| `gh <args>`, `tiers`, `cache-serve`, `cache-status`, `--raw` | Pass-through to `gh`; tier information; cache serve and status. |
| `search search`, `get-chunk` | Search the index and fetch a chunk. The `serve` alias is also accepted. |
| `laptop-agent health`, `jobs`, `kill`, `exec`, `run`, `poll`, `fetch` | Laptop-agent broker operations. |

Note: `scripts/fleet` is a directory of helper scripts, not the multicall
binary.
