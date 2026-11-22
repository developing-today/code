# Atlas chunk-hash compatibility boundary

Slice: `A9113-atlas-profile-contract` · issue [#10179](https://github.com/developing-today/code/issues/10179)

## Disposition

The required 9110-R2 producer research is complete, but its verified result is
**producer unavailable**: no checked-in production producer or pinned
byte-level contract for Atlas corpus `chunks.content_hash` was established.
The governing evidence is the [9110-R2 research record](../crosscut/research-results/atlas-content-hash-producer.md)
and its vector file, which records `producer_unavailable` with no chunk
algorithm, input bytes, profile, or known-answer vectors.

Consequently, Atlas `chunks.content_hash` remains an opaque foreign identifier.
This slice does not recompute it, map it to an ID `ContentId`, or claim that it
matches Iroh's `raw-blake3-256` (or any other profile). No source hash or
preimage is fabricated to fill the empty vector set.

## Typed boundary

The ID model's `atlas-chunk-sha256-utf8` profile remains a distinct typed
profile for the previously observed snapshot shape; its presence is not proof
of the production producer contract and must not be used to assert
Atlas/ID interoperability. Profiled parsing preserves the tag, and conversion
to an Iroh legacy hash returns `profile_mismatch`. A raw-only parse policy
rejects the Atlas tag as `unsupported_profile`.

Compatibility can be reconsidered only after producer evidence supplies the
exact input bytes, canonicalization, algorithm, and an independently
recomputed known-answer vector. Until then, callers must preserve the Atlas
value as opaque text and reject cross-profile conversion claims.

## Verification

The golden fixture is
[`pkgs/id/test-vectors/atlas-content-hash-v1.json`](../../../../pkgs/id/test-vectors/atlas-content-hash-v1.json).
The seam test is `cargo test -p id-core --test atlas_content_profile_vectors`;
it checks the unavailable disposition and the non-converting typed boundary.
