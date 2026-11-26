# Conductor-silence heartbeat (`conductor-silence`)

Implements the watchdog criterion of code#15123: the fleet conductor
— stall detection / nudges / session recovery for non-lane sessions —
stopped completing cycles after 2026-11-24T04:25Z, and the silence
went unnoticed for ~33h because nothing watched the conductor. The
fleet-watchdog checks Muse cron jobs; the conductor was a laptop
OpenCode session series with **no schedule definition on any
surface** (verified on #15123: no job in the live scheduler, no
definition file), so no job-based watchdog could see it stop.

## What it does

`check_conductor.py` evaluates the age of the newest cycle timestamp
in the conductor's own cycle log (`conductor-log.md`). Past the
threshold (default 2h — the conductor ran ~hourly, 23 cycles in the
24h before it stopped, so 2h is two missed cycles) the conductor is
_silent_ and gets exactly one tracking issue — titled
`Conductor silence: fleet-conductor — …`, labelled `kind/bug`,
`source/ci-watch`, `priority/high` — filed in
`developing-today/code` and **updated, not duplicated**, on later
silent passes. Closing the issue re-arms the probe: a later silence
files a fresh one.

The probe never consults scheduler state (cron lists, OpenChamber
scheduledTasks, systemd timers) as evidence of health: the verified
cause of the 2026-11-24 silence is that no schedule existed at all,
so scheduler state is exactly the surface that read "nothing wrong".
Its only reads are the conductor log and the tracking-issue search.

## Running it

```bash
# Observe-only (default): print the verdict + planned action, write nothing
python3 check_conductor.py --log-file <path-to-conductor-log.md>

# File/update the tracking issue
python3 check_conductor.py --log-file <path-to-conductor-log.md> --apply

# Offline dry-run against a fixture (used by the tests)
python3 check_conductor.py --fixture fixture.json --now 2026-11-25T13:29:00Z
```

Output is one JSON document: the `evaluation` (status, age, last
cycle) and the planned `actions` (`file` / `update` / `none`).

## Wiring

The shipped systemd user timer (`systemd/`) runs the probe every
30 minutes on the laptop, where the conductor log lives, so a silence
surfaces within ~2.5h (threshold + one tick) instead of 33h. It is
repo-shipped, **not yet installed**: installing it is
`cp systemd/conductor-silence.{service,timer} ~/.config/systemd/user/`
plus `systemctl --user enable --now conductor-silence.timer`, and
correcting the log path in the service unit if the conductor log
lives somewhere other than
`~/workspace/goals/keep-the-four-repos-healthy/hidden_files/conductor-log.md`.
The timer is registered in `doc/fleet-schedule-registry.md`
(Surface 2b) in the same pass, per the registry's same-pass rule.
The probe is observe-only until
`~/.config/conductor-silence/mode` reads `apply`.

## Behaviour on bad input

A completed pass exits 0 whatever it found: silence is a finding in
the output, not a probe failure. Every `gh` call is bounded by
`GH_TIMEOUT_S` (60s). An unreadable or missing conductor log is
`status: "error"` — never a silence verdict, so a down log host files
nothing — and a failed tracking read or write is reported the same
way; either makes the exit 1, so a wired-in step never reads a
partial pass as clean (code#11468). Widen
`DEFAULT_THRESHOLD_HOURS` when the conductor's normal cycle gap
grows, rather than ignoring its alerts.

## Tests

```bash
python3 -m unittest test_check_conductor -v
```

Offline, fixture-driven: the actual #15123 silence (last cycle
2026-11-24T04:25Z) fires exactly one tracking issue, the verdict
already fires at the 2h threshold, a fresh cycle stays quiet, an
existing tracking issue is updated not duplicated, no-cycles fires,
threshold boundary, newest-cycle selection, future timestamps do not
mask a silence, log-text parsing, and a missing log is an error.
