## Problem and goal
Expose historical/current verification dimensions and define key rotation/revocation resolution seam without deciding SSO.

## Version and design basis
Version: V4. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9124--signer-local-worlds-and-horizons (section: **#9124 — signer-local worlds and horizons**).
Tier: **contingent**.
Statement IDs: S4.93
Enables: Auditable world horizon and compromise response.

## Scope and ownership
Parent: #9124 (native component/meta parent).
- `pkgs/id/crates/id-core/src/world_key_status.rs` (new; owner: A9124-world-key-history)
- `pkgs/id/crates/id-core/tests/world_key_status_model.rs` (new; owner: A9124-world-key-history)
Hard dependencies (native blockers): A9124-world-definition — Requires signed immutable signer-local world definition and coordinate profile from A9124-world-definition.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/world_key_status_model.rs` first. Proposed command: `cargo test -p id-core --test world_key_status_model`
Oracle: **model** — expected `Explicit key history state machine; cryptographic validity, currentness and verifier trust are independent outputs.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new world definitions/current-authority decisions; preserve signed historical definitions and report unknown current key status as not currently authorized.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9124-world-key-history -->
