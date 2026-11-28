## Problem and goal
Bound each response page while keeping address length/result domain unbounded and exposing truncation, cancellation and cursor state.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9119--shorthand-and-streaming-query-api (section: **#9119 — shorthand and streaming query API**).
Tier: **recommended**.
Statement IDs: S4.41, S4.42, S4.43, S4.44
Enables: Streaming interfaces that do not silently truncate.

## Scope and ownership
Parent: #9119 (native component/meta parent).
- `pkgs/id/crates/id-store/src/array_query.rs` (new; owner: A9119-query-cursor)
- `pkgs/id/crates/id-store/tests/array_query_model.rs` (new; owner: A9119-query-cursor)
Hard dependencies (native blockers): A9118-array-order — Requires total null/unsigned-byte/shorter-prefix ordering comparator for array addresses from A9118-array-order.; A9116-registry-state — Requires versioned local registry read/mutate operations and mutation authorization check from A9116-registry-state.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/array_query_model.rs` first. Proposed command: `cargo test -p id-store --test array_query_model`
Oracle: **model** — expected `Reference sorted snapshot result; pages form disjoint ordered slices and cursor binds scope/query/snapshot.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the shorthand/paging endpoint and keep the underlying array records unchanged; invalidate only cursors issued under the removed policy with an explicit typed error.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9119-query-cursor -->
