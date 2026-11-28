## Problem and goal
Represent initial content ID as raw exact-byte Iroh BLAKE3-256 while retaining profile token and legacy binary/text forms.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9120--content-address-core (section: **#9120 — content-address core**).
Tier: **crucial**.
Statement IDs: S3.4, S3.5, S3.6, S3.7, S3.8, S3.9, S3.10, S4.19, S4.20, S4.21, S4.22
Enables: Portable immutable content core with no metadata or location in identity.

## Scope and ownership
Parent: #9120 (native component/meta parent).
- `pkgs/id/crates/id-core/src/content_id.rs` (new; owner: A9120-content-id)
- `pkgs/id/crates/id-core/tests/content_id_vectors.rs` (new; owner: A9120-content-id)
Hard dependencies (native blockers): A9113-shape-types — Requires typed ContentId/ArrayId shape variants, shape-version tag and unsupported-critical-shape error from A9113-shape-types.; A9113-profile-dispatch — Requires static/configured profile-token dispatch and typed unsupported-profile result from A9113-profile-dispatch.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/content_id_vectors.rs` first. Proposed command: `cargo test -p id-core --test content_id_vectors`
Oracle: **golden** — expected `Pinned Iroh Hash::new implementation and published golden bytes; source profile mismatch is explicit error.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the new content writer/admission route; keep legacy content readers and all previously verified content-addresses intact. Never delete or rehash data as rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9120-content-id -->
