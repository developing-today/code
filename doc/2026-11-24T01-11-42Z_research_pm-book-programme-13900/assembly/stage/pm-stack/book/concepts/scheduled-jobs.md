# Scheduled jobs and the history they keep

## Two components, two jobs

This chapter covers two components with similar names and different jobs:

- **opencode-scheduled-jobs** (`pm-stack/opencode-scheduled-jobs/`) runs work on a schedule: an OpenCode session fired by a systemd timer.
- **scheduled-history** (`pm-stack/scheduled-history/`) keeps the record: a reconciler that collects what OpenChamber's scheduled tasks actually ran, and freezes the result.

The first component produces runs. The second component makes runs auditable after the fact.

## The scheduled job: one pilot

opencode-scheduled-jobs ships exactly one pilot job. `jobs.json` defines it; its name is `quota-freshness-summary`. A systemd timer fires an OpenCode session that summarises quota freshness.

The installer is `install-laptop.sh`. It enables the timer only when run with `--enable`. The component README states the pilot's current state in its own words: "this pilot is currently enabled."

Outputs land in a fixed state directory: `~/.local/state/opencode-scheduled/quota-freshness-summary/`, holding `latest.log` (the newest run's output) and `history.jsonl` (one line per run, appended). The history file is the job's own memory: latest state for a reader who wants the current answer, and an append-only line history for a reader who wants the sequence.

The component's last commit in the surveyed clone is 2026-11-22.

## The history collector

scheduled-history is a polling reconciler for OpenChamber scheduled-task runs (#2174). Its collector, `collect.py`, works in four steps:

1. **List.** It reads scheduled runs from `openchamber schedule list --json`.
2. **Key.** Each run is keyed by the pair (task id, `lastScheduledFor`). The key is the identity of a scheduled occurrence: the same task scheduled again is a new run, and the same occurrence polled again is the same run.
3. **Enrich.** Token and cost figures for the run are read from the OpenCode session store and attached to the run's row.
4. **Freeze.** Terminal rows are frozen. A finished run's record stops changing; later polls do not rewrite it.

A watchdog (`watchdog.py`) accompanies the collector. The component ships service and timer units, and tests. Its last commit in the surveyed clone is 2026-11-17.

## The shared session store

Both halves of this chapter meet in the OpenCode session store. scheduled-history enriches its rows from that store. A third component reads the same store for a different surface: opencode-usage-export (`pm-stack/opencode-usage-export/`) runs `opencode-usage-export.mjs` under a service and timer and exports OpenCode usage from the session store (`opencode.db`, the #2072 exporter) for the quota and usage surfaces. The store is therefore a record three readers depend on: the history collector for per-run cost, the usage exporter for quota surfaces, and the sessions registry on the dashboard. Those surfaces are described in [Health, quota and usage surfaces](health-surfaces.md).

## What the history guarantees, and what it does not

The guarantees come from the keying and freezing rules:

- A scheduled occurrence has exactly one identity, (task id, `lastScheduledFor`), across polls.
- A terminal row, once frozen, is the final record for that occurrence.
- Cost and token figures on a row come from the session store, not from the scheduler's own report.

One limit is stated plainly: the survey establishes the collector's design from its code and units, and does not probe a live run. Whether the collector's timer is firing on the laptop today is UNVERIFIED in the survey this chapter rests on. The same limit applies to the pilot job's recent output: the README states the pilot is enabled, and this chapter reports that statement as a repo record, not as a probed live state.

## Neighbours

Scheduled jobs in this stack are deliberately small: one pilot job, one history collector. The stack's recurring timers as a class — sync guards, sweeper, health collectors, exporters — are inventoried with their surfaces in [Health, quota and usage surfaces](health-surfaces.md) and in [The Kaneo sync and the other PM apps](kaneo-sync.md). The claimant's polling cycle is not a scheduled job in this chapter's sense; it is an execution lane, treated in [The claimant: keeping sessions honest](claimant.md).

Sources. Survey `apps.md` §§20, 21, 24; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
