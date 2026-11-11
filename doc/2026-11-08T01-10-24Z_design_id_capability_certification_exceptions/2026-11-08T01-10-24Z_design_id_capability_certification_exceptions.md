# Design: capability-certification exception policy

- **Date:** 2026-11-08
- **Status:** Proposed policy for review; registry field names remain owned by #5958.
- **Issue:** [#6000](https://github.com/developing-today/code/issues/6000), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955). This policy feeds the derivation-tool design [#5998](https://github.com/developing-today/code/issues/5998) and the capability-observability design [#6017](https://github.com/developing-today/code/issues/6017).
- **Inputs:** [#5958](https://github.com/developing-today/code/issues/5958) and its C2 ruling; [#5997](../2026-11-05_design_id5997.md), which establishes call-graph derivation over the closed platform namespace and rejects runtime-discovered capability checks as a security boundary; the current #5998 and #6017 proposals.
- **Scope:** Define the boundary of an exact derived-used set, the registry and UX treatment when that boundary is crossed, and the import/grant policy. This note does not implement a runtime check or settle #5958's registry schema.

## Decision summary

| Question | Policy | Consequence |
| --- | --- | --- |
| What can be certified? | A specific immutable world/library release and its complete transitive dependency closure, when the versioned analyzer can prove every possible platform-function call from the analyzed artifact. | Certification is tied to the release and analysis inputs, not a library name or a previous release. |
| What voids certification? | Executable runtime loading, unresolved reflective or data-driven platform dispatch, an unbounded FFI escape, or any language/compiler feature the analyzer cannot soundly cover. | The result is uncertified/unknown, never an authoritative empty used set. |
| How is higher-order code handled? | If all possible function targets are statically enumerable, include their union conservatively. If the analyzer cannot establish a complete target set, certification fails. | Over-approximation is permitted; an unbounded target set is not. |
| How is an uncertified library used? | It cannot enter a certified world. Execution, if allowed at all, requires an explicit server-admin grant in an opt-in capability-enabled tier and remains subject to the server's capability ceiling and runtime gates. | No import-time or server-default grant is inferred from a requested set or a partial analysis. An escape the runtime cannot bound is denied pending a separate isolation design. |
| Which registry fields are added? | The registry must express certification state, its reason, and the analysis provenance needed to interpret it. Exact names and wire shape remain with #5958. | #6000 defines semantics; it does not create a parallel schema. |

## 1. Certification boundary

The certified subset is the code for which call-graph reachability can enumerate the complete set of platform functions that might be called. A certified `discovered-used` set is a static claim about that release's possible calls; it is not a runtime observation and it is not a grant.

Certification is per immutable release identity, including its transitive dependency closure. A source, dependency, platform ABI, compiler, analyzer, or certification-policy change invalidates the prior result unless the matching analysis is rerun and succeeds. Missing or mismatched inputs, an analysis error, or an unsupported construct fail closed.

### Features that void certification

1. **Executable runtime loading.** Loading plugins, modules, bytecode, generated executable code, or other code after the analyzed closure is fixed voids certification. Loading ordinary data does not by itself do so. Compile-time generation is compatible only when the final generated artifact is fixed and included in the analysis.
2. **Reflection or unresolved name lookup.** Reflective lookup/invocation of platform functions, or converting runtime names/data into platform calls, voids certification when the analyzer cannot prove a complete target set.
3. **Unbounded data-driven dispatch.** Dispatch selected by data voids certification if its possible platform-function targets are not statically enumerable. Ordinary finite branches and higher-order function passing remain certifiable when the analyzer includes the union of every possible target; it must not silently omit indirect calls.
4. **FFI or another escape from the analyzed boundary.** For this policy version, FFI is unsupported for certification if foreign code could call platform functions, reach host authority, or bypass the platform capability gate. A later design may certify a narrower foreign interface only after it proves and tests a sound boundary. An administrator grant alone cannot contain an escape the runtime does not mediate.
5. **Unsupported or newly introduced language/compiler behavior.** A feature that changes call reachability, function identity, or access to platform authority voids certification until a versioned analyzer and policy explicitly cover it. New Roc features are not presumed safe by default.

These are certification failures, not proof that a library is malicious. They mean the tooling cannot make the exactness claim. Diagnostics may list a known subset or the offending construct, but that partial information is not the certified used set and must not be used to reduce the permission envelope.

## 2. Registry semantics

#5958 owns the canonical registry schema and vocabulary. This policy requires the schema to distinguish these facts without prescribing field names:

- **Requested/declaration:** what the release asks for. A request is not an effective grant.
- **Discovered-used:** the analyzer's complete static result, authoritative only when certification succeeded.
- **Effective grant:** what the server/admin actually permits under its policy and ceilings.
- **Certification state and provenance:** whether the result is certified, not certified/unknown, why, and the release/dependency identity plus platform, compiler, analyzer, and policy versions that produced it.

For an unsupported construct, missing analysis, stale identity, or failed analysis, the authoritative discovered-used value is absent/unknown—not an empty set. A genuinely empty set is meaningful only when a complete analysis succeeded. If diagnostic partial results are retained, they must be separately identified as partial and non-authoritative. Runtime usage counters (such as the current `CapsReport` usage counts) remain observations and must not be relabeled as a static discovered-used set.

The registry must retain enough reason and provenance for a client or administrator to explain a certification transition. Stable capability IDs and the capability-catalog version should be used where #5958 defines them. #6000 does not add an independent history or registry format; any durable history remains a #5958 schema decision.

## 3. Import, permission, and tier treatment

- A library without a current certified result may be inspected and shown in a dependency inventory, but it cannot be represented as a certified dependency or silently included in a certified world. Re-certification is required after the unsupported feature is removed or the analyzer gains sound support.
- If a server chooses to execute an uncertified library, admission requires an explicit server-admin decision in an opt-in capability-enabled tier. The decision must be scoped to the artifact/release and to the capability envelope the server makes reachable; a self-declared requested set or partial analysis is not evidence that narrower authority is sufficient. Existing runtime capability checks and ceilings still apply.
- An FFI or other escape that can bypass those runtime checks is not made acceptable by a capability grant. Reject that import/execution path until a separate isolation design establishes a bound the server can enforce.
- Users and authors see **“not certified / used set unknown”** with the reason and analysis provenance. Show requested capabilities and the effective grant separately. Never render an absent or partial used set as “uses none,” and never use it to relax a grant or skip an approval.
- The C2 posture remains unchanged: isolation with no filesystem or network access is the default; additional file/network capability is opt-in and off by default on servers; publication using those capabilities is first-party-only for now. This policy does not authorize general publishing or server-default grants. Future general enablement remains a separate decision after testing and refinement.

## 4. Inputs to the follow-ups

### #5998 — derivation tool

The tool must analyze the complete release/dependency closure and report whether it proved a complete target set. For supported higher-order calls it conservatively includes every possible target. A dynamic load, unresolved reflective/data-driven call, unbounded FFI boundary, missing input, or unsupported feature produces no certified used-set and a reason suitable for registry/UX display. The tool emits into the schema #5958 settles; when that schema defines the declaration/request set, a certified derived set must be checked against it rather than silently widening it.

### #6017 — observability

Views show requested, discovered-used, and effective grant as separate facts, alongside certification state, reason, and provenance. An uncertified or stale result is visibly unknown/not certified, never an empty used-set. Users, authors, and admins may see different details according to #6017's audience matrix, but no surface may present uncertified status as safe or as proof that no capabilities are used.

## 5. Rationale and deferred details

Call-graph derivation is exact only while the platform namespace and analyzed call graph are closed. Dynamic loading, reflection, or an unbounded foreign boundary breaks that premise; guessing a used set would overstate what the analyzer knows. Failing closed preserves the distinction between “proved not to use” and “not proved,” while explicit admin grants keep authority decisions in the server's existing policy boundary. The default isolated posture and server ceilings remain the backstop, not a claim that unknown usage is harmless.

Deferred to the owning designs:

- Canonical field names, schema versioning, stable reason-code vocabulary, and history shape — #5958.
- Exact analyzer diagnostics and support rules for individual finite higher-order patterns — #5998.
- Audience-specific presentation and history queries — #6017.
- Any future third-party publication or general server enablement of capability-bearing worlds — the worlds program after further testing; this note does not pre-approve it.

## Sources

- [#5958 capability model](https://github.com/developing-today/code/issues/5958), including the C2 ruling [comment](https://github.com/developing-today/code/issues/5958#issuecomment-6650954322).
- [#5997 effect-language borrowings design](../2026-11-05_design_id5997.md), especially its closed-call-graph boundary and prohibition on runtime-discovered checks as a security boundary.
- [#5998 used-capability derivation tool](https://github.com/developing-today/code/issues/5998).
- [#6017 capability observability proposal](https://github.com/developing-today/code/issues/6017), including the separation of requested, discovered-used, and effective grant.
- [`world_caps.rs`](../../pkgs/id/crates/id-core/src/world_caps.rs), for the current provider catalog and runtime usage-report vocabulary; the current implementation is not the future #5958 registry schema.

## Change log

- 2026-11-08: Initial policy proposal for #6000, based on the live #5958 C2 ruling, #5997, and the current #5998/#6017 inputs.
