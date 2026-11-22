# Fleet CLI command reference

This reference covers the commands dispatched by the `fleet` multicall binary. It follows `fleet-registry` for top-level domains and the domain handlers for real verbs and nested groups. Update this file and the registry help inventory in the same change whenever the shipped command surface changes.

## Find the exact usage

```sh
fleet --help
fleet capabilities
fleet help queue
fleet queue --help
fleet queue dispatch --help
fleet queue dispatch --dispatch --help
```

Use `fleet <domain> --help` to see a domain inventory and a supported leaf's `--help` for its exact flags. The binary variants share the full help registry, but a command may be unavailable in a particular build or disabled by host config; check `fleet capabilities` before assuming it can run. Capability-unavailable commands exit 69.

Invocation is `fleet <domain> <verb> [args...]`. The executable aliases `repo-queue`, `queue-broker`, `secrets-broker`, `laptop-agent-broker`, `atlas-search`, and `laptop-agent` route to their Fleet domain surfaces.

## Built-in commands

| Command | Purpose |
| --- | --- |
| `fleet init` | Start this host's configured Fleet roles when they are not already running. |
| `fleet version` | Print the build, derived version, source revision, and enabled capabilities. `--version` and `-V` are supported. |
| `fleet capabilities` | Print the full registry and this binary's effective capability set as JSON. |
| `fleet help [DOMAIN]` | Print all-domain help or the registry entry for one domain. |

## `queue` — work queue and remote execution

Client for the repo-queue broker. It supports direct broker transport and the configured proxy path; broker unavailability is an execution error, not a successful empty result.

| Verb | Purpose |
| --- | --- |
| `validate` | Validate a queue packet without enqueueing it. |
| `enqueue` | Submit a packet to the queue. |
| `list` | List queue items using the requested filters. |
| `show` | Read the full record for one item. |
| `claim` | Atomically claim the highest-priority eligible item for a worker and lane. |
| `release` | Release a held claim; requires a reason. |
| `lane` | Inspect lane state or change a lane's state. |
| `bind` | Bind an item to its task/session reference. |
| `heartbeat` | Extend the current claim lease. |
| `checkpoint` | Save progress and the next action for a claimed item. |
| `park` | Park an item until a delay or wake. Waiting on a person requires a hands item and a concrete action. |
| `wake` | Return a parked item to eligibility. |
| `block` | Block an item with a reason class. Human waits follow the same hands-item rule as `park`. |
| `unblock` | Clear an item's block. |
| `dep-recheck` | Re-evaluate recorded dependency edges. |
| `quarantine-recheck` | Re-evaluate quarantined items. |
| `cancel` | Cancel an item. |
| `revive` | Reopen a failed or cancelled item. |
| `fail` | Mark a claimed item failed with a reason class. |
| `complete` | Complete a claimed item with a structured result. |
| `verify-disposition` | Verify a recorded disposition. |
| `timeline` | Print one item's event timeline. |
| `history` | Alias for the item event timeline. |
| `models` | Show model catalog and pool information. |
| `health` | Show broker health and queue counts. |
| `signal` | Send a signal to a worker or item. |
| `fallback` | Use the GitHub-issue fallback claim workflow; see the nested commands below. |
| `dispatch` | Plan or submit one remote command on an eligible Fleet runner. Dry-run by default; details and caching rules are in the [remote build and test runbook](../doc/agents/remote-build-and-test.md). |
| `session` | Inspect, list, or retrieve results for remote dispatch sessions. |
| `surfaces` | Print the placement surface registry as JSON; accepts `--registry PATH` for another registry file. |
| `outbox` | Inspect and drain the creation outbox. |
| `stack` | Record and operate on dependent pull-request stacks. |

### `queue fallback`

| Subcommand | Purpose |
| --- | --- |
| `scan` | Scan fallback repositories for live or stale claim markers. |
| `status` | Show the folded claim state for one repository issue. |
| `claim` | Claim a fallback issue using a worker marker. |
| `heartbeat` | Refresh the claim with a progress marker. |
| `release` | Release a claim; `done` keeps its terminal state and `free` returns it to the pool. |

### `queue dispatch`

`fleet queue dispatch --repo OWNER/REPO --command COMMAND` plans a remote run. The default is a dry run; add `--dispatch` to submit. Automatic placement prefers eligible GitHub Actions capacity and may fall back to Blacksmith within its configured budget. Linux is the default. Request macOS only when required. `--surface ID` pins a registered surface; `--followup HANDLE_OR_OWNER/REPO#RUN_ID` links a previous run for host affinity and artifact reuse. Container and Actions-cache flags are available; use the runbook for complete examples, outputs, supported runner behavior, and fallback details.

### `queue session`

| Subcommand | Purpose |
| --- | --- |
| `status HANDLE_OR_OWNER/REPO#RUN_ID` | Refresh and show run state and URL. |
| `result HANDLE_OR_OWNER/REPO#RUN_ID` | Show the run result and whether it is terminal. |
| `list` | List locally recorded sessions; `--mine` filters to the current submitter. |

Use the portable `OWNER/REPO#RUN_ID` form to inspect or follow up on a run from another workspace with GitHub access. Local `fp_...` handles depend on the local session store.

### `queue outbox`

| Subcommand | Purpose |
| --- | --- |
| `enqueue` | Add a creation request with its payload and deduplication key. |
| `list` | List requests, optionally filtered by repository or state. |
| `metrics` | Show outbox metrics. |
| `claim` | Claim the next outbox request for a drainer. |
| `resolve` | Record the outcome of a claimed request. |
| `drain` | Process pending creation requests. |

### `queue stack`

| Subcommand | Purpose |
| --- | --- |
| `record` | Record one branch's position and ownership in a stack. |
| `list` | Read a stack and its recorded intents. |
| `transition` | Change a stack intent's state. A conflicted state requires resolution detail. |
| `restack` | Record the new base after a branch is restacked. |
| `replay` | Execute the recorded stack walk, including Git operations and PR actions. Run with `--dry-run` to inspect the plan before allowing changes. |

## `serve` — daemon roles

`fleet serve ROLE` runs a daemon role in the foreground. Roles are `queue-broker`, `secrets-broker`, `search`, and `proxy`. Two or more distinct roles may be listed together in one invocation. Role availability depends on the binary's capabilities. The standalone aliases `queue-broker` and `secrets-broker` are also supported.

| Command | Purpose |
| --- | --- |
| `fleet serve queue-broker` | Run the repo-queue broker. |
| `fleet serve secrets-broker` | Run the Fleet secrets broker. |
| `fleet serve search` | Run the Atlas search service. |
| `fleet serve proxy` | Run the session proxy broker. `fleet proxy serve` is the same role. |

## `proxy` — session proxy

| Verb | Purpose |
| --- | --- |
| `serve` | Run the proxy broker in the foreground. |
| `connect` | Connect to the proxy, ensure a broker answers, synchronize PATH shims, and optionally use stdio or issue a request. |
| `init` | Start this host's configured Fleet roles, as with the root `fleet init`. |
| `ssh` | Open the configured recovery SSH path through the proxy. |
| `splice` | Relay a raw TCP stream through the proxy. |
| `get` | Copy a remote laptop path to a local destination. |
| `put` | Copy a local file or stdin to a remote laptop path. |

`connect`, `get`, and `put` have argument-specific help. File transfers support mode and resume controls; see the leaf usage before scripting them.

## `dispatch` and `dispatch-test`

| Command | Purpose |
| --- | --- |
| `fleet dispatch status` | Check pm-dispatch service health. |
| `fleet dispatch launch` | Launch an agent session with a repository and prompt. |
| `fleet dispatch enqueue` | Enqueue a titled task and prompt for a repository. |
| `fleet dispatch-test scan` | Find eligible Fleet repositories with available GitHub Actions capacity. |
| `fleet dispatch-test submit` | Submit the configured test job for a named submodule. |
| `fleet dispatch-test status JOB_ID` | Read a previously submitted dispatch-test job. |

`fleet dispatch` is the pm-stack agent-session API. `fleet queue dispatch` is the runner placement and command API; use the latter for builds and tests.

## `secrets`

| Verb | Purpose |
| --- | --- |
| `get NAME` | Resolve one secret by name. |
| `list` | List available secret names without exposing their values. |

The resolver uses the configured Fleet secrets broker and supported fallback stores. Request a secret by name; do not put its value into command arguments or build artifacts.

## `gh` — GitHub CLI wrapper

`fleet gh <gh args...>` passes normal arguments through to the installed `gh` CLI with Fleet's token rotation and cache. The supported Fleet-specific commands are:

| Command | Purpose |
| --- | --- |
| `tiers` | Show token-source rate-limit headroom. |
| `cache-serve [--bind ADDR]` | Run the local GitHub-token cache daemon. |
| `cache-status` | Probe configured token-cache daemons. |
| `closures [ITEM]` | Read locally recorded closure records, optionally filtered by item number. |
| `--raw <gh args...>` | Run the real `gh` with ambient credentials and no Fleet token injection. |

Leading `--cache-addr ADDR` options and `--no-cache` control the token cache for a call. Normal `gh` flags and subcommands pass through unchanged.

## `bridge` — Fleet messaging

| Verb | Purpose |
| --- | --- |
| `send` | Send a message or command to agents, groups, or all agents using the owner token. |
| `agents` | List registered agents and groups with unread counts. |
| `log` | Read bridge message history with filters. |
| `hello` | Register or refresh an agent and its groups. |
| `inbox` | Read unread messages or commands; can mark returned rows read or wait for arrivals. |
| `post` | Reply to central or send a message/instruction to an address. |
| `ack` | Mark one or more inbound messages read. |
| `health` | Check the bridge health endpoint. |

`fleet bridge --help` lists the central and agent forms and their accepted flags.

## `agent` — agent front door

| Verb | Purpose |
| --- | --- |
| `kinds` | List agent kinds that can be enrolled. |
| `enroll` | Register an agent and save its profile. |
| `doctor` | Report which work-intake legs are reachable from this seat. |
| `next` | Return the next available task from the configured intake ladder. |
| `shortlist` | Manage the local task shortlist; nested commands are below. |

`fleet agent next` can use pushed commands, the queue, the local shortlist, and GitHub priority sources. A no-work result is exit code 5; unavailable required legs use exit code 3.

### `agent shortlist`

| Subcommand | Purpose |
| --- | --- |
| `status` | Show the local shortlist. |
| `take` | Take the next shortlist item. |
| `clear` | Clear the local shortlist. |
| `add` | Add one issue with an optional rank and explanation. |
| `fill` | Populate the shortlist from the selected repository. |

## `shell` — laptop, Codespaces, or local

`fleet shell [OPTIONS] [-- COMMAND...]` plans or runs a command. `fleet shell targets` reports laptop and Codespace reachability and what automatic selection would choose.

| Option | Purpose |
| --- | --- |
| `--target auto|laptop|codespace|local` | Select the execution target; default is automatic fallback. |
| `--codespace NAME` | Select a named Codespace; otherwise the first available is used. |
| `--cwd DIR` | Set the working directory on the selected target. |
| `--build` | Take a laptop `fleet-build-lock` slot and allow a long build timeout. |
| `--timeout SECS` | Set a laptop command timeout. |
| `--detach` | Start a laptop command as a job and return its job ID. |
| `--print` | Print the selected plan as JSON without running it. |

Automatic selection honors `FLEET_SHELL_TARGET`, then prefers the current laptop seat, a reachable laptop, a Codespace, and finally the local environment. An explicit target does not fall back. `--detach` is for laptop jobs; use `fleet laptop-agent poll JOB --wait N` to follow one.

## `search` — Atlas

| Command | Purpose |
| --- | --- |
| `fleet search QUERY` or `fleet search search QUERY` | Search the Atlas corpus using its filters and output modes. |
| `fleet search get-chunk ...` | Fetch a chunk from the Atlas corpus. |
| `fleet search serve` | Start the search service role; prefer the equivalent `fleet serve search` spelling. |

The `atlas-search` executable alias is supported. `init` and `fetch-model` belong to the standalone `atlas-search` artifact, not the Fleet client.

## `laptop-agent` — laptop job broker

| Verb | Purpose |
| --- | --- |
| `health` | Check laptop-agent broker reachability. |
| `jobs` | List laptop jobs. |
| `kill JOB` | Stop a laptop job. |
| `exec [--cwd DIR] [--timeout SECS] [--shell] -- CMD...` | Execute a guarded command and wait for its result. |
| `run [--cwd DIR] [--shell] -- CMD...` | Submit a guarded laptop command as a job. |
| `poll JOB [--wait SECS]` | Read a job result or wait for it to finish. |
| `fetch REMOTE_PATH LOCAL_PATH` | Fetch a file from the laptop. |

Use `fleet laptop-agent --help` for exact forms. This daemon interface is distinct from `fleet queue dispatch --surface laptop`, which currently records a handoff and does not execute arbitrary laptop commands.

## Exit and availability

Usage errors generally exit 2. Capability stubs exit 69. Some domains define additional statuses, such as queue transport unavailable or `agent next` having no work. A successful command's exit code and output belong to its handler; preserve both when wrapping Fleet in scripts.