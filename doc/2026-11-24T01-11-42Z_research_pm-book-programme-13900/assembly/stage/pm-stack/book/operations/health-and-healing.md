# Health checks and the heal stack

The PM stack's healing layer detects, records, and in a defined set of cases repairs. It does not improvise repairs: each component in this chapter has a fixed trigger, a fixed action set, and a fixed record. Read the layer in that order — probes first, then the snapshot, then the hooks, then the guards that repair.

For the failure shapes these components produce and consume, see [Troubleshooting](troubleshooting.md). For the units that run them, see [Running the stack as services](services.md).

## The health assembly

One assembly feeds both the `/health` page and its twin `/api/health` (`healthContext` in `aggregator/src/server.ts`). Its sources in code:

- `aggregator/src/health.ts`
- `aggregator/src/stack-health.ts`
- `aggregator/src/host-health.ts`
- `aggregator/src/build-health.ts`
- `aggregator/src/watchdog*.ts`
- `aggregator/src/slo*.ts`
- `pm-stack/host-health-collector.py`

The page reports: live probes for every stack service, feed section ages, last job runs, the dashboard's own render history, and the services and ports topology. `/api/health` adds the per-page render registry: route, last rendered time, and outcome for each manifest page. One page's renders sit outside that registry (D8): `/decisions/&lt;setId&gt;` has no entry in `SUBROUTE_PARENTS`, so its renders carry no registry record.

Health-adjacent machine routes: `/api/author-identities` and `/api/admin/identity-registry` for identity registry administration, and `POST /api/stack-token/exchange` for the break-glass and stack-token path.

## The host-health collector

`pm-stack/host-health-collector.py` is a laptop user service on a 5-minute timer, `pm-host-health.{service,timer}`. It writes the JSON snapshot the packaged aggregator cannot collect itself:

- The claimant mode file.
- The backup last-run record.
- User-unit and timer states for pm-git-sync, pm-projector-reconcile, pm-kaneo-sync-guard, and pm-dashboard-backup.

The aggregator derives its watchdog tier items from that snapshot (#10141). A stale snapshot is a collector problem, not a service problem: check the timer before checking the services it reports on.

## The sentinel and the health hooks

The stack sentinel, `aggregator/scripts/stack-sentinel.ts`, watches probe results. On a 3-consecutive-failure transition it fires a health hook once. On recovery it fires once. It does not fire per failed probe.

The hooks live in `pm-stack/health-hooks/`: the Python runner `run.py` and the wrappers `pm-stack-reachable.sh` and `opencode-working.sh` (#9571). Their contract:

- Failure-triggered and read-only. A hook troubleshoots and reports. No remediation ships in a hook.
- One shared 60-second deadline covers a hook run.
- Incidents append as JSONL to `~/.local/state/pm-aggregator/health-hook-incidents.jsonl`.
- A hook posts to the alert throat on a best-effort basis, with the `stackHealth` extension. That post is what turns a hook incident into a `stack-health` or `incident` notification; the trace is in [Tutorial: trace a push notification from event to answer](../tutorials/trace-a-push.md).

Two more callers fire the same hooks, fire-and-forget: the claimant cap step and `launch_one.py`, under the caller name `lane-launch`; and the aggregator live source, under the caller name `aggregator-tick`.

Sentinel state is authoritative over health-derived notification candidates when both exist (`NotificationSnapshot.stackHealthStates` in `aggregator/src/notifications.ts`).

## The watchdog and guard scripts

These scripts are the repair-capable part of the layer. Each has a bounded repair set.

### Kaneo sync guard

`pm-stack/kaneo-sync-guard.py` asserts every Kaneo GitHub integration's status transitions: no target `done`; merge → in-review; open → in-review; push → in-progress. On drift it heals the transitions back, in compact-JSON form, and exits 1 loudly. `deploy.sh` runs it at deploy time and the `pm-kaneo-sync-guard` timer runs it hourly. Its tests are `pm-stack/tests/test_kaneo_sync_guard.py`. The recorded verification in `pm-stack/KANEO-GITHUB-SYNC.md`: on 2026-11-14 the guard read clean on all 4 integrations, and induced drift healed in one run.

### Closure watchdog

`pm-stack/closure-watchdog.py` polls issue-close events in the Kaneo-covered repos. It reopens an automation close when the issue still has open sub-issues, or when the closing pull request carried no closing keyword. Keyword closes and human closes stay untouched (#10105). It runs every 15 minutes from a Muse sandbox cron job; its scheduler lives outside the laptop stack. The repo copy and its tests are the durable record, after the original sandbox-only script was lost in the 2026-11-21 sandbox recovery.

### Invariant sweeper

`pm-stack/invariant-sweeper.py` sweeps the four Kaneo-synced repos on a slow cadence (#10139) for three invariants:

1. A closed issue with non-completed sub-issues.
2. An open issue carrying `status:done`.
3. A Kaneo↔GitHub state mismatch.

Default mode reports only. `--act` repairs the first two classes under the closure watchdog's guards, and still only reports the third. Its units are `pm-invariant-sweeper.{service,timer}`.

### Quota watchdog and the exporter watchdog core

The `pm-quota-watchdog` service and timer run the quota watchdog leg. Its exporter counterpart is the watchdog core in `pm-stack/exporters/`: `src/watchdog-state.ts` holds the failure classes and the repair ladder as pure functions, and `src/watchdog-escalation.ts` holds the rung-4 hand-off — incident issue first, then a queue task, then the NEEDS DREW surfacing line. `src/watchdog.ts` is the oneshot tick. In the surveyed tree the only exporter adapter present is a synthetic fixture; live per-provider production use is UNVERIFIED.

### Report-only monitors

`pm-stack/fleet-monitor/` carries two report-only Python monitors: `epic_close_check.py`, the weekly adapter for the epic closure gate, where a blocked epic is a finding and not a failure; and `review_loop_checks.py`. Report-only means the monitor writes a finding and changes nothing.

`pm-stack/lane-guard/lane_guard.py` is a guard of a different kind: deterministic ownership and park-note checks for queue lanes. Only explicit markers count as a hold — `held`, `user/owned`, `do-not-auto-edit`. Assignees and `work/assigned` never count. Its `note` check suppresses repeat park notes, after the repo-sweeper's blanket assignment produced 50+ duplicate park notes in two days (#7616).

### Scheduled-history watchdog

`pm-stack/scheduled-history/` reconciles OpenChamber scheduled-task runs: `collect.py` keys runs, enriches them from the OpenCode session store, and freezes terminal rows; `watchdog.py` watches that collector. Service and timer units ship with it.

## The scheduled-job surface

One pilot scheduled job runs an OpenCode session on a systemd timer: `pm-stack/opencode-scheduled-jobs/`, the `quota-freshness-summary` job. Its outputs land in `~/.local/state/opencode-scheduled/quota-freshness-summary/` as `latest.log` and `history.jsonl`. Its installer enables the timer only with `--enable`; its README states the pilot is currently enabled.

## What the layer does not do

- No hook remediates. Diagnosis and reporting are the hook contract.
- No monitor in `fleet-monitor/` changes state.
- The sweeper's third invariant class is report-only in every mode.
- The notify relay's dead-man's switch is not part of the running layer: the `pm-notify` Worker is not deployed (D5), so total laptop silence has no relay leg. Whether any other watchdog covers total laptop silence is UNVERIFIED in the surveys.

Sources. survey/website-pages.md §C; survey/apps.md §12, §15, §17–§18, §20, §24, §26, §29–§32; survey/push.md §1, §5; survey/disagreements.md D5, D8. Ground truth: laptop clone at 4d937b3, 2026-11-23.
