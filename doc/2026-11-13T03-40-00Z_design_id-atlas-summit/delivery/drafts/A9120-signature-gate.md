## Problem and goal
Require signature, signer binding, semantic validation, namespace/tenant authorization and policy checks at every admission path, including local producers after signing.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9120--content-address-core (section: **#9120 — content-address core**).
Tier: **crucial**.
Statement IDs: S1.12, S1.13, S1.14, S1.15, S1.16, S1.17, S1.18, S1.19, S1.20, S3.4, S3.5, S3.6, S3.7, S3.8, S3.9, S3.10, S4.19, S4.20, S4.21, S4.22, S4.23, S4.24, S4.25, S4.26, S4.27, S4.28
Enables: One auditable acceptance invariant without trusted-local bypass.

## Scope and ownership
Parent: #9120 (native component/meta parent).
- `pkgs/id/crates/id-core/src/admission_gate.rs` (new; owner: A9120-signature-gate)
- `pkgs/id/crates/id-core/tests/admission_gate.rs` (new; owner: A9120-signature-gate)
- `pkgs/id/hosted-worker/src/admission-gate.ts` (new; owner: A9120-signature-gate)
- `pkgs/id/hosted-worker/test/admission-gate.test.ts` (new; owner: A9120-signature-gate)
Hard dependencies (native blockers): A9114-envelope-identities — Requires PayloadId, canonical Body, detached-signature preimage and nonrecursive EnvelopeId types from A9114-envelope-identities.; A9116-registry-state — Requires versioned local registry read/mutate operations and mutation authorization check from A9116-registry-state.; A9120-content-store — Requires immutable content adapter that commits only after complete digest validation from A9120-content-store.; 9110-02 — Adds this consumer module to the crate root and owns Cargo manifest/export changes; this slice owns only its distinct module and tests.; 9083 — Consumes the authenticated tenant/scope context contract; this slice does not define tenancy.
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/crates/id-core/tests/admission_gate.rs` first. Proposed command: `cargo test -p id-core --test admission_gate && cd pkgs/id/hosted-worker && bun test admission-gate.test.ts`
Oracle: **model** — expected `Ingress model: every accepted state transition has verified signature and all validation decisions; no route bypass exists.`; verifier: `Generate the listed operations against the specified state/reference model and compare every resulting state and error.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: not applicable.
Rollback: Disable the new content writer/admission route; keep legacy content readers and all previously verified content-addresses intact. Never delete or rehash data as rollback.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:A9120-signature-gate -->
