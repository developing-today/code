## Problem and goal
Reuse Part B B19 V2 schema and bind responder, subject digest, scope, verified_present assertion kind, policy epoch and observation time.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9115--existence-acknowledgements-location-harvester-privacy (section: **#9115 — Existence acknowledgements, location harvester, privacy**).
Tier: **crucial**.
Statement IDs: S3.23, S3.24, S3.25
Enables: Interoperable evidence without divergent receipt formats or future-retention promises.

## Scope and ownership
Parent: #9115 (native component/meta parent).
- `pkgs/id/crates/id-core/src/existence_receipt.rs` (new; owner: A9115-existence-receipt)
- `pkgs/id/crates/id-core/tests/existence_receipt.rs` (new; owner: A9115-existence-receipt)
Hard dependencies (native blockers): A9114-envelope-identities — Requires PayloadId, canonical Body, detached-signature preimage and nonrecursive EnvelopeId types from A9114-envelope-identities.; B19 — Consumes Part B B19 V2 shared receipt schema output; #9115 must not fork the receipt type.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/existence_receipt.rs` first. Proposed command: `cargo test -p id-core --test existence_receipt`
Oracle: **model** — expected `Part B B19 V2 schema/vector; the slice requires that shared contract and must not fork it.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the affected acknowledgement or derived location-index writer/query; preserve signed receipts, sidecars and tombstones so the projection can be rebuilt without losing evidence.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9115-existence-receipt -->
