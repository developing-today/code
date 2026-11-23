# Dispatch and dispatch-test

Two different domains share the word "dispatch", and a third verb besides.
This chapter keeps them apart:

- **`fleet dispatch`** — a thin client over pm-dispatch, an existing
  TypeScript/Bun service that is _not_ rewritten in fleet. Its verbs are
  `status`, `launch`, and `enqueue`.
- **`fleet dispatch-test`** — an explicit-dispatch CI test: scan fleet repos
  for free Actions capacity, submit a job, track its stored status. Nothing
  here is trigger-driven; every verb runs only when a caller invokes it.
- Not this chapter: `fleet queue dispatch`, which is placement — see
  [Placement and remote dispatch](placement.md).

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-dispatch/src/lib.rs and https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-dispatch-test/src/lib.rs

## `fleet dispatch`: the pm-dispatch client

Per-task status is deliberately **not** here: a task's state is the repo
queue itself, so per-task status reads ride the queue domain
(`fleet queue list`). The client's routes are exactly three:

| Verb                       | Route               | Auth            |
| -------------------------- | ------------------- | --------------- |
| `fleet dispatch status`    | `GET /health`       | unauthenticated |
| `fleet dispatch launch …`  | `POST /v1/sessions` | Bearer          |
| `fleet dispatch enqueue …` | `POST /v1/tasks`    | Bearer          |

**The connection ladder** (transport selection, in order):

1. **Direct URL**: env `FLEET_DISPATCH_URL` or fleet.toml `[dispatch] url`
   (must be `http://host[:port]` with **no path** — a path is a usage error;
   a missing port gets the configured port appended).
2. **Laptop zero-config**: probe `127.0.0.1:{port}` with a 750 ms connect
   timeout; if it answers, direct to loopback. The port is `[dispatch]
port`, default **4879**.
3. If config `proxy_direct()` is set: direct to `{target_host}:{port}`.
4. **Tunnel**: a CONNECT through the fleet proxy egress to the target host's
   dispatch port (the sandbox route). The client opens its own tunnel on
   the egress because the framed session-broker shape forwards only an
   `X-Agent-Token` header, which pm-dispatch does not accept.

Auth is `Authorization: Bearer <token>`, the token resolved **by name**
through fleet-secrets: default name `pm-dispatch.api-token`, overridable
with `[dispatch] token_name`. The value enters exactly one header per
request and is never printed, logged, or stored. An HTTP 401 is reported as
a transport-class error naming the mismatch with the service's own token
env (name only).

Exit codes: **0** ok, **2** usage, **3** transport down / credentials
unresolved or rejected, **4** the service answered with an HTTP error
status. Success bodies print to stdout as pretty JSON exactly as the
service sent them; errors go to stderr prefixed `fleet dispatch: `. The
shared argument parser accepts `--flag value` and `--flag=value`; duplicate
value flags, unknown options, values on boolean flags, and stray
positionals are all usage errors.

### `status`

No arguments (any argument is a usage error; `--help`/`-h` prints usage and
succeeds without contacting the service). It prints the service's `/health`
body as a JSON object **plus** an added `"transport"` field describing the
route taken, for example `direct http://127.0.0.1:4879` or
`tunnel via proxy egress to {host}:4879`. A non-object health body is
wrapped as `{"health": <body>, "transport": …}`.

### `launch`

```
fleet dispatch launch --repo <owner/name> --prompt <text> [options]
  --prompt-file <path>    read the prompt from a file
  --title <text>          default: the prompt's first line
  --model <provider/id>   model hint for the session
  --directory <path>      must be the repo's laptop primary or inside it
  --priority <p>          integer 10-90, or critical|high|normal|low
  --issue / --no-issue    create the tracking issue (default: issue)
  --wait / --no-wait      wait for the launch to complete (default: wait)
```

`--repo` must be an allowlisted fleet repo (the allowlist check is
service-side). The POST to `/v1/sessions` creates the tracking issue and
queue task, starts the session, and assigns the task to it — all
service-side. The client sends **only the fields the caller set**: `repo`
and `prompt` always; `title`, `model`, `directory`, `priority` when given;
`issue` only when `--issue`/`--no-issue` was given; `wait` only when
`--wait`/`--no-wait` was given. The defaults named in the help (title =
first prompt line, issue on, wait on) are the **service's** defaults,
applied because the client omits the fields. `--prompt` + `--prompt-file`
together is a usage error (never a silent preference), as are
`--issue --no-issue` and `--wait --no-wait` in either order. `--priority`
crosses the wire as a JSON number when it parses as an integer, otherwise
as the raw string — validating the 10–90 range and the named scale is the
service's job, not the thin client's.

### `enqueue`

```
fleet dispatch enqueue --repo <owner/name> --title <text> --prompt <text> [options]
  --prompt-file <path>    read the prompt from a file
  --kind <kind>           triage|small_fix|pr_shepherd|ci_investigation (default: triage)
  --priority <p>          integer 10-90, or critical|high|normal|low
  --model <provider/id>   stored as the payload's model hint only
  --issue                 create a tracking issue first (default: no issue)
  --issue-number <n>      attach to this existing issue instead
  --dedupe-key <key>      default: canonical key from the issue, else unique
```

Enqueue only, no session (POST `/v1/tasks`). Differences from launch:
`--title` is required; `--kind` **is** validated client-side against the
four kinds — an unknown kind is a usage error listing the four — but is
only sent when given (the `triage` default is service-side); `--issue` is
a one-way boolean with no `--no-issue`, and the service default is _no_
issue — the opposite default from launch; `--issue-number` must parse as
an integer (sent as `issue_number`); `--dedupe-key` is sent as
`dedupe_key`; `--model` goes in the task payload as a hint only.

## `fleet dispatch-test`: scan, submit, status

Verbs: `scan`, `submit`, `status`. Exit codes: **0** ok, **2** usage,
**3** transport down / credentials unresolved / **no eligible scope**,
**4** an HTTP error status from GitHub or Turso (including an unknown job
id on `status`).

**Execution homes.** One repo — the fleet repo, config `[dispatch-test]
fleet_repo`, default **`dezren39/fleet`** — holds the other fleet repos as
git submodules and carries the generic workflow `fleet-dispatch-test.yml`.
Eligible homes are its _copies_: the source repo plus its fork network,
discovered per invocation by breadth-first traversal of
`GET /repos/{r}/forks`, **depth-capped at 8 levels**, deduplicated, source
first, then copies in API order — unless `[dispatch-test] scopes` pins an
explicit list (the pin wins; scan output reports `scopes_source: "config"`
vs `"fork-network"`).

**Capacity counting.** For each scope, ACTIVE = (jobs with status
`in_progress` or `queued` across all `in_progress` runs — fetched paginated,
page size 100, max 100 pages) + (the number of `queued` runs, from
`total_count`, counted as **1 active unit each** — a queued run has not
started and exposes no jobs, so it counts once regardless of its eventual
fan-out). **Workflow presence**: the repo's workflows list contains a
workflow whose `path` ends with `fleet-dispatch-test.yml` and whose `state`
is `active` (a missing state counts as active). **Eligible** = workflow
present **and** active < threshold. The threshold is `--threshold N`, else
`[dispatch-test] threshold`, else **40**; `--threshold 0` is a usage error
("can never be satisfied") — a rule the help text does not mention.

**Auth, by name through fleet-secrets only**: the GitHub token name is
`[dispatch-test] token_name`, default `github.ghcr-classic-pat`; the Turso
token name is `[dispatch-test] turso_token_name`, default
`turso.fleet-dispatch-jobs-token`. Each value enters exactly one
`Authorization` header per request.

### `scan`

```
fleet dispatch-test scan [--threshold N] [--json]
```

Scans every scope in order. Human output: a first line
`scopes source: {config|fork-network} (fleet repo {repo})`, then one line
per scope `<repo> active=<n> workflow=<yes|no> eligible=<yes|no>`, then
`chosen: <repo|none>` (the first eligible scope). `--json` prints
`{threshold, fleet_repo, scopes_source, scopes: [{repo, active, workflow,
eligible}], chosen}`.

### `submit`

```
fleet dispatch-test submit --submodule NAME [--threshold N]
                           [--repo owner/name] [--ref REF]
                           [--name TEXT]
                           [--commands TEXT | --commands-file PATH]
```

`--submodule` is required: a single path segment (alphanumerics, `-`, `_`,
`.`; never empty, `.` or `..`, never a slash). Examples: `code`, `opencode`,
`openchamber`, `nix`, `hatch`. `--ref` is a ref _inside the submodule repo_;
default empty = the submodule stays at the umbrella's recorded SHA.
`--commands` / `--commands-file` are mutually exclusive; the default
commands are `set -eux; echo dispatch-test smoke; git rev-parse HEAD;
uname -a`. The job id is generated client-side (a time-based base36 string
plus a random hex suffix).

Forcing `--repo` **bypasses the eligibility rule**: the forced repo is
scanned only to report `active_at_scan`; `active < threshold` and workflow
presence are not checked for it.

The sequence is load-bearing, in this order:

1. Connect to GitHub and to the Turso jobs store; run the idempotent schema
   migration.
2. INSERT the job row with `status='queued'`, `target_repo=NULL`,
   `target_ref`, `script` and `commands` (both carry the commands, so
   MVP-era readers still see the payload), `submodule`, and
   `run_id/run_url/exit_code/output_tail=NULL`.
3. Pick the home: the forced `--repo`, else the scan rule. No eligible
   scope is a transport-class error (exit 3): "no eligible scope … (job
   {id} remains queued in the store)".
4. Dispatch: UPDATE the row's `target_repo`; POST the
   `fleet-dispatch-test.yml` workflow dispatch with `{"ref": <that repo's
default_branch>, "inputs": {"job_id": id}}` — the job id is the
   workflow's **only** input; only then UPDATE the row to
   `status='dispatched'`. Any failure stops the sequence: a failed dispatch
   leaves the row `queued`, with its target repo recorded.
5. Print `{job_id, repo, submodule, ref (null when empty), active_at_scan,
threshold, status: "dispatched"}` as JSON.

### The Turso jobs store

The endpoint is `[dispatch-test] turso_url`, default
`https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io`; requests are
`POST {url}/v2/pipeline` in the hrana-over-HTTP v2 shape. A non-2xx is exit
4 with the first 300 characters of the body; a non-JSON body or a
pipeline-level error object is a failure, never an empty result.

The `jobs` table's full row is: `id, created_at, name, target_repo,
target_ref, script, status, run_id, run_url, exit_code, output_tail,
updated_at, submodule, commands`. The phase-2 migration (adding `submodule`
and `commands`) is applied idempotently by submit and status; a
"duplicate column" error means already-migrated and is ignored.

Row lifecycle: `queued` (the client's insert) → `dispatched` (the client,
after the workflow POST) → `running` (the workflow itself writes this, with
`run_id`/`run_url`, when it fetches the job) → a terminal status with
`exit_code`/`output_tail`, written back by the workflow **always**, even on
failure — with one exception: a cancelled run is best-effort, and its row
may stay `running`, which `status` reports honestly.

The workflow itself (`fleet-dispatch-test.yml` in the fleet repos) is
`workflow_dispatch` only, with the single required input `job_id`. Its one
job runs on `ubuntu-24.04` with a 120-minute timeout in the container
`ghcr.io/developing-today/fleet-build:full`: it reads the job from the
Turso store, marks it `running` with the run id and URL, checks out the
fleet repo, checks out the named submodule at the named ref, runs the
commands inside it, and writes status and output back. Its baked-in Nix
substituters are `https://nix-cache.developing-today.workers.dev` and
`https://developing-today.cachix.org`.

### `status`

```
fleet dispatch-test status <job-id>
```

Exactly one positional (else a usage error). It prints the stored row as
pretty JSON with **all columns**, values typed from the Turso cells. An
unknown id is an HTTP-class error 404, **exit 4**: "job {id} not found in
the jobs store". This reads the store only — it never queries GitHub for
live run state.
