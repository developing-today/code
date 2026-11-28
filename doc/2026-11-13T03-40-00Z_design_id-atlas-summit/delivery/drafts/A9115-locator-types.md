## Problem and goal
Represent locations by typed provider and scoped locator fields rather than opaque untyped path strings.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9115--existence-acknowledgements-location-harvester-privacy (section: **#9115 — Existence acknowledgements, location harvester, privacy**).
Tier: **crucial**.
Statement IDs: S3.21, S3.22, S3.26, S3.27, S3.28, S3.29, S3.30
Enables: Safe location resolution, privacy filtering and backend-specific freshness handling.

## Scope and ownership
Parent: #9115 (native component/meta parent).
- `pkgs/id/crates/id-core/src/location_ref.rs` (new; owner: A9115-locator-types)
- `pkgs/id/crates/id-core/tests/location_ref.rs` (new; owner: A9115-locator-types)
Hard dependencies (native blockers): A9113-shape-types — Requires typed ContentId/ArrayId shape variants, shape-version tag and unsupported-critical-shape error from A9113-shape-types.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): 9084 — Coordinate the reference/copy adapter seam before integration; this slice makes no reference-versus-copy decision.

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/location_ref.rs` first. Proposed command: `cargo test -p id-core --test location_ref`
Oracle: **invariant** — expected `Typed enum reference model; no coercion between locator kinds and no scope-less locator accepted.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the affected acknowledgement or derived location-index writer/query; preserve signed receipts, sidecars and tombstones so the projection can be rebuilt without losing evidence.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9115-locator-types -->
