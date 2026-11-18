# Legacy artifact and hash compatibility

Slice A9113-legacy-hash-vectors (#10225), design A-9113.2 and A-9114.3, statements S3.4–S3.10.

This page records the byte formats that already exist in `pkgs/id`, the vectors that pin them, and one place where the documentation and the code disagree. Nothing here changes a serializer, a hash or a signature. Old data keeps verifying over its exact old bytes.

## Content IDs: `raw-blake3-256`

- Digest: unkeyed BLAKE3-256 over the exact content bytes. No prefix, normalization, metadata, location or media fields.
- This is exactly `iroh_blobs::Hash::new(bytes)`. Iroh's binary form (the bare 32-byte digest) and text form (written as 64 lowercase hex, read as hex or 52-character unpadded base32) are unchanged.
- Profile token `raw-blake3-256`, binary profile tag `0x01`.

Pinned in `pkgs/id/crates/id-core/tests/legacy_hash_vectors.rs` with the published BLAKE3 test vectors (input byte `i` is `i % 251`):

| Input length | BLAKE3-256 |
| --- | --- |
| 0 | `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` |
| 1 | `2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213` |
| 1023 | `10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11` |
| 1024 | `42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7` |

## Signed artifacts (`id_sync::artifact`)

The format as the code writes it:

- **Preimage:** compact `serde_json::to_vec` of `{author, seq, prev, kind, body, after}` in that field order. `after` is omitted when empty. `body` is a `serde_json::Value`, so its object keys are sorted.
- **`hash`:** lowercase hex SHA-256 of the preimage.
- **`signature`:** lowercase hex Ed25519 over the same preimage.
- **Wire form:** the signed fields, then `hash`, then `signature`, as one JSON object.

Pinned in `pkgs/id/crates/id-sync/tests/legacy_artifact_vectors.rs`. The vectors were computed outside the codebase (Python `hashlib` and `cryptography`) from a hand-written preimage, with the RFC 8032 section 7.1 TEST 1 key (seed `9d61b19d…7f60`, public key `d75a9801…511a`):

| Artifact | SHA-256 `hash` |
| --- | --- |
| seq 1, `document` | `afab18aed3d6447b6b33fb3441d43f9e1ea909c4d3ef0335674670bbe412bc37` |
| seq 2, `tag` on seq 1 | `1932e31b0e5829b03621f21fc0bdb164f48d2058f5b872b7f65838b02b96a26d` |

The full preimages and signatures are in the test file.

### Documentation and code disagree

The doc comment on `artifact::signed_bytes` says the preimage is "length-prefixed and domain-separated". It is neither. The code serializes plain compact JSON with no domain prefix and no length framing, and every artifact ever signed uses those bytes. The B05 slice (`federation/slices/b05_signer_binding.rs`, #10208) found the same thing.

The code is the record. The fix is never to change the preimage or re-sign old artifacts. New signed records use the deterministic CBOR profile with a domain prefix (A-9114.3, `signed_envelope.rs` in id-core). Correcting the doc comment is a separate, wording-only change.

## The two SHA-256 IDs are different identities

The artifact `hash` is a domain-specific commitment to a signed statement. It is not a content profile, and no content-profile token names it: `sha256`, `artifact-sha256` and similar tokens are `unsupported_profile`.

The Atlas text profile `atlas-chunk-sha256-utf8` is also SHA-256, over UTF-8 chunk text. An artifact preimage is UTF-8 JSON, so its Atlas content ID has the same digest bytes as its artifact hash. They are still two identities: one names chunk text, the other names a signed statement, and nothing converts between them. Under `raw-blake3-256` the same digest bytes do not verify the preimage.

## Rules

1. Never change a legacy serializer, hash or signature preimage.
2. Never re-hash or re-sign stored artifacts to move them to a new profile.
3. If a pinned vector fails, restore the old bytes. Do not re-baseline the vector.
4. New formats are additive and carry an explicit profile or version.
