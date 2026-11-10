# C010: F12.14 portals bilateral cross-world messaging by mutual grant — D1-032

[Main reference](../main.md#c010)

## Relevant transcript excerpts

<!-- workflow:excerpt C010 59 63 -->

[Transcript lines 59-63](../transcript.txt#L59-L63)

```text
- [ ] **D1-032 — F12.14 Portals: cross-world messaging granted by both admins** (L, design)
  A capability both worlds' admins grant, with bounded rate and size and an audit entry on each side.
  - Status: not started. Only host-level cross-world write toggles exist (`CrossWorldWrite`/`ForceCrossWorldWrite` in the WorldCommand enum, cli.rs:1388); there is no guest-visible portal capability in CATALOG (world_caps.rs:55-73).
  - Source: .opencode/plans/phase-12-platform-flexibility.md:56; doc/2026-10-10T04-59-57Z_feature_cross_world_access/
  - Prerequisites: D1-022 F12.4 Capability provider registry (manifested components) (in this list), D1-024 F12.6 Structured audit log with OpenTelemetry export (in this list)
```

## Normalized purpose

Intake for portals as a new bilateral guest capability with bounded rate/size and audit entries on both worlds. Reconciles open #1068 and keeps the existing host-level cross-world switch out of scope.

## Initial thoughts

- Distinguish the new guest capability from the host policy switch at intake; do not broaden the host switch.
- Carry the registry (F12.4) and audit (F12.6) implementation dependencies without blocking portal research.

## Inferred suggestions

- Inference: frame bilateral grant and dual audit entries as the minimum viable portal contract for S2.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
