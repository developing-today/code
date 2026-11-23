# fleet laptop-agent

`fleet laptop-agent` is the client for the laptop-agent daemon — the Python service on the laptop at `100.99.113.103:4799`. The client speaks HTTP JSON to it; the token is the laptop-agent token file (`~/tools/laptop-agent-token`), read per request and sent as `X-Agent-Token` on the direct legs; on the broker leg the token rides in the framed request's named token file instead. The domain's argv0 alias is `laptop-agent` itself.

See also [fleet proxy](proxy.md) and [fleet shell](shell.md).

Known discrepancy: there is **no `--help` handling anywhere in this domain**. The shared USAGE text is printed only as part of error messages, and a `--help` argument is treated as data — `laptop-agent jobs --help` errors `unknown command "jobs"` (because `jobs` accepts no trailing arguments), `poll --help` actually polls a job named `--help` and prints its real `HTTP 404: no such job`, and `kill --help` prints nothing. This chapter is written from source.

## Shared behaviour (all verbs)

Strict parsing: anything unaccounted for is a usage error (exit 2) before any transport is touched. Job ids are restricted to the characters the daemon mints (`[A-Za-z0-9_-]`), because a job id goes into a URL path.

Each call gets a fresh **trip id** (sent as the `X-Trip-Id` header on HTTP legs / the `trip` field on the broker leg, and used for exec cancellation and log correlation). The default call deadline is **660 s**, overridable via `LAPTOP_AGENT_DEADLINE_S`. Failures print `laptop-agent: <error>` and exit 1, except the timeout and usage cases noted per verb.

### The transport ladder

Every verb's call goes down a four-leg **ladder**, in order; the first leg that answers wins, and when an earlier leg failed the client prints one line: `laptop-agent: note: answered by <leg> after <legs> failed`.

1. **Broker** — the warm session-broker socket (default `~/.cache/laptop-agent/broker.sock`): the framed protocol (8-byte big-endian length + JSON `{method, path, body, trip}`; reply framed the same way, `{status, body}`). Absent when no socket file exists.
2. **Cloudflare** — TLS through the egress proxy to the tunnel hostname, with the Access service token resolved by name (secret name `cloudflare.access-laptop-agent`) _only when this leg is tried_, so a healthy broker never pays for the resolution; a resolution failure is cached 60 s.
3. **Tailscale** — HTTP CONNECT through the egress proxy at port **3130** to `100.99.113.103:4799`.
4. **SSH** — a named **stub**: always absent ("ssh leg not implemented"). The ladder's fourth leg exists in the ordering but never carries traffic in this build.

**Replay rules:** each leg classifies its failures as **absent** (not configured here), **unsent** (failed before the request was written), or **sent** (failed after). A request may fall through to the next leg freely when absent/unsent, but a _sent_ failure replays only for idempotent calls — non-idempotent calls (`exec`, `run` submits) are never replayed on a leg that may already have sent them, so a submitted job or an exec is never duplicated by the ladder.

The USAGE block (printed only inside error messages) is:

```
usage: fleet laptop-agent <command>
  health
  jobs
  kill JOB
  exec [--cwd DIR] [--timeout SECS] [--shell] -- CMD...
  run  [--cwd DIR] [--shell] -- CMD...
  poll JOB [--wait SECS]
  fetch REMOTE_PATH LOCAL_PATH
```

## fleet laptop-agent health

```
fleet laptop-agent health
```

| Flag | Argument | Default | Meaning                                                             |
| ---- | -------- | ------- | ------------------------------------------------------------------- |
| —    | —        | —       | Takes no arguments. Any trailing argument is a usage error, exit 2. |

### Behaviour

Issues `GET /health` to the daemon (via the ladder) and prints the daemon's health JSON verbatim.

### Exit codes

- `0` — the daemon answered.
- `1` — no leg answered.
- `2` — usage.

### Example

```console
$ fleet laptop-agent health
```

Prints the daemon's health JSON.

## fleet laptop-agent exec

```
fleet laptop-agent exec [--cwd DIR] [--timeout SECS] [--shell] -- CMD...
```

| Flag        | Argument | Default        | Meaning                                                                                                                  |
| ----------- | -------- | -------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `--cwd`     | `DIR`    | daemon default | Working directory for the command (sent as the spec's `cwd`).                                                            |
| `--timeout` | `SECS`   | `120`          | Daemon-side timeout in seconds. The client deadline is timeout + 15 s transport grace.                                   |
| `--shell`   | —        | off            | Run the joined command string as a shell command (`{cwd, shell: "cmd string"}`) instead of an argv spec (`{cwd, argv}`). |
| `--`        | —        | required       | Separator; everything after it is the command. No command is a usage error ("no command given (use -- cmd...)").         |

### Behaviour

**Synchronous** run: `POST /exec` with body `{cmds: [spec], timeout_s}`. Prints the first result's stdout to stdout and stderr to stderr, and returns the command's own exit code. Output capture is capped at **1 MiB per stream** (tail kept) with a truncation warning on stderr. On client-deadline expiry it best-effort cancels by trip (`POST /exec/<trip>/cancel`, 3 s budget) and exits **124**; a daemon-reported timeout also exits 124. `exec` is non-idempotent for the ladder: it is never replayed after a sent failure.

### Exit codes

- The command's own exit code.
- `1` — transport failure.
- `2` — usage.
- `124` — timeout (client deadline or daemon-reported).

### Example

```console
$ fleet laptop-agent exec --timeout 300 -- cargo build
```

Runs `cargo build` synchronously on the laptop with a 300 s timeout and returns its exit code.

## fleet laptop-agent run

```
fleet laptop-agent run [--cwd DIR] [--shell] -- CMD...
```

| Flag        | Argument | Default        | Meaning                                                                                                                                  |
| ----------- | -------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `--cwd`     | `DIR`    | daemon default | Working directory for the job.                                                                                                           |
| `--shell`   | —        | off            | Run the joined command string as a shell command instead of an argv spec.                                                                |
| `--`        | —        | required       | Separator; everything after it is the command.                                                                                           |
| `--timeout` | —        | —              | **Not accepted** — passing it is an unknown-option error. Detached jobs have no client-side timeout; use `poll --wait` to bound waiting. |

### Behaviour

**Detached** submit: `POST /jobs`. Prints the returned `job_id` and exits 0. Follow up with `poll`. `run` is non-idempotent for the ladder: it is never replayed after a sent failure.

### Exit codes

- `0` — job submitted, `job_id` printed.
- `1` — transport failure.
- `2` — usage (including `--timeout`).

### Example

```console
$ fleet laptop-agent run -- ./long-build.sh
job_…
```

Submits a detached job and prints its job id.

## fleet laptop-agent poll

```
fleet laptop-agent poll JOB [--wait SECS]
```

| Flag               | Argument | Default  | Meaning                                                  |
| ------------------ | -------- | -------- | -------------------------------------------------------- |
| `JOB` (positional) | job id   | required | Job id (`[A-Za-z0-9_-]` only).                           |
| `--wait`           | `SECS`   | —        | How long to keep polling before reporting still-running. |

### Behaviour

Incremental output streaming: `GET /jobs/<job>?out_off=N&err_off=M`. The client persists per-job output offsets in a local offsets file (`<job>.json` in the laptop-agent state dir), sends them on each poll, appends only the new stdout/stderr, and saves the returned `out_next`/`err_next`. It polls every ≤3 s until the job's state is `done` — then prints `[laptop-agent] job <job> done exit=<e> duration_ms=<d>` on stderr and returns the job's exit code — or until the wait deadline passes, printing `still running` and returning **0** (still-running is not a failure). A reply without offsets is an error, exit 1.

### Exit codes

- The job's exit code, once the job is done.
- `0` — still running at the wait deadline.
- `1` — transport failure, or a reply without offsets.
- `2` — usage.

### Example

```console
$ fleet laptop-agent poll job_… --wait 600
```

Streams new output every ≤3 s for up to 600 s, then returns the job's exit code (or 0 with "still running").

## fleet laptop-agent jobs

```
fleet laptop-agent jobs
```

| Flag | Argument | Default | Meaning                                                                        |
| ---- | -------- | ------- | ------------------------------------------------------------------------------ |
| —    | —        | —       | Takes no arguments. Any trailing argument is an unknown-command error, exit 2. |

### Behaviour

Issues `GET /jobs` and prints the job list pretty-printed.

### Exit codes

- `0` — ok.
- `1` — transport failure.
- `2` — usage.

### Example

```console
$ fleet laptop-agent jobs
```

Prints the daemon's job list as pretty JSON.

## fleet laptop-agent kill

```
fleet laptop-agent kill JOB
```

| Flag               | Argument | Default  | Meaning                                    |
| ------------------ | -------- | -------- | ------------------------------------------ |
| `JOB` (positional) | job id   | required | Exactly one job id (`[A-Za-z0-9_-]` only). |

### Behaviour

Issues `POST /jobs/<job>/kill` with `{}` and prints the reply.

### Exit codes

- `0` — ok.
- `1` — transport failure.
- `2` — usage.

### Example

```console
$ fleet laptop-agent kill job_…
```

Kills the named job and prints the daemon's reply.

## fleet laptop-agent fetch

```
fleet laptop-agent fetch REMOTE_PATH LOCAL_PATH
```

| Flag                       | Argument | Default  | Meaning                                                  |
| -------------------------- | -------- | -------- | -------------------------------------------------------- |
| `REMOTE_PATH` (positional) | path     | required | File path on the laptop.                                 |
| `LOCAL_PATH` (positional)  | path     | required | Destination path on this host. This verb takes no flags. |

### Behaviour

Hash-checked copy. First it hashes the remote file _on the laptop_ by running `shasum -a 256` through `/exec`, then pulls the file in **1 MiB** raw ranges via `GET /file?path=…&off=…&len=1048576` until an empty chunk, verifies the assembled bytes' SHA-256 against the remote hash, and only then writes the destination — a mismatch writes nothing and exits 1.

### Exit codes

- `0` — ok, hash verified and destination written.
- `1` — transport failure, or a SHA-256 mismatch (nothing written).
- `2` — usage.

### Example

```console
$ fleet laptop-agent fetch /home/user/build/result.tar ./result.tar
```

Copies the file in 1 MiB ranges and writes it locally only after its SHA-256 matches the remote hash.
