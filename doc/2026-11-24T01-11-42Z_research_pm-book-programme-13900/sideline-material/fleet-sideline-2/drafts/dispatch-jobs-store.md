# The dispatch-test jobs store: fleet-dispatch-jobs

<!-- id: fleet-dispatch-jobs since: pre-release -->

`fleet-dispatch-jobs` is a Turso (libSQL) database whose single table,
`jobs`, is the stored record of the explicit-dispatch CI test jobs
submitted by `fleet dispatch-test`. The operational story — how
`submit`, `scan` and `status` behave — lives in
[Dispatch and dispatch-test](dispatch.md) and the
[fleet dispatch-test](../reference/dispatch-test.md) reference. This
chapter treats the database itself as the subject: its connection by
name, its schema and provenance, its two writers, and its place in
the estate.

Three different things share the word "dispatch", and only one of
them touches this database:

| Name | What it is | Touches `fleet-dispatch-jobs`? |
| --- | --- | --- |
| The **dispatch** domain | A thin client over the pm-dispatch service | No |
| **pm-dispatch** | The laptop service (`pkgs/pm-dispatch`, `pm-stack/dispatch`) | No |
| **dispatch-test** | The `fleet dispatch-test` domain (design #6580) | Yes — its sole owner in this repo |

**Status (2026-11-23): live, small and current.** The #13844 Turso
audit read the store on 2026-11-23 and counted `jobs`, 6 rows. The
owner crate is in the fleet binary's workspace, and its phase-2
migration runs idempotently on every submit and status, so the schema
below is the schema the code maintains today.

## Connection by name

The connection names are verified in
`pkgs/fleet/crates/fleet-config/src/lib.rs`:

- The default URL constant `DEFAULT_DISPATCH_TEST_TURSO_URL` is
  `https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io` —
  config key `[dispatch-test] turso_url`. The database's name is the
  URL's subdomain.
- The Turso token is resolved **by name only** through fleet-secrets:
  the default name is `turso.fleet-dispatch-jobs-token` — config key
  `[dispatch-test] turso_token_name`. The GitHub token for the same
  domain defaults to the name `github.ghcr-classic-pat`.
- Token values enter exactly one `Authorization` header per request
  and are never printed, logged or stored. Requests are
  `POST {url}/v2/pipeline` — hrana over HTTP v2, one execute and one
  close per call.

## The jobs table

The table has 14 columns: `id`, `created_at`, `name`, `target_repo`,
`target_ref`, `script`, `status`, `run_id`, `run_url`, `exit_code`,
`output_tail`, `updated_at`, `submodule` and `commands`.

Phase 2 (#6595) gave two of those columns their current meaning:
`target_ref` carries the *submodule* ref (empty means the umbrella's
recorded SHA), and `script` mirrors `commands` so MVP-era readers
still see the payload. Job ids are generated client-side in the form
`dt-<unix_millis base36>-<4 hex chars>`.

### Schema provenance: the migration that is not a creation

No `CREATE TABLE` for `jobs` exists anywhere in the searched repo —
not in `pkgs/fleet`, `pkgs/pm-dispatch`, `pm-stack/dispatch`, `home/`
or `pkgs/fleet/migrations/` (those migrations belong to the
`fleet-queue` database, not this one). The table predates the crate:
it is an MVP-era artifact created outside the repo's recorded
history. The only DDL the code runs is the idempotent phase-2
migration in `ensure_schema()` — `ALTER TABLE jobs ADD COLUMN
submodule TEXT` and `ALTER TABLE jobs ADD COLUMN commands TEXT` —
applied by `submit` and `status`, where a "duplicate column" error
means already-migrated and is ignored. The schema above is the schema
the code maintains; no repo migration creates the table, and this
chapter does not imply one (disagreement D34).

## The write path

The store has exactly two writers.

**The client crate** (`pkgs/fleet/crates/fleet-dispatch-test/src/lib.rs`,
verbs `scan` / `submit` / `status`) writes in the load-bearing #6598
order:

1. `INSERT` the row with `status='queued'`, `target_repo=NULL` and
   `run_id`/`run_url`/`exit_code`/`output_tail` all NULL.
2. `UPDATE` the row's `target_repo` to the execution-home copy of the
   fleet repo (`dezren39/fleet` or a fork) picked for its free GitHub
   Actions capacity.
3. Dispatch the generic workflow `fleet-dispatch-test.yml` in that
   copy, with the job id as its only input.
4. Only then, `UPDATE` the row to `status='dispatched'`.

The order is the failure semantics: a failed dispatch leaves the row
`queued` with its target repo recorded, and "no eligible scope" is a
transport error (exit 3) with the row left queued — never a dispatched
row pointing at nothing.

**The generic workflow** — `fleet-dispatch-test.yml` in the fleet
repos (`dezren39/fleet` and its copies, not in this repo) — is the
second writer. Per the dispatch chapters' description, it writes
`status='running'` with `run_id` and `run_url` when it fetches the
job, and the terminal status with `exit_code` and `output_tail` at
the end — always, even on failure. A cancelled run is best-effort and
may leave the row `running`. No code for that write-back exists in
developing-today/code to verify it against, and this chapter says so
rather than presenting the workflow's behaviour as surveyed here.

## Reads

`fleet dispatch-test status <id>` is the only in-repo reader: a
`SELECT` of all 14 columns `WHERE id = ?`. An unknown id is HTTP 404,
exit 4. Status reads the store only, never GitHub — it reports the
stored row, not live workflow state, which is why a cancelled run's
row can honestly read `running`.

`scan` never opens the store at all. Capacity counting is GitHub-only:
active means jobs `in_progress` or `queued` across `in_progress` runs,
paginated at 100 per page to a 100-page bound, plus queued runs
counted as one active unit each from `total_count`; a scope is
eligible when the workflow is present and its active count is under
the threshold (default 40).

## Failure shapes

The domain's exit codes are 0 ok, 2 usage, 3 transport (credentials
unresolved, store unreachable, no eligible scope) and 4 for an HTTP
status from GitHub or Turso, including the unknown-job-id 404. The
Turso-specific shapes: a non-2xx response is exit 4 with the first
300 body characters; a top-level `{"error": …}` or a per-request
`results[i].type != "ok"` is a transport error, never an empty
result; a non-JSON body is a transport error. `--threshold 0` is a
usage error, and `[dispatch-test] threshold = 0` is a config
validation error (#6603).

## Estate position

The #13844 Turso-to-D1 audit grouped this database under §G8 as
MOVE-CLEAN, alongside small product stores. The facts behind that
grouping — one table, 6 rows, tiny, join-free — are not in dispute;
the classification is. The database's sole owner is fleet-plane CI
infrastructure, configured from the fleet binary's own
`[dispatch-test]` section, not a product surface, and this chapter
does not inherit the product-store framing (disagreement D33).
MOVE-CLEAN is a recommendation only: nothing in the code moves the
store, and it stays Turso-shaped — hrana pipeline, typed cells, the
pipeline failure shapes above — until a programme decides otherwise.

Earlier coverage note: the dispatch chapters already named this
store's endpoint, token name, table and row lifecycle before this
chapter existed; what was missing was the database treated as a
subject in its own right (disagreement D30).

## Sources

Owner crate `pkgs/fleet/crates/fleet-dispatch-test/src/lib.rs`;
config `pkgs/fleet/crates/fleet-config/src/lib.rs`
(`DispatchTestConfig`); design #6580; issues #6589, #6595, #6598,
#6603, #6605, #6622, #6623, #13844. Survey: programme #13900, lane
FLEET-GAPS-SURVEY, 2026-11-23 (disagreements D30, D33, D34).
