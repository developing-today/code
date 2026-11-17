# CI run-silence heartbeat (`ci-silence`)

Implements code#10917: a run-silence heartbeat for the four fleet
repos, closing the detection gap behind the 2026-11-11 Actions
blackout on developing-today-forks/openchamber — a repo-level Actions
disable created **no runs at all** for ~2 days, so the failure-based
CI watch had nothing to file on while workflow/permissions API state
read healthy.

## What it does

`check_silence.py` evaluates, per fleet repo, the age of the newest
**completed** Actions run of any kind (`gh run list --status
completed` — run history is the only evidence; workflow state and the
actions/permissions API are never consulted). Past the repo's
threshold (default 6h, per-repo map in the script), the repo is
_silent_ and gets exactly one tracking issue — titled
`CI run silence: <repo> — …`, labelled `kind/bug`, `source/ci-watch`,
`priority/high` — filed in that repo and **updated, not duplicated**,
on later silent passes. Closing the issue re-arms the probe: a later
silence files a fresh one.

## Running it

```bash
# Observe-only (default): print verdicts + planned actions, write nothing
python3 check_silence.py

# File/update tracking issues
python3 check_silence.py --apply

# Offline dry-run against a fixture (used by the tests)
python3 check_silence.py --fixture fixture.json --now 2026-11-17T12:00:00Z
```

Output is one JSON document: per-repo `evaluations` (status, age,
last run) and the planned `actions` (`file` / `update` / `none`).

## Wiring

Run it on the CI watch's normal cadence as a sibling probe to the
failure watch (the main-ci watch pass adds one step:
`check_silence.py --apply` after its failure detection; its exit is
always 0 on a completed evaluation — silence is a finding in the
output, not a probe failure). Thresholds live in
`REPO_THRESHOLDS_HOURS`; widen a repo's entry there when its normal
quiet gap grows, rather than ignoring its alerts.

## Tests

```bash
python3 -m unittest test_check_silence -v
```

Offline, fixture-driven: stale timestamp fires exactly one tracking
issue, fresh stays quiet, an existing tracking issue is updated not
duplicated, no-runs fires, threshold boundary, newest-run selection,
and per-repo independence.
