## Problem and goal
Update head only from expected generation and snapshot digest; refuse unsupported multiwriter adapters.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9114--complete-metadata-sidecars-and-mutable-head (section: **#9114 — Complete metadata sidecars and mutable head**).
Tier: **crucial**.
Statement IDs: S3.15, S3.16, S3.17, S3.18, S3.19, S3.20
Enables: Lost-update safe current snapshot pointer.

## Scope and ownership
Parent: #9114 (native component/meta parent).
- `pkgs/id/crates/id-store/src/sidecar_head.rs` (new; owner: A9114-cas-head)
- `pkgs/id/crates/id-store/tests/sidecar_head_model.rs` (new; owner: A9114-cas-head)
Hard dependencies (native blockers): A9114-sidecar-snapshot — Requires complete scoped signed sidecar snapshot record with payload/envelope IDs, parent and generation from A9114-sidecar-snapshot.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/sidecar_head_model.rs` first. Proposed command: `cargo test -p id-store --test sidecar_head_model`
Oracle: **model** — expected `State machine permits one CAS per expected head; each accepted generation has exact previous snapshot parent.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new snapshot/head writes and keep the previous head readable; preserve immutable snapshots and legacy metadata readers. Rebuild the head only from verified signed parent chains.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9114-cas-head -->
