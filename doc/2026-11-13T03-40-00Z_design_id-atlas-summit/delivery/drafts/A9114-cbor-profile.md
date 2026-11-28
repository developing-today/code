## Problem and goal
Define deterministic CBOR profile, duplicate-key rejection, critical field behavior and cross-language canonical vectors.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9114--complete-metadata-sidecars-and-mutable-head (section: **#9114 — Complete metadata sidecars and mutable head**).
Tier: **crucial**.
Statement IDs: S3.11, S3.12, S3.13, S3.14
Enables: Stable body signatures and envelope IDs across Rust and hosted TypeScript.

## Scope and ownership
Parent: #9114 (native component/meta parent).
- `pkgs/id/crates/id-core/src/canonical_cbor.rs` (new; owner: A9114-cbor-profile)
- `pkgs/id/crates/id-core/tests/canonical_cbor_vectors.rs` (new; owner: A9114-cbor-profile)
- `pkgs/id/hosted-worker/src/canonical-cbor.ts` (new; owner: A9114-cbor-profile)
- `pkgs/id/hosted-worker/test/canonical-cbor.test.ts` (new; owner: A9114-cbor-profile)
Hard dependencies (native blockers): 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/canonical_cbor_vectors.rs` first. Proposed command: `cargo test -p id-core --test canonical_cbor_vectors && cd pkgs/id/hosted-worker && bun test canonical-cbor.test.ts`
Oracle: **golden** — expected `Independent reference vectors and byte equality across Rust/TypeScript; decoder rejects any ambiguous signed encoding.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new snapshot/head writes and keep the previous head readable; preserve immutable snapshots and legacy metadata readers. Rebuild the head only from verified signed parent chains.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9114-cbor-profile -->
