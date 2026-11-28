# ADR: Split `pkgs/id` at low-dependency module seams without changing callers

- **Date:** 2026-11-01
- **Status:** accepted
- **Supersedes:** none
- **Related:** [`pkgs/id` crate split: Phase 1 architecture](../../../doc/2026-11-01T15-42-00Z_architecture_id_crate_split/2026-11-01T15-42-00Z_architecture_id_crate_split.md); [crate split design](../../../doc/2026-11-01T16-53-00Z_design_id_crate_split/2026-11-01T16-53-00Z_design_id_crate_split.md); [crate-split follow-on mapping](../../../doc/2026-11-04_id-program-essay-answers.md); [#3447](https://github.com/developing-today/code/issues/3447); [#3471](https://github.com/developing-today/code/pull/3471); [#3195](https://github.com/developing-today/code/issues/3195)

## Context

The `id` compilation unit is large, but changing its public paths, CLI
behavior, wire formats, or test coverage would make a compilation-boundary
refactor a behavior change. The source record identifies low-dependency leaf
modules as a separable seam while keeping the binary and root-only
construction in the existing package.

## Decision

Keep `pkgs/id` as the package and workspace root and extract the selected
low-dependency leaves into an `id-core` workspace member. Move modules rather
than copy them; move leaf-owned types with their defining module, and keep
root-only construction behind narrow interfaces. Re-export extracted items
at their historical `crate::` paths and explicitly forward relevant feature
selections with defaults disabled. Keep test utilities dev-only, include both
crates in workspace checks, and update freshness/build guards to account for
the new crate boundary. As an ADR framing of the tradeoff (the source record
does not enumerate alternatives), extracting without historical-path
re-exports would break callers; extracting more modules in Phase 1 would
expand risk beyond the low-dependency seam.

## Consequences

The dependency seam can be compiled and checked independently while existing
callers, CLI output, wire formats, and tests remain stable. The workspace, feature forwarding, build freshness, and shipped-dependency
guards must all recognize both crates. Validation commitments are formatting,
all-features workspace tests with unchanged test count, unchanged shipped
binary and `id --help`, the existing Nix build, and cold Cargo/Nix build-time
measurements using [issue #3195](https://github.com/developing-today/code/issues/3195)
as the method. Follow-up: the separately queued `id-worlds` extraction
([#4408](https://github.com/developing-today/code/issues/4408); [source record](../../../doc/2026-11-01T15-42-00Z_architecture_id_crate_split/2026-11-01T15-42-00Z_architecture_id_crate_split.md)) remains out of scope; later sync, groups, web, and binary-facing extractions require separate decisions.
