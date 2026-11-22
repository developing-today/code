# C3 health hooks — Slice D validation record (issue #9571)

Status: **Slice D partial — hook-level validation complete; production
cycles and the 14-day cutover remain.** Queue task 20918, run
2026-11-22 by agent felix (Muse sandbox seat). Design under test:
`doc/2026-11-20T00-46-00Z_design_c3-health-hooks/note.md`. Code under
test: `pm-stack/health-hooks/` on `main` at `9794191ad` (Slices A+B,
PR #12077) plus the Slice C audit-event mapping in open PR #12226.

## What was run

Real failure + recovery cycles for **both** hooks, end-to-end through
the shipped runner (`run.py` on main, unmodified), in the Muse
sandbox against disposable local stand-ins — no production service was
touched, stopped, or probed destructively:

- `pm-stack-reachable`: `ALERTS_INTAKE_URL` pointed at a throwaway
  intake on `127.0.0.1:14870` (`drill/capture_server.py`). The failure
  was real in two forms: nothing listening (connection refused), then
  the intake up but `GET /board` answering 503. Recovery: `/board`
  answering 200. The stand-in captured every `POST /api/alerts` body
  (the `Authorization` value was never recorded — presence only; the
  bearer the hook presented was the real one, resolved by the hook
  itself from Bitwarden Secrets Manager).
- `opencode-working`: the hook probes a fixed URL,
  `http://127.0.0.1:13420/global/health` — the production port, verified
  free in this sandbox before the drill. Down = nothing listening
  (genuinely true of this seat); recovery = a throwaway server serving
  `/global/health` 200 on 13420, stopped immediately after the cycle.
- Probes that read the environment ran for real: BWS secret resolution
  (`pm-aggregator.alerts-intake-token` → `resolved`), `systemctl --user`
  state (no such user units in the sandbox — recorded as the probes saw
  it), recent commits from the drill checkout, and the OpenCode session
  store (`sessionsLastHour: 0` from the seat's real, read-only DB).
  `PM_PRIMARY_CHECKOUT_DIR` pointed at the drill worktree.
- Incident files were redirected (`PM_HEALTH_HOOKS_INCIDENT_FILE`) to
  the drill directory; the seat's real state directory was not written.

Harness: `drill/run_drill.sh`, `drill/capture_server.py`. Raw evidence:
`evidence/` (hook stdout per phase, both incident JSONL files, captured
POSTs, drill log). Re-running the harness elsewhere reproduces the
cycles; incident ids and timestamps will differ.

## Cycle results

### pm-stack-reachable — incident `d50ada41-77a1-4fdd-a91f-4a3b79bbcf27`

| Phase | At (UTC)  | Condition               | Hook verdict                                    | Record                                                                                           |
| ----- | --------- | ----------------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| PM-1  | 14:54:57Z | nothing on :14870       | `down`, severity `critical`                     | JSONL down record; throat POST failed (connection refused — the intake itself was the outage)    |
| PM-2  | 14:55:02Z | `/board` 503, intake up | `down`, **same incidentId** (open-outage reuse) | second JSONL record; POST captured (`evidence/captures.jsonl` #1)                                |
| PM-3  | 14:55:07Z | `/board` 200            | `up`, severity `info`                           | recovery record, `recoveryOf` = the down incident, `recoveryDelivered: true`; POST captured (#2) |
| PM-4  | 14:55:14Z | `/board` 200            | `transient`                                     | no record, no post — correct steady-state behaviour                                              |

### opencode-working — incident `295e4e8b-b67f-4fb8-ae01-acd718caf66c`

| Phase | At (UTC)  | Condition                             | Hook verdict               | Record                                                                                           |
| ----- | --------- | ------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------ |
| OC-1  | 14:55:16Z | nothing on :13420                     | `down`, severity `warning` | JSONL down record; POST captured (#3)                                                            |
| OC-2  | 14:55:20Z | stand-in serving `/global/health` 200 | `up`, severity `info`      | recovery record, `recoveryOf` = the down incident, `recoveryDelivered: true`; POST captured (#4) |
| OC-3  | 14:55:23Z | stand-in still up                     | `transient`                | no record, no post                                                                               |

Every firing finished inside the design's 60 s sequence cap. The
runner's own suite on main also passes: `pytest test_run.py` → **22
passed** (run with a sandbox-local venv; `uv` is unavailable on this
seat).

## C6/C8 shape verification

Three shapes were diffed against the design contract:

1. **Hook-local incident record (JSONL)** — conforms. All contract
   fields present (`source, key, severity, title, body, target, state,
caller, firedAt, observedAt, incidentId, recoveryOf, evidence{…}`),
   severities match the design's mapping, `recoveryOf` lineage is
   correct, evidence sections match with additive fields only
   (`confirmFailure`, `reachability.tcpOk/tcpError`,
   `serviceState.activeEnterTimestamp`, `recentChanges.gitOk`,
   `hostHealth.present`, `recoveryDelivered`).
2. **Throat POST body** — reduced by construction. It carries only
   `{source, key, severity, title, body, stackHealth{…}}`: no
   `evidence`, no `incidentId`/`recoveryOf` (the id survives only
   inside `key`; lineage only as prose in `body`), no `caller`.
3. **Events-stream `AuditEvent`** (PR #12226's
   `health-hook-audit-event.ts`, not yet on main) — conforms exactly to
   the design's "Event-store append" section: `kind =
health-hook-incident`, `ref = health-hook:<target>`, `actor =
health-hook`, `summary = title`, attribution `unknown` /
   `asserted=false`, source identity `health-hook/<target>`,
   deterministic replay id. Verified by running the PR's tests in a
   scratch worktree: `bun test test/health-hook-audit-event.test.ts` →
   **3 pass**, including appends of a down + recovery pair to the real
   `EventsStore`.

**The gap between (1) and (3) is the finding.** The design's Report
format presents the full incident record as what is posted via the
throat and recorded in the event store; in the implementation the full
record never leaves the hook host's local JSONL, and the stream's
closed `StreamEvent` shape (`at, actor, kind, ref, summary`) cannot
carry evidence as-is. Consequences: a C6 incident page built from the
stream cannot pair a recovery with its down event except by `ref` +
time order, and has no probe evidence for the incident story; C8's
first-appearance error strings live in `evidence.reachability.error`
and the record `body`, neither of which reaches the stream. Filed as
**#12599** (design decision: extend the POST + stream schema, or amend
the design and give C6/C8 a detail read path), with a triage packet
enqueued in the same pass (task 21050). Minor diffs filed with it: the
JSONL record omits the `stackHealth` member the design's shape shows;
recovery title is "… is up", not the design's "… recovered";
`authHealth.status` emits `unknown` where the design's vocabulary is
`resolved|missing|error`.

So acceptance bullet 3 of #9571 ("incident records land in the event
store in the shape C6/C8 consume") is **partially** satisfiable today:
the audit projection lands (once #12226 merges) in exactly the shape
the design's append section specifies; the incident record itself does
not land, pending the #12599 decision. C6 (#9574) and C8 (#9576) have
no in-tree consumer code for a richer shape yet, so nothing downstream
is broken today — the gap is between the C3 design text and the C3
implementation.

## Production state (laptop, read-only checks, 2026-11-22 ~14:53Z)

- `pm-aggregator`, `opencode`, and the stack sentinel are `active`;
  the sentinel's state shows all probed services `up` and
  `hookDispatchedStates` unset — **no C3 hook has ever fired in
  production**: `~/.local/state/pm-aggregator/health-hook-incidents.jsonl`
  does not exist on the laptop, and the events stream contains zero
  `health-hook-incident` records (the append code is still in PR
  #12226).
- The drill cycles above are therefore the first end-to-end
  failure + recovery cycles recorded for these hooks anywhere. They
  validate the hooks; they are **not** production cycles and do not
  count toward the cutover test, whose condition 3 requires the cycles
  in the production events stream.

## Cutover test — honest state

**Not started; day count 0 of 14.** The design's test requires, for 14
consecutive days: (1) every watchdog-recorded down for the two targets
also recorded by the matching hook within 60 s with matching lineage,
(2) no false-positive hook incidents escalated, and (3) at least one
real failure + recovery cycle per target in the events stream. None
can hold yet: the hooks are not live in production (Slice C, PR #12226,
unmerged — lane-launcher and aggregator-tick triggers plus the
events-stream append are not on main), and no production incident
records exist to compare against `fleet-watchdog`.

Day 0 is the first full day after **all** of: PR #12226 merged; the
laptop's aggregator + sentinel running the merged code; hooks firing
from production callers. Nothing is retired now — `fleet-watchdog` and
`fleet-minder` keep all current duties, including their HTTP probes of
these two targets. Retirement remains a separate gated PR with Drew
approval after the 14 days pass, per the design.

## What remains on Slice D / #9571

1. Merge PR #12226 (Slice C) — prerequisite for everything below.
2. Production failure + recovery cycle per hook, recorded in the
   production events stream (induce via the hooks' designed triggers
   during a real or staged outage on the laptop; do not fabricate).
3. The #12599 design decision, then aligning the record path.
4. Start the 14-day cutover clock at day 0 as defined above; only then
   propose retiring the overlapping watchdog HTTP-probe duties.
5. Acceptance bullet 2 of #9571 stays open until (2) lands; bullet 3
   until (1) + (3) land.
