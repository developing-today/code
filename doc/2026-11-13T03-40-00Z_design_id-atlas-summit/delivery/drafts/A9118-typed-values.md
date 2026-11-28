## Problem and goal
Preserve type ID plus bytes for values and require explicit codec profiles for structured objects.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9118--array-address-core (section: **#9118 — Array-address core**).
Tier: **recommended**.
Statement IDs: S4.34, S4.35, S4.36, S4.37, S4.38, S4.39, S4.40
Enables: Type-sensitive equality and safely extensible object serialization.

## Scope and ownership
Parent: #9118 (native component/meta parent).
- `pkgs/id/crates/id-core/src/typed_value.rs` (new; owner: A9118-typed-values)
- `pkgs/id/crates/id-core/tests/typed_value.rs` (new; owner: A9118-typed-values)
Hard dependencies (native blockers): A9114-cbor-profile — Requires deterministic CBOR rules, duplicate-key rejection and cross-edition golden bytes from A9114-cbor-profile.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/typed_value.rs` first. Proposed command: `cargo test -p id-core --test typed_value`
Oracle: **invariant** — expected `Typed-value equality model and declared codec reference vectors.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop new writes using this array codec/order profile; retain the decoder and stored bytes until an explicit migration proves round-trip equivalence.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9118-typed-values -->
