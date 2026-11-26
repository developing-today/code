# Fleet context env contract (`FLEET_CTX_*`)

Build 2/5 of fleet context propagation (developing-today/code#14188;
decision record `doc/2026-11-24_design_fleet-context-propagation-10357.md`
at the repo root). Every fleet action carries its origin through a small,
fixed set of environment variables, captured once and passed unchanged
through every carrier below. The code of record is
`crates/fleet-config/src/ctx.rs` (`fleet_config::ctx`); this page mirrors it.

## Variables

| Variable                     | Header                         | Producer                      | Holds                                                                           |
| ---------------------------- | ------------------------------ | ----------------------------- | ------------------------------------------------------------------------------- |
| `FLEET_CTX_ID`               | `X-Fleet-Ctx-Id`               | init sourcing / launcher      | join key across stores: `task-<id>` when queue-launched, else `origin-host-pid` |
| `FLEET_CTX_ORIGIN`           | `X-Fleet-Ctx-Origin`           | init sourcing                 | `drew-interactive`, `opencode`, `claude`, `systemd-unit[:inner]`, `unknown`     |
| `FLEET_CTX_HOST`             | `X-Fleet-Ctx-Host`             | init sourcing                 | hostname at capture                                                             |
| `FLEET_CTX_USER`             | `X-Fleet-Ctx-User`             | init sourcing                 | acting OS user at capture                                                       |
| `FLEET_CTX_PID`              | `X-Fleet-Ctx-Pid`              | init sourcing                 | pid of the capturing shell                                                      |
| `FLEET_CTX_ANCESTRY`         | `X-Fleet-Ctx-Ancestry`         | init sourcing / session hooks | `/`-joined ancestor summary, e.g. `cron/muse/opencode`                          |
| `FLEET_CTX_TASK_ID`          | `X-Fleet-Ctx-Task-Id`          | queue worker / launcher       | fleet queue task id                                                             |
| `FLEET_CTX_TASK_NAME`        | `X-Fleet-Ctx-Task-Name`        | queue worker / launcher       | queue task name, slug form                                                      |
| `FLEET_CTX_MUSE_SESSION`     | `X-Fleet-Ctx-Muse-Session`     | Muse launcher                 | Muse session id                                                                 |
| `FLEET_CTX_MODEL`            | `X-Fleet-Ctx-Model`            | session hooks                 | internal model name serving the session                                         |
| `FLEET_CTX_OPENCODE_SESSION` | `X-Fleet-Ctx-Opencode-Session` | OpenCode session hook         | OpenCode session id                                                             |
| `FLEET_CTX_OPENCODE_PARENT`  | `X-Fleet-Ctx-Opencode-Parent`  | OpenCode session hook         | OpenCode parent session id                                                      |
| `FLEET_CTX_SCHEDULED_JOB`    | `X-Fleet-Ctx-Scheduled-Job`    | OpenChamber scheduler         | originating scheduled job                                                       |
| `FLEET_CTX_MESSAGE_ID`       | `X-Fleet-Ctx-Message-Id`       | session hooks                 | triggering message id                                                           |
| `FLEET_CTX_GIT_HEADS`        | `X-Fleet-Ctx-Git-Heads`        | init sourcing                 | `repo=sha` pairs, `,`-joined                                                    |
| `FLEET_CTX_INVOCATION`       | `X-Fleet-Ctx-Invocation`       | init sourcing under systemd   | systemd invocation id (Q3 default)                                              |

Every variable is optional. **Absence** is representable everywhere: an
empty record renders no header, no export, no frame field and no log line,
and no carrier treats it as an error.

## Value grammar

1 to 256 bytes of `[A-Za-z0-9._:@/+=,~-]`. The charset is safe unquoted in
an HTTP header, a POSIX shell word and a log line. A value outside the
grammar is **dropped**, never escaped and forwarded; names outside the
fixed set never travel. Values are ids and hashes only, never secrets: the
context is logged widely by design.

## Carriers

| Surface                       | Carrier                                                                                                                          | Where                                                  |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Process env                   | `FLEET_CTX_*` (canonical record, Q1 default)                                                                                     | everywhere                                             |
| laptop-agent exec / jobs      | spec `env` object; daemon merges it, records it in the job spool (`meta.json`) and puts `ctx_id` in `trips.jsonl`                | `fleet-laptop-agent` `command_spec`, `laptop_agent.py` |
| laptop-agent SSH fallback leg | `export K=V; …` from the spec env                                                                                                | `fleet-laptop-agent` `ssh_inner`                       |
| `fleet ssh exec`              | explicit `export K='V' …;` prefix on the remote line (works whatever `AcceptEnv` says)                                           | `crates/fleet/src/ssh.rs`                              |
| Queue broker                  | `X-Fleet-Ctx-*` headers on the direct TCP/UDS legs; `ctx` object on session-proxy frames                                         | `fleet-queue` client transport                         |
| Secrets broker                | `X-Fleet-Ctx-*` headers (direct UDS); `ctx` frame field (session proxy)                                                          | `fleet-secrets`                                        |
| Session proxy                 | forwards the frame's `ctx` as `X-Fleet-Ctx-*` headers on every attempt                                                           | `fleet-proxy` serve                                    |
| File transfer                 | `fleet proxy` files frames carry `ctx`; `laptop-agent fetch` carries it in its hashing exec spec and logs `ctx_id` on every trip | `fleet-proxy` files, `fleet-laptop-agent`              |

## Per-trip logging

Both brokers log one line per authenticated request that carried context:

    queue-broker: trip POST /claim status=200 FLEET_CTX_ID=task-22655 FLEET_CTX_TASK_ID=22655

The fields are the canonical rendering (`Ctx::log_fields`): every leg logs
the same bytes, so `grep FLEET_CTX_ID=task-22655` follows one context
across the client trip log, the daemon's trip log and job spool, and both
brokers' journals. Requests without context log nothing new; unauthorized
requests never log context.

## Queue event store (#14191)

While the queue broker serves a request that carried context, every
`events` row the request writes gets the context id in its `detail` as
`ctx_id` (`turso_queue::ctx_scope`). A detail that already names a
`ctx_id`, or that is not a JSON object, is stored as written; rows from
requests without context, and from the broker's own sweepers, are
unchanged. So the same id joins the broker trip line above and the
durable events `repo-queue events <task>` lists.

## Proof

`crates/fleet/tests/ctx_propagation.rs` launches the three legs as separate
processes with no model in the loop — laptop-agent exec spec over an empty
environment, the SSH hop under a scrubbed shell, and a queue-broker trip
served by the real `uds_http` server — and asserts the id, and the whole
record, are byte-identical in each leg's log.
