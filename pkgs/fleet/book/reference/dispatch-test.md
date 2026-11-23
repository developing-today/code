# fleet dispatch-test

`fleet dispatch-test` is an explicit-dispatch CI test harness: scan fleet repos for free Actions capacity, submit a job, and track its stored status. Nothing here is trigger-driven: every verb runs only when a caller invokes it, and `submit` dispatches a `workflow_dispatch` run explicitly.

See also [fleet dispatch](dispatch.md) and [Dispatch and dispatch-test](../concepts/dispatch.md).

## Shared machinery

- **Execution homes:** ONE repo — the fleet repo, config `[dispatch-test] fleet_repo`, default **`dezren39/fleet`** — holds the other fleet repos as git submodules and carries the generic workflow `fleet-dispatch-test.yml`. Eligible homes are its *copies*: the source repo plus its fork network, discovered per invocation by breadth-first traversal of `GET /repos/{r}/forks`, **depth-capped at 8 levels**, deduplicated, source first, then copies in API order — unless `[dispatch-test] scopes` pins an explicit list (pin wins; scan output reports `scopes_source: "config"` vs `"fork-network"`).
- **Capacity counting:** for each scope, ACTIVE = (jobs with status `in_progress` or `queued` across all `in_progress` runs — fetched paginated, page size 100, max 100 pages) + (number of `queued` runs, from `total_count`, counted as **1 active unit each** — a queued run has not started and exposes no jobs, so it counts once regardless of its eventual fan-out).
- **Workflow presence:** the repo's workflows list (paginated) contains a workflow whose `path` ends with `fleet-dispatch-test.yml` and whose `state` is `active` (a missing state counts as active).
- **Eligible** = `workflow_present && active < threshold`.
- **Threshold:** `--threshold N`, else `[dispatch-test] threshold`, else **40**. `--threshold 0` is a usage error ("can never be satisfied").
- **Pagination contract:** paged reads are complete, bounded, or say so; truncation is never silent.
- **Auth, by name through fleet-secrets only:** GitHub token name `[dispatch-test] token_name`, default `github.ghcr-classic-pat`; Turso token name `[dispatch-test] turso_token_name`, default `turso.fleet-dispatch-jobs-token`. Each value enters exactly one `Authorization` header per request. GitHub calls use `User-Agent: fleet-dispatch-test` against `https://api.github.com`.
- **Turso jobs store:** endpoint `[dispatch-test] turso_url`, default `https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io`; requests are `POST {url}/v2/pipeline`. Table `jobs`, columns: `id, created_at, name, target_repo, target_ref, script, status, run_id, run_url, exit_code, output_tail, updated_at, submodule, commands`. The phase-2 migration (`ALTER TABLE jobs ADD COLUMN submodule TEXT`, `… ADD COLUMN commands TEXT`) is applied idempotently by submit and status; a "duplicate column" error means already-migrated and is ignored.

Known discrepancy: the help text understates three source behaviours — scan's human output has a leading `scopes source: …` line the help omits; `--threshold 0` is a usage error the help does not mention; and submit's forced `--repo` also skips the eligibility checks (below).

## fleet dispatch-test scan

```
fleet dispatch-test scan [--threshold N] [--json]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--threshold` | `N` | config, else `40` | Eligibility threshold: a scope is eligible when `active < threshold` and the workflow is present. `0` is a usage error. |
| `--json` | — | off | Print the facts as JSON instead of lines. |

### Behaviour

Scans every scope in order. Human output: a first line `scopes source: {config|fork-network} (fleet repo {repo})`, then one line per scope `<repo> active=<n> workflow=<yes|no> eligible=<yes|no>`, then `chosen: <repo|none>` (the first eligible scope). `--json` prints:

```json
{"threshold": 40, "fleet_repo": "dezren39/fleet", "scopes_source": "fork-network",
 "scopes": [{"repo": "dezren39/fleet", "active": 3, "workflow": true, "eligible": true}],
 "chosen": "dezren39/fleet"}
```

### Exit codes

- `0` — ok (including `chosen: none`).
- `2` — usage (including `--threshold 0`).
- `3` — transport down / credentials unresolved / no eligible scope class failures on the scan path.
- `4` — an HTTP error status from GitHub or Turso.

### Example

```console
$ fleet dispatch-test scan
scopes source: fork-network (fleet repo dezren39/fleet)
dezren39/fleet active=3 workflow=yes eligible=yes
chosen: dezren39/fleet
```

## fleet dispatch-test submit

```
fleet dispatch-test submit --submodule NAME [--threshold N] [--repo owner/name] [--ref REF] [--name TEXT] [--commands TEXT | --commands-file PATH]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--submodule` | `NAME` | required | Fleet-repo submodule to run in — a single path segment: alphanumerics, `-`, `_`, `.`; never empty, `.` or `..`, never a slash. Examples: `code, opencode, openchamber, nix, hatch`. |
| `--threshold` | `N` | config, else `40` | Capacity threshold for the scan rule. `0` is a usage error. |
| `--repo` | `owner/name` | — | Force this execution-home copy. Forcing **bypasses the eligibility rule**: the code scans the forced repo only to report `active_at_scan`; it does not check `active < threshold` or workflow presence. |
| `--ref` | `REF` | empty | Ref **inside the submodule repo**; empty = the submodule stays at the umbrella's recorded SHA. Stored in the row's `target_ref`. |
| `--name` | `TEXT` | — | Free-text job name recorded on the row. |
| `--commands` | `TEXT` | trivial smoke set | Shell commands run inside the submodule. Mutually exclusive with `--commands-file`. Default: `set -eux; echo dispatch-test smoke; git rev-parse HEAD; uname -a`. |
| `--commands-file` | `PATH` | — | Read the commands from a file. Mutually exclusive with `--commands`. |

### Behaviour

The job id is generated client-side (time-based base36 + random hex suffix). The sequence order is load-bearing:

1. Connect to GitHub and to the Turso jobs store; run the idempotent schema migration.
2. INSERT the job row with `status='queued'`, `target_repo=NULL`, `target_ref=<ref>`, `script=<commands>`, `submodule=<name>`, `commands=<commands>`, `run_id/run_url/exit_code/output_tail=NULL`, `created_at`/`updated_at` = now (RFC 3339 UTC). (`script` mirrors `commands` so MVP-era readers still see the payload.)
3. Pick the home: forced `--repo`, else the scan rule over discovered/pinned scopes. No eligible scope is a transport-class error (exit 3): "no eligible scope: no execution-home copy has active < {threshold} with the generic workflow present (job {id} remains queued in the store)".
4. Dispatch sequence: (a) `UPDATE jobs SET target_repo=?` — (b) dispatch `POST /repos/{repo}/actions/workflows/fleet-dispatch-test.yml/dispatches` with `{"ref": <that repo's default_branch>, "inputs": {"job_id": id}}` — **the job id is the workflow's only input** — (c) only then `UPDATE jobs SET status='dispatched'`. Any failure stops the sequence: a failed dispatch leaves the row `queued` (with its target repo recorded).
5. Prints JSON: `{job_id, repo, submodule, ref (null when empty), active_at_scan, threshold, status: "dispatched"}`.

Row lifecycle: `queued` (insert) → `dispatched` (client, after the workflow POST) → `running` (the workflow itself writes this, with `run_id`/`run_url`, when it fetches the job) → terminal status + `exit_code`/`output_tail` written back by the workflow, always, even on failure (a cancelled run is the best-effort exception — its row stays `running`, which `status` reports honestly).

### Exit codes

- `0` — ok, row dispatched.
- `2` — usage (bad submodule, `--commands` with `--commands-file`, `--threshold 0`).
- `3` — transport down / credentials unresolved / no eligible scope.
- `4` — an HTTP error status from GitHub or Turso.

### Example

```console
$ fleet dispatch-test submit --submodule code --name "smoke the code submodule"
{"job_id": "…", "repo": "dezren39/fleet", "submodule": "code", "ref": null, "active_at_scan": 3, "threshold": 40, "status": "dispatched"}
```

## fleet dispatch-test status

```
fleet dispatch-test status <job-id>
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<job-id>` (positional) | job id | required | Exactly one positional; anything else is a usage error. This verb takes no flags. |

### Behaviour

Prints the stored row as pretty JSON with **all columns** (`id, created_at, name, target_repo, target_ref, submodule, commands, script, status, run_id, run_url, exit_code, output_tail, updated_at`), values typed from the Turso cells. This reads the store only — it never queries GitHub for live run state.

### Exit codes

- `0` — ok.
- `2` — usage (not exactly one positional).
- `3` — transport down / credentials unresolved.
- `4` — an HTTP error status, including an **unknown job id**: "job {id} not found in the jobs store" exits 4.

### Example

```console
$ fleet dispatch-test status <job-id>
{
  "id": "…",
  "status": "dispatched",
  "submodule": "code",
  "target_repo": "dezren39/fleet"
}
```
