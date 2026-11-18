# ADR: Separate world presentation from transport and bound host effects

- **Date:** 2026-10-14
- **Status:** accepted
- **Supersedes:** none
- **Related:** [Portable world surfaces and controlled execution profiles](../../../doc/2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md); [World presentation, rendering, and fuel](../../../doc/2026-10-31T17-47-20Z_design_world_presentation_and_fuel/2026-10-31T17-47-20Z_design_world_presentation_and_fuel.md)

## Context

World programs may be presented by terminals, browsers, or other transports.
The alternatives were transport-specific ANSI/HTML/CSS, which couple guests
to one renderer, or arbitrary guest markup, which bypasses host validation.
Likewise, interpreting “direct control” as unrestricted host access would
exceed the intended operator-controlled capability boundary.

## Decision

Use a validated, versioned presentation value as the program-to-transport
seam. The guest emits structured rows and style spans (with legacy text
fallback); the host validates bounds and shape, then each transport renders
the same model. Keep effects explicit and named under per-world ceilings and
validated limits. The planned source-only policy accepts source for the configured trusted
compiler when enabled; it is not yet implemented. Raw Wasm remains a
trusted-artifact path. A configuration
toggle does not grant ambient filesystem, network, or process access; any
future direct host integration requires a separately audited least-privilege
adapter.

## Consequences

Multiple presenters can share one validated guest model, and legacy
text-only consumers continue to work. The host owns parsing, validation, and
rendering adapters. Follow-up work includes P9.2 input events and P9.3 source-only mode,
compiler provenance, and per-world profiles ([Phase 9 plan](../../../.opencode/plans/phase-9-portable-platform.md); [source record](../../../doc/2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md)). New host effects require explicit review and mediation; compiler identity/version provenance is not solved here.
