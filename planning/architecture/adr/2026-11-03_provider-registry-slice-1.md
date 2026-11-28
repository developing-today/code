# ADR: Derive provider readers from one descriptive registry

- **Date:** 2026-11-03
- **Status:** accepted
- **Supersedes:** none
- **Related:** [F12.4 Provider Registry — Slice 1](../../../doc/2026-11-03T10-06-40Z_architecture_provider_registry_slice1/2026-11-03T10-06-40Z_architecture_provider_registry_slice1.md); [#1061](https://github.com/developing-today/code/issues/1061); [#838](https://github.com/developing-today/code/issues/838); [#4656](https://github.com/developing-today/code/pull/4656)

## Context

The capability catalog, ledger, compile gate, and CLI risked drifting as
separately maintained views. After merged PR #4656 established `CatalogEntry`, `CATALOG_ENTRIES`, and the
narrower `default_ceiling_names()` seam, a second registry would duplicate
authority. Issue #838 was the safety gate preventing registered providers
from widening existing world ceilings. The initial task packet
also predated that catalog and is not the settled design.

## Decision

Extend the existing `CatalogEntry`/`CATALOG_ENTRIES` seam from merged PR
#4656, statically registering the current 17 capabilities only; do not add
providers or dynamic loading in this slice. Record provider names,
versions, argument schemas, bounds, required scopes, and default ceilings as
descriptive manifests, and derive the legacy catalog, ledger, compile-gate,
and CLI readers from that registry. Preserve the distinction between the
full catalog and `default_ceiling_names()`; every default path must use the
narrower guard so registering a provider cannot widen world ceilings. Keep
request, subscription, and
platform-module manifest boundaries distinct; metadata does not move
validation or dispatch from their existing boundaries, and parser `wants`
limits remain global.

## Consequences

Readers share one inventory without changing accepted requests, refusal
codes, or execution behavior. The default-ceiling API remains intentionally
narrower than the explicit catalog, and future providers or dynamic loading
require separate decisions under the broader provider-registry work
([#1061](https://github.com/developing-today/code/issues/1061); later F12.4
slice 2). Slice 1 does not authorize a behavioral change to validation,
dispatch, or parser limits.
