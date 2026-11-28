## Problem and goal
Order kinds by explicit stable type tag and compare values within kind without materializing huge exponent powers.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9122--numeric-registry-family (section: **#9122 — Numeric registry family**).
Tier: **recommended**.
Statement IDs: S4.69, S4.70, S4.71, S4.72, S4.73
Enables: Correct range scans over large numeric values without coercion.

## Scope and ownership
Parent: #9122 (native component/meta parent).
- `pkgs/id/crates/id-core/src/numeric_order.rs` (new; owner: A9122-numeric-order)
- `pkgs/id/crates/id-core/tests/numeric_order_prop.rs` (new; owner: A9122-numeric-order)
Hard dependencies (native blockers): A9122-numeric-encoding — Requires normalized arbitrary-precision integer/decimal/binary-float wire values from A9122-numeric-encoding.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/numeric_order_prop.rs` first. Proposed command: `cargo test -p id-core --test numeric_order_prop`
Oracle: **invariant** — expected `The reference comparator orders negatives by descending absolute magnitude, then canonical zero, then positives by ascending magnitude; type tag orders mixed kinds first, and exponent comparison terminates or returns resource_limit without allocating exponent-sized powers.`; verifier: `Compare generated cases with an exact arbitrary-precision reference model and fixed negative/zero/positive, cross-sign, mixed-kind, large-exponent and allocation-budget vectors; assert -10 < -2 < 0 < 2 < 10 and the specified failure class.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new numeric writes under this profile; retain its decoder for stored values and do not coerce or rewrite existing keys during rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9122-numeric-order -->
