# OpenCode 2 fleet migration

Living document · Status: shipped (2026-10-24) · Register entry F3.1 · Tracking epic: developing-today/code#8805

## Purpose

- Move the fleet exclusively to OpenCode 2.
- Directive (2026-10-24): "my fleet should be exclusively using opencode2.
  please immediately fix anywhere that isn't"
- Scope in the directive: every fleet surface — sandbox server, laptop
  standalone, repo configs, plugins — from OpenCode v1 to v2.

## What landed (features, and how each works)

- Migration core shipped:
  - The fleet demonstrably runs OpenCode 2.x everywhere in the November
    records (register triage wording).
  - That is the basis for the (b) DONE-WITHOUT-ISSUE classification of
    the migration core.
- Launch (same night, 2026-10-24):
  - Three coordinators launched:
    - sandbox server → 2.1.4
    - laptop standalone → 2.1.4, with OpenChamber's embedded v1 as a
      named exception
    - repo workstreams
- Recon facts recorded (2026-10-24 day-file):
  - Three breaking changes identified: plugin API, /api/* contracts,
    and `tui.json` → `cli.json`.

## Rough architecture

- Not recorded as a design artifact in the register entry.
- Surfaces in scope, per the directive and launch record:
  - sandbox OpenCode server
  - laptop standalone OpenCode
  - repo configs
  - plugins (see open follow-ups — ports were left unstarted)
- Named exception at launch: OpenChamber's embedded v1.

## Status

- Design status in register: decided (directive).
- Implementation status in register: partial — core shipped, with one
  still-live follow-up (below).
- Tracking: NEVER-ISSUED — searched "opencode 2 migration fleet" and
  "v2 migration" in the opencode fork. The record says an origin tracking
  issue was part of coordinator 3's launch, but no such issue is findable.
- Triage: (b) DONE-WITHOUT-ISSUE for the migration core, with one
  (a) STILL-LIVE follow-up.

## Next steps — immediate / near-term / long-term

- Immediate: complete the still-live follow-up below (plugin ports +
  fork rebase) — tracked separately, see Links.
- Near-term: not recorded beyond that follow-up.
- Long-term: not recorded.

## Open follow-ups

- STILL-LIVE follow-up:
  - v1 plugin ports: mcpx bridge, mcpx-session, subtask2.
  - The fork rebase.
  - The register states these were "explicitly left unstarted in the
    record" and that no later slice records them done.
  - Tracked separately in: developing-today-forks/opencode#2290
    (named in the tasking for this document; the register entry itself
    places this follow-up in its filing manifest, §7).

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entry: F3.1 in `~/workspace/design-audit-60d/REGISTER.md`
- Still-live follow-up: developing-today-forks/opencode#2290
- Related register entries: F3 (OpenCode platform, parent),
  F3.2 (one shared OpenCode serve), F2 (mcpx platform program)
- Tracking epic: developing-today/code#8805
- Design artifact: none found; recon facts in the 2026-10-24 day-file
