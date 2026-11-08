# C007: F12.8 Rust and AssemblyScript-or-Zig guest SDKs with WIT/component-model decision — D1-026

[Main reference](../main.md#c007)

## Relevant transcript excerpts

<!-- workflow:excerpt C007 44 48 -->

[Transcript lines 44-48](../transcript.txt#L44-L48)

```text
- [ ] **D1-026 — F12.8 Rust and TypeScript/Zig guest SDKs; WIT/component-model decision** (XL, research package)
  SDKs for Rust (wasm32-unknown-unknown) and AssemblyScript or Zig with screen/json/grid/cap helpers equal to the Roc libraries, each with an example world and a passing conformance run. Record a decision on the Wasm component model / WIT.
  - Status: not started. No sdk directory (`ls pkgs/id/sdk pkgs/id/sdks`: no such directory); all eight examples are Roc (pkgs/id/examples/{lounge,roc-chat,roc-counter,roc-grid,roc-screen,roc-world,territory,tic-tac-toe}).
  - Source: .opencode/plans/phase-12-platform-flexibility.md:44; .opencode/plans/phase-12-platform-flexibility.md:17
  - Prerequisites: D1-025 F12.7 Versioned guest ABI spec, plaza_abi_version export, conformance suite (in this list)
```

## Normalized purpose

Intake for the F12.8 research package: Rust and AssemblyScript-or-Zig SDKs equal to the Roc helpers, each with an example world and a passing conformance run, plus the WIT/component-model decision. Reconciles open #1065 and the title/body language discrepancy (TypeScript/Zig vs AssemblyScript or Zig).

## Initial thoughts

- Record the language ambiguity as an explicit S1 research question, not a silent default.
- Bind all SDKs to the same F12.7 conformance suite (D1-025) so the decision and the contract stay aligned.

## Inferred suggestions

- Inference: ask S2 to compare the generated host ABI source against a guest-SDK requirement list at intake.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
