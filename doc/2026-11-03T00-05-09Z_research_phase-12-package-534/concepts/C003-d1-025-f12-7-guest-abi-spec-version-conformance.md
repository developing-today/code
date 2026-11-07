# C003: F12.7 versioned guest ABI spec, plaza_abi_version export and conformance suite — D1-025

[Main reference](../main.md#c003)

## Relevant transcript excerpts

<!-- workflow:excerpt C003 24 28 -->

[Transcript lines 24-28](../transcript.txt#L24-L28)

```text
- [ ] **D1-025 — F12.7 Versioned guest ABI spec, plaza_abi_version export, conformance suite** (L, design)
  Write the `plaza_*` exports and capability protocol down as a versioned spec, add a `plaza_abi_version` export, and publish a conformance suite built on F11.16 fixtures and the #239 recordings. Part of the recommended first slice.
  - Status: not started. The ABI is described only in module docs (pkgs/id/src/sandbox.rs:6-8 lists plaza_out_len, plaza_alloc, memory). `grep -rn plaza_abi_version pkgs/id` finds nothing, and there is no spec document or conformance test directory.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:43; .opencode/plans/phase-12-platform-flexibility.md:84
  - Prerequisites: #544 — D1-016 F11.16 Replay self-check (`id world verify`)
```

<!-- workflow:excerpt C003 166 176 -->

[Transcript lines 166-176](../transcript.txt#L166-L176)

```text
[Comment by @dezren39 at 2026-11-01T18:20:55Z; https://github.com/developing-today/code/issues/534#issuecomment-6561986981]
## Design summary

Decision note: [Phase 12 workflow design](https://github.com/developing-today/code/blob/design/id-534/doc/2026-11-01_design_id534.md).
- Keep #534 as the umbrella; use one shared research package with one concept per its 11 listed D1 items.
- Reconcile code and later comments first: F12.1 manifest/apply/diff exists; F12.3 has role bits and journal records, but install still requires the admin token on the live path.
- Reuse the existing F12 implementation issues; do not create duplicates or treat code alone as a completed design.
- Keep each concept's design/research PR independently mergeable; dependencies gate implementation, not ready research.
- Tick a D1 only after its committed artifact and linked implementation issue are verified; leave #534 open.

PR: https://github.com/developing-today/code/pull/3912
```

## Normalized purpose

Intake for the versioned plaza_* guest ABI spec, the plaza_abi_version export, and the conformance suite built on fixtures/recordings. Reconciles open #1064 with the #544 verify prerequisite claim.

## Initial thoughts

- Keep the spec work distinct from determinism fixtures: versioning/compatibility and the conformance contract are the deliverable.
- Record the #544 prerequisite reconciliation question explicitly so S2 answers it instead of assuming it.

## Inferred suggestions

- Inference: enumerate the current exports/imports and the existing verify surface as the S1 evidence table.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
