# Fleet roles — ONE fleet minder, dropped-thread catcher, Builder's Advocate

Living document · Status: shipped (2026-11-08) · Register entry F4.7 · Tracking epic: developing-today/code#8805

## Purpose

- Restructure fleet tending roles so coverage does not depend on many
  overlapping minder / watcher jobs.
- Replace many Muse jobs with a small, named set of roles, each with a
  clear duty.
- Source: Drew's structural rulings, 2026-11-08 ~18:19 CST.

Drew's rulings as recorded:

1. Exactly ONE Muse minder job survives — checks ALL sessions and tasks,
   and must actively KICK, not just watch.
2. Issue-making / priority / design functions: build ONE Muse version and
   ONE OpenChamber version, completely separate, rewritten from scratch
   per Drew's actual wants.
3. NEW HOURLY Muse task — dropped-thread catcher (see below).
4. Blocked-claim validator duty in the same task (see below).

## What landed (features, and how each works)

- Fleet minder:
  - One Muse minder job: `fleet-minder`, every 15m.
  - Checks all sessions and tasks; actively kicks, not just watches.
- Dropped-thread catcher:
  - Hourly Muse task: `dropped-thread-catcher`.
  - Reviews ALL chats from the past 2 hours, plus older open issues.
  - Catches problems mentioned then dropped.
  - Action: create an issue, or create an issue AND solve it.
  - Authority: full authority to approve and handle whatever it believes
    it can.
  - Includes the blocked-claim validator duty: every "blocked, nothing
    can be done" claim is assumed WRONG until validated.
- Builder's Advocate:
  - Muse job: `builders-advocate`, every 4h.
  - This is the Muse side of ruling (2) — the issue-making / priority /
    design function, rewritten from scratch.
  - The register records the cron bodies themselves as the design artifact
    for all three roles.
- Creation and cleanup:
  - Cron bodies created 2026-11-08 18:29–18:31 CST
    (fleet-minder 15m, dropped-thread-catcher hourly, builders-advocate 4h).
  - Absorption deletions of 11 absorbed Muse crons completed
    2026-11-08 22:30–22:42 CST, after the first clean advocate run.

## Rough architecture

- Three Muse cron jobs, no overlapping minders:
  - `fleet-minder` — 15m — watch + kick across all sessions/tasks.
  - `dropped-thread-catcher` — hourly — dropped threads + blocked-claim
    validation, with issue-create and solve authority.
  - `builders-advocate` — 4h — issue-making / priority / design (Muse side).
- A separate OpenChamber version of the advocate function was ruled to be
  built completely separately (ruling 2) — see open follow-ups.
- Role map: developing-today/code#2131 and #2152, both CLOSED, titled
  "Fleet roles: Make / Do / Tend".

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped (Muse side) — 2026-11-08.
- Tracking: no single issue; role map code#2131 CLOSED and #2152 CLOSED.

## Next steps — immediate / near-term / long-term

- Immediate: complete the OpenChamber-side recreations (below).
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- OpenChamber-side recreations were still RUNNING at the 18:30 flush
  (2026-11-08):
  - Handled by Lane A.
  - The OpenChamber builder's advocate text was delivered to Lane A,
    pending its handoff.
  - No completion for the OpenChamber side is recorded in this entry.

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entry: F4.7 in `~/workspace/design-audit-60d/REGISTER.md`
- Role map: developing-today/code#2131 (CLOSED), developing-today/code#2152 (CLOSED)
- Related register entries: F4 (fleet orchestration & dispatch, parent),
  P1.5 (a flow-back failure later addressed structurally by the
  dropped-thread catcher, per that entry), P2.2 (decisions-page flow-back
  runs through the dropped-thread catcher)
- Tracking epic: developing-today/code#8805
- Design artifact: the cron bodies themselves (fleet-minder,
  dropped-thread-catcher, builders-advocate)
