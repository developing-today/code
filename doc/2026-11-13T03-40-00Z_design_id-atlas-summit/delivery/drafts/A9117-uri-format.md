## Problem and goal
Emit lowercase aliases, uppercase percent escapes and settled null/bytes/content syntax; require selected local resolver alias equality.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9117--uri-syntax-id-and-encoding (section: **#9117 — URI syntax (`id://`) and encoding**).
Tier: **crucial**.
Statement IDs: S4.11, S4.12, S4.13, S4.14, S4.15, S4.16, S4.17, S4.18
Enables: Stable URI output without DNS/global resolution behavior.

## Scope and ownership
Parent: #9117 (native component/meta parent).
- `pkgs/id/crates/id-core/src/id_uri_format.rs` (new; owner: A9117-uri-format)
- `pkgs/id/crates/id-core/tests/id_uri_format.rs` (new; owner: A9117-uri-format)
Hard dependencies (native blockers): A9117-uri-parser — Requires typed parse result for authority alias, UTF-8 namespace, V1 content ID or array ID from A9117-uri-parser.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/id_uri_format.rs` first. Proposed command: `cargo test -p id-core --test id_uri_format`
Oracle: **golden** — expected `Golden URI examples and parse(format(value)) equals typed value; format(parse(canonical)) byte-identical.`; verifier: `Compare output bytes and typed outcomes with the cited fixed golden vectors; fail on any difference.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Stop emitting this URI encoding and retain its parser for already published links; fall back only to an explicitly selected prior syntax, never global lookup or silent reinterpretation.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9117-uri-format -->
