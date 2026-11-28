## Problem and goal
Keep original name spelling and all authorized signed target claims; do not silently choose one winner.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9121--string-path-names-lenses-and-authority-groups (section: **#9121 — String-path names, lenses and authority groups**).
Tier: **recommended**.
Statement IDs: S4.45, S4.46, S4.47, S4.48, S4.49, S4.50, S4.51, S4.52
Enables: Ambiguity-aware name search and later lens projection.

## Scope and ownership
Parent: #9121 (native component/meta parent).
- `pkgs/id/crates/id-store/src/name_candidates.rs` (new; owner: A9121-name-candidates)
- `pkgs/id/crates/id-store/tests/name_candidates.rs` (new; owner: A9121-name-candidates)
Hard dependencies (native blockers): A9116-registry-state — Requires versioned local registry read/mutate operations and mutation authorization check from A9116-registry-state.; A9118-typed-values — Requires type-ID plus payload value representation and explicit object-codec dispatch from A9118-typed-values.; A9114-sidecar-snapshot — Requires complete scoped signed sidecar snapshot record with payload/envelope IDs, parent and generation from A9114-sidecar-snapshot.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/name_candidates.rs` first. Proposed command: `cargo test -p id-store --test name_candidates`
Oracle: **reference** — expected `Reference signed candidate multiset and configured comparison profile.`; verifier: `Run the named independent reference procedure on the same inputs and compare typed outputs/errors exactly.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the affected name projection/lens while retaining signed candidate assertions; revert query presentation without rewriting names or authority claims.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9121-name-candidates -->
