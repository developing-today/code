# C3 — Health hooks: failure-triggered checks that troubleshoot and report

Status: **draft (design-first slice for queue task 20429 / issue #9571).
Statement S1.3 of the reporting-architecture programme (starter epic
#9568). Successor to the disabled `opencode-session-minder` detection
role: hooks, not a revived cron.**

## Request and intent

Drew's directive, Reports side chat, 2026-11-13 09:11 CST (Thread 1):
"PM stack reachable" and "OpenCode working" stop being interval polls and
become **hook scripts that fire on failure-to-connect** from any caller.
On a failed connect the hook fires immediately, runs a bounded read-only
troubleshoot sequence (reachability, service state, auth/credential
health, recent changes), and **reports what occurred** — a note to the
queues/PM-health category chat, escalating to Reports when it is an
exception and to the main chat when Drew is blocked. The hook records the
incident into the PM-stack event store so C6 can render an incident page
and C8 can ingest it (first-appearance data for error strings starts
here). Recovery is also an event: the same hook reports the recovery (the
watchdog's new-alert/recovery pattern, event-driven).

This component is the **successor to the disabled `opencode-session-minder`**
detection role — it closes the detection gap left by that disabled monitor
(no durable monitor covered sessions after 2026-11-08) by trigger, not by
schedule. `fleet-watchdog` / `fleet-minder` keep their current duties
until the hooks prove out; this design names the overlap and the cutover
test.

## Boundaries / collisions (held)

- **Hooks, not a revived cron.** No interval polling is added by this
  component; the existing `stack-sentinel` (30 s probe loop) and
  `fleet-watchdog` remain as-is until the cutover test passes.
- **Read-only probes first.** Troubleshoot actions are bounded and named;
  no state-changing remediation ships in this component. Remediation is a
  separate, gated design decision.
- **No C6 / C8 implementation.** This component only owes the incident
  record shape those consumers read.
- **No retirement** of `fleet-watchdog` / `fleet-minder` before the hooks
  prove out and the cutover test passes.

## Trigger surface

### What counts as failure-to-connect

A *failure-to-connect* is any attempt by a caller to reach a monitored
target that does not yield the target's "alive" answer within the
target's timeout. Concretely, per target:

| Target id | Monitored surface | Alive answer | Timeout |
|-----------|-------------------|--------------|---------|
| `pm-stack-reachable` | aggregator HTTP API (`$ALERTS_INTAKE_URL` / board) | HTTP 2xx | 10 s |
| `opencode-working` | OpenCode global health (`http://127.0.0.1:13420/global/health`) | HTTP 200 | 10 s |

A *connect* is any of: an HTTP fetch that throws or times out, an HTTP
response outside the alive set, or a caller-reported failure (a lane
launch, an aggregator tick, a PM-stack surface) that names the target as
unreachable. The hook is **caller-agnostic**: any caller may invoke it;
the hook dedupes by `(target, incident key)` so a storm of callers does
not produce a storm of incidents.

### Who calls the hooks

1. **The stack sentinel** (`pm-stack/aggregator/scripts/stack-sentinel.ts`)
   — on a 3-consecutive-failure transition (existing damping), instead of
   only posting a `stackHealth` transition, it also fires the matching
   hook. This is the primary trigger.
2. **Lane launchers** (`ready-for-agent` / `opencode-claimant`) — when a
   lane cannot reach the aggregator or OpenCode at launch, they fire the
   hook with `caller=lane-launch`.
3. **Aggregator ticks** — when an aggregator source reports the target
   unreachable during a refresh, it fires the hook with
   `caller=aggregator-tick`.
4. **PM-stack surfaces** (board, pages) — when a browser/API client
   reports the target unreachable, it fires the hook with
   `caller=pm-surface`.

Callers invoke the hook **asynchronously and fire-and-forget**: a hook
failure never blocks the caller. Callers pass `{ target, caller,
observedAt, detail }`; the hook owns dedup, probing, reporting, and
recording.

### Hook entry points

Two hook scripts, one per target, sharing one implementation module:

- `pm-stack/health-hooks/pm-stack-reachable.sh` (or `.py`) — fires on
  `pm-stack-reachable` failure-to-connect.
- `pm-stack/health-hooks/opencode-working.sh` (or `.py`) — fires on
  `opencode-working` failure-to-connect.

Plus a shared runner `pm-stack/health-hooks/run.py` that takes
`--target {pm-stack-reachable,opencode-working}` and the caller context
on stdin/args. Shell wrappers exist so any caller (systemd, lane, cron)
can invoke them uniformly.

## Bounded read-only probe sequence

Each hook, on fire, runs this fixed sequence. Every step is read-only;
no step mutates state. Each step has a hard timeout; the whole sequence
is capped at 60 s.

### Common prelude (both targets)

1. **Confirm the failure** — re-probe the target once (same alive answer,
   5 s timeout). If it now answers, record a *transient* outcome and
   exit (no incident, no report). This absorbs single slow probes.
2. **Timestamp + caller** — capture `firedAt` (ISO, UTC), `caller`,
   `observedAt` (caller's observation), `target`.

### `pm-stack-reachable` probe sequence

3. **Reachability** — DNS resolve + TCP connect to the aggregator host/port;
   HTTP GET `/board` with 5 s timeout; record status code or error string.
4. **Service state** — `systemctl --user is-active pm-aggregator` (and the
   matching `.service`); record `ActiveState`, `SubState`, `Result`,
   `ExecMainExitTimestamp`. If systemd is unavailable, record `unknown`
   with reason (never invent healthy).
5. **Auth / credential health** — verify the alerts-intake bearer resolves
   by name from Bitwarden Secrets Manager (`pm-aggregator.alerts-intake-token`);
   record `resolved | missing | error:<msg>` (never print the secret).
6. **Recent changes** — last 5 git commits on `main` in `~/git/code`
   (`git log --oneline -5`); the deploy marker file mtime if present;
   whether `pm-git-sync` last run succeeded (from host-health snapshot).
7. **Host-health snapshot** — read `~/.local/state/pm-aggregator/host-health.json`
   if present; record its `collectedAt` and any unit reported down.

### `opencode-working` probe sequence

3. **Reachability** — TCP connect to `127.0.0.1:13420`; HTTP GET
   `/global/health`; record status or error.
4. **Service state** — `systemctl --user is-active opencode` (or the
   opencode service unit name in use); record ActiveState/SubState/Result.
   If no user unit exists, record `no-unit` (not `unknown`).
5. **Auth / credential health** — OpenCode's own health endpoint returns
   the model route; record the declared route ref from
   `.opencode/opencode.jsonc` default model (read-only, no secret).
6. **Recent changes** — last 5 git commits on `main`; whether a recent
   deploy/merge landed in the last 15 min (from git log timestamps).
7. **Session liveness** — count of opencode sessions in the last hour from
   the session store if readable; otherwise `unknown` with reason.

### Sequence terminator

After the probes, the hook computes a **verdict**:
`{ target, state: "down" | "transient" | "up", summary, evidence }`
where `evidence` is the structured probe output. `down` requires the
confirm-failure probe (step 1) to have failed AND at least one of
service-state / reachability to report down. Otherwise `transient`.

## Report format

### Incident record (event-store shape, C6/C8 contract)

Posted to the PM-stack event store via the alert throat (`POST /api/alerts`
with the `stackHealth` extension, source `health-hook`), and also
recorded as an `AuditEvent` (kind `health-hook-incident`) in the events
stream. Shape (JSON):

```json
{
  "source": "health-hook",
  "key": "health-hook:<target>:<incidentId>",
  "severity": "critical | warning | info",
  "title": "Health hook: <target> is down" | "Health hook: <target> recovered",
  "body": "<one-line human summary>",
  "target": "pm-stack-reachable | opencode-working",
  "state": "down | up | transient",
  "caller": "<caller id>",
  "firedAt": "<ISO UTC>",
  "observedAt": "<ISO UTC | null>",
  "incidentId": "<uuid>",
  "recoveryOf": "<incidentId | null>",
  "evidence": {
    "reachability": { "ok": bool, "status": int|null, "error": string|null, "latencyMs": int },
    "serviceState": { "unit": string, "activeState": string, "subState": string, "result": string, "exitTimestamp": string|null },
    "authHealth": { "status": "resolved|missing|error", "detail": string },
    "recentChanges": { "commits": [string], "deployMarkerMtime": string|null, "gitSyncOk": bool|null },
    "hostHealth": { "collectedAt": string|null, "unitsDown": [string] } | null,
    "sessionLiveness": { "sessionsLastHour": int|null }  // opencode-working only
  },
  "stackHealth": { "serviceId": "<target>", "name": "<target name>", "state": "down|up", "since": "<ISO>", "detail": "<summary>" }
}
```

`severity` uses the alert intake's accepted vocabulary: `critical` when
`pm-stack-reachable` is down (Drew is blocked), `warning` when
`opencode-working` is down, and `info` for recovery. `recoveryOf` links a
recovery incident to the down incident it closes (by `incidentId`).

### Category-chat note (insert-only, never reads the chat)

A one-line note is written to the **PM-health / queues** category chat
(Muse `msg` with `deliver: "none"` so it is recorded, not woken):

```
health-hook <target> <state> @<firedAt> caller=<caller> incident=<incidentId> — <summary>
```

Escalation:
- **Exception** (target down AND not a known-flap per damping) → also
  note to **Reports** category chat.
- **Drew blocked** (`pm-stack-reachable` down) → also note to **main chat**
  (the `page-health-push` precedent).
- **Recovery** → note to PM-health only, plus Reports if the down was
  escalated to Reports.

### Event-store append (audit)

The incident is appended to the events stream as an `AuditEvent`:
`kind = "health-hook-incident"`, `ref = "health-hook:<target>"`,
`actor = "health-hook"`, `summary = <title>`, attribution confidence
`unknown`/`asserted=false` (automated, not authenticated), source
identity `health-hook/<target>`. Retention follows the events stream
contract (30 days / 5000).

## Escalation rules

| Condition | PM-health note | Reports note | Main chat |
|-----------|---------------|--------------|-----------|
| target down, first occurrence | yes | no | no |
| target down, exception (not a known flap; damping confirms) | yes | yes | only if pm-stack-reachable |
| target down, Drew blocked (pm-stack-reachable) | yes | yes | yes |
| recovery (after a recorded down) | yes | yes if down was escalated | no |
| transient (confirm-failure passed) | no | no | no |

Flap damping: a target flips to `down` only after 3 consecutive failed
hook firings within 5 minutes (reusing `src/sentinel-damping.ts`
semantics). A single firing records a *pending* incident that is
upgraded to a real incident on the 3rd consecutive failure; transient
firings in between reset the streak. Recovery posts only after a down
was actually posted.

## Cutover test (overlap with fleet-watchdog / fleet-minder)

The hooks prove out when, for **14 consecutive days**:
1. Every `pm-stack-reachable` and `opencode-working` down recorded by
   `fleet-watchdog` is also recorded by the corresponding hook within
   60 s of the first caller failure, with matching `incidentId` lineage.
2. No false-positive hook incidents (transient probe that later
   confirms up) are escalated to Reports or main chat.
3. The events stream contains at least one real failure + recovery cycle
   per target with the full evidence shape above.

Only after (1)–(3) hold may `fleet-watchdog`'s HTTP-probe duties for
these two targets be retired (its other duties remain). Retirement is a
separate, gated change with its own PR and Drew approval.

## Implementation plan (slices)

Slice A (this PR, design only): this document + the incident-record
shape + escalation table. No code.

Slice B: shared runner `pm-stack/health-hooks/run.py` + shell wrappers;
common prelude + `pm-stack-reachable` probe sequence; wired into the
stack sentinel's transition path. Plus one recorded failure + recovery
cycle in a test fixture.

Slice C: `opencode-working` probe sequence + wiring into lane launchers
and aggregator ticks.

Slice D: real failure + recovery cycles recorded end-to-end in the event
store; verification against C6/C8 consumers.

## Open questions

1. Should hook firings from lane launchers be rate-limited per-caller,
   or is the global dedup by `(target, incident key)` sufficient?
   (Recommendation: global dedup is sufficient; per-caller rate limiting
   adds state without benefit.)
2. Does the `incidentId` lineage need to survive process restarts? The
   events stream is the source of truth; the hook reads the latest open
   incident for the target from the stream to set `recoveryOf`. (Yes —
   read from the stream, do not keep in-memory.)
3. Where do hook scripts live for non-laptop callers (Cloudflare Workers)?
   Out of scope for V1; V1 hooks are laptop-local. Cloudflare-side
   triggers are a C2 concern.
4. Is 60 s for the whole sequence too tight under load average 8–10?
   Recommend measuring in Slice B and raising to 90 s if needed.
