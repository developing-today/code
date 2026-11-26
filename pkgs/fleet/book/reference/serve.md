# fleet serve

`fleet serve` runs one or more fleet daemon roles in the foreground. Each role is a blocking serve loop that also exists as a leaf elsewhere in the binary; `serve` is the composition point a service manager (systemd) supervises.

Roles:

| Role token       | Capability       | Runs                                      | Same leaf as                                     |
| ---------------- | ---------------- | ----------------------------------------- | ------------------------------------------------ |
| `queue-broker`   | `queue-broker`   | the repo work queue as a resident service | argv0 `queue-broker`                             |
| `secrets-broker` | `secrets-broker` | the Bitwarden-backed secrets cache        | argv0 `secrets-broker`                           |
| `search`         | `search-serve`   | the Atlas search service                  | `fleet search serve`, argv0 `atlas-search serve` |
| `proxy`          | `proxy-serve`    | the session proxy broker                  | `fleet proxy serve`                              |

See also [fleet proxy](proxy.md), [fleet secrets](secrets.md), [fleet search](search.md), and [Secrets and the secrets broker](../concepts/secrets.md).

## Role-list semantics

```
fleet serve <role>
fleet serve <role> [<role> ...]
```

- **One role:** `fleet serve <role>` runs that role in the foreground.
- **Two or more roles:** `fleet serve <role> [<role> ...]` with two or more _distinct_ role tokens and nothing else runs all of them in one process. The parser only engages as a list when at least the first two tokens are both role tokens; it then validates the whole tail.
- A duplicate is a hard error: `error: duplicate role 'queue-broker' in 'fleet serve' list`.
- Any non-role token is a hard error, for example: `error: unexpected argument '--help' in 'fleet serve' list: a serve list is role tokens only (queue-broker, secrets-broker, search, proxy)`.
- **All roles are capability-checked before anything binds.** If any role is not available in this invocation, the whole list stubs with exit 69 and `fleet serve: role '<role>' cannot run in this invocation` — there is no partial start.
- Each role's blocking loop runs on **its own thread** in the one process. Startup prints `fleet serve: starting N roles in one process: <tokens>`.
- Supervision is fail-fast: the supervisor polls the threads on a 100 ms tick; **the first role thread to return or panic takes the whole process down** (the external supervisor restarts the unit). The process exit code is the role's own code, except that a role exiting cleanly (code 0) is converted to exit **1** — a daemon role returning at all is abnormal — and a panic is also exit 1. The role is named on stderr either way.
- One composition subtlety: the `proxy` role inside a list runs co-hosted, which **disables the proxy's idle-exit**, so an idle proxy cannot exit the shared process and kill its siblings.

Help: appending `--help` to a two-role list prints the serve domain help and exits 0 — a list has no single role to name. Single-role `--help` prints that role's own usage, for the two brokers as well (see below).

If the invocation cannot use a role at all (not in this build, narrowed out by the invocation name, or disabled by config), `serve` answers with the capability stub on stderr, always **exit 69**.

### argv0 aliases

Invoking the binary as `queue-broker` or `secrets-broker` routes straight into the serve domain with that role prepended. Help, stub, and argument-rejection behaviour are identical to the spelled-out form.

## fleet serve queue-broker

```
fleet serve queue-broker
```

| Flag | Argument | Default | Meaning                                    |
| ---- | -------- | ------- | ------------------------------------------ |
| —    | —        | —       | This role takes no flags and no arguments. |

### Behaviour

Runs the repo work queue as a resident service. Any trailing argument is rejected before the daemon starts with `unexpected argument '<first>' for 'fleet serve queue-broker'`, exit 2 — through the serve spelling and through the argv0 alias alike. A help flag in the flag position (`fleet serve queue-broker --help`) is answered first with the _generic serve domain help_, exit 0.

- **Bind:** TCP `QUEUE_BROKER_BIND`, default `100.99.113.103:4801` (the laptop tailnet address), **plus a Unix-socket dual-bind**: `QUEUE_BROKER_SOCKET`, default `~/.config/queue-broker/broker.sock`. Routes and auth are identical on both transports. A Unix-socket bind failure degrades to "TCP only" with a log line.
- **Token:** `QUEUE_BROKER_TOKEN_FILE`, default `~/.config/queue-broker/token`; an unreadable token file is a startup error, exit 3. Requests carry `X-Agent-Token`; a pre-body gate refuses unauthenticated or oversized requests from headers alone (401 / 413) before any body byte is read. The only unauthenticated route is the health probe.
- **TLS (BT-3, Phase 1):** with `QUEUE_BROKER_TLS_CERT_FILE` and `QUEUE_BROKER_TLS_KEY_FILE` naming a `broker-server` leaf chain and its on-host key (PEM), the TCP leg presents TLS server authentication; the same port dual-accepts plaintext Phase 0 connections, which are logged (design rules M1/M2). Exactly one of the pair set, or an unusable pair, is a startup error, exit 3 — never a silent plaintext fallback. Token auth is unchanged and the Unix socket never takes TLS. Clients validate with `REPO_QUEUE_TLS_TRUST_STORE` (a BT-2 trust store; fail closed on an untrusted leaf) or skip validation under the named break-glass `REPO_QUEUE_TLS_BREAK_GLASS=queue-broker`, which logs on every use.
- **Protocol:** `POST /<command>` with a JSON args object; response envelope `{"ok": true, ...}` or `{"ok": false, "exit": N, "error": "..."}` where `exit` mirrors the queue client's codes (2 validation, 3 config, 4 HTTP, 5 no row).
- **Health:** `GET /healthz` returns `{"service": "queue-broker"}`, unauthenticated.
- **Backend:** Turso only, on the dedicated `fleet-queue` database at `QUEUE_BROKER_TURSO_URL`, token from `QUEUE_BROKER_TURSO_TOKEN_FILE` or resolved from the secrets broker. `QUEUE_BROKER_BACKEND` unset, empty, or `turso` is accepted; anything else is a startup config error, exit 3.
- **Secrets dependency:** `QUEUE_BROKER_SECRETS_URL` (default `http://100.99.113.103:4800`, or `unix://<path>` for the secrets broker's socket) and `QUEUE_BROKER_SECRETS_TOKEN_FILE` (default `~/.config/secrets-broker/token`).
- **Startup ordering:** the listener binds _first_, then the Turso backend comes up on a startup thread, so `/healthz` answers immediately and queue verbs answer exit 3 with a `turso_pending` reason until the backend is ready. A backend that cannot come up within its retry budget still exits the broker with 3 so the supervisor restarts it.

Help: `fleet serve queue-broker --help` prints this role's own usage — the environment configuration surface below — and exits 0.

### Exit codes

- `0` — only via `--help`; a running broker returning 0 in a serve list is converted to 1 by the supervisor.
- `2` — unexpected argument.
- `3` — token-file or backend startup failure.
- `69` — role not available in this invocation.

### Example

```console
$ fleet serve queue-broker
```

Runs the queue broker in the foreground on TCP port 4801 and its Unix socket until stopped.

## fleet serve secrets-broker

```
fleet serve secrets-broker
```

| Flag | Argument | Default | Meaning                                    |
| ---- | -------- | ------- | ------------------------------------------ |
| —    | —        | —       | This role takes no flags and no arguments. |

### Behaviour

Runs the secrets broker in the foreground, dual-bound on TCP and a Unix socket. Any trailing argument is rejected before the daemon starts with `unexpected argument '<first>' for 'fleet serve secrets-broker'`, exit 2. A help flag in the flag position is answered first with the generic serve domain help, exit 0.

The broker's full semantics — the serving gate, cache-first reads, read-through on miss, refresh, and per-secret install — are covered in [Secrets and the secrets broker](../concepts/secrets.md). The client side is [fleet secrets](secrets.md). Its configuration:

| Env var                     | Default                                | Meaning                                                                                                                               |
| --------------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `SECRETS_BROKER_BIND`       | `100.99.113.103:4800`                  | TCP bind. On the sandbox (`FLEET_LOCATION=sandbox`) the bind is forced to `127.0.0.1:4800` at the bind itself, whatever the env says. |
| `SECRETS_BROKER_SOCKET`     | `~/.config/secrets-broker/broker.sock` | Unix-socket dual-bind; a bind failure degrades to TCP-only with a log line.                                                           |
| `SECRETS_BROKER_TOKEN_FILE` | `~/.config/secrets-broker/token`       | Daemon token file; unreadable means startup exit 3.                                                                                   |
| `SECRETS_BROKER_TTL_S`      | `900` (minimum `30`)                   | Refresh interval in seconds. Unparseable falls back to 900 with a warning; parsed but below 30 refuses to start, exit 2.              |

Endpoints (header `X-Agent-Token` required on everything except `/health`; identical over TCP and the Unix socket): `GET /health`, `GET /secrets` (names only, sorted), `GET /secret/<name>`, `POST /refresh`.

Help: `fleet serve secrets-broker --help` prints this role's own usage — the environment configuration surface below — and exits 0.

### Exit codes

- `0` — only via `--help`.
- `2` — unexpected argument, bad home, or a refused TTL.
- `3` — token-file or bind failure.
- `69` — role not available in this invocation.

### Example

```console
$ fleet serve secrets-broker
```

Runs the secrets broker in the foreground on TCP port 4800 and its Unix socket until stopped.

## fleet serve search

```
usage: fleet serve search [options]
```

| Flag            | Argument | Default                                                                        | Meaning                                   |
| --------------- | -------- | ------------------------------------------------------------------------------ | ----------------------------------------- |
| `--config`      | `PATH`   | `~/.local/state/atlas-search/service.json` (env `ATLAS_SEARCH_SERVICE_CONFIG`) | Service config file.                      |
| `--listen`      | `ADDR`   | from config                                                                    | Listen address override (loopback only).  |
| `--job-ttl-ms`  | `N`      | `600000`                                                                       | Completed-job TTL override, milliseconds. |
| `--hard-ttl-ms` | `N`      | `3600000`                                                                      | Submit hard-cap override, milliseconds.   |
| `-h`, `--help`  | —        | —                                                                              | Print this role's usage and exit 0.       |

### Behaviour

This is the only flag-bearing serve role. It runs the Atlas search service in the foreground: the loopback HTTP job protocol over one shared Snowflake embedder and a read-only Turso backend — the atlas-search service rehomed into fleet, same config, same wire. The role is also reachable as `fleet search serve`; both spellings require the `search-serve` capability. See [fleet search](search.md) for the client.

- The embedder is the Snowflake `snowflake-arctic-embed-m-v2.0` model at a pinned snapshot revision, loaded lazily on the first vector query only — FTS-only jobs and a cold service work without it. The model directory resolves from `ATLAS_MODEL_DIR` / the Nix closure.
- The backend is a read-only Turso backend; its token comes only from the service config chain, never the environment.
- The job protocol is a bounded async queue with in-memory temp state; `/health` reports the embedder load state.
- `init` (client table) and `fetch-model` deliberately stay on the standalone atlas-search artifact. Rollback is pointing the unit's ExecStart back at `atlas-search serve`.

### Exit codes

- `0` — only via `--help`; a running service never exits 0.
- `1` — runtime failure.
- `2` — usage / config error.
- `69` — role not available in this invocation.

### Example

```console
$ fleet serve search --job-ttl-ms 300000
```

Runs the search service with a five-minute completed-job TTL.

## fleet serve proxy

```
fleet serve proxy
```

| Flag | Argument | Default | Meaning                                    |
| ---- | -------- | ------- | ------------------------------------------ |
| —    | —        | —       | This role takes no flags and no arguments. |

### Behaviour

Runs the session proxy (the broker) in the foreground, binding the broker socket from fleet config (`FLEET_BROKER_SOCKET` / `[proxy] broker_socket`, default `~/.cache/laptop-agent/broker.sock`). It is the same leaf as `fleet proxy serve`; see [fleet proxy](proxy.md) for the full broker semantics — warm pools, the hello negotiation, keepalive, and idle exit. Inside a multi-role serve list the proxy runs co-hosted and its idle-exit is disabled (see Role-list semantics above). Unlike the two broker roles, this role has its own usage text: `fleet serve proxy --help` prints the proxy-role usage, exit 0.

### Exit codes

- `0` — clean exit (for example `--help`, or a live broker already owning the socket); in a serve list a clean role exit is converted to 1.
- `2` — unexpected argument.
- `69` — role not available in this invocation.

### Example

```console
$ fleet serve queue-broker secrets-broker proxy
fleet serve: starting 3 roles in one process: queue-broker secrets-broker proxy
```

Runs all three roles in one supervised process; the first role to exit takes the process down.
