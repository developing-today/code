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
store-shape refusal). The worked example for slice 1 is
`install.mjs --dry-run` against the live store plus a shadow
`poll.py` run: both are read-only and are recorded on #3026.

## Known gap found by the first shadow poll (2026-10-31)

`poll.py` counts every root session `GET /api/session` returns — 41 at
the first shadow run, most of them finished-but-unarchived lane
sessions. So `cap_room` reads 0 even when few sessions are active.
Shadow mode is unaffected (it launches nothing), but the gate packet
must refine the count to active sessions (recent activity / busy
status) before live mode is enabled; until then the cap fails closed,
which is the safe direction.
