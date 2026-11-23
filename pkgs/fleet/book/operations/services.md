# Running the brokers as services

Fleet's long-running roles are broker services. Where they run depends on the seat: on the laptop they are systemd services owned by the deployment machinery; on a sandbox seat they are detached processes spawned by `fleet init`. This chapter covers both models. For deploying new generations of the laptop services, see [Release and rollout](../development/release-and-rollout.md). For checking them, see [Health checks and the heal stack](health-and-healing.md).

## The laptop model: `deploy.py` profiles

On the laptop, services are defined in `pkgs/deploy/services.nix` and deployed by `pkgs/deploy/deploy.py` (see [Release and rollout](../development/release-and-rollout.md) for the full profile, generation, and rollback mechanics). The properties that matter day to day:

- Each service gets a per-service Nix profile under the profile base (system scope: `/nix/var/nix/profiles/per-service`).
- `deploy.py` renders a systemd unit per service whose `ExecStart` points at the profile's **`current`** link — `<base>/<name>/current/<entryPoint>` — never at a bare store path. A generation flip is therefore a profile repoint plus a restart of that one unit; the unit file does not change.
- Activation restarts **only that unit**, health-checks it, and flips the profile back to the previous generation on failure.
- The generation history is the append-only JSONL log per target; `deploy status` folds it to report current and previous generations.

### The four laptop services

Three of the laptop's services are fleet's own; the fourth group is the pm stack that shares the same deployment machinery and the same host.

| Service | Runs | Port | Restart | Health |
|---|---|---|---|---|
| `queue-broker` | `fleet serve queue-broker`, backend `turso` | 4801 | `on-failure` | Unauthenticated `GET /healthz` |
| `secrets-broker` | `fleet serve secrets-broker` | 4800 | `on-failure` | `GET /health` (503 until the first successful refresh — that is a state, not a crash; see [Troubleshooting](troubleshooting.md)) |
| `laptop-agent` | `bin/laptop-agent` (its own package, not the fleet binary) | 4799 | `always` | No unauthenticated endpoint; verified through the authenticated client |
| pm stack neighbours | `pm-aggregator`, `pm-projector`, `pm-tunnel`, and the other Cloudflare/pm-stack targets declared in the same `services.nix` registry | per definition | per definition | per definition |

Operational notes:

- The queue-broker binds its HTTP surface with the agent token required on every route except `GET /healthz`. Its Turso backend comes up on a startup thread, so `/healthz` and `/validate` answer before the backend is ready; a broker that answers `/healthz` but fails verbs with transport errors is still starting, or its backend token is wrong.
- The secrets-broker binds the laptop tailnet address only on the laptop. Sandbox consumers reach it through the session proxy — except on the sandbox itself, where `fleet init` runs a local loopback copy (next section).
- `laptop-agent` restarts `always` on purpose: under `on-failure`, a clean SIGTERM or success exit leaves the daemon down. Do not "fix" that back.
- `fleet serve` can also run several roles in one process: `fleet serve queue-broker secrets-broker …` parses a multi-role list, capability-checks the **whole list before anything binds** (a role this invocation cannot run stubs the entire list at exit 69, with no partial start), and spawns one thread per role. A role thread's failure fails the process — the supervisor (systemd) restarts the unit. The deployed profiles nonetheless spell one role per unit, so each broker restarts and health-checks independently.

### `fleet init` in systemd mode (laptop)

`fleet init` never detached-spawns a daemon systemd owns. For each role in the host manifest it reports the `fleet-<role>.service` state via `systemctl --user is-active`: an active unit is a no-op; an inactive but existing unit is started with `systemctl --user start`; a missing unit is reported and skipped, not an error; unavailable `systemctl` is a report, not a spawn. Roles the manifest disables are reported as skipped with their reason, and the `[domains]` config gate wins over the manifest.

## The sandbox model: roles spawned by `fleet init`

A sandbox seat has no systemd ownership of fleet roles. There, `fleet init` uses **spawn mode**, which exists for exactly two roles; any other role in a spawn manifest is reported and skipped, never spawned. `fleet init` also runs session initialisation first; a session error exits 2 before any role work. A build whose usable surface lacks `proxy-serve` can only stub `init` (exit 69).

1. **proxy (the session proxy / session broker)** — init probes the broker socket and says hello. A live broker, in either protocol dialect, means status plus PATH-shim sync, and nothing more: a strict no-op. Init never spawns over, restarts, or fights an incumbent broker. Only an unanswered socket spawns `fleet serve proxy`, detached, after which init syncs shims from the new broker's services map.
2. **secrets-broker** — init probes its Unix socket; a live answer means status and out. Otherwise it spawns `fleet serve secrets-broker` detached (setsid, output appended to `secrets-broker.log` beside the socket) with `SECRETS_BROKER_BIND=127.0.0.1:4800` set explicitly — and the broker forces loopback itself when the seat's location is the sandbox, so the sandbox broker is loopback-only twice over. Init then waits (50 × 100 ms probes) for the socket to answer before reporting success.

Two more init behaviours to know:

- **Report mode.** Given no roles to start, init reports state — every source consulted, plus detected hostname/OS/systemd for the report only (detection never selects roles) — starts nothing, and exits 69. "Never guess-start a role" is the rule.
- **Provision before the binary flip.** `fleet init` resolves location and roles from the `[host]` section of `fleet.toml`. When rolling out a binary that understands a new config shape, write the `[host]` config first, pin the init call site, and only then deploy the binary. Rollback warning: pre-location-aware binaries parse-error on a `fleet.toml` containing `[host]` (strict unknown-field rejection), so config and binary roll back **together**.

## Restart and health expectations

- A broker that is down is restarted by its supervisor: systemd on the laptop (per the profile's restart policy), the heal stack on a sandbox seat (see [Health checks and the heal stack](health-and-healing.md)).
- Do not restart-loop a broker that is *up but not serving*. The canonical case is the secrets-broker before its first successful refresh: the process is alive, `/health` answers 503 with `serving=false`, and restarting it only restarts the refresh clock. Diagnose the refresh instead — see [Troubleshooting](troubleshooting.md).
- After any restart, verify with the health surfaces, not with process existence: `/healthz` for the queue-broker, `/health` for the secrets-broker, `fleet queue health` for the queue as a client sees it, and `fleet agent doctor` for the seat as a whole.
