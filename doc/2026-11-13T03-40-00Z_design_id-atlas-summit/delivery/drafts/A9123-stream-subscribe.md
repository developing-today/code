## Problem and goal
Subscribe/resume by chain cursor and return typed completion target with reauthorization rather than implicit redirect.

## Version and design basis
Version: V3. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9123--addressable-live-streams (section: **#9123 — addressable live streams**).
Tier: **recommended**.
Statement IDs: S4.79, S4.80, S4.81, S4.82
Enables: Live and replay consumers with explicit lifecycle outcomes.

## Scope and ownership
Parent: #9123 (native component/meta parent).
- `pkgs/id/crates/id-store/src/stream_subscribe.rs` (new; owner: A9123-stream-subscribe)
- `pkgs/id/crates/id-store/tests/stream_subscribe.rs` (new; owner: A9123-stream-subscribe)
Hard dependencies (native blockers): A9123-stream-records — Requires signed stream descriptor/chunk-chain types and CAS terminal lifecycle transition from A9123-stream-records.; A9119-query-cursor — Requires snapshot-bound max-items page operation and scope/query/sort-bound opaque cursor from A9119-query-cursor.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-store/tests/stream_subscribe.rs` first. Proposed command: `cargo test -p id-store --test stream_subscribe`
Oracle: **model** — expected `Reference stream chain prefix/suffix model plus current authorization result.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop stream creation/subscription for the affected protocol revision; retain signed chunk chains and terminal records for replay/read-only recovery.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9123-stream-subscribe -->
