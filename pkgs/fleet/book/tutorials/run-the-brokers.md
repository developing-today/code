# Tutorial: run the brokers

This tutorial runs the queue broker in the foreground, proves it healthy
two different ways, adds the secrets broker beside it in the same
process, and stops everything cleanly. It is the manual form of what a
service manager runs in production — useful on a new host, in a test
environment, and whenever you want a broker's startup output in front of
you instead of in a log.

It assumes the `fleet` binary is installed (see
[Installation and bootstrap](../getting-started/installation.md)).

Two facts frame everything below:

- The broker roles take **no arguments**. Through `fleet serve`, any
  trailing argument to `queue-broker` or `secrets-broker` is rejected
  before the daemon starts (`unexpected argument '<first>' for 'fleet
serve <role>'`, exit `2`) — through the spelled-out form and through
  the `queue-broker` / `secrets-broker` argv0 aliases alike. There is no
  per-role help for the two brokers either: `fleet serve queue-broker
--help` answers with the generic serve domain help, exit `0`.
- In production these roles are supervised (systemd on the laptop), and
  the supervisor restarts them. In this tutorial **you** are the
  supervisor: the process runs until you stop it.

## Step 1: Run the queue broker in the foreground

```bash
fleet serve queue-broker
```

(The argv0 spelling `queue-broker`, invoked as its own program name,
lands in exactly the same place.)

What happens, in order:

1. The broker loads its token from its token file —
   `QUEUE_BROKER_TOKEN_FILE`, default `~/.config/queue-broker/token`.
   Requests carry that token in the `X-Agent-Token` header; an
   unreadable token file is a startup error, exit `3`. Token files are
   loaded by reading and trimming the file; a missing, unreadable, empty
   or whitespace-only file is a hard error naming the file, and the
   broker refuses to start.
2. It binds **two** listeners with identical routes and auth: TCP on
   `QUEUE_BROKER_BIND` (default `100.99.113.103:4801`, the laptop's
   tailnet address; port `4801` is the queue broker's fleet-wide port),
   and a Unix socket on `QUEUE_BROKER_SOCKET` (default
   `~/.config/queue-broker/broker.sock`). If the socket bind fails, the
   broker degrades to TCP-only with a log line rather than dying.
3. The listener comes up **before** the backend: the Turso backend
   connects on a startup thread, so the health probe and validation
   answer immediately while queue verbs answer exit `3` with a
   `turso_pending` reason until the backend is ready. A backend that
   cannot come up within its retry budget exits the broker with `3`, so
   a supervisor restarts it.
4. Optional (BT-3, Phase 1): if `QUEUE_BROKER_TLS_CERT_FILE` and
   `QUEUE_BROKER_TLS_KEY_FILE` name a `broker-server` leaf chain and
   its on-host key, the TCP leg also presents TLS server
   authentication. Plaintext clients keep working on the same port
   (their accepts are logged); clients configured with
   `REPO_QUEUE_TLS_TRUST_STORE` validate the leaf against their BT-2
   trust store and fail closed. Setting exactly one of the pair is a
   startup error, exit `3`.

The process now blocks in the foreground. Leave it running and work from
a second terminal for the next steps.

What can go wrong, and the fix:

- Exit `3` at startup: the token file, the bind, or the backend config.
  The backend accepts only Turso: `QUEUE_BROKER_BACKEND` unset, empty or
  `turso` is fine; anything else is a startup config error, exit `3`.
  The backend's credential comes from `QUEUE_BROKER_TURSO_TOKEN_FILE`,
  or is resolved through the secrets broker by the secret **name**
  `turso.fleet-queue-rw-token`; its URL is `QUEUE_BROKER_TURSO_URL`,
  defaulting to the dedicated `fleet-queue` database.
- The broker also needs the secrets broker for some resolutions: it
  reads `QUEUE_BROKER_SECRETS_URL` (default `http://100.99.113.103:4800`,
  or `unix://<path>` for the secrets broker's socket) and
  `QUEUE_BROKER_SECRETS_TOKEN_FILE` (default
  `~/.config/secrets-broker/token`). Step 3 runs that broker too.
- Sandbox clients never reach this process directly: they reach it
  through the session proxy, which forwards to port `4801`. A broker
  that is healthy locally (Step 2) but unreachable from a sandbox has a
  proxy problem, not a broker problem.

## Step 2: Health-check the queue broker — two different checks

These two commands answer different questions. Do not substitute one for
the other.

The unauthenticated probe, which any monitor can call:

```bash
curl -s http://100.99.113.103:4801/healthz
```

It answers `{"service":"queue-broker"}`. `/healthz` is the broker's only
unauthenticated route, and it is served without the Turso backend — so
it proves the process is up and listening, **not** that the backend is
ready. (Everything else on the broker is a `POST` with the token header;
a non-POST request gets `405 {"error":"POST only"}`, a wrong or missing
token `401 {"error":"unauthorized"}`, and an oversized body `413` — the
pre-body gate refuses from headers alone, before any body byte is read.
The body cap is 16 MiB.)

The queue-level health verb, which goes through the broker's real
request path:

```bash
fleet queue health
```

What the output means: a first line `project: <netloc> (resolved via
<source>)`, then a table of per-queue task counts by status, built from
whatever keys the broker's health rows carry — or `(no queues have
tasks)` when the store is empty. Exit `0` means the broker **and** its
backend answered a real queue request. If `/healthz` answers but this
verb exits `3` in the first moments after startup, the backend is still
connecting (`turso_pending`); give it its retry window before treating
it as a failure. Note that `fleet queue health` is itself on the
fallback-switch path: a dead primary store switches the verb to the
GitHub fallback with exit `6` rather than a plain failure.

## Step 3: Add the secrets broker

Stop the Step 1 process (Step 5 shows the clean way) and start both
roles in one process, as a serve list:

```bash
fleet serve queue-broker secrets-broker
```

What the output means: `fleet serve: starting 2 roles in one process:
queue-broker secrets-broker`. A serve list is **role tokens only, two or
more distinct ones, and nothing else**:

- A duplicate is a hard error: `error: duplicate role 'queue-broker' in
'fleet serve' list`.
- Any non-role token is a hard error — including `--help`. A serve list
  has **no `--help` form at all**.
- **All roles are capability-checked before anything binds.** If any
  role cannot run in this invocation, the whole list stubs with exit
  `69` and no partial start.
- Each role's blocking loop runs on its own thread. Supervision is
  fail-fast: the first role thread to return or panic takes the whole
  process down, naming the role on stderr. A role exiting _cleanly_
  (code `0`) is converted to process exit `1` — a daemon role returning
  at all is abnormal — and a panic is also exit `1`.

The secrets broker in this list behaves exactly as it does alone
(`fleet serve secrets-broker` in its own terminal works too, as does the
`secrets-broker` argv0 alias):

- It binds TCP on `SECRETS_BROKER_BIND` (default
  `100.99.113.103:4800`) plus its Unix socket
  (`SECRETS_BROKER_SOCKET`, default `~/.config/secrets-broker/broker.sock`),
  with its token from `SECRETS_BROKER_TOKEN_FILE` (default
  `~/.config/secrets-broker/token`; unreadable is a startup exit `3`).
  On a sandbox seat (`FLEET_LOCATION=sandbox`) the bind is **forced to
  `127.0.0.1:4800`** at the bind itself, whatever the environment says.
- It is a resident Bitwarden Secrets Manager **cache**: at startup a
  background thread runs a full refresh (one `bws secret list` call
  carrying the whole vault, installed atomically and, on later refreshes,
  applied as a delta against the persisted watermark), then repeats on
  the TTL — `SECRETS_BROKER_TTL_S`, default `900` seconds, minimum `30`.
  An unparseable TTL falls back to 900 with a warning naming the value;
  a parsed TTL below 30 makes startup **refuse to run**, exit `2`,
  because one interval costs a full vault refresh and 0 would spin that
  loop with no sleep at all.
- Until the first refresh _succeeds_, the broker is up but **not
  serving**: its health endpoint answers `503`. That gate is the subject
  of Step 4.

What can go wrong, and the fix:

- The whole list stubs at exit `69` naming a role: this invocation
  cannot run that role (a narrowed build such as `fleet-connect`, or the
  domain disabled in the fleet config). Nothing bound — fix the
  invocation, not the roles.
- One role's startup failure (for example the secrets broker's token
  file) takes the list down by design. Read the stderr line naming the
  role, fix that role's cause, and start the list again.

## Step 4: Health-check the secrets broker

```bash
curl -s http://100.99.113.103:4800/health
```

What the output means: `{ok, serving, secrets, last_refresh_unix,
refreshing, oldest_secret_age_seconds}`.

- `serving: true`: at least one refresh has succeeded. `secrets` is the
  cached name count; `oldest_secret_age_seconds` is the stalest value
  the broker can currently serve; reads are cache-first, with a
  read-through to Bitwarden on a miss.
- HTTP `503` with `serving: false` means the first successful refresh is
  pending. Follow `refreshing` and `last_refresh_unix` on public health.
  An authenticated secret read while unready can return a 503 with
  `refresh_error` when a refresh has failed.
- The client-side cross-check is `fleet secrets list`, which lists the
  broker cache's names and never falls back (see
  [Tutorial: resolve a secret](resolve-a-secret.md)).

## Step 5: Know where state lives, then stop cleanly

Broker state is mostly **not** in local files, which is what makes
stopping and restarting these processes safe:

- **Queue state** lives in the dedicated Turso database `fleet-queue`
  (org `dezren39`, group `great-star-lord`) — the broker's only backend.
  The broker process itself holds connections and in-flight requests,
  not the queue. Tasks, leases, lanes, events, the creation outbox and
  merge-intent stacks are all rows in that store.
- **Secrets state** is a cache, held in memory: the authoritative copy
  is Bitwarden itself, and a restarted broker rebuilds its cache from
  `bws` on its first refresh. Nothing is lost by stopping it — only
  freshness, until the first refresh lands.
- **Local files** are configuration and credentials only: the token
  files (`~/.config/queue-broker/token`,
  `~/.config/secrets-broker/token`), the sockets
  (`~/.config/queue-broker/broker.sock`,
  `~/.config/secrets-broker/broker.sock`), and the fleet config that
  resolved them. The placement session store
  (`~/.local/state/fleet-place/sessions.json`) belongs to the dispatch
  client, not to either broker.

To stop: send the foreground process `Ctrl-C` (or `SIGTERM` to its
process group under your own supervisor). A clean stop is simply the
process exiting — there is no drain protocol to run, no state to flush,
and no shutdown verb. In a serve list, stopping the process stops every
role in it together; remember the fail-fast rule in reverse: if one role
dies on its own, the process exit you observe (the role's own code, or
`1` for a clean/panic exit) names that role on stderr, and the correct
response is to fix the named role and restart the list — exactly what
systemd does with the production units.

What can go wrong, and the fix:

- A restarted queue broker answers `/healthz` but queue verbs exit `3`:
  the backend is still connecting. This is the designed startup
  ordering, not a wedge — unless the backend's retry budget expires, at
  which point the broker exits `3` and your supervisor (or you) restarts
  it.
- A restarted secrets broker serves `503` longer than a refresh should
  take: check the readiness fields in Step 4 and the most recent refresh
  failure in the broker logs or the authenticated not-loaded response.

## You are done when

- `curl -s http://100.99.113.103:4801/healthz` answers
  `{"service":"queue-broker"}` **and** `fleet queue health` exits `0`
  with a counts table — both checks, not just the probe.
- `fleet serve queue-broker secrets-broker` starts both roles in one
  process, and the secrets broker's `/health` reports `serving: true`
  with a set `last_refresh_unix`.
- You can name where each piece of state lives — queue rows in the
  Turso `fleet-queue` database, secrets as an in-memory cache over
  Bitwarden, only tokens/sockets/config as local files — and you have
  stopped the foreground process cleanly with no state lost.
