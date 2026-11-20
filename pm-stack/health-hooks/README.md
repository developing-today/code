# PM-stack health hooks (C3, issue #9571)

Failure-triggered, read-only troubleshoot + report hooks for two monitored
targets: `pm-stack-reachable` and `opencode-working`. Successor to the
disabled `opencode-session-minder` detection role — hooks, not a cron.

## Files

- `run.py` — shared runner. `--target {pm-stack-reachable,opencode-working}`.
- `pm-stack-reachable.sh`, `opencode-working.sh` — shell wrappers so any
  caller (systemd, lane, aggregator) can invoke uniformly.
- `test_run.py` — smoke tests.

## Trigger

Primary trigger: `pm-stack/aggregator/scripts/stack-sentinel.ts` fires the
matching hook on a 3-consecutive-failure transition (or recovery) for
`aggregator` / `opencode`. Secondary triggers (lane launchers, aggregator
ticks, PM-stack surfaces) may invoke the wrappers directly with
`--caller <id>`.

## Incident records

Appended (JSONL) to `~/.local/state/pm-aggregator/health-hook-incidents.jsonl`
(override with `PM_HEALTH_HOOKS_INCIDENT_FILE`). Best-effort POST to the
alert throat (`$ALERTS_INTAKE_URL/api/alerts`) with the `stackHealth`
extension. Shape documented in
`doc/2026-11-20T00-46-00Z_design_c3-health-hooks/note.md`.

## Probes are read-only

No probe mutates state. No state-changing remediation ships in this
component.