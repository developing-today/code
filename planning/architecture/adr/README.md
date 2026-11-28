# Architecture decision records (ADRs)

An ADR records one decision: the context, the decision, and its
consequences. ADRs here complement the dated design records in
[`doc/`](../../../doc/README.md) — a major change still gets a dated
record per [`doc/DOCUMENTATION_PROTOCOL.md`](../../../doc/DOCUMENTATION_PROTOCOL.md);
an ADR is for a decision that is smaller than a design, or that
revises an earlier record without rewriting it.

## Rules

- One file per decision: `adr/<UTC date>_<slug>.md`, from
  [TEMPLATE.md](TEMPLATE.md).
- An accepted ADR is not edited. A decision that changes is a new ADR
  that supersedes the old one; both files say so, and this index marks
  the superseded one.
- Every ADR is listed below in the PR that adds it.

## Index

- [2026-03-29 — Put tests at the closest layer with a decisive oracle](2026-03-29_testing-at-the-closest-decidable-layer.md) — accepted; [source](../../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md)
- [2026-10-07 — Run Roc worlds as server-authoritative, mediated programs](2026-10-07_server-authoritative-roc-worlds.md) — accepted; [source](../../../doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- [2026-10-14 — Separate world presentation from transport and bound host effects](2026-10-14_portable-world-surfaces-and-bounded-effects.md) — accepted; [source](../../../doc/2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md)
- [2026-10-17 — Establish trusted foundations before expanding platform flexibility](2026-10-17_foundations-before-platform-flexibility.md) — accepted; [source](../../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [2026-11-01 — Split `pkgs/id` at low-dependency module seams without changing callers](2026-11-01_id-crate-split-phase-1.md) — accepted; [source](../../../doc/2026-11-01T15-42-00Z_architecture_id_crate_split/2026-11-01T15-42-00Z_architecture_id_crate_split.md)
- [2026-11-03 — Derive provider readers from one descriptive registry](2026-11-03_provider-registry-slice-1.md) — accepted; [source](../../../doc/2026-11-03T10-06-40Z_architecture_provider_registry_slice1/2026-11-03T10-06-40Z_architecture_provider_registry_slice1.md)
