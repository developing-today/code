## Problem and goal
Expand a bare UTF-8 string into slots using required configured default index; missing policy is an error.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9119--shorthand-and-streaming-query-api (section: **#9119 — shorthand and streaming query API**).
Tier: **recommended**.
Statement IDs: S4.29, S4.30, S4.31, S4.32, S4.33
Enables: Compact human query without hard-coding example index 2.

## Scope and ownership
Parent: #9119 (native component/meta parent).
- `pkgs/id/crates/id-core/src/array_shorthand.rs` (new; owner: A9119-shorthand-policy)
- `pkgs/id/crates/id-core/tests/array_shorthand.rs` (new; owner: A9119-shorthand-policy)
Hard dependencies (native blockers): A9116-registry-record — Requires namespace definition bound to exactly the content-addressed or array-addressed core ID from A9116-registry-record.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/array_shorthand.rs` first. Proposed command: `cargo test -p id-core --test array_shorthand`
Oracle: **model** — expected `Pure expansion function compared with configured index reference; no fallback index exists.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the shorthand/paging endpoint and keep the underlying array records unchanged; invalidate only cursors issued under the removed policy with an explicit typed error.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9119-shorthand-policy -->
