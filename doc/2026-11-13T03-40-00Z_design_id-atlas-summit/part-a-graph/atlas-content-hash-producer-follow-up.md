# Follow-up: Atlas `content_hash` producer contract

- **Status:** required cross-part follow-up; owner and issue are not assigned here, no issue filed.
- **Evidence basis:** current summit `baseline/v1-audit.md` §“Atlas chunk identity” and “Unknown / bounded-audit gaps”; `baseline/seam-map.md` Atlas identity seam.
- **Why this exists:** checked-in Atlas schema and Rust/Python search code consume `content_hash`, but the baseline found no checked-in production producer for that field. A downstream graph, sidecar, or location index must not infer that identity from a column name or reuse another hash profile.

## Required decisions and evidence before consumers depend on it

1. Name the producer repository/package and exact function or pipeline step that emits a content hash. If producer lives outside this checkout, link source and pin version/revision.
2. Define exact source bytes and boundaries: original file vs normalized text vs chunk bytes vs other derived artifact; canonical framing, encoding, normalization, and whether chunk offsets/metadata participate.
3. Define distinct typed identity profile: algorithm, digest length, output spelling, domain/context, version and migration behavior. State whether this is `PayloadId` (raw payload bytes) or `ContentId` (exact named-profile bytes); do not alias Iroh BLAKE3 blob hashes, event BLAKE3 commitments, signed-artifact SHA-256, Atlas content identity, or embedding identity.
4. Check in cross-language compatibility vectors with input bytes, profile name/version, expected digest bytes, printable form, and rejection cases; independent implementations must verify them.
5. Define backfill/dual-read and collision handling without rewriting old IDs or silently changing existing references. A profile change is additive and explicitly versioned.
6. Confirm the resolution with the existing Atlas identity owner and update the relevant design/issue; this follow-up does not assert a new authority or alter Atlas semantics.

## Done oracle

The producer and independent verifier agree on every checked-in vector; every content-hash consumer names the profile and source-byte contract; legacy rows remain readable without rehash; unknown/mismatched profile returns a typed failure rather than comparing digests from unlike domains. Until then, graph and namespace designs carry the Atlas reference as an opaque externally-owned identifier and do not compute it.
