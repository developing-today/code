# ADR: Establish trusted foundations before expanding platform flexibility

- **Date:** 2026-10-17
- **Status:** accepted
- **Supersedes:** none
- **Related:** [Next stage of the platform: foundations, then flexibility](../../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)

## Context

Feature work depends on trustworthy CI and observable runtime behavior. The
source record identifies test setup failures and missing measurements as
foundational risks, and names rollback and host-reach boundaries that should
not be deferred until after new journal entries or effects ship. The source
record states the principles but does not enumerate alternatives; this ADR
frames the tradeoff as expanding features in parallel with foundation repair,
which makes failures harder to attribute and recovery less reliable, versus
optimizing before measuring, which leaves no trustworthy baseline.

## Decision

Repair and trust the foundation before feature expansion: CI repair is a
prerequisite, and performance work is measured before optimization. Keep one
authoritative client projection, require a versioned format/check/backup
rollback path before introducing a journal entry type, prefer data registries
and manifests for extensibility, and make every new capability ceilinged,
audited, and off by default.

## Consequences

Feature sequencing may be slower until the checks and recovery path are
reliable, but changes have a measurable baseline and a defined rollback
route. Extensibility can be added through data without creating a second
authority for visibility. New effects cannot gain reach merely through a
configuration switch; longer-term protocol choices explicitly left open in
the source record remain undecided. Follow-up: repair and validate the CI and
rollback foundations in [Phase 11](../../../.opencode/plans/phase-11-foundations.md) before the extensibility work in [Phase 12](../../../.opencode/plans/phase-12-platform-flexibility.md), including journal safeguards ([#227](https://github.com/developing-today/code/issues/227), [#244](https://github.com/developing-today/code/issues/244)) and the measured fuel work ([#247](https://github.com/developing-today/code/pull/247), [#248](https://github.com/developing-today/code/pull/248)); CI deadline adjustment is tracked by [#224](https://github.com/developing-today/code/issues/224).
