# Census A — Codex fleet lanes: trigger mechanism + 24h run census

Window: 2026-11-26T04:10:00Z → 2026-11-27T04:10:00Z (Thu 2026-11-26 22:10 CST ± 24h).
Compiled 2026-11-26 ~22:40 CST from: laptop systemd units + the lane wrapper script,
per-unit journals, the fleet queue ledger (list / timeline / show via the laptop
`fleet` CLI), process table, and GitHub spot-checks of merged PRs.

## Part 1 — How the Codex lanes are triggered

- **What starts a lane:** nothing fires per job. Each lane is a systemd user
  template instance `codex-fleet-lane@agent-next-<n>-local.service`
  (n = 19..27), `Type=simple`, `WantedBy=default.target`, `Restart=always`,
  `RestartSec=10`. The instances start at laptop boot/login and never stop.
  There is **no timer, path unit, or dispatcher** for these lanes
  (`list-timers` shows only fleet-queue housekeeping timers; `pm-dispatch.service`
  is a separate authenticated intake daemon for OpenCode sessions + queue
  intake — it does not trigger Codex lanes).
- **What the unit runs:** `ExecStart=/home/user/.local/bin/codex-fleet-lane %i`,
  `WorkingDirectory=/home/user/git/code`. The script (19 lines of bash) is an
  infinite loop: run one `codex exec` session, `sleep 60`, repeat:
  `codex exec --dangerously-bypass-approvals-and-sandbox --model gpt-6-luna
-c model_reasoning_effort=high --cd /home/user/git/code --json "<prompt>"`.
- **What triggers each job pickup:** the session itself. The fixed prompt tells
  the Codex session to enroll (`fleet agent enroll --kind opencode --name
<lane> --groups opencode,laptop`), `fleet bridge hello`, then
  `fleet agent next --agent <lane> --lane <lane> --repo developing-today/code`
  to claim a task from queue `repo-health-code-pilot`, follow it to a verified
  terminal disposition (checkpointing/heartbeating during long work), and exit
  cleanly if no work is available. One `codex exec` iteration may claim and
  finish several tasks in sequence (observed: lane 24 completed 29 tasks inside
  one still-running session); other lanes exit after one task and re-poll after
  the 60 s sleep.
- **Between jobs:** 60 s sleep inside the loop; if the wrapper script itself
  ever dies, systemd restarts it in 10 s. In this window: all 9 wrappers
  started at laptop boot (Thu 2026-11-26 03:40:10 CST) with **NRestarts=0**
  and SubState=running at census time.

## Window shape — the laptop was down for the first ~5.5 hours

The window opens 2026-11-25 22:10 CST. The laptop was off the tailnet (the
#15131 outage) and only booted at **2026-11-26 03:40 CST** (uptime + all unit
ActiveEnterTimestamps agree). So lane activity covers only
2026-11-26T09:40Z → 2026-11-27T04:10Z (~18.5 h). Everything below is inside
that span unless noted.

## Part 2 — Census of the 9 instances

Method: per worker, every queue task currently attributed to the worker was
enumerated (`fleet queue list --worker`), and every terminal task's event
timeline was read (`fleet queue timeline --json`); an in-window completion is
a `completed` event with timestamp inside the window. Two additional
completions (24250, 24298) have no `completed` event on the timeline route
(read-path gap, below) but are status=done with in-window claim/expiry and —
for 24298 — a merged PR inside the window; they are counted separately.
Journal evidence is limited to the last ~2.8 h (see caveats).

### Totals

- **132 completions attributable to the 9 lanes in the window**
  (130 timeline-verified + 2 status-corroborated).
- **0 tasks in `failed` status** for any of the 9 workers at census time;
  **0 cancels** in window; **2 `failed` events** in window, both recovered
  (details below).
- **45 `lease_expired` events** in window on these workers' terminal tasks —
  every examined case still reached `done` (re-claimed and completed).
- 9 of 9 units running at census time; every unit's journal has entries up
  to the census minute.

### Per instance

| Lane          | Category                    | Completions in window    | Current session since (CST)        | Notes                                                                                  |
| ------------- | --------------------------- | ------------------------ | ---------------------------------- | -------------------------------------------------------------------------------------- |
| agent-next-19 | producing                   | 13 (12 + task 24250)     | Nov 26 04:25 (one ~17.8 h session) | Flagship slice landed (PR #15899). Holding 2 live claims at census end.                |
| agent-next-20 | idle-but-alive (low output) | 2                        | Nov 26 03:50 (one ~18.4 h session) | Marathon session, almost no terminal output under its own name; see dig.               |
| agent-next-21 | producing                   | 18                       | Nov 26 20:17                       | Session refreshed in-window; steady completions all day.                               |
| agent-next-22 | producing                   | 10                       | Nov 26 04:22                       | 6 lease expiries on its tasks, all recovered to done.                                  |
| agent-next-23 | producing                   | 24                       | Nov 26 04:06                       | Burns long-tail backlog too (tasks 5803, 20397 completed in window). Holding 2 claims. |
| agent-next-24 | producing                   | 29                       | Nov 26 04:14                       | Top producer; 29 completions inside one session. Holding 1 claim.                      |
| agent-next-25 | producing                   | 15                       | Nov 26 21:18                       | Owns the window's one self-failed task — failed it properly, reclaimed, completed.     |
| agent-next-26 | producing                   | 16                       | Nov 26 04:20                       | Holding task 15848 (#1822 fix, attempts=24) with live heartbeats.                      |
| agent-next-27 | producing, churny           | 4 (+1 just after window) | Nov 26 21:22 (4th session in ~3 h) | Frequent session cycling; every expired claim still landed.                            |

Category counts: **producing 7, producing-but-churny 1, idle-but-alive 1,
erroring-looping 0, down 0.**

### Per-lane detail

- **19 — producing.** Completions (timeline): 23842, 23848, 23857, 23967,
  24172, 24519, 25500, 25731, 25830, 23809, 24030, 25540; plus 24250
  (corroborated). Flagship: task 25540 "Implement #13659 slice 3: auth key
  lifecycle CLI" → PR
  [#15899](https://github.com/developing-today/code/pull/15899)
  `feat(mcpx): add daemon auth key lifecycle CLI (#13659)` **merged
  2026-11-27T03:44:36Z** (built by this lane; shepherd-merged by a Muse
  work-loop lane). Still holding at census end: task 23747
  (`main-broken: #14665`, attempts=4, claimed 2026-11-27T01:55:40Z) and task
  20747 (second-pass #8017, claimed 03:49:20Z). One cosmetic journal error:
  `2026-11-27T03:03:16Z ERROR codex_models_manager::manager: failed to
refresh available models: request timed out` — session continued.
- **20 — idle-but-alive.** One ~18.4 h session (PID 126781, since 03:50 CST)
  with only **2 completions** in window: 24056, and 25641 (triage of
  openchamber#1439; claimed 2026-11-27T01:26:00Z, completed 04:02:41Z after
  **five** `fleet queue complete` attempts visible in its journal — the
  early ones rejected, see failure F2). Late-window journal shows it working
  tasks 24127 (shepherd for opencode PR #2794 → now `blocked`, a legitimate
  disposition for that PR pair), 25657 (checkpointed by 20; failed once at
  02:22:52Z by a holder; ultimately completed by worker `repo-worker-code`),
  24420 (completed by lane 23). It claimed task 25219 at 04:09:38Z (still
  claimed at census). No `turn.failed` events, no failed tasks: the lane is
  alive and polling (13 `agent next` mentions in the retained journal) but
  converts very little into terminal dispositions under its own name — its
  claims keep ending under other workers or after the window.
- **21 — producing.** 18 completions: 23897, 24177, 24225, 24325, 24369,
  24375, 24537, 24753, 25621, 25847, 25982, 23995, 24031, 24045, 24063,
  24084, 24093, 24338. 3 lease expiries, all recovered. Current session
  started 20:17 CST (2 thread starts in retained journal).
- **22 — producing.** 10 completions: 24299, 23876, 23963, 23971, 23977,
  24378, 24018, 24319, 24401, 22895. 6 lease expiries, all recovered.
- **23 — producing.** 24 completions: 24368, 20397, 23829, 23832, 23902,
  23976, 23989, 24420, 24733, 24855, 24880, 25272, 25301, 25497, 24043,
  24073, 24091, 24097, 5803, 22675, 22727, 22900, 23653, 23641. 10 lease
  expiries, all recovered. Holding: task 23375 (merge-watch, opencode PR
  #2499) and task 24183 (#15577 slice 1), both claimed ~02:31–02:39Z.
  `claim_refused` events appear on its tasks — normal contention when a
  session polls a task another lane just took.
- **24 — producing (top).** 29 completions: 23830, 23843, 23844, 23887,
  23892, 23893, 23947, 23962, 24164, 24189, 24219, 24527, 24666, 24669,
  24698, 25133, 25366, 25449, 25490, 25653, 25655, 24042, 24071, 24090,
  24395, 24403, 23486, 23214, 23779. 6 lease expiries, all recovered.
  Holding task 24385 (triage #14446, claimed 01:13:18Z, attempts=2).
- **25 — producing.** 15 completions: 23765, 23938, 23957, 23970, 23990,
  23994, 24187, 24293, 24376, 24415, 25410, 25505, 24000, 24002, 24008.
  Owns failure F1 (below), handled properly. One transient journal error
  event: `Nov 26 20:14:59 {"type":"error","message":"Reconnecting... 2/5
(stream disconnected before completion: websocket closed by server before
response.completed)"}` — reconnected, no work lost. Just after window
  end it claimed task 20173 (#8570 fix, attempts=9).
- **26 — producing.** 16 completions: 23894, 23933, 23959, 23968, 25631,
  25654, 25695, 25794, 26003, 22884, 23758, 23997, 24083, 24092, 23718, 23719. Holding task 25690 (triage #9157) and task 15848
  (`Fix #1822`, **attempts=24**, claimed 02:07:46Z, lease extended to
  08:17:02Z — heartbeats are landing, so this is active long work, not a
  dangling claim).
- **27 — producing, churny.** 4 completions in window: 24058, 24087, 24271,
  25021 (triage openchamber#21; claimed 00:10:36Z, completed 03:01:28Z);
  plus 24298 corroborated (Fix #15658; its fix PR
  [#15705](https://github.com/developing-today/code/pull/15705)
  `test(pm-stack): align alerts ledger fixture with sent_at` merged
  2026-11-26T20:09:50Z while the task was still claimed; lease expired
  20:54:02Z; task now done). 5 claims in window, 5 lease expiries among its
  terminal tasks — all landed. Sessions cycle fast (4 `thread.started` in
  the ~3 h retained journal; 51 `agent next` mentions). Task 23791
  (#12833 slice 1) was claimed 03:25:26Z and completed **minutes after the
  window closed**, after repeated rejected completes (F2).

### What landed

The completions are queue-verified terminal dispositions, dominated by
triage tasks (the broker serves triage heavily to these workers), plus
fix/slice work. Spot-verified code landings:

- Task 25540 (lane 19) → PR #15899, mcpx daemon auth key lifecycle CLI —
  merged 2026-11-27T03:44:36Z.
- Task 24298 (lane 27) → PR #15705, pm-aggregator alerts-ledger fixture fix
  for #15658 — merged 2026-11-26T20:09:50Z.
- Task 24250 (lane 19), "Fix #15625: org-run lane4-oc2798 red on main" —
  done after an evaporation cycle; no merged PR references #15625 (the
  disposition was diagnostic/verified-closure class, not a code PR).

## Failures — dug in

**F1. Task 24187 explicitly failed by its lane, then recovered (lane 25).**
Timeline: claimed 2026-11-26T13:52:16Z → `failed`
(`fail_repo_task_turso`) 15:25:45Z → claimed 15:41:42Z → completed
16:38:20Z. Title: "Triage developing-today/code#14400: broker-tls BT-2…".
The worker used the `fail` verb rather than abandoning the claim — the
proper shape. The timeline event payload carries **no reason field**, so
why the session judged its first attempt failed is not recorded in the
queue; the task itself ended done. A second in-window `failed` event,
task 25657 ("Implement #9178 §2 slice 2"), shows the same healthy pattern:
claimed 01:59:09Z → failed 02:22:52Z → claimed 02:38:49Z → completed
03:44:48Z by `repo-worker-code`.

**F2. Completion calls rejected for missing evidence (exit 2) — the
window's most repeated real failure.** Sessions repeatedly ran
`fleet queue complete` without an evidence reference and were refused,
verbatim from lane 27's journal:

> `repo-queue: primary queue store failed on /complete (exit 2):
complete_task: feature_slice completion requires an evidence reference
on the result (merged PR, commit, or verified-closure note); the task
remains claimed`

Observed: lane 27 ×3 completes on task 23791 before succeeding (it then
supplied an issue-comment URL as evidence and the completion landed);
lane 20 ×5 completes on task 25641 in its retained journal before the
04:02:41Z completion. Root cause: the Codex sessions' first completion
attempts omit the `evidence` field the broker requires for feature_slice
tasks; the broker correctly refuses and keeps the claim, so the cost is
wasted turns and delayed dispositions, not lost work. Exit-code histogram
of lane 27's retained journal is consistent: 443× exit 0, 11× exit 2,
13× exit 1, 4× exit 5, 1× 128, 1× 130.

**F3. Lease expirations are routine, not lost work.** 45 `lease_expired`
events in window on these workers' terminal tasks (19:5, 21:3, 22:6,
23:10, 24:6, 25:6, 26:4, 27:5; lane 20: 0). Leases run 4 h; sessions doing
long work (or holding a task while verifying) overrun them. Type case,
task 24250 (lane 19): claimed 2026-11-26T14:13:47Z → `lease_expired`
18:14:02Z → re-claimed 18:15:02Z → done. Every examined expired task
reached `done`; none ended failed or stranded. Where a task shows both
`lease_expired` and `completed`, the expiry came first and a re-claim
finished it.

**F4. Transient provider/transport errors (no work lost).** Lane 25's
websocket disconnect (quoted above, reconnect 2/5 succeeded); lane 19's
models-manager refresh timeout. Zero real `turn.failed` events in any
lane's retained journal (loose grep hits for "turn.failed" were lanes
reading issue text about turn.failed, not events).

**F5. Lane 20's low conversion (the one soft failure).** Not an error
state: the lane is alive, polling, and checkpointing, but across ~18.5 h
it produced 2 completions under its own name while peers produced
10–29. Its visible work kept terminating elsewhere: 24420 completed by
lane 23, 25657 completed by `repo-worker-code` after a fail, 24127 ended
`blocked` (legitimate for the PR #2794 shepherd), 25219 claimed only at
04:09:38Z. Contributing factors visible in evidence: repeated F2
rejections on 25641 burned its late window, and its single marathon
session (no fresh thread since 03:50 CST) contrasts with lane 24's
equal-length session that completed 29 — so session age alone doesn't
explain it; the difference is how much of its claimed work it carried to
a disposition itself versus handing off / timing out into other workers.

## Caveats — read-path defects found while censusing (infrastructure, not lane faults)

1. **Journal retention is ~2.8 h**, not 24 h: every unit's oldest retained
   entry is 2026-11-26 19:25–19:26 CST (72 MB user journal, rotated under
   the lanes' own JSON volume). Journal evidence therefore covers current
   state and failure color only; the 24 h census rests on the queue ledger.
2. **`fleet queue list --worker` undercounts.** It missed at least three
   verifiable tasks that direct reads found: 25021 (done, lane 27),
   25219 (live claim, lane 20), 25641 (done, lane 20). Per-worker counts
   here were corrected with direct `show`/`timeline` reads where the
   journal pointed at a task; totals remain floors.
3. **`fleet queue show --json` drops fields**: `completed_at` and
   `updated_at` return null even for long-done tasks (e.g. 24250).
4. **`fleet queue timeline` omits events for some tasks**: 24250 and
   24298 show only claim + lease_expired although both are done and
   24250's re-claim (18:15:02Z) is visible in `show`. Completion counts
   split "timeline-verified" vs "status-corroborated" for this reason.
