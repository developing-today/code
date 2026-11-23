# fleet actions

Live short-lived exec sessions inside GitHub Actions runners
(#13774), coordinator-handed-off. A seat opens a session at the
queue broker (the coordinator), dispatches the `fleet-exec.yml`
workflow in `dezren39/fleet`, and binds the resulting run — the
**handoff** is the session handle plus the portable
`OWNER/REPO#RUN_ID` form and the run attempt. Commands then run
on the runner with their IO streamed back through the broker;
files leave through the workflow artifact and, when configured,
R2. Nothing connection-shaped is ever printed: the workflow
inputs carry only the handle and non-secret parameters, and the
runner proves its run membership to the broker with the job's
GitHub OIDC token, verified against GitHub's JWKS with its
`repository`, `job_workflow_ref`, `run_id` and `run_attempt`
claims bound to the session row.

## The lease

Every session carries two clocks, both enforced by the broker
and by the runner-side agent (which exits at expiry even if
partitioned from the broker):

- the **lease** — set by `start --timeout`, required, no default;
  renewable through the coordinator, clamped at the max;
- the **max lifespan** — `--max-lifespan`, default 30 min,
  ceiling 2 h (the fleet leg cap). Past it, only an **emergency
  extension** helps: `exec extend` records the request and the
  broker raises a needs-drew alert (source `fleet-actions-exec`);
  Drew's approve/deny returns through `exec decide`, and approval
  adds up to +30 min to the max and the lease. While a decision
  is pending the session holds a 300 s grace past its lease.

## Commands

- `fleet actions exec start --repo OWNER/REPO --timeout S
  [--max-lifespan S] [--ref R] [--target-repo OWNER/REPO]
  [--hide-logs] [--detach] [--broker-url URL]` — open, claim
  (the #13765 coordinator claim is attempted first; while those
  verbs are not live the session proceeds on the singular path
  and the output names it via `placement_path`), dispatch, bind,
  print the handoff, then attach unless `--detach`. `--repo` is
  the workflow host repo (default `dezren39/fleet`); sessions are
  recorded in the local store under the state dir so the
  portable form resolves later.
- `fleet actions exec run <ref> [--timeout S] -- <command...>` —
  enqueue one command, stream its output, exit with its code.
- `fleet actions exec attach <ref>` / `watch <ref>` — stream the
  session's IO after handoff; `watch` also reports the run and
  session status.
- `fleet actions exec renew <ref> --timeout S` — extend the
  lease within the max.
- `fleet actions exec extend <ref> [--minutes M] [--reason T]` —
  request the emergency extension (raises the alert).
- `fleet actions exec decide <ref> --approve|--deny` — record
  Drew's answer.
- `fleet actions exec close <ref>` — close the session.
- `fleet actions agent …` — the runner half, invoked by the
  `fleet-exec-session` composite action in `dezren39/fleet`: it
  registers with the OIDC token, long-polls the broker, executes
  commands in the workdir, streams IO back, installs
  `fleet-exec-put` on the session PATH, and enforces the lease
  locally.
- `fleet actions put <file>` — the egress helper: copy the file
  into the session outbox (uploaded as the run's
  `fleet-exec-out` artifact) and, when the workflow populated the
  `FLEET_EXEC_R2_*` environment from masked repo secrets, PUT it
  to R2 under `fleet-exec/<handle>/` with a SigV4 signature.

## Log visibility

The agent echoes command IO to its own stdout by default, so it
lands in the public Actions run log; `start --hide-logs` turns
the echo off. Either way the broker is the transport — its IO
chunk store is the cache of record — and the seat's `run`,
`attach` and `watch` read from it. The OIDC token and the R2
credentials are never written to any log; the log audit is part
of the mode's acceptance bar.

## The broker surface

The verbs live in `crates/fleet-queue/src/turso_queue/
actions_exec.rs` beside placement's: `/actions-exec-open`,
`-bind`, `-command`, `-stream`, `-renew`, `-extend-request`,
`-extend-decide`, `-close` (seat side, broker token) and
`/actions-exec-register`, `-poll`, `-io`, `-result` (agent side,
OIDC through the public door at `exec.security.cab`, which is
path-scoped to these verbs). Session state lives in the
`actions_exec_sessions` / `actions_exec_commands` /
`actions_exec_io` tables; every transition is a conditional
UPDATE, so two actors racing a session resolve to exactly one
winner.
