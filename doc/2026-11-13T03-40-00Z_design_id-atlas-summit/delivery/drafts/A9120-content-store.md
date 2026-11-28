## Problem and goal
Commit content only after complete exact-byte digest match; preserve existing Iroh storage adapter boundary.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9120--content-address-core (section: **#9120 — content-address core**).
Tier: **crucial**.
Statement IDs: S4.19, S4.20, S4.21, S4.22
Enables: Uniform immutable content contract without rewriting event/world stores.

## Scope and ownership
Parent: #9120 (native component/meta parent).
- `pkgs/id/crates/id-store/src/content_store.rs` (new; owner: A9120-content-store)
- `pkgs/id/crates/id-store/tests/content_store.rs` (new; owner: A9120-content-store)
Hard dependencies (native blockers): A9120-content-id — Requires raw exact-byte Iroh BLAKE3-256 ContentId type and fixed legacy-compatible text/binary parser from A9120-content-id.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/content_store.rs` first. Proposed command: `cargo test -p id-store --test content_store`
Oracle: **model** — expected `Store-state model: object exists iff full stream length and digest validate; no partial staging object is accepted.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the new content writer/admission route; keep legacy content readers and all previously verified content-addresses intact. Never delete or rehash data as rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9120-content-store -->
