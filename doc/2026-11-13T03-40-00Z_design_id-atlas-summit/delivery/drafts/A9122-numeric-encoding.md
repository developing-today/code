## Problem and goal
Add unbounded typed numeric significand/exponent values, canonical positive zero and reject NaN/infinity in ordered profile.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9122--numeric-registry-family (section: **#9122 — Numeric registry family**).
Tier: **recommended**.
Statement IDs: S4.64, S4.65, S4.66, S4.67, S4.68, S4.69, S4.70, S4.71
Enables: Infinitely growable numeric registry over array-address core.

## Scope and ownership
Parent: #9122 (native component/meta parent).
- `pkgs/id/crates/id-core/src/numeric_value.rs` (new; owner: A9122-numeric-encoding)
- `pkgs/id/crates/id-core/tests/numeric_value_prop.rs` (new; owner: A9122-numeric-encoding)
Hard dependencies (native blockers): A9118-typed-values — Requires type-ID plus payload value representation and explicit object-codec dispatch from A9118-typed-values.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/numeric_value_prop.rs` first. Proposed command: `cargo test -p id-core --test numeric_value_prop`
Oracle: **invariant** — expected `Arbitrary precision reference integer and exact exponent/significand model.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new numeric writes under this profile; retain its decoder for stored values and do not coerce or rewrite existing keys during rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9122-numeric-encoding -->
