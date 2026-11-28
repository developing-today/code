## Problem and goal
Define PayloadId over raw bytes, canonical Body over payload ID/length/domain/context, signature over domain prefix plus Body, and EnvelopeId over full signed envelope.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9114--complete-metadata-sidecars-and-mutable-head (section: **#9114 — Complete metadata sidecars and mutable head**).
Tier: **crucial**.
Statement IDs: S3.11, S3.12, S3.13, S3.14, S3.15, S3.16, S3.17, S3.18, S3.19, S3.20
Enables: Non-recursive signed snapshots and reusable receipt/assertion envelopes.

## Scope and ownership
Parent: #9114 (native component/meta parent).
- `pkgs/id/crates/id-core/src/signed_envelope.rs` (new; owner: A9114-envelope-identities)
- `pkgs/id/crates/id-core/tests/signed_envelope_identity.rs` (new; owner: A9114-envelope-identities)
Hard dependencies (native blockers): A9114-cbor-profile — Requires deterministic CBOR rules, duplicate-key rejection and cross-edition golden bytes from A9114-cbor-profile.; A9113-profile-dispatch — Requires static/configured profile-token dispatch and typed unsupported-profile result from A9113-profile-dispatch.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/signed_envelope_identity.rs` first. Proposed command: `cargo test -p id-core --test signed_envelope_identity`
Oracle: **invariant** — expected `Reference functions over canonical bytes; no signed preimage can contain the identity digest of its enclosing signed envelope.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new snapshot/head writes and keep the previous head readable; preserve immutable snapshots and legacy metadata readers. Rebuild the head only from verified signed parent chains.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9114-envelope-identities -->
