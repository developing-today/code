# fleet

`fleet` is one multicall binary for the fleet's tools. A _multicall binary_ is a
single executable that behaves as many different programs depending on the name
it is invoked under and the first argument it is given. The fleet binary is
version `0.1.0`. Invoked as `fleet`, it dispatches to one of twenty-two _domains_ —
a domain is a named tool area, such as the work queue or the secrets resolver —
and invoked under one of its alias names (`repo-queue`, `queue-broker`,
`secrets-broker`, `laptop-agent-broker`, `atlas-search`, `laptop-agent`) it
routes straight into the corresponding domain. There is one binary to build,
one binary to install, and one binary to keep current on every machine the
fleet runs on.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/Cargo.toml

## Why fleet exists

Before fleet, each fleet tool was its own script or program, installed its own
way, and each one re-solved the same four problems. Fleet solves them once,
inside one binary:

1. **A shared work queue.** Agents on different machines need one place where
   work is filed, claimed by exactly one worker at a time under a _lease_ (a
   time-limited claim that expires if the holder stops reporting in), and
   completed with evidence. The queue domain and its resident queue broker
   provide that queue, backed by a dedicated Turso database. See
   [The queue: tasks, lanes and leases](concepts/queue.md).
2. **Secrets without vault sprawl.** Resolving a secret by shelling out to the
   Bitwarden Secrets Manager command-line tool (`bws`) on every call cost
   10–40 seconds per invocation. Fleet instead runs a _secrets broker_: a
   resident cache of the vault that answers by name in milliseconds, refreshes
   itself on a timer, and falls back to `bws` only when it must. Callers name a
   secret; they never hold a vault credential of their own. See
   [Secrets and the secrets broker](concepts/secrets.md).
3. **Remote build placement.** A limited sandbox is the wrong place to compile.
   Fleet decides, from live capacity data, which GitHub Actions copy of the
   fleet repositories should run a build — a personal copy, an organisation
   copy, a forks copy, a Blacksmith runner, or a codespace — and dispatches the
   job there. See [Placement and remote dispatch](concepts/placement.md).
4. **Agent coordination.** Agents need a mailbox, a way to prove what their
   seat can reach, and one rule for what work to take next. The bridge domain
   provides the mailbox, the agent domain provides enrolment, a four-leg
   health check called _doctor_, and the _intake ladder_ that decides what an
   agent works on next. See [The bridge: fleet messaging](concepts/bridge.md)
   and [Agents: enrolment, doctor and the intake ladder](concepts/agents.md).

## Who it serves

- **Agents on seats.** A _seat_ is one running agent instance in one place: a
  Muse session in a sandbox, an OpenCode session on the laptop, a chat
  assistant connected over HTTPS. An agent uses fleet to enrol, check its
  seat, take work, resolve secrets, message other agents, and run builds where
  builds belong.
- **Operators.** A human (or a supervising agent) running the fleet uses fleet
  to inspect queue health, read the message log, check which token sources
  still have GitHub rate-limit headroom, and run the daemon roles in the
  foreground under a supervisor.
- **Developers.** A developer changing fleet itself uses the same binary's
  registry and capability output to see exactly what a given build contains,
  and uses the Nix build variants described in
  [Installation and bootstrap](getting-started/installation.md).

## The domain map

Every build of fleet knows every domain. Whether a given invocation can _use_
a domain depends on the capabilities compiled into that build, on the name it
was invoked under, and on the host's configuration; a domain it cannot use
answers with a stub message and exit code 69. The full mechanics are in
[The shape of the binary](getting-started/shape.md).

| Domain          | Purpose                                                                                                                                                                          |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `queue`         | The repo work queue: enqueue, claim, and complete tasks through the queue broker. Alias: `repo-queue`.                                                                           |
| `serve`         | Run a fleet daemon role (queue-broker, secrets-broker, search, proxy) in the foreground. Aliases: `queue-broker`, `secrets-broker`.                                              |
| `proxy`         | The session proxy: serve the broker socket, connect or init through it, run recovery SSH, splice raw TCP streams, or transfer files (`get`/`put`). Alias: `laptop-agent-broker`. |
| `dispatch`      | Thin client over pm-stack dispatch: status, launch, and enqueue verbs.                                                                                                           |
| `dispatch-test` | Explicit-dispatch CI test: scan fleet repos for free Actions capacity, submit a job, track its stored status.                                                                    |
| `secrets`       | Resolve secrets by name: the secrets broker first, Bitwarden as fallback.                                                                                                        |
| `gh`            | The real `gh` CLI, passed through under a rotating fleet token (the fleet's GitHub Apps, then personal last); rate-limit and permission failures fail over to the next token.    |
| `bridge`        | Fleet messaging through the message bridge: central sends messages or commands to an agent, a group, or all; agents read their mailbox, answer, and ack.                         |
| `orgs`          | Turso-backed GitHub org, App and installation registry.                                                                                                                          |
| `kv`            | Key/value bytes for this session in the host-local store: get, set, list, delete, clear-override and revert, per namespace; snapshot and restore back up the whole host store.   |
| `id`            | Iroh-native ID transport: `session-key` mints this session's NodeId stamp.                                                                                                       |
| `agent`         | The agent's front door: enrol an agent of any kind, check what this seat reaches, and take the next piece of work down the intake ladder.                                        |
| `shell`         | Run a command, or open a shell, where builds belong: the laptop, a codespace, or here.                                                                                           |
| `search`        | Atlas corpus search over the session history (hybrid full-text plus vector search). Alias: `atlas-search`.                                                                       |
| `cert`          | PKI/X.509: root and issuer CAs, leaf certificates, chains and bundles. Alias: `pki`.                                                                                             |
| `laptop-agent`  | Client for the laptop-agent daemon: run and manage jobs on the laptop. Alias: `laptop-agent`.                                                                                    |
| `commander`     | Tier T1 (commander/fleet) of the laptop route forms: run commands on the laptop through the Desktop Commander MCP endpoint.                                                      |
| `http`          | Tier T2 (http/fleet) of the laptop route forms: the laptop-agent daemon HTTP API as a fleet subcommand (HTTP legs only; never ssh).                                              |
| `ssh`           | Tier T3 (ssh/fleet) of the laptop route forms: real SSH to the laptop under a binary-owned client contract.                                                                      |
| `laptop`        | Tier T0 (tiered/fleet) of the laptop route forms: the generic tiered command — walks T1 → T2 → T3 and reports the serving tier.                                                  |
| `host`          | Resolve a host name through the machine registry and select a transport offer and invocation form with `fleet-pool`; `fleet host laptop` resolves the amd record.                |
| `actions`       | Live short-lived exec sessions inside GitHub Actions runners: start a session in a runner, run commands in it, stream its IO, renew or extend its lease, close it.               |

Alongside the domains there are five built-in commands that are not domains:
`init` (autostart this host's roles per the host manifest), `session bind`
(validate and persist a session identity), `version`, `capabilities`, and
`help`.

## How to read this book

- **New seat, first hour:** [Installation and bootstrap](getting-started/installation.md),
  then [Quickstart](getting-started/quickstart.md). You will have a working
  binary and will have run the five commands that prove the seat works.
- **Agent or operator who wants to understand the system:**
  [The shape of the binary](getting-started/shape.md), then
  [Architecture](concepts/architecture.md), then the concept chapter for the
  domain you are working in.
- **Someone debugging a specific behaviour:** go straight to that domain's
  concept chapter. Every concept chapter is self-contained: it defines its
  terms, states its defaults and exit codes, and does not require reading the
  others first.
- **Someone changing fleet:** the concept chapters state the behaviour your
  change must preserve; the development part of this book covers how changes
  are built, tested, and shipped.

## Conventions used in this book

- **Commands** are shown as they are typed, for example `fleet agent doctor`.
  Inside the queue domain, the binary's own usage strings spell the program
  `repo-queue` (the argv0 alias), so queue examples may show either
  `fleet queue claim` or `repo-queue claim`; they are the same code path.
- **Flags and defaults.** When a flag has a default, the default is stated as a
  number or a literal value, for example "lease default 2700 s (45 minutes)".
  Where the help text and the source disagree, this book states the _source_
  behaviour and says so.
- **Exit codes.** Fleet uses a small shared vocabulary: 0 success, 2 usage
  error, 3 transport down / unresolved / a needed leg down, 69 capability
  unavailable (the stub). Individual domains add codes — 1, 4, 5, 6 appear in
  the queue, bridge, agent, and dispatch chapters — and each chapter lists its
  own codes in full.
- **Secrets are named, never valued.** This book gives secret _names_ (for
  example `fleet.secrets-broker-token`) and token _file paths_. It never
  prints a secret value, and neither does the binary: values go to the one
  place each command defines (standard output for `fleet secrets get`, a
  single header for an authenticated request) and nowhere else.
- **Source references** are provenance only. Each chapter names the source
  files it was written from, as absolute URLs under
  https://github.com/developing-today/code/blob/main/pkgs/fleet/. The chapter
  text itself is complete without following them.
