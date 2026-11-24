# Numeric registry namespaces over the byte system

Date: 2026-11-24  
Status: focused design record and implementation-contract consolidation for [#9122](https://github.com/developing-today/code/issues/9122). This record narrows and makes actionable the numeric-family contract already specified by [Part A, §3 / A-9122.1](2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9122--numeric-registry-family); it does not supersede that shared design.

## 1. Problem and context

The ID program wants ordered registry keys for integers, decimals, binary floats, and arbitrarily large integer magnitudes. A fixed-width integer representation cannot preserve the stated unbounded magnitude/precision goal. Treating every key as an untyped byte string also loses numeric kind and can make equivalent spellings compare differently. A separate numeric storage engine would duplicate the array-address core and its registry's namespace, authorization, and versioning rules.

The source request explicitly hypothesizes that numeric namespaces are array-address bytes plus numeric-style validation at write time, rather than a new engine. The parent Part A design already fixes a typed `NumericValue`, canonical forms, deterministic type-first ordering, and operational—not semantic—resource limits (Part A §3, lines 143–147). The current implementation has `NumericRegistry` as an admission boundary over the array core (`pkgs/id/crates/id-store/src/numeric_registry.rs`) and numeric encoding/order code in `id-core`; its integration tests exercise kind rejection, resource budgets, large admitted values, and continued registry authorization (`pkgs/id/crates/id-store/tests/numeric_registry.rs`). The design question is therefore resolved by consolidation and explicit rollout/conformance criteria, not by inventing another numeric subsystem.

## 2. Goals and non-goals

### Goals

- Store numeric keys as typed, canonical array-address values; do not add a third storage core.
- Support integer, decimal, and binary-float values with extensible byte encoding and no semantic maximum magnitude or precision.
- Define stable canonicalization, exact comparison, cross-kind ordering, write admission, and resource-limit behavior.
- Keep namespace authorization, generation/CAS, and storage mutation at the existing registry and array-core seams.
- Make the contract portable across serializers and implementations through golden vectors and independent comparator oracles.

### Non-goals

- Implicit conversion between integer, decimal, and binary float; a key's type is part of its identity.
- IEEE NaN or infinities in the ordered registry. A compatibility profile may preserve them only outside ordered-key operations.
- A separate numeric engine, global namespace directory, or numeric-specific authorization system.
- Choosing a UI spelling, arithmetic package, or application-level rounding policy beyond explicit conversion metadata.
- Imposing a semantic maximum magnitude/precision. Deployments may impose operational byte/work budgets and return `resource_limit`.

## 3. Alternatives and tradeoffs

| Option | Benefit | Cost / reason not selected |
| --- | --- | --- |
| Do nothing / leave numeric ordering to callers | No new contract or code | Callers can disagree on canonical forms, type identity, ordering, and bounds; does not meet the requested numeric registry. |
| One fixed-width integer or IEEE-754 `f64` key | Compact and familiar; straightforward comparison | Loses arbitrary magnitude/precision; float conversion rounds integers/decimals and has NaN/infinity edge cases. Fails the stated domain. |
| Separate numeric storage engine | Could optimize numeric range scans independently | Duplicates array core, persistence, namespace generation, authorization, and migration. Adds a third core contrary to Part A's two-core architecture. |
| Untyped lexicographically ordered byte strings | Reuses byte storage and can be extensible | Requires every caller to independently preserve kind and canonicalize; lexical bytes alone do not define the required typed numeric order. |
| **Selected: derived numeric namespace over array-address bytes with typed write admission and a reference comparator** | Reuses storage and auth boundaries; exact behavior is testable and extensible | Requires canonical type-tagged values and bounded-work comparison. Complexity is concentrated at codec/comparator boundaries and covered by the slices below. |

## 4. Chosen approach and contract

### Data model and API seam

```text
NumericValue =
    Integer(BigInt)
  | Decimal(coefficient: BigInt, exponent10: BigInt)
  | BinaryFloat(sign, significand: BigInt, exponent2: BigInt)

NumericNamespacePolicy = { version, allowed_kinds, operational_limits }
NumericRegistry::admit(value) -> NumericKey | KindNotAllowed | ResourceLimit
```

The numeric family is a derived namespace whose validated key is encoded as a typed array-address value. A stable kind tag is included in equality and serialization. Admission validates the kind against that namespace's policy, validates canonical value invariants and comparator work against its operational budget, then returns an opaque/admitted key. It does not perform the namespace write itself. The caller passes the admitted key through the existing versioned registry mutation path, which checks authenticated tenant/namespace authority and generation/CAS. No direct storage or authorization bypass is introduced.

### Canonical forms and ordering

- Integer uses arbitrary-precision signed magnitude with one canonical zero.
- Decimal is `(coefficient, exponent10)`; remove factors of ten from nonzero coefficient while increasing exponent by the same count. All signed zeros normalize to positive zero.
- Binary float is exact `(sign, significand, exponent2)`; remove factors of two from nonzero significand while increasing exponent by the same count. All signed zeros normalize to positive zero. This is an exact binary value representation, not a conversion through host `f64`.
- Ordered keys reject NaN and infinities. If a non-ordered compatibility representation is later required, it gets an explicit profile and cannot be admitted as an ordered key.
- Compare type tag first in the stable order `Integer < Decimal < BinaryFloat`; within a type, compare exact mathematical values. There is no implicit cross-kind coercion. Conversions are explicit and carry a rounding/loss policy.
- Within each kind, order negatives by descending absolute magnitude, then zero, then positives by ascending magnitude. This yields `-10 < -2 < 0 < 2 < 10` within a kind.
- Compare decimal/binary magnitudes using effective digit/bit length and virtual zero padding; do not materialize `10^exponent` or `2^exponent`. Comparator ordering must agree with the independent reference comparator, not with host floating-point approximations.
- Encoding is self-delimiting/versioned and extensible: no fixed-width numeric field. The canonical encoder has `encode(decode(bytes)) == bytes` for canonical input and `decode(encode(value)) == value` for admitted values. The array core supplies byte-slot ordering; the numeric codec/comparator supplies numeric semantics.

### Errors, limits, security, and capability boundaries

Invalid syntax, noncanonical encodings, unsupported kind/version, or forbidden kind are semantic refusals. A configured parser/comparator budget exceeded is operational `resource_limit`, never an out-of-range numeric value. Limits are configurable per namespace and bound magnitude bytes, parse work, and comparison work. The domain itself has no fixed magnitude or precision cap.

Numeric admission is not authorization. Authenticated caller context, tenant/namespace matching, and write policy remain enforced by the existing registry authorizer on every mutation. Reads and range scans must apply authorization before exposing keys or candidate counts. Unknown critical encoding versions fail closed; adapters unable to preserve type tags or exact values report a capability/conformance refusal rather than silently coercing. Resource limits must prevent exponent-driven CPU/memory exhaustion without changing numeric meaning for values inside budget.

## 5. Dependency-ordered build slices

The shared design's `implementation-slices.json` already tracks these slices (`A9122-numeric-encoding`, `A9122-numeric-order`, `A9122-numeric-registry`). These are the implementation order; this record does not file duplicate implementation tickets. The repository currently contains the encoding/order/registry seams, so a future implementation pass should verify or extend those slices rather than restart them.

| Order | Slice (one lane/session) | Depends on | Acceptance criteria |
| --- | --- | --- | --- |
| 1 | Numeric value model and canonical extensible encoding (`A9122-numeric-encoding`) | Array-address typed value/serialization contract (#9118) | Golden vectors preserve kind tags, signed extremes, normalized equivalent decimals/binary values, and positive zero; no fixed-width truncation; round-trip and canonical re-encoding properties pass. |
| 2 | Exact total comparator (`A9122-numeric-order`) | Slice 1 | Independent reference sort agrees for all golden/property-generated values; antisymmetry/transitivity hold; signed ordering and cross-kind type-first ordering are pinned; huge exponents terminate within budget or return `resource_limit` without exponent expansion. |
| 3 | Namespace write admission (`A9122-numeric-registry`) | Slices 1–2 and namespace registry authorization/generation seam | Allowed-kind policy is enforced before mutation; rejected kinds and over-budget values return stable errors; admitted keys still require ordinary tenant/namespace authorization and versioned registry mutation. |

## 6. Rollout and rollback

Introduce the codec/version and reference vectors before enabling writes. Deploy readers that understand the numeric profile and return typed unsupported-version errors before any writer emits it. Keep the numeric namespace opt-in and configure allowed kinds and operational budgets per namespace. Enable one namespace at a time after same-version round-trip, comparator, and authorization checks pass.

For existing data, do not rewrite keys in place. If a prior representation exists, use an explicit, auditable migration mapping with collision detection; preserve original bytes and type provenance. During migration, dual-read only when the old representation is unambiguous and write the new version only after readers are deployed. Roll back by disabling new writes and routing reads to retained prior-version data/adapter; never reinterpret an unknown version as another kind or silently coerce it. If no reversible migration exists, keep the affected namespace read-only until an explicit migration decision is made.

## 7. Verification plan

1. **Codec properties:** `decode(encode(x)) == x`, canonical bytes stable on re-encode, kind/type identity preserved, and null/empty/zero representations remain distinct at the enclosing array-address layer.
2. **Ordering oracle:** compare generated triples against a small independent exact reference comparator; check antisymmetry, transitivity, equality consistency, negative reversal, zero normalization, type-first cross-kind order, and signed extremes.
3. **Adversarial/resource tests:** hostile large exponents and magnitude inputs either compare exactly within configured work or return `resource_limit`; prove no giant power-of-ten/power-of-two allocation occurs. Reject NaN/infinity and malformed/noncanonical encodings.
4. **Admission/model tests:** model namespace creation/update and unauthorized, wrong-tenant, stale-generation, and concurrent mutation cases through `RegistryState`; an admitted `NumericKey` alone must never authorize a write.
5. **Cross-implementation vectors:** Rust/native and hosted implementations, if/when both expose this profile, must produce the same canonical bytes, comparison result, and typed error for the same vectors. Unsupported adapter capabilities fail explicitly.
6. **Current baseline:** retain and extend the existing `numeric_registry` integration checks for kind rejection, budget error class, large admitted value, and continued registry authorization. Targeted Rust test command: `cargo test -p id-store --test numeric_registry` (run through the repository's approved Fleet test path before claiming implementation verification). This design-only task does not claim a new test run.

## 8. Open questions and defaults

| Question | Default for this design | Decider / when to revisit |
| --- | --- | --- |
| Should comparison across different numeric kinds compare mathematical value or kind first? | Stable type-tag-first order, no implicit coercion; cross-kind equality is false. | ID core maintainers with the #9107 design owner; before any public interoperability promise. |
| Should IEEE NaN/infinity be admitted at all? | Reject from the ordered registry; any future preservation is a separate non-ordered profile. | ID core maintainers; only if a concrete consumer requires it. |
| What default resource budgets should deployments use? | Keep operational limits configurable and conservative; never describe them as semantic domain limits. | Each namespace operator, with ID core maintainers defining safe defaults after measurements. |
| How should legacy numeric keys migrate if encountered? | Preserve old bytes, use explicit mapping/collision report, and do not rewrite automatically. | Owning namespace operator and ID migration maintainer before enabling writes. |

## 9. Record and tracking

This dated record is the focused #9122 design document. The source tracking issue is [developing-today/code#9122](https://github.com/developing-today/code/issues/9122); its parent program is #9106, under epic #9107. The broad architecture and normative numeric definition remain in [Part A, §3](2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9122--numeric-registry-family), and the ordered implementation slices remain in that design's `implementation-slices.json`. Queue task 21565 tracks this design-slice acceptance. The implementation already present is evidence of current repository state, not evidence that every deployment is released or that cross-edition parity has been proven.

### Evidence checked

- Source issue #9122 and its current comments, read 2026-11-24.
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`, A-9122.1 and implementation-slice references.
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/implementation-slices.json`, A9122 encoding/order/registry entries.
- `pkgs/id/crates/id-store/src/numeric_registry.rs` and `pkgs/id/crates/id-store/tests/numeric_registry.rs`.
- `pkgs/id/book/research/design-review.md`, #9122 entry and its next-action note.
