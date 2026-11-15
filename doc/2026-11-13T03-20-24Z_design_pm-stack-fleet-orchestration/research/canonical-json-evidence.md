# Canonical JSON and digest evidence

**Retrieved:** 2026-11-13 UTC. **Primary source:** RFC Editor,
[RFC 8785, JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785.txt),
June 2020, Informational (not an Internet Standards Track specification).
**Scope:** design input for shared command/result, priority replay, idempotency and context
manifests. No crypto/PKI deliverable, library installation or production change.

## What the source establishes

- Section 3.1 constrains input to I-JSON: no duplicate properties, valid Unicode, and numbers
  representable in IEEE-754 double precision. It recommends strings for larger or more precise
  numbers. This supports decimal-string IDs; it does not permit rounding a queue ID.
- Sections 3.2.1–3.2.4 prescribe primitive serialization, recursive property sorting by UTF-16
  code units, unchanged array element order and final UTF-8 output. Locale sorting, UTF-8 key
  sorting or a default JSON encoder is not automatically the same algorithm.
- String values are preserved; JCS does not apply Unicode normalization. Invalid surrogate
  data, NaN and Infinity require failure, not coercion into a hashable alternative.
- Appendices B/D/E provide numeric and string-subtype interoperability examples. Converting
  a numeric/date string to a native value and serializing it differently changes the data.
- Appendix F's detached signature example removes the signature property before canonicalizing.
  The corresponding manifest-digest exclusion here is a **Fleet design choice**, not a
  claim RFC 8785 defines Fleet's manifest or idempotency semantics.

## Alternatives and choice

| Approach                            | Benefit                                                                                            | Cost / limit                                                                                 | Design choice                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| RFC 8785 over a typed intent DTO    | Existing cross-language specification and test vectors; separates wire pretty-printing from digest | Strict input validation and correct numeric/key serialization required                       | Default for new work-command v1                                          |
| Hand-written fixed-field encoding   | Small for a permanently tiny tuple                                                                 | Escaping, null/default, extension and ordering rules become a custom protocol                | Retain as an alternative only if a reviewed equivalence corpus proves it |
| Language-default JSON serialization | Convenient                                                                                         | Property order, number forms and Unicode behavior may differ; silent mismatch across Rust/TS | Not a protocol definition                                                |
| New binary/canonical-CBOR wire      | Precise type model possible                                                                        | New client/wire/tooling burden without measured need                                         | Not selected; no new binary or infrastructure                            |

The choice is bounded to the new interface. Do **not** rewrite legacy creation-outbox payload
hashes or declare them equivalent without an explicit compatibility fixture/migration.
Hashing is not authentication, authorization, semantic validation or remote idempotency.

## Required implementation fixtures and oracles

Own these with the existing `fleet-queue` shared `work/contracts.rs` contract slice (CLI-02),
and the hydration manifest slice HYD-01. Registry wiring remains CLI-13. The concrete tests
are future implementation work, not tests claimed to have run in this design pass.

1. RFC key-order and numeric vectors produce identical canonical bytes/hash on Rust and TS.
2. Duplicate keys, invalid surrogates, non-finite values and a numeric ID above the exact range
   fail before accepting an operation; the decimal-string form round-trips without conversion.
3. Property order and JSON whitespace do not alter the digest. Array order/string bytes do
   unless the typed command schema explicitly normalizes a semantic set before JCS.
4. New operation/delivery IDs, output format and wait budget do not change intent digest;
   changed target, arguments, reason, expected revision, fence or snapshot do. A matching
   digest never bypasses current principal/capability checks.
5. Omitted/default/null distinctions follow the typed DTO schema, not encoder accidents.
6. A manifest digest excludes only its own digest field; any included source version, artifact
   hash or completeness flag change is detected. Artifact hashes cover exact stored bytes.
7. Authority-qualified work IDs `9` and `10` compare numerically with exact integers despite
   string wire representation. Opaque session IDs are not parsed as work IDs.

**Oracle:** published RFC canonical bytes plus independent cross-language golden DTOs; application
intent-inclusion/exclusion tables and schema validation; exact artifact-byte SHA-256. A JSON
object that merely hashes itself with the same encoder is not an independent oracle.

## Revisit triggers

Revisit for a demonstrated existing canonicalizer mismatch, a future non-JSON target, incompatible
numeric/subtype requirements, or measured serialization cost. Any replacement needs an explicit
version, migration/rollback contract and the same negative/conformance fixtures. None of these
choices changes live queue values, claim semantics, dispatch or the bounded outbox pilot.
