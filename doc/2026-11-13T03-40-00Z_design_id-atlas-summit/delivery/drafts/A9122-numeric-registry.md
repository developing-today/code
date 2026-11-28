## Problem and goal
Apply numeric-kind validation at array writes while distinguishing operational parser limits from numeric domain limits.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9122--numeric-registry-family (section: **#9122 — Numeric registry family**).
Tier: **recommended**.
Statement IDs: S4.64, S4.65, S4.66, S4.67, S4.68, S4.69, S4.70, S4.71, S4.72, S4.73
Enables: Numeric semantics without a third address core.

## Scope and ownership
Parent: #9122 (native component/meta parent).
- `pkgs/id/crates/id-store/src/numeric_registry.rs` (new; owner: A9122-numeric-registry)
- `pkgs/id/crates/id-store/tests/numeric_registry.rs` (new; owner: A9122-numeric-registry)
Hard dependencies (native blockers): A9116-registry-state — Requires versioned local registry read/mutate operations and mutation authorization check from A9116-registry-state.; A9122-numeric-order — Requires type-first exact comparator with negative magnitude reversal and bounded-work exponent compare from A9122-numeric-order.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/numeric_registry.rs` first. Proposed command: `cargo test -p id-store --test numeric_registry`
Oracle: **reference** — expected `Namespace policy plus NumericValue validator; no max magnitude/precision semantic rule.`; verifier: `Run the named independent reference procedure on the same inputs and compare typed outputs/errors exactly.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new numeric writes under this profile; retain its decoder for stored values and do not coerce or rewrite existing keys during rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9122-numeric-registry -->
