# ADR: Put tests at the closest layer with a decisive oracle

- **Date:** 2026-03-29
- **Status:** accepted
- **Supersedes:** none
- **Related:** [Testing Architecture](../../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md)

## Context

The repository has distinct Rust, TypeScript, browser, deployment, and Nix
surfaces. A single test layer cannot give fast feedback for pure logic and
also prove real user-facing or deployment behavior. The reference record catalogs these layers and their coverage; it does not
record an explicit alternatives analysis. As this ADR's synthesis, the tradeoff
is between relying mainly on narrow unit tests, which miss cross-system
behavior, and pushing coverage into end-to-end tests, which is slower and
often depends on host state.

## Decision

Place each test at the closest layer that can decisively check its behavior:
unit tests for pure logic, integration or CLI tests for command behavior,
TypeScript tests for frontend logic, Playwright for interactive web behavior,
and VM/deployment tests for system integration. Keep CI checks hermetic;
retain local full end-to-end coverage for cases that depend on host state.

## Consequences

Contributors can choose a test layer using the behavior under test rather
than defaulting to one broad suite. Fast, hermetic checks remain suitable for
CI, while local end-to-end coverage can catch cross-system behavior that a
sandbox cannot faithfully exercise. No layer is a substitute for another
when its oracle cannot decide the behavior in question.
