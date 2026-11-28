## Problem and goal
Project candidate sets through local override, popularity, cohort, standing and selected authority-group lenses with deterministic tie-breaks.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9121--string-path-names-lenses-and-authority-groups (section: **#9121 — String-path names, lenses and authority groups**).
Tier: **recommended**.
Statement IDs: S4.53, S4.54, S4.55, S4.56, S4.57, S4.58, S4.59, S4.60, S4.61, S4.62, S4.63
Enables: User-selectable name resolution without a global naming authority.

## Scope and ownership
Parent: #9121 (native component/meta parent).
- `pkgs/id/crates/id-store/src/name_lens.rs` (new; owner: A9121-lenses-groups)
- `pkgs/id/crates/id-store/tests/name_lens_model.rs` (new; owner: A9121-lenses-groups)
Hard dependencies (native blockers): A9121-name-candidates — Requires signed collision-preserving name-to-target candidate set from A9121-name-candidates.; A9115-existence-receipt — Requires B19 verified_present receipt binding and separate inventory-only stored_unverified assertion from A9115-existence-receipt.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/name_lens_model.rs` first. Proposed command: `cargo test -p id-store --test name_lens_model`
Oracle: **model** — expected `Pure lens function over authorized candidate/evidence model; deterministic ordering for same policy version.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the affected name projection/lens while retaining signed candidate assertions; revert query presentation without rewriting names or authority claims.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9121-lenses-groups -->
