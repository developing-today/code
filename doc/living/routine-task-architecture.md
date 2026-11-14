# Routine-task architecture redesign — one merged job, time-based sections

Living document · Status: shipped (2026-10-04/05) · Register entry F4.2 · Tracking epic: developing-today/code#8805

## Purpose

- Merge two routine jobs into one, without losing their separate schedules
  or their separate todo sources.
- Jobs merged: sandbox-boot-init (5m) and nix-mcpx-15min-check (15m).
- Directives ran 2026-10-03 → 2026-10-04, with a simplification pass on
  2026-10-05.

Drew's rulings in the redesign:

- Time-based per-section scheduling instead of a run counter —
  "a counter breaks if the job interval changes".
- Per-section `todo_sources` allowlists.
- mcpx rebuilds ONLY on a new upstream tag.

## What landed (features, and how each works)

- One merged job:
  - Cron job: `boot-init-todo-check`.
  - Sections run on their own time-based intervals inside the one job.
- Time-based sections:
  - State file records, per section, `interval_minutes` and `todo_sources`.
  - Sections named in the register: boot-init / todo-sweep / deal-hunt.
- Per-section todo allowlists:
  - Each section carries its own `todo_sources` allowlist in the state file,
    so a section only draws todos from its approved sources.
- Simplifications applied 2026-10-05 ("yes all these look good. apply 1 to 4"):
  1. Timeout 900s → 1800s.
  2. Tiered upstream check.
  3. Codified push pattern.
  4. Less log noise.
  - Plus: sweep cadence dynamic 30 / 90 / 120m.
  - Applied 2026-10-05 11:45–11:50, verified with cron.view read-back.
- Related cadence directive (2026-10-07):
  - "cut the scheduled jobs to run half as often" — applied the same day:
    boot-init-todo-check 10m → 20m, airthings-alerts 6h → 12h,
    heartbeat 30m → 1h.

## Rough architecture

- Single cron job: `boot-init-todo-check`.
- State file:
  `~/workspace/goals/nix-real-store-mcpx-opencode-setup/hidden_files/schedule-state.json`
  - Holds the sections (boot-init / todo-sweep / deal-hunt), each with
    `interval_minutes` + `todo_sources`.
- Append-only events log alongside the state file (named in the register
  as `events.log` in the same design artifact description).
- The predecessor 15-minute mcpx check cron from the F2 program was
  absorbed into this redesign (see F2 parent entry).

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped — verified working from
  2026-10-04 00:24 onward; simplifications applied 2026-10-05.
- Tracking: NEVER-ISSUED — searched "boot-init todo-check".
- Triage: (b) DONE-WITHOUT-ISSUE.

## Next steps — immediate / near-term / long-term

- Immediate: none recorded.
- Near-term: none recorded.
- Long-term: none recorded.

## Open follow-ups

- None recorded in the register entry.

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entry: F4.2 in `~/workspace/design-audit-60d/REGISTER.md`
- Related register entries: F2 (mcpx platform program — its 15-min check
  cron was absorbed here), F4.1 (fleet sweep/review job family — the
  boot-init-todo-check cadence was later changed there, 20m → 30m on
  2026-10-19)
- Tracking epic: developing-today/code#8805
- Design artifact: cron job `boot-init-todo-check` + schedule-state.json
  (path above)
