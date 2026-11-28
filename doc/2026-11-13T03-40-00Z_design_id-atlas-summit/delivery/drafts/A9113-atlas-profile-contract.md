## Problem and goal
After crosscut 9110-R2 pins the producer profile or records it unavailable, add an explicit Atlas/ID compatibility vector and typed foreign-profile seam without owning the producer investigation.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9113--extensible-id-shapes-and-content-portability (section: **#9113 — Extensible ID shapes and content portability**).
Tier: **recommended**.
Statement IDs: S3.4, S3.5, S3.6, S3.7, S3.8, S3.9, S3.10
Enables: Consumers can verify compatible Atlas IDs only when R2 supplies a known-answer vector; otherwise Atlas content_hash remains opaque and cannot be misidentified as raw-byte Iroh BLAKE3.

## Scope and ownership
Parent: #9113 (native component/meta parent).
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/atlas-hash-compatibility.md` (new; owner: A9113-atlas-profile-contract)
- `pkgs/id/test-vectors/atlas-content-hash-v1.json` (new; owner: A9113-atlas-profile-contract)
- `pkgs/id/crates/id-core/tests/atlas_content_profile_vectors.rs` (new; owner: A9113-atlas-profile-contract)
Hard dependencies (native blockers): 9110-R2 — Requires 9110-R2's pinned producer/profile and known-answer vector, or its explicit producer-unavailable disposition, before defining any Atlas interoperability mapping.; 9110-02 — Adds this consumer test/module to crate-root exports and Cargo manifests; the slice owns only its separate compatibility vector and module.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/atlas_content_profile_vectors.rs` first. Proposed command: `cargo test -p id-core --test atlas_content_profile_vectors`
Oracle: **golden** — expected `If 9110-R2 provides an available producer profile, the Atlas adapter matches its exact byte-level known-answer vector; if R2 records unavailable, parsing/verification keeps Atlas content_hash opaque and rejects an interop claim. No branch aliases it to raw-blake3-256.`; verifier: `Read R2's report status and vector: recompute the specified profile over the exact fixture bytes and compare digest, or assert the unavailable disposition yields UnsupportedProfile for cross-system ContentId conversion.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the Atlas compatibility adapter and keep Atlas content_hash opaque; retain the vector file for audit, and do not rewrite or re-sign existing ID/Atlas identifiers.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9113-atlas-profile-contract -->
