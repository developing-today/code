## Problem and goal
Record exact Iroh raw BLAKE3-256 vectors and separately pin legacy SHA-256 artifact serialization/signature bytes, including the docs/code preimage discrepancy.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9113--extensible-id-shapes-and-content-portability (section: **#9113 — Extensible ID shapes and content portability**).
Tier: **crucial**.
Statement IDs: S3.4, S3.5, S3.6, S3.7, S3.8, S3.9, S3.10
Enables: Safe new profile parsing without silently changing identifiers or re-signing old artifacts.

## Scope and ownership
Parent: #9113 (native component/meta parent).
- `pkgs/id/crates/id-core/tests/legacy_hash_vectors.rs` (new; owner: A9113-legacy-hash-vectors)
- `pkgs/id/crates/id-sync/tests/legacy_artifact_vectors.rs` (new; owner: A9113-legacy-hash-vectors)
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/legacy-artifact-compatibility.md` (new; owner: A9113-legacy-hash-vectors)
Hard dependencies (native blockers): A9113-profile-dispatch — Requires static/configured profile-token dispatch and typed unsupported-profile result from A9113-profile-dispatch.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/legacy_hash_vectors.rs` first. Proposed command: `cargo test -p id-core --test legacy_hash_vectors && cargo test -p id-sync --test legacy_artifact_vectors`
Oracle: **golden** — expected `Known source implementation and fixed byte-level golden vectors. No serializer or signature preimage is changed by this slice.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting the new profile/shape; retain every existing parser and verifier so stored IDs remain readable. Do not rewrite or re-sign persisted identifiers.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9113-legacy-hash-vectors -->
