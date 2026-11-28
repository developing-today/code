## Problem and goal
Filter unauthorized locator values and counts before ranking, caching or returning results.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9115--existence-acknowledgements-location-harvester-privacy (section: **#9115 — Existence acknowledgements, location harvester, privacy**).
Tier: **crucial**.
Statement IDs: S3.21, S3.22, S3.26, S3.27, S3.28, S3.29, S3.30, S3.31, S3.32, S3.33, S3.34, S3.35, S3.36, S3.37, S3.38
Enables: Privacy-safe location harvesting in shared storage deployments.

## Scope and ownership
Parent: #9115 (native component/meta parent).
- `pkgs/id/crates/id-store/src/location_policy.rs` (new; owner: A9115-privacy-filter)
- `pkgs/id/crates/id-store/tests/location_privacy.rs` (new; owner: A9115-privacy-filter)
Hard dependencies (native blockers): A9115-complete-harvest — Requires complete per-scope fold of validated locations, source-version refs and signed stale/delete tombstones from A9115-complete-harvest.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/location_privacy.rs` first. Proposed command: `cargo test -p id-store --test location_privacy`
Oracle: **model** — expected `Authorization model: output/candidate counts are functions only of the authorized visible subset.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the affected acknowledgement or derived location-index writer/query; preserve signed receipts, sidecars and tombstones so the projection can be rebuilt without losing evidence.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9115-privacy-filter -->
