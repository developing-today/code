# Legacy signed-artifact profile — compatibility finding and known-answer evidence

Slice: **9110-R1** · Issue: [#10146](https://github.com/developing-today/code/issues/10146) ·
Program: #9106 / parent #9110 · Tier: crucial
Design basis: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
section **Cross-part identity profiles consumed by the ports** (statements **S0.7**, **S0.13**, **S0.15**)
Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`
Source of truth in code: `pkgs/id/crates/id-sync/src/artifact.rs`
(`signed_bytes`, `SignedBody`, `SignedArtifact::build_signed`, `check_authenticity`)
Known-answer vectors: `pkgs/id/test-vectors/legacy-artifact-signatures.json`
Validator: `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/validate_legacy_artifact_research.py`

## Finding 1 — the legacy profile, byte for byte

Existing signed artifacts use exactly one signed preimage, produced by
`signed_bytes()` in `artifact.rs`:

- The signed bytes are `serde_json::to_vec(SignedBody)` — compact JSON, no
  whitespace, UTF-8.
- `SignedBody` fields serialize in declaration order: `author`, `seq`,
  `prev`, `kind`, `body`, `after`.
- `after` is **omitted entirely** when empty
  (`#[serde(skip_serializing_if = "Vec::is_empty")]`); when non-empty it is
  a JSON array of lowercase-hex SHA-256 strings in order.
- `body` is a `serde_json::Value`. The workspace does **not** enable
  serde_json's `preserve_order` feature, so object keys inside `body`
  serialize in sorted (BTreeMap) order regardless of construction order.
- String escaping is serde_json's: `\"`, `\\`, `\n`, `\r`, `\t` short forms;
  other C0 control characters as `\u00xx` (lowercase hex); non-ASCII text is
  emitted as raw UTF-8 (no `\uXXXX` escaping).
- `hash` = lowercase hex SHA-256 over the signed bytes.
- `signature` = lowercase hex Ed25519 signature over the signed bytes
  (ed25519-dalek; verification on read is `verify_strict`, and the stored
  `hash` is recomputed and compared before the signature is checked).

These bytes are **immutable legacy behavior**. Any follow-on that adds
records must preserve every existing artifact's bytes exactly: no
re-serialization with different key order, no re-signing, no rehashing.

## Finding 2 — the docs/code mismatch (do not "fix" in place)

The doc comment on `signed_bytes` describes the bytes as
_"length-prefixed and domain-separated"_. The implementation applies **no length prefixes and
no domain separation** — the preimage is exactly
`serde_json::to_vec(SignedBody)` and nothing else. Until an owner-level
reconciliation lands, the comment must not be treated as an interoperability
claim, and the code must not be "corrected" to match the comment: doing so
would change every existing artifact's preimage and invalidate its hash and
signature. This report records the mismatch; it does not resolve it.

## Finding 3 — known-answer vectors independently verify the legacy bytes

`legacy-artifact-signatures.json` carries four vectors produced by the real
`signed_bytes()` at the current main, covering: a minimal `document`
artifact with `after` omitted; a `home_declared` artifact with a two-entry
`after`; string escaping and raw-UTF-8 unicode in `body` (quote, backslash,
newline, tab, a C0 control, an integer and a float); and a `tag` artifact
with a three-entry `after`. The validator re-derives each vector's bytes
from its recorded fields with an independent serializer, and confirms the
SHA-256 and the Ed25519 signature (test-only key, seed `0x07`×32 — never a
production key) for every vector. All four verify unchanged.

## Finding 4 — the new profile is distinct and self-excluding

Per the settled V2 design (S0.7, S0.13, S0.15), the additive profile is a
different profile, never a reinterpretation of the legacy one:

- Encoding: deterministic CBOR (RFC 8949 §4.2: shortest-form heads, map keys
  sorted by their encoded bytes, no indefinite lengths).
- A canonical `Body` commits the payload ID, payload length, domain and
  context — and nothing else.
- The detached signature is computed over domain-prefix + canonical Body.
- The new profile excludes its signature and envelope ID from its own signed preimage:
  neither value appears in, or is derivable from a commitment inside, the
  preimage.
- `EnvelopeId` hashes the complete canonical signed envelope **including**
  the signature, and never includes itself in its own preimage.
- Profile conflicts are reported, never implicitly coerced: a legacy JSON
  artifact is never silently treated as a CBOR record or vice versa.

The vectors file includes one worked design-stage example so these
invariants are machine-checkable today. It is explicitly **not** a
production format: no production producer exists yet, and the example's
concrete choices where the design leaves a digest unnamed (SHA-256 for the
example `PayloadId`/`EnvelopeId`, `utf8(domain) || 0x00` as the domain
prefix) are marked in the vectors file as instantiation notes for the
follow-on implementation slice to adopt or amend. The follow-on Rust
compatibility test is a separate file owned by 9110-11.

## Scope compliance

This slice changed no source artifact bytes, no signature preimages, and
not the versioned-design file. Deliverables are exactly the three owned
files: this report, `pkgs/id/test-vectors/legacy-artifact-signatures.json`,
and `validate_legacy_artifact_research.py`. Rollback is discarding those
three files.
