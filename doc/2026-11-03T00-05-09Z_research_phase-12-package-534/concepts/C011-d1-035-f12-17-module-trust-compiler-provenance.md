# C011: F12.17 module signature trust list and compiler-version provenance — D1-035

[Main reference](../main.md#c011)

## Relevant transcript excerpts

<!-- workflow:excerpt C011 64 112 -->

[Transcript lines 64-112](../transcript.txt#L64-L112)

```text
- [ ] **D1-035 — F12.17 Module signature trust list and compiler-version provenance** (M, design)
  Verify module signatures against an admin trust list, add the compiler version to observed provenance, journal the trust decision; an unsigned module is refused when the list is non-empty.
  - Status: not started. Observed provenance exists (`ModuleProvenance` fields at pkgs/id/src/world.rs:299, :527, :586), but world.rs:310 states it is "not a signature or a trust attestation"; there is no trust list. The plan cites PR #225, which is CLOSED; provenance reached main another way.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:65; doc/2026-10-16T09-00-00Z_feature_world_module_provenance/; doc/2026-10-16T09-30-00Z_feature_world_module_provenance/
  - Prerequisites: #541 — D1-018 F11.18 Pinned Roc compiler for checked-in example modules + CI rebuild job

## Source

- [`.opencode/plans/phase-12-platform-flexibility.md:16`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L16)
- [`.opencode/plans/phase-12-platform-flexibility.md:17`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L17)
- [`.opencode/plans/phase-12-platform-flexibility.md:21`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L21)
- [`.opencode/plans/phase-12-platform-flexibility.md:31`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L31)
- [`.opencode/plans/phase-12-platform-flexibility.md:33`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L33)
- [`.opencode/plans/phase-12-platform-flexibility.md:37`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L37)
- [`.opencode/plans/phase-12-platform-flexibility.md:38`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L38)
- [`.opencode/plans/phase-12-platform-flexibility.md:39`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L39)
- [`.opencode/plans/phase-12-platform-flexibility.md:43`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L43)
- [`.opencode/plans/phase-12-platform-flexibility.md:44`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L44)
- [`.opencode/plans/phase-12-platform-flexibility.md:45`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L45)
- [`.opencode/plans/phase-12-platform-flexibility.md:49`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L49)
- [`.opencode/plans/phase-12-platform-flexibility.md:56`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L56)
- [`.opencode/plans/phase-12-platform-flexibility.md:65`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L65)
- [`.opencode/plans/phase-12-platform-flexibility.md:80`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L80)
- [`.opencode/plans/phase-12-platform-flexibility.md:84`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L84)
- [`.opencode/plans/phase-12-platform-flexibility.md:85`](https://github.com/developing-today/code/blob/main/.opencode/plans/phase-12-platform-flexibility.md#L85)
- [`doc/2026-10-10T04-59-57Z_feature_cross_world_access/`](https://github.com/developing-today/code/tree/main/doc/2026-10-10T04-59-57Z_feature_cross_world_access/)
- [`doc/2026-10-16T09-00-00Z_feature_world_module_provenance/`](https://github.com/developing-today/code/tree/main/doc/2026-10-16T09-00-00Z_feature_world_module_provenance/)
- [`doc/2026-10-16T09-30-00Z_feature_world_module_provenance/`](https://github.com/developing-today/code/tree/main/doc/2026-10-16T09-30-00Z_feature_world_module_provenance/)
- [`doc/2026-10-17T08-00-00Z_feature_world_roles_directory/`](https://github.com/developing-today/code/tree/main/doc/2026-10-17T08-00-00Z_feature_world_roles_directory/)

## Size

XL overall (items M to XL)

## Prerequisites

Listed per item above.

<!-- discovery:D1-019 discovery:D1-022 discovery:D1-025 discovery:D1-021 discovery:D1-024 discovery:D1-023 discovery:D1-026 discovery:D1-027 discovery:D1-028 discovery:D1-032 discovery:D1-035 -->

## Acceptance criteria

- [ ] Every item listed above has its design (or research package, where marked) committed to the repo
- [ ] Every item has a linked implementation issue, and its checkbox above is ticked only once both are true
- [ ] No item is closed out on the basis of a plan or report reference alone — the committed artifact is the evidence


--- DECISION-BEARING ISSUE COMMENTS (verbatim bodies) ---

```

<!-- workflow:excerpt C011 182 183 -->

[Transcript lines 182-183](../transcript.txt#L182-L183)

```text
2. **F12.17 prerequisite:** options are (A) plan's F11.10 gate or (B) D1-035's #541/F11.18 gate. **Recommend B**: compiler provenance is part of the trust decision; retain the more specific pinned-compiler prerequisite and reconcile the plan.

```

## Normalized purpose

Intake for module-signature verification against an admin trust list, compiler-version provenance, the journaled trust decision, and unsigned-module refusal. Reconciles CLOSED #1069 and carries the unresolved F11.10-vs-#541/F11.18 prerequisite question with the option-B recommendation.

## Initial thoughts

- Keep observation distinct from trust: provenance is not attestation, and the intake must not conflate them.
- Record the prerequisite discrepancy explicitly so S2 proposes the recorded gate instead of re-deriving it.

## Inferred suggestions

- Inference: verify at S1 what #1069 closure evidence says before the later design cites it.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
