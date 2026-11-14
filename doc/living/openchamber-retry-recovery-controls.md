# OpenChamber retry-recovery controls — restart-with-new-model + force-next-pool-option

- Living document · Status: shipped (2026-10-19) · Gap-addendum entry G6 (companion to the 60-day design register, developing-today/code pm-stack/design-register-2026-11-11.md, epic code#8805)

## Purpose

When a session hits an error that stops progression but triggers retries —
the plan limit is the canonical case — the user should not have to watch the
same route fail in a loop. OpenChamber gets manual recovery controls: restart
the turn on a different model, or force the route pool to its next option.

## What landed

- **Restart with a new model**: a button, available when an error stops
  progression, that restarts with a model picked by the user (model picker).
- **Force next pool option**: when the session is using route pools, a
  control that forces the next route-pool option instead of retrying the
  failed one.
- Both controls are available at all times, and it is configurable whether a
  control force-stops the current attempt or applies on the next message.
- Implemented as OpenChamber PR #385 ("prompt retry recovery"); the detailed
  review (settings restructure + sessionKnowledge API conflicts) ran as
  developing-today-forks/openchamber#388, now closed.

## Rough architecture

OpenChamber UI/session layer: the recovery actions surface on the errored
turn (the fork carries the recovery UI in packages/ui — MessageRecoveryActions,
applyRetryOverlay); the restart path re-dispatches the turn against the
user-picked model or the next entry in the session's route pool, per the
force-stop / next-message setting.

## Status

Shipped 2026-10-19 (asked by Drew in a direct OpenCode session the same day:
"fully implement it"). The ask lived only in that session and the build only
in PR #385 — neither the register nor any doc recorded the design until this
living document (gap pass, 2026-11-11).

## Next steps

- Immediate: none recorded.
- Near-term: none recorded.
- Long-term: none recorded.

## Open follow-ups

- None recorded in the gap pass; the review issue (#388) is closed.

## Future ideas not yet specced

- None recorded.

## Links

- Gap addendum entry G6 (~/workspace/design-audit-60d/GAP-ADDENDUM.md)
- Build: developing-today-forks/openchamber PR #385
- Review: developing-today-forks/openchamber#388 (closed)
- Origin: OpenCode session opencode:ses_eabfb2c22ffem (2026-10-19)
