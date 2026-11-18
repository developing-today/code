# scheduled-history — per-run history for OpenChamber scheduled tasks (#2174 Gap 1)

The first evidence gap from issue #2174: OpenChamber's live
scheduled-task records retain only latest-run fields, and no consumer
persisted run events anywhere durable, so no per-run
outcome/token/elapsed dataset existed for the Muse↔OpenChamber
go/no-go comparison. This is that consumer.

## Design — polling reconciler, not an event consumer

Recon on #2174 (task 12585) found the scheduled-task-ran events are
ephemeral, emitter errors are swallowed, and the events that exist
report enqueue/send rather than assistant completion — an SSE-only
collector cannot guarantee a durable terminal record. Instead, each
tick polls the authoritative surface,
`openchamber schedule list --json --dir <project>`, and reconciles:

- Runs are keyed `<task id>@<lastScheduledFor>`. Exactly one durable
  row per run, ever.
- A row's outcome follows the record until the status is terminal
  (`success`, `error`, `failed`, `cancelled`, `timeout`,
  `disabled`, `skipped`, …), then the row is frozen.
- When a newer run of the same task appears, an older non-terminal
  row is finalized as-is with `finalized_by: superseded` — the
  record no longer describes it, so it can never change again.
- `duration_ms` is the scheduler-reported figure, recorded as
  reported; session wall-clock is carried separately
  (`session_created_at`/`session_updated_at`).
- Token/cost enrichment comes from the OpenCode session store
  (`session_v2` in `~/.local/share/opencode/opencode.db` — the
  #2072 source), keyed by the run's `lastSessionId`. When the
  session row is absent, `tokens` stays null: absence is recorded,
  never invented. Enrichment is refreshed on every poll while the
  session row exists — the outcome freezes at terminal, but a
  session can keep accumulating tokens after its run is first seen,
  so usage fields follow the session store rather than freezing at
  first sight.

## Store

`~/.local/state/openchamber-scheduled-history/history.json` — one
JSON document (`{"schema": "scheduled-history/v1", "rows": {...}}`)
written atomically (tmp + rename). Fields per row: run_key, task_id,
task_name, project_dir, enabled, scheduled_for, started_at,
finished_at, duration_ms, session_id, outcome, outcome_terminal,
finalized_by, cost, tokens {input, output, reasoning, cache_read,
cache_write}, session_created_at, session_updated_at, token_source,
first_seen_at, last_seen_at.

## Read path (the comparison / Gap 3 watchdog input)

```
collect.py list [--task <id>] [--since <ISO>] [--until <ISO>] [--json]
```

## Run it

```
python3 collect.py collect --dir ~/git/code [--dir <other project> ...]
python3 test_collect.py   # 13 checks
```

Deployment on the laptop: `scheduled-history.service` +
`scheduled-history.timer` (user units, 5-minute tick, mirroring
opencode-usage-export). Installing this collector changes NO existing
Muse cron job, OpenChamber scheduled task, cadence, or lane count —
it only reads scheduled-task records (#2174 acceptance constraint).

## Known limits

- History begins at installation; runs from before the first tick
  appear only as each task's then-latest run (one row per task at
  backfill).
- `lastStatus` is the scheduler's own outcome for the run; where a
  run's session outlives the scheduler's status transition, the
  session timestamps and token totals in the row are the deeper
  evidence (that layering is deliberate — see #2174 Gap 3).

## Gap 3 — watchdog over the run history

`watchdog.py check` consumes this store and flags, per scheduled
task: `missing_run` (no run scheduled within the expected cadence
window), `stale` (newest run older than the stale threshold,
default 2x the window, with nothing in flight), and
`failed_latest` (most recent terminal outcome is an error class).
The expected window is a declared per-task window (`--windows`
JSON, task id -> seconds) when one exists, otherwise the observed
schedule — the minimum interval between consecutive
`scheduled_for` instants in the task's own history; the source is
stated in every alert's evidence. A task whose history is younger
than one window is in its first window and cannot alert for
missing/stale. Output is one JSON document
(`scheduled-history-watchdog/v1`) on stdout — task, condition, and
evidence timestamps per alert — with exit 1 when any alert fires:
the alert path is the fleet-watchdog cron, which consumes that
document. Known limit: a task whose observed history mixes a burst
of test runs with its real cadence derives a too-tight window from
the burst; declare its window in `--windows` instead.

```
python3 watchdog.py check [--store PATH] [--windows FILE] [--stale-multiplier N]
python3 test_watchdog.py   # 13 checks
```
