## Problem and goal
Parse required lowercase resolver alias, UTF-8 namespace, v1 shape and typed array/content components without normalization or double decode.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9117--uri-syntax-id-and-encoding (section: **#9117 — URI syntax (`id://`) and encoding**).
Tier: **crucial**.
Statement IDs: S4.11, S4.12, S4.13, S4.14, S4.15, S4.16, S4.17, S4.18
Enables: Unambiguous RFC 3986-compatible local URI resolution.

## Scope and ownership
Parent: #9117 (native component/meta parent).
- `pkgs/id/crates/id-core/src/id_uri_parse.rs` (new; owner: A9117-uri-parser)
- `pkgs/id/crates/id-core/tests/id_uri_parse.rs` (new; owner: A9117-uri-parser)
Hard dependencies (native blockers): A9116-registry-record — Requires namespace definition bound to exactly the content-addressed or array-addressed core ID from A9116-registry-record.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; A9120-content-id — Requires raw exact-byte Iroh BLAKE3-256 ContentId type and fixed legacy-compatible text/binary parser from A9120-content-id.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/id_uri_parse.rs` first. Proposed command: `cargo test -p id-core --test id_uri_parse`
Oracle: **invariant** — expected `RFC 3986 component parse followed by exactly one field-specific decode and exact grammar validation.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting this URI encoding and retain its parser for already published links; fall back only to an explicitly selected prior syntax, never global lookup or silent reinterpretation.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9117-uri-parser -->
