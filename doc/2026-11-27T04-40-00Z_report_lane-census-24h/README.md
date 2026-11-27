# Lane census — past 24 hours (2026-11-26T04:10Z → 2026-11-27T04:10Z)

Synthesis of the four census slices in this folder: A (Codex lanes), B (OpenCode
sessions), C (Muse work-loop lanes), D (queue ledger). Window opens mid-outage:
the laptop was off the tailnet until its 03:40 CST boot (#15131), so the first
~5 hours show almost no laptop-side activity anywhere.

## How Codex lanes are triggered

Nothing dispatches them per job. Each lane is an always-on systemd user template
instance (`codex-fleet-lane@agent-next-<n>-local.service`, Restart=always,
started at laptop boot) running a 19-line bash loop: one
`codex exec --model gpt-6-luna` session, `sleep 60`, repeat. The session itself
enrolls and claims via `fleet agent next --repo developing-today/code` from
repo-health-code-pilot, works tasks to terminal dispositions (sometimes several
per session), and exits when no work is available. No timer, no dispatcher;
pm-dispatch is a separate OpenCode intake daemon.

## The four systems, side by side

| System | Volume (24h) | Outcome |
|---|---|---|
| Queue ledger (all workers) | 1,887 terminal events | 1,706 completed / 24 failed / 157 cancelled |
| Muse work-loop lanes (Archer) | 36 cron runs, ≈549 lane instances | 160 PRs merged, 73 issues closed |
| OpenCode sessions (laptop) | 1,603 sessions created | 1,036 succeeded (64.6%) / 530 errored (33.1%) / ~1 evaporated |
| Codex lanes (9 instances) | 132 attributable completions | 0 failed-status tasks, 0 cancels |

Completions by worker (queue ledger): repo-worker-code 1,241 (72.7%), archer 148,
xaltha 63, repo-worker-openchamber 42, the nine Codex lanes 158 combined
(agent-next-24-local top at 29), lane11-claude / claude-lane4 9 each.

## What actually landed

- **Muse work-loop lanes: 160 PRs merged** (133 code, 12 opencode, 8 openchamber,
  4 darwin-nix, 1 each developing-today/fleet, dezren39/fleet, hatch-personal) and
  **73 issues closed**. Spot-verification: 10 sampled merges checked on GitHub —
  all real, 6 with merge commits exactly matching the lane records. One paper-trail
  defect: code PR #15882's merge is real but no lane record claims it.
- **Codex lanes:** mostly queue-verified triage dispositions; spot-verified code
  landings incl. task 25540 → PR #15899 (mcpx auth key lifecycle CLI, merged
  2026-11-27T03:44:36Z) and task 24298 → PR #15705 (merged 2026-11-26T20:09:50Z).
  Per instance: 7 producing, 1 producing-but-churny (27), 1 idle-but-alive (20:
  2 completions in ~18.5h — its work kept terminating under other workers).
- **OpenCode sessions:** landings aren't provable from the session store (merges
  happen via gh off-store); sampled succeeded lanes do run claim→shepherd→merge
  flows. ~956M input / ~9M output tokens; recorded cost 0 everywhere (routes
  don't write cost). 87% of sessions ran in the shared primary clone
  /home/user/git/code; fleet worktree dirs held only ~99.
- **Failure lane 2159-F** (first run of the standing failure lane): 25 stale
  failure entries resolved against live GitHub state, 6 re-checked with new
  facts, 0 merges of its own — everything resolvable had already landed.

## Failures, grouped by cause

**Queue tasks (24 failed of 1,887 terminal):**
- 9 out-of-scope cross-repo claims — the broker served another repo's pilot task
  to a code-scoped lane; failed untouched.
- 8 disposition-gate / packet-contract rejections — work done or correctly
  triaged, completion rejected (`invalid_no_work`) or packet defective at import.
- 2 `fleet gh` close-verb defect (#9221 class), 2 Drew-authored-PR hard stops
  (scope guard working as designed).
- Only **3 real work failures**, and only 1 unrecovered: task 23951 (PR #15550
  failed Darwin acceptance).

**OpenCode sessions (530 errored):**
- **500 of 576 terminal failure events (87%) are one defect:** the workhorse
  route offered sessions the model `muse-wonder-1.0-contributor` on the `_muse`
  door, which answers "model not found" — 912 route attempts pinned to that one
  door/combination. A route/catalog mismatch, not 500 separate failures.
- 35 `aborted` ("interrupted repeatedly, will not be resumed") — including the
  window's largest session (44.2M tokens, oc-code-07) dying after one successful
  cycle. 5 of the 8 largest sessions are failed queue lanes burning 10–44M tokens.
- Attempt-level weather: 15,000 of 81,978 route attempts (18.3%) timed out;
  worst combos on _openai.main (walled pools: Claude Pro 5h 100%, popesdrew
  weekly 100%) and the flaky broker tunnel.
- Surprise: the **Fleet conductor series is alive** (56 sessions, latest 22:00
  CST) — contradicting the 2026-11-24 record that it stopped being launched.

**Muse work-loop lanes:**
- 7 cron runs timed out → 21 lanes evaporated mid-flight, ~14 cancelled.
- 37 new-system lanes gate-held properly: CI-on-main-only + the deployed fleet
  binary predating merged PR #15841 → Actions-exec TLS UnknownIssuer/403 all
  evening. Correct behavior; the landing rate is capped until a post-#15841
  binary deploys.
- 33 triage-only failures: queue reconciliation re-imports Drew-gated/terminal
  tracking homes repeatedly and auto-spawns follow-ons, so lanes could only
  re-disposition.
- 20 lanes parked across four GitHub REST 403 token-exhaustion waves.
- Broker/laptop tunnel outages stranded park/complete verbs (mostly healed by
  replacement lanes); sandbox EDQUOT (~19:00–19:25, code#15835) blocked one
  run's bookkeeping — records preserved via the Lanes chat.
- One merge with incomplete recorded verification: openchamber PR #2659 (its
  new test never ran).

**Codex lanes:**
- Most repeated friction: completion calls rejected (exit 2) for a missing
  evidence reference on feature_slice tasks — wasted turns, always recovered
  (lane 27: 3 attempts on task 23791; lane 20: 5 on task 25641).
- 45 lease-expiry events on their terminal tasks (4h leases overrun by long
  sessions); every examined case was re-claimed and completed.

**Claim evaporations (queue-wide): 400 events / 309 distinct tasks** — churn,
not loss: 60% of evaporated tasks still reached done. Two clusters: 103 events
in the 10:00Z hour (the broker's recovery sweep after the outage) and a
17:00–00:00Z plateau of 25–39/hr under the flaky tunnel (baseline 4–6/hr).

**Cancellations (157):** ~85% are triage auto-follow-on churn — duplicates 75,
issue-closed 40, already-done 10, superseded 7, false-premise spawns ~13.

## Instrumentation defects found by the census itself

- The Turso fleet-queue copy is a **frozen mirror** (newest completion
  2026-11-26T13:18:38Z); the live store is D1. The lane-share producer is
  running on the frozen source — its worker attributions are stale.
- The lane ledger has ~zero coverage of the window in either store.
- Codex journal retention is only ~2.8h (rotated under the lanes' own volume).
- Queue read-path defects: `list --worker` missed ≥3 verifiable tasks;
  `show --json` returns null completed_at/updated_at; `timeline` omits
  re-claim/completion events for some tasks.
- OpenCode's legacy `session` table is stale (newest 2026-10-05); live data is
  `session_v2`. Its v2 store leaves summary/diff fields all-zero, so landings
  can't be proven from the store alone.
- Fleet health counters undercount failures by 1 on code-pilot (12 vs 13) and
  nix (3 vs 4).

## Bottom line

Handled properly: the overwhelming majority — 1,706 completions against 24
failures, 160 verified merges from the Muse lanes, gates held where evidence
was missing, evaporations recovered. The losses concentrate in four fixable
places: (1) the `_muse` door offering a model it doesn't carry (~500 OpenCode
sessions), (2) the stale deployed fleet binary capping verification (the
post-#15841 deploy), (3) reconciliation re-importing terminal/Drew-gated homes
(the triage-only churn), and (4) cron-run timeouts evaporating Muse lanes
mid-flight. Token waste is real but bounded: a handful of giant OpenCode queue
lanes (10–44M tokens each) dying aborted is the biggest single sink.
