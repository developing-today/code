## Problem and goal
Persist local registry generations and authorize mutations without merging unrelated ID world/group authority.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9116--local-namespace-registry (section: **#9116 — Local namespace registry**).
Tier: **crucial**.
Statement IDs: S4.2, S4.3, S4.4, S4.5, S4.6, S4.7, S4.8, S4.9, S4.10, S4.18
Enables: Version-aware, auditable local namespace configuration.

## Scope and ownership
Parent: #9116 (native component/meta parent).
- `pkgs/id/crates/id-store/src/namespace_registry.rs` (new; owner: A9116-registry-state)
- `pkgs/id/crates/id-store/tests/namespace_registry_model.rs` (new; owner: A9116-registry-state)
Hard dependencies (native blockers): A9116-registry-record — Requires namespace definition bound to exactly the content-addressed or array-addressed core ID from A9116-registry-record.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/namespace_registry_model.rs` first. Proposed command: `cargo test -p id-store --test namespace_registry_model`
Oracle: **model** — expected `Registry state machine; mutation requires injected authorized context and increments one generation.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new registry mutations and resolve through the last verified registry generation; preserve existing Iroh namespace registries and IDs unchanged.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9116-registry-state -->
