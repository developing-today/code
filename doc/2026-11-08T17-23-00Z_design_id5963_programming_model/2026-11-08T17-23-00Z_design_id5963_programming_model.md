# Design: explicit platform contexts for pure libraries

- **Date:** 2026-11-08
- **Status:** proposed source-level contract; no implementation is included.
- **Issue:** [#5963](https://github.com/developing-today/code/issues/5963), under the worlds program [#5955](https://github.com/developing-today/code/issues/5955).
- **Inputs:** the settled mechanism and checking constraints in [the #5997 note](../2026-11-05_design_id5997.md); the import boundary in [#5956](https://github.com/developing-today/code/issues/5956); the capability vocabulary/ceiling design in [#5958](https://github.com/developing-today/code/issues/5958) and its open [PR #7599](https://github.com/developing-today/code/pull/7599); and the static used-set follow-up [#5998](https://github.com/developing-today/code/issues/5998). The open [#5994 policy PR #7764](https://github.com/developing-today/code/pull/7764) is used only for its distinction between package requirements, world requests, and discovered use.
- **Method:** inspected `pkgs/id/examples/roc-world/RocCap.roc`, `pkgs/id/src/world_compile.rs`, and `pkgs/id/crates/id-core/src/world_caps.rs` at `origin/main` commit `20d4b36898c0db1402101fb00808bc7049e196b7`, plus fresh GitHub state on 2026-11-08. The proposals below are source-level contracts, not claims about current implementation.

## Decision summary

| Question | Decision |
| --- | --- |
| Passing mechanism | One explicit `context` value. Its `platform` member is a typed record of named capabilities: `context.platform.<domain>.<operation>`. |
| Granularity | A per-library, least-authority projection: nested per-domain records containing only the required operation fields. Do not pass the whole platform. |
| Required-surface contract | The library's exported context type is the source-level contract. A versioned `platform.requires` metadata view lists its canonical capability IDs and is generated from that type plus the transitive import closure, not hand-maintained as a second source of truth. |
| Import check | Require `requires(L) ⊆ world.effective_grant` and a type-compatible context from the world's pinned platform profile. Libraries reach the platform only through that context; direct platform-module imports are confined to the core adapter that builds projections, enforced at package level by #5956. `discovered_used(L)` remains separate and must be certified as a subset of the declared package surface; it never grants authority. Exact manifest encoding and diagnostics belong to #5956/#5998. |
| Versioning | Pin each world build to the vocabulary/catalog identity and platform profile it was built against. A capability or semantic expansion is never silently added to a running world's context. |
| User functions | Pass them as values only within the same attenuated context boundary. Revocation follows reachability. Resolve function values before emission; never put a function value or closure in the replicated causal log. |

The convention is explicit and ordinary Roc data. It adds no ambient lookup, runtime capability discovery, or implementation of the checker in this task.

## 1. Current substrate and the Roc data point

`RocCap.roc` says its helpers are pure and the app owns its model. For example, `issue : RocCap, Str -> { session : RocCap, id : Str }` and `attach : RocCap, Str, RocJson -> RocCap` take state as ordinary values and return updated state. This is a useful first data point for Roc libraries: state and inputs are explicit, and reusable functions need no hidden host authority. It is **not** an example of a platform-context import; the file imports only `RocJson`.

The existing import gate is narrower than the contract proposed here. `world_compile.rs::check_imports` scans source imports of platform modules and rejects a module the world's effective grants do not allow; it runs before invoking the compiler. `world_caps.rs` supplies the closed provider catalog. #5963 defines how a library's source API and metadata describe its needed platform surface; it does not replace that gate or implement library resolution.

A package-level requirement is not the same as direct import authority. A library's platform access must go through its projected `context`; it may not reach around that boundary with direct platform-module imports. The core adapter that constructs projections is the only component that imports the platform namespace. Enforcement for library packages (a package-scoped import restriction, or an equivalent source check that rejects ungranted direct platform imports in non-adapter code) belongs to #5956 and must be in place before the projection guarantee is treated as enforced.

The settled #5997 starting point is retained: platform functions remain explicitly passed under `context.platform`, over the closed namespace; used-capability derivation is a compile-time audit; declared ceilings are checked at compile/admission time and fail closed. No ambient or runtime-discovered authority is introduced.

## 2. Passing convention and authority granularity

An exported library entry point takes one explicit context parameter. The platform portion is grouped by domain, and each domain record contains only the operation fields that library needs. Domain grouping is for readable, stable Roc APIs; the security identity remains the canonical leaf capability ID from the catalog.

Illustrative shape (schematic, not a compiler-verified Roc declaration):

```text
Library.run :
    { platform: { chat: { say: ChatSay }, time: { now: TimeNow } } }
    , Model
    -> Result
```

This library's contract is `chat.say` plus `time.now`, not the complete `chat` or `time` domains and not the world's entire platform. A pure library has no platform fields in its context requirement. A library importing other libraries exposes the union of its own requirements and its transitive dependencies' requirements.

The core app constructs a separate projected context for each imported library from the world's effective grant. The projection contains only the required leaf operations; a library is never handed the full context merely because its type can ignore extra fields. If any required operation is unavailable, resolution fails before execution. Passing a context does not itself grant anything, and no import can widen the world's ceiling or grants.

### Rejected alternatives

- **Pass the whole platform record:** rejected. It gives every library a broader authority object than its contract needs, makes capability additions visible without an import-boundary decision, and makes least-authority review harder.
- **Pass every capability as a separate function argument:** rejected as the general convention. It is explicit, but forces every layer of ordinary helper composition to repeat a growing parameter list and loses the useful domain grouping. A small helper may still take a capability directly when that is its natural standalone API; this is not the library boundary convention.
- **Use an unstructured capabilities map or runtime name lookup:** rejected. It weakens the typed boundary and makes unknown or missing authority a runtime concern rather than a compile/admission-time check.
- **Use ambient globals, implicit host services, or runtime discovery:** rejected by the settled #5997 security boundary.

## 3. Library requirements and mechanical subset checks

For a library `L`, define `requires(L)` as the finite set of canonical platform IDs named by its exported context types, unioned with the requirements of the imported dependency closure. The set is conservative at package granularity: an import does not rely on which exported function a consumer happens to call today. A package with no platform-dependent exports and no platform-dependent imports has an empty requirement set and is pure.

The package carries a machine-readable `platform.requires` view of that contract, together with the vocabulary/catalog identity that gives those names meaning. The Roc type shape and metadata must be checked for agreement; neither a handwritten `pure` label nor a manually maintained used-set is authoritative. The exact file format and how #5956 obtains the view from compiler/type information are implementation details for that issue. Illustrative normalized view:

```json
{
  "platform": {
    "vocab_version": 1,
    "catalog_commit": "<pinned catalog identity>",
    "requires": ["chat.say", "time.now"]
  }
}
```

The required relation is:

```text
discovered_used(L) ⊆ requires(L) ⊆ world.effective_grant
```

`discovered_used` is the static analyzer's result owned by #5998; it is distinct from the API requirement set. The world request, world ceiling, effective grant, and library requirement set remain distinct facts. The import checker verifies known IDs, the pinned platform/type compatibility, and the subset relation before execution. An unknown name, incompatible version, missing requirement, or uncertified result fails closed; none is repaired by silently widening a grant. Runtime observations may help expose a mismatch but never expand authority.

This aligns with the open #5994 policy proposal, which keeps a package's requirements separate from the world's requested ceiling and from analyzer-discovered use. It also preserves #5997's rule that actual used capabilities are derived, not hand-written.

## 4. Platform versioning and capability ceilings

Use #5958's canonical capability vocabulary and profile identity; do not create a parallel tier or provider namespace here. At the time of this record, PR #7599 proposes `vocab_version`, `catalog_commit`, and a named/custom platform-profile identity for the registry. `platform.requires` uses that same vocabulary version and catalog identity; its wire representation is finalized with #5956/#5958 rather than independently here.

A world build pins the vocabulary/catalog identity and platform profile against which its context types and grants were checked. A new capability, a changed capability meaning, or a profile expansion is an explicit platform update: rebuild and re-check the import closure, then pass the resulting context only under the world's approved effective grant. No running world or imported library silently gains a newly added context field. An update that broadens authority must go through the existing request/ceiling/grant path and its consent/admission rules; otherwise the build remains on its pinned profile or fails closed.

This chooses a pinned-per-world surface over silent capability expansion. `vocab_version` identifies the vocabulary contract; `catalog_commit` and profile identity bind the concrete catalog/profile used by the build. The capability model and its tier names remain owned by #5958.

## 5. User-passed functions

A user-supplied function crosses the same explicit boundary as any other capability-bearing value. It cannot amplify the caller's authority: the receiving library can invoke it only under the function's attenuated requirements and the effective context the core app supplied. Revocation takes effect by reachability: revoke the root grant and every derived/passed function reachable only through it becomes unusable; no copy of the function may preserve a revoked path.

Function values must be resolved to an allowed, statically identifiable target before an action is emitted. If the target or its required platform surface cannot be resolved/certified, the action is rejected rather than granting an unknown surface. Resolution and exact target-analysis mechanics belong to #5998 and the certification-policy follow-up, not this source-level contract.

The replicated causal log contains no function value or closure. Resolution happens before emission; the log stores only the serializable event/data form permitted by the existing protocol. This preserves the existing delegation invariants: attenuation only, revocation by reachability, and no function values in replicated causal history.

## 6. Boundaries, costs, and follow-ups

This record settles the source-level convention and its security invariants. It does not implement platform passing, derive the used set, define capability vocabulary/ceilings, or specify all package-import error messages.

| Follow-up | Owns |
| --- | --- |
| [#5956](https://github.com/developing-today/code/issues/5956) | Library import resolution, the concrete `platform.requires` encoding, type/metadata agreement, and user-facing failures. |
| [#5958](https://github.com/developing-today/code/issues/5958) | Canonical capability vocabulary, ceilings, catalog/profile schema, and their versioning. |
| [#5998](https://github.com/developing-today/code/issues/5998) | Static used-capability derivation and certification/checking implementation. |
| [#6000](https://github.com/developing-today/code/issues/6000) | Which dynamic-loading or unresolved-dispatch shapes void a certified result. |

The main trade-off is duplicated structure: the library has a typed context contract and a normalized metadata view. Generating the latter from the former, and checking their agreement, avoids manual drift. The per-library projection adds construction work at the boundary, but makes attenuation concrete rather than relying only on a type that hides fields in a wider runtime value.

The precise Roc export/type-inspection hook and manifest serialization remain with #5956; they must preserve this contract and may not replace it with a hand-written `used` list or an implicit full-platform argument.

## Sources checked

- [`RocCap.roc`](../../pkgs/id/examples/roc-world/RocCap.roc), especially lines 10–43: pure helpers and explicit state transitions.
- [`world_compile.rs`](../../pkgs/id/src/world_compile.rs), `check_imports` around lines 111–143 and its tests: current platform-module import gate.
- [`world_caps.rs`](../../pkgs/id/crates/id-core/src/world_caps.rs): closed provider catalog and current capability identities.
- [#5997 settled note](../2026-11-05_design_id5997.md), especially §§1, 3–5.
- [#5958 design PR #7599](https://github.com/developing-today/code/pull/7599) and [#5994 policy PR #7764](https://github.com/developing-today/code/pull/7764), both open when checked.

<!-- agent-note:v1 agent=repo-worker-code lane=9aca43b4-5439-4939-9f71-323f5d32521b state=design-record task=12864 at=2026-11-08T17:23:00Z -->
