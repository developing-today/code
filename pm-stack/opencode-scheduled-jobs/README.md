# OpenCode scheduled jobs

This directory is the source for small, fire-and-forget OpenCode jobs on
the laptop. Each job has an explicit systemd service/timer pair, a route
visible in its service unit, a read-only prompt, and a stable per-job
state directory for the latest log and append-only run history.

## Registry

`jobs.json` is the local job registry. Fleet-wide schedule inventory is
in [`doc/fleet-schedule-registry.md`](../../doc/fleet-schedule-registry.md);
update it in the same change as any deployed schedule.

| Job | Route | Cadence | State |
| --- | --- | --- | --- |
| `quota-freshness-summary` | `opencode-route/workhorse` | hourly | disabled pending explicit enablement |

The pilot reads `~/.local/state/pm-aggregator/quota-current.json`, but
`summarize.py` passes only provider names, status, timestamps, and window
names/states to OpenCode. It omits credential identifiers, raw usage,
limits, balances, and provider error strings. The gatherer and its
authoritative files are never modified.

## Install and enable

Run on the laptop from this checkout:

```bash
./pm-stack/opencode-scheduled-jobs/install-laptop.sh --dry-run
./pm-stack/opencode-scheduled-jobs/install-laptop.sh
```

The default installation copies the job and unit files, reloads the user
manager, and leaves the timer disabled. Enable it only after explicit
approval:

```bash
./pm-stack/opencode-scheduled-jobs/install-laptop.sh --enable
systemctl --user list-timers opencode-quota-freshness-summary.timer --no-pager
```

Disable it with `systemctl --user disable --now
opencode-quota-freshness-summary.timer`.

## Logs and monitoring

The stable monitor-readable paths are:

```text
~/.local/state/opencode-scheduled/quota-freshness-summary/latest.log
~/.local/state/opencode-scheduled/quota-freshness-summary/history.jsonl
```

The same output is sent to the systemd journal. The history row records
the job, start/finish time, exit code, explicit route, and log path. A
monitor can correlate the timestamp/title with directory-scoped OpenCode
server API reads; `opencode session list` is not a reliable discovery
surface. The timer does not provide stall supervision; that remains the
babysitter's responsibility.

## Verification

```bash
./pm-stack/opencode-scheduled-jobs/test.sh
./pm-stack/opencode-scheduled-jobs/install-laptop.sh
systemd-analyze --user verify opencode-quota-freshness-summary.service \
  opencode-quota-freshness-summary.timer
```

Tests use a fake `opencode` executable and do not start a timer or spend
provider tokens. The install command leaves the timer disabled. A manual
service start tests execution only; record a scheduled firing as observed
only after the explicitly enabled timer fires on its cadence and
`latest.log`/`history.jsonl` agree.
