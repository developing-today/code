## Problem and goal
Store complete, scoped signed sidecar snapshots with parent generation and digest-derived snapshot ID.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9114--complete-metadata-sidecars-and-mutable-head (section: **#9114 — Complete metadata sidecars and mutable head**).
Tier: **crucial**.
Statement IDs: S3.11, S3.12, S3.13, S3.14, S3.18
Enables: Reproducible metadata versioning and location-index reconstruction.

## Scope and ownership
Parent: #9114 (native component/meta parent).
- `pkgs/id/crates/id-store/src/sidecar_snapshot.rs` (new; owner: A9114-sidecar-snapshot)
- `pkgs/id/crates/id-store/tests/sidecar_snapshot.rs` (new; owner: A9114-sidecar-snapshot)
Hard dependencies (native blockers): A9114-envelope-identities — Requires PayloadId, canonical Body, detached-signature preimage and nonrecursive EnvelopeId types from A9114-envelope-identities.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/sidecar_snapshot.rs` first. Proposed command: `cargo test -p id-store --test sidecar_snapshot`
Oracle: **model** — expected `Canonical signed-envelope digest and a model of complete scope; snapshot digest is not recursively embedded in signed Body.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new snapshot/head writes and keep the previous head readable; preserve immutable snapshots and legacy metadata readers. Rebuild the head only from verified signed parent chains.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9114-sidecar-snapshot -->
