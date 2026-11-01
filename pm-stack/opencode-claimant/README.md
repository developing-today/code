# opencode-claimant — laptop-side claimant/executor for the opencode lane

Implements slice 1 of
[#3026](https://github.com/developing-today/code/issues/3026): queue
tasks assigned the **opencode execution lane** get launched from the
laptop side, with the session id bound in the task record at launch
time — no Muse cron boots or hands off to OpenCode.

## Mechanism (Drew's correction on #3026, 2026-10-31)

The claimant is an **OpenChamber native scheduled task** on the laptop
(cron `*/10 * * * *`, America/Chicago; route
`opencode-route`/`workhorse`, agent `build`). OpenCode itself has no
scheduler (#2019, upstream anomalyco/opencode#41906); the systemd-timer
composition from #2019 is fallback-only and is NOT used here. No new
service unit is needed: the claimant is a scheduled session, and the
worker sessions it launches run on the standing OpenCode server
(127.0.0.1:13420), the same surface pm-dispatch launches through.

## Pieces

- `prompt.md` — the scheduled task's prompt: gate, cap, candidate
  handling, launch-time binding, terminal-state duties, supervision.
  This file is the source of truth; the installer embeds it verbatim.
- `poll.py` — read-only candidate discovery the prompt runs each fire:
  claimed tasks whose `progress.lane` is `opencode` with no bound
  `session_id` (across the four repo-health queues), plus the live
  root-session count for the 12-session cap. It cannot mutate anything.
- `install.mjs` — idempotent installer: merges the task definition
  (stable id `f1b4a33b-14d5-4a69-9ad6-d221e42e3ac9`) into the laptop's
  code-project OpenChamber store, preserving all other tasks, with a
  timestamped backup before any write. `--dry-run` prints exactly what
  would be installed and writes nothing.

## Modes and the gate

The prompt reads its mode from `~/.local/state/opencode-claimant/mode`;
anything but `live` means **shadow**: discover, verify, and log what
would launch — launch nothing. The task also installs **disabled**.
Shadow is what makes the worked example safe beside the live
conductor: no double-launch is possible until the parallel-run gate
(#3026 slice 2, enqueued when slice 1 completes) enables the task and
flips the mode — the stand-down of the conductor's launch role waits
for that gate, per the issue.

## Supervision choice (recorded on #3026)

Stall supervision is the claimant itself on its next fire: bound
sessions are re-checked against the OpenCode server, a vanished
session with no terminal state is noted and only relaunched after its
lease expires, never over a live session. Tend's fleet-watchdog,
reading the lane ledger, is the backstop — the same division #3026
names.

## Binding, and why it closes the Oct 28 gap

The launch PATCH-then-ledger order copies pm-dispatch (the one
precedent that binds a known task to a known session): create session,
PATCH `progress.session_id` + `launched_by`, checkpoint `launched`,
agent-note on the issue. A session without a binding is treated as a
defect, not a launch.

## Tests

`bun test` in this directory (merge idempotence, prompt embedding,
store-shape refusal); `python3 -m unittest test_poll.py` here covers
poll.py's active-only cap counting. The worked example for slice 1 is
`install.mjs --dry-run` against the live store plus a shadow
`poll.py` run: both are read-only and are recorded on #3026.

## Known gap found by the first shadow poll (2026-10-31) — RESOLVED by the gate

`poll.py` originally counted every root session `GET /api/session`
returns — 41 at the first shadow run, most of them
finished-but-unarchived lane sessions, so `cap_room` read 0 even when
few sessions were active. The gate refinement (task 4408) counts only
**active** roots toward the cap: a finished session carries
`time.idle` and an `outcome` (both verified on the live server: 50
roots, 46 finished, at the gate check), a running one carries neither,
and a marker-less root silent for 30+ minutes is treated as a zombie,
not a worker. A root the server itself holds in `GET
/api/session/active` always counts — a resumed session keeps its
stored outcome from the earlier run while it is mid-run again, so the
server's active set overrides the stored markers (the same rule the
aggregator's session mapper applies). `poll.py` still reports the raw root total as
`live_root_sessions` alongside the new `active_root_sessions`;
`cap_room` derives from the active count. An unreadable server still
yields `cap_room: null` — the cap fails closed, which is the safe
direction.
