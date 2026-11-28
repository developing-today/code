## Problem and goal
Add typed shape discriminator and strict version/profile parsing without changing any existing ID bytes.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9113--extensible-id-shapes-and-content-portability (section: **#9113 — Extensible ID shapes and content portability**).
Tier: **crucial**.
Statement IDs: S3.1, S3.2, S3.3
Enables: Content and array cores can share dispatch while retaining separate identity semantics.

## Scope and ownership
Parent: #9113 (native component/meta parent).
- `pkgs/id/crates/id-core/src/id_shape.rs` (new; owner: A9113-shape-types)
- `pkgs/id/crates/id-core/tests/id_shape.rs` (new; owner: A9113-shape-types)
Hard dependencies (native blockers): 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/id_shape.rs` first. Proposed command: `cargo test -p id-core --test id_shape`
Oracle: **invariant** — expected `A finite typed reference enum; every known encoding maps to exactly one variant, unknown critical tags return UnsupportedShape, and canonical encode(parse(x)) equals x.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting the new profile/shape; retain every existing parser and verifier so stored IDs remain readable. Do not rewrite or re-sign persisted identifiers.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9113-shape-types -->
