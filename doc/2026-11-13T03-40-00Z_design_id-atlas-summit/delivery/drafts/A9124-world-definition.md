## Problem and goal
Sign immutable world definition over array coordinates, root reference, profile, signer and version with signer-local authority semantics.

## Version and design basis
Version: V4. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9124--signer-local-worlds-and-horizons (section: **#9124 — signer-local worlds and horizons**).
Tier: **contingent**.
Statement IDs: S4.83, S4.84, S4.85, S4.86, S4.87, S4.88, S4.89, S4.90, S4.91, S4.92
Enables: Portable coordinate resolution without global registry or protocol monopoly.

## Scope and ownership
Parent: #9124 (native component/meta parent).
- `pkgs/id/crates/id-core/src/world_definition.rs` (new; owner: A9124-world-definition)
- `pkgs/id/crates/id-core/tests/world_definition.rs` (new; owner: A9124-world-definition)
Hard dependencies (native blockers): A9114-envelope-identities — Requires PayloadId, canonical Body, detached-signature preimage and nonrecursive EnvelopeId types from A9114-envelope-identities.; A9116-registry-state — Requires versioned local registry read/mutate operations and mutation authorization check from A9116-registry-state.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; A9120-content-store — Requires immutable content adapter that commits only after complete digest validation from A9120-content-store.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/world_definition.rs` first. Proposed command: `cargo test -p id-core --test world_definition`
Oracle: **model** — expected `COSE/reference signature vectors and world definition model; valid signature never implies global trust.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new world definitions/current-authority decisions; preserve signed historical definitions and report unknown current key status as not currently authorized.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9124-world-definition -->
