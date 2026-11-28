## Problem and goal
Model addressable open stream with signed chained immutable chunks and one CAS terminal transition.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9123--addressable-live-streams (section: **#9123 — addressable live streams**).
Tier: **recommended**.
Statement IDs: S4.74, S4.75, S4.76, S4.77, S4.78, S4.79, S4.80, S4.81, S4.82
Enables: Text/chat/voice/video streams that are addressable while in progress.

## Scope and ownership
Parent: #9123 (native component/meta parent).
- `pkgs/id/crates/id-core/src/stream_record.rs` (new; owner: A9123-stream-records)
- `pkgs/id/crates/id-core/tests/stream_lifecycle_model.rs` (new; owner: A9123-stream-records)
Hard dependencies (native blockers): A9114-envelope-identities — Requires PayloadId, canonical Body, detached-signature preimage and nonrecursive EnvelopeId types from A9114-envelope-identities.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; A9120-content-id — Requires raw exact-byte Iroh BLAKE3-256 ContentId type and fixed legacy-compatible text/binary parser from A9120-content-id.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/stream_lifecycle_model.rs` first. Proposed command: `cargo test -p id-core --test stream_lifecycle_model`
Oracle: **model** — expected `Finite lifecycle model with chain sequence/hash invariants; exactly one terminal record.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop stream creation/subscription for the affected protocol revision; retain signed chunk chains and terminal records for replay/read-only recovery.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9123-stream-records -->
