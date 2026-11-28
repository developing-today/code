## Problem and goal
Use the stdlib reference codec's deterministic random cases and malformed corpus as Rust/TypeScript conformance vectors.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9117--uri-syntax-id-and-encoding (section: **#9117 — URI syntax (`id://`) and encoding**).
Tier: **crucial**.
Statement IDs: S4.12, S4.13, S4.14, S4.15, S4.16, S4.17
Enables: Cross-edition URI parity and protection against URI parser differentials.

## Scope and ownership
Parent: #9117 (native component/meta parent).
- `pkgs/id/crates/id-core/tests/id_uri_property.rs` (new; owner: A9117-uri-reference-pbt)
- `pkgs/id/hosted-worker/test/id-uri-vectors.test.ts` (new; owner: A9117-uri-reference-pbt)
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/uri_codec_reference.py` (existing; owner: A9117-uri-reference-pbt)
Hard dependencies (native blockers): A9117-uri-parser — Requires typed parse result for authority alias, UTF-8 namespace, V1 content ID or array ID from A9117-uri-parser.; A9117-uri-format — Requires canonical V1 URI formatter and caller-selected resolver-alias equality check from A9117-uri-format.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/id_uri_property.rs` first. Proposed command: `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/uri_codec_reference.py self-test && cargo test -p id-core --test id_uri_property`
Oracle: **golden** — expected `Reference self-test passes readable array byte vectors, one raw-blake3 content golden, 10000 deterministic ArrayId round trips, 10000 deterministic ContentId parse/format round trips, and rejects invalid digest length, uppercase/non-hex digest, unsupported parse/formatter profile, and malformed URI vectors; Rust/TypeScript results match the reference.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting this URI encoding and retain its parser for already published links; fall back only to an explicitly selected prior syntax, never global lookup or silent reinterpretation.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9117-uri-reference-pbt -->
