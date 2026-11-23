# fleet dispatch

`fleet dispatch` is a thin client over pm-dispatch, the pm-stack dispatch service. The service is a working TypeScript/Bun suite that is not rewritten; this domain is verbs over its existing HTTP routes — and those routes are exactly three. Per-task status is **not** here: a task's state is the repo queue itself, so per-task status reads ride the queue domain (`fleet queue list`).

| Verb      | Route               | Auth            |
| --------- | ------------------- | --------------- |
| `status`  | `GET /health`       | unauthenticated |
| `launch`  | `POST /v1/sessions` | Bearer          |
| `enqueue` | `POST /v1/tasks`    | Bearer          |

See also [fleet dispatch-test](dispatch-test.md) and [Dispatch and dispatch-test](../concepts/dispatch.md).

## Connection ladder and auth (all verbs)

Transport selection, in order:

1. **Direct URL:** env `FLEET_DISPATCH_URL` or fleet.toml `[dispatch] url`. It must be `http://host[:port]` with **no path** — a path is a usage error; a missing port gets the configured port appended.
2. **Loopback probe:** probe `127.0.0.1:{port}` with a 750 ms connect timeout; if it answers, go direct to loopback. Port: `[dispatch] port`, default **4879**.
3. If config `proxy_direct()` is set: direct to `{target_host}:{port}`.
4. **Proxy-egress CONNECT:** a CONNECT tunnel through the fleet proxy egress to the target host's dispatch port (the sandbox route). The client opens its own tunnel on the egress because the framed session-broker shape forwards only an `X-Agent-Token` header, which pm-dispatch does not accept.

Auth is `Authorization: Bearer <token>`, the token resolved **by name** through fleet-secrets: default name `pm-dispatch.api-token`, overridable with `[dispatch] token_name`. The value enters exactly one header per request and is never printed, logged, or stored. HTTP 401 is reported as a transport-class error naming the mismatch with the service's own token env (name only).

Success bodies print to stdout as pretty JSON exactly as the service sent them; errors go to stderr, prefixed `fleet dispatch: `. For HTTP error bodies the `error` string is used, and if the body has more fields the whole body is appended in brackets (a failed launch's 502 names its cancelled `task_id`); messages are truncated at 600 chars.

The argument parser is shared by `launch` and `enqueue`: `--flag value` and `--flag=value` are both accepted; duplicate value flags, unknown options, values on boolean flags, and stray positionals are all usage errors.

## fleet dispatch status

```
fleet dispatch status
```

| Flag | Argument | Default | Meaning                                                                                                                    |
| ---- | -------- | ------- | -------------------------------------------------------------------------------------------------------------------------- |
| —    | —        | —       | Takes no arguments. Any argument is a usage error. `--help`/`-h` prints usage and succeeds without contacting the service. |

### Behaviour

Issues `GET /health` on pm-dispatch: is the service up, and which build. Prints the service's health body as a JSON object **plus** an added `"transport"` field describing the route taken, for example `"direct http://127.0.0.1:4879"` or `"tunnel via proxy egress to {host}:4879"`. A non-object health body is wrapped as `{"health": <body>, "transport": …}`.

### Exit codes

- `0` — ok.
- `2` — usage.
- `3` — transport down / credentials unresolved or rejected.
- `4` — the service answered with an HTTP error status.

### Example

```console
$ fleet dispatch status
{"ok": true, "transport": "direct http://127.0.0.1:4879"}
```

## fleet dispatch launch

```
fleet dispatch launch --repo <owner/name> --prompt <text> [options]
```

| Flag                     | Argument        | Default                                  | Meaning                                                                                                                                                                  |
| ------------------------ | --------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--repo`                 | `<owner/name>`  | required                                 | An allowlisted fleet repo (allowlist checked service-side).                                                                                                              |
| `--prompt`               | `<text>`        | required (or `--prompt-file`)            | The session prompt.                                                                                                                                                      |
| `--prompt-file`          | `<path>`        | —                                        | Read the prompt from a file. Together with `--prompt` is a usage error.                                                                                                  |
| `--title`                | `<text>`        | service default: the prompt's first line | Tracking-issue title.                                                                                                                                                    |
| `--model`                | `<provider/id>` | —                                        | Model hint for the session.                                                                                                                                              |
| `--directory`            | `<path>`        | —                                        | Must be the repo's laptop primary or inside it.                                                                                                                          |
| `--priority`             | `<p>`           | —                                        | Integer 10–90, or `critical\|high\|normal\|low`. Sent as a JSON number when it parses as an integer, otherwise as the raw string; range validation is the service's job. |
| `--issue` / `--no-issue` | —               | service default: issue                   | Create the tracking issue. Both together, in either order, is a usage error. Sent only when given.                                                                       |
| `--wait` / `--no-wait`   | —               | service default: wait                    | Wait for the launch to complete. Both together is a usage error. Sent only when given.                                                                                   |

### Behaviour

POSTs `/v1/sessions` — creates the tracking issue and queue task, starts the session, and assigns the task to it (all service-side). The client sends **only the fields the caller set**: `repo` and `prompt` always; `title`, `model`, `directory`, `priority` when given; `issue: bool` only when `--issue`/`--no-issue` was given; `wait: bool` only when `--wait`/`--no-wait` was given. The defaults named in the help (title = first prompt line, issue on, wait on) are the **service's** defaults, applied because the client omits the fields.

### Exit codes

- `0` — ok.
- `2` — usage.
- `3` — transport down / credentials unresolved or rejected.
- `4` — the service answered with an HTTP error status.

### Example

```console
$ fleet dispatch launch --repo developing-today/code --prompt "Fix the failing smoke test" --no-wait
```

Creates the tracking issue and queue task, starts a session on the laptop, and returns without waiting for the launch to complete.

## fleet dispatch enqueue

```
fleet dispatch enqueue --repo <owner/name> --title <text> --prompt <text> [options]
```

| Flag             | Argument        | Default                                                    | Meaning                                                                                                                                                    |
| ---------------- | --------------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--repo`         | `<owner/name>`  | required                                                   | An allowlisted fleet repo (allowlist checked service-side).                                                                                                |
| `--title`        | `<text>`        | required                                                   | Task title.                                                                                                                                                |
| `--prompt`       | `<text>`        | required (or `--prompt-file`)                              | The task prompt.                                                                                                                                           |
| `--prompt-file`  | `<path>`        | —                                                          | Read the prompt from a file. Together with `--prompt` is a usage error.                                                                                    |
| `--kind`         | `<kind>`        | service default: `triage`                                  | One of `triage\|small_fix\|pr_shepherd\|ci_investigation`. Validated client-side; an unknown kind is a usage error listing the four. Sent only when given. |
| `--priority`     | `<p>`           | —                                                          | Integer 10–90, or `critical\|high\|normal\|low` (same wire rule as launch).                                                                                |
| `--model`        | `<provider/id>` | —                                                          | Stored as the payload's model hint only.                                                                                                                   |
| `--issue`        | —               | service default: no issue                                  | Create a tracking issue first. One-way boolean — there is no `--no-issue`; omitting it sends nothing. The opposite default from launch.                    |
| `--issue-number` | `<n>`           | —                                                          | Attach to this existing issue instead. Must parse as an integer; sent as `issue_number`.                                                                   |
| `--dedupe-key`   | `<key>`         | service default: canonical key from the issue, else unique | Sent as `dedupe_key`.                                                                                                                                      |

### Behaviour

POSTs `/v1/tasks`: enqueue only, no session. Wire names are snake_case (`issue_number`, `dedupe_key`); launch's are single words.

### Exit codes

- `0` — ok.
- `2` — usage (including an unknown `--kind`).
- `3` — transport down / credentials unresolved or rejected.
- `4` — the service answered with an HTTP error status.

### Example

```console
$ fleet dispatch enqueue --repo developing-today/code --title "Triage the red nightly" --prompt "Find why the nightly failed" --kind ci_investigation
```

Enqueues a CI-investigation task without launching a session.
