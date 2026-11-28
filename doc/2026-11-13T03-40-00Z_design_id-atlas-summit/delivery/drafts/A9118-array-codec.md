## Problem and goal
Encode/decode zero-or-more Option<Bytes> slots, preserving null versus empty bytes and slot order.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9118--array-address-core (section: **#9118 — Array-address core**).
Tier: **crucial**.
Statement IDs: S4.23, S4.24, S4.25, S4.26, S4.27, S4.28
Enables: General array-address core semantics and URI slot payload model.

## Scope and ownership
Parent: #9118 (native component/meta parent).
- `pkgs/id/crates/id-core/src/array_address.rs` (new; owner: A9118-array-codec)
- `pkgs/id/crates/id-core/tests/array_address.rs` (new; owner: A9118-array-codec)
Hard dependencies (native blockers): 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/array_address.rs` first. Proposed command: `cargo test -p id-core --test array_address`
Oracle: **golden** — expected `Typed reference vector and byte-for-byte canonical round trip; values with different null/empty structure never collide.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop new writes using this array codec/order profile; retain the decoder and stored bytes until an explicit migration proves round-trip equivalence.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9118-array-codec -->
