## Problem and goal
Dispatch self-describing profile tokens through static/configured code; prohibit per-object translation maps.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9113--extensible-id-shapes-and-content-portability (section: **#9113 — Extensible ID shapes and content portability**).
Tier: **crucial**.
Statement IDs: S3.2, S3.3, S3.4, S3.5, S3.6, S3.7, S3.8, S3.9, S3.10
Enables: Portable IDs without requiring an extra runtime registry lookup.

## Scope and ownership
Parent: #9113 (native component/meta parent).
- `pkgs/id/crates/id-core/src/hash_profile.rs` (new; owner: A9113-profile-dispatch)
- `pkgs/id/crates/id-core/tests/hash_profile.rs` (new; owner: A9113-profile-dispatch)
Hard dependencies (native blockers): A9113-shape-types — Requires typed ContentId/ArrayId shape variants, shape-version tag and unsupported-critical-shape error from A9113-shape-types.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/hash_profile.rs` first. Proposed command: `cargo test -p id-core --test hash_profile`
Oracle: **invariant** — expected `Profile token selects exactly one compiled/configured verifier; all unsupported tokens reject before content processing.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting the new profile/shape; retain every existing parser and verifier so stored IDs remain readable. Do not rewrite or re-sign persisted identifiers.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9113-profile-dispatch -->
