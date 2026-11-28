## Problem and goal
Bind a server-local namespace name to one of exactly two core IDs and explicit handler/config profiles.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9116--local-namespace-registry (section: **#9116 — Local namespace registry**).
Tier: **crucial**.
Statement IDs: S4.1, S4.2, S4.3, S4.4, S4.5, S4.6, S4.7, S4.19, S4.20, S4.21, S4.22
Enables: Stable local resolution while preserving divergent domain-local registries.

## Scope and ownership
Parent: #9116 (native component/meta parent).
- `pkgs/id/crates/id-core/src/namespace_definition.rs` (new; owner: A9116-registry-record)
- `pkgs/id/crates/id-core/tests/namespace_definition.rs` (new; owner: A9116-registry-record)
Hard dependencies (native blockers): A9113-shape-types — Requires typed ContentId/ArrayId shape variants, shape-version tag and unsupported-critical-shape error from A9113-shape-types.; A9118-array-codec — Requires lossless ordered Option<Bytes> address encoder/decoder with null/empty distinction from A9118-array-codec.; A9120-content-id — Requires raw exact-byte Iroh BLAKE3-256 ContentId type and fixed legacy-compatible text/binary parser from A9120-content-id.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/namespace_definition.rs` first. Proposed command: `cargo test -p id-core --test namespace_definition`
Oracle: **invariant** — expected `Finite model keyed by explicit resolver/registry identity; a definition cannot name an unknown third core.`; verifier: `Check the stated property over generated inputs and assert the exact invariant and failure classification.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable new registry mutations and resolve through the last verified registry generation; preserve existing Iroh namespace registries and IDs unchanged.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9116-registry-record -->
