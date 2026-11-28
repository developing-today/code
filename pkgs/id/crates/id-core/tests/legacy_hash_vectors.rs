//! Legacy hash golden vectors and profile separation (slice
//! A9113-legacy-hash-vectors, issue #10225; design A-9113.2, S3.4–S3.10).
//!
//! Oracle: **golden** — fixed byte-level vectors from known sources:
//! the published BLAKE3 test vectors (input byte `i` is `i % 251`) and
//! Iroh's own `Hash::new` / `Hash::EMPTY`, run through the configured
//! profile dispatch. The legacy SHA-256 signed-artifact commitment is a
//! separate, domain-specific ID: it has no content-profile token, and a
//! numerically equal digest under the Atlas text profile is still a
//! different identity. The signed-artifact bytes themselves are pinned
//! in `id-sync/tests/legacy_artifact_vectors.rs`.
//!
//! The crate-root export of `hash_profile` belongs to slice 9110-02, so
//! until that lands this test includes it, and `id_shape`, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

use hash_profile::ProfileDispatch;
use id_shape::{ContentId, HashProfile, IdShapeError};

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn vector_input(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

/// Published BLAKE3 vectors: (input length, hash).
const BLAKE3_VECTORS: [(usize, &str); 4] = [
    (
        0,
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
    ),
    (
        1,
        "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
    ),
    (
        1023,
        "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
    ),
    (
        1024,
        "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
    ),
];

/// The legacy artifact preimage pinned in id-sync, and its SHA-256.
const ARTIFACT_PREIMAGE: &str = concat!(
    r#"{"author":"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a","#,
    r#""seq":1,"prev":"","kind":"document","#,
    r#""body":{"content":{"tags":["a","b"],"text":"hello"},"media_type":"application/json"}}"#,
);
const ARTIFACT_HASH: &str = "afab18aed3d6447b6b33fb3441d43f9e1ea909c4d3ef0335674670bbe412bc37";

#[test]
fn raw_blake3_profile_reproduces_published_vectors_and_iroh() {
    let dispatch = ProfileDispatch::default();
    let verifier = dispatch.resolve_token("raw-blake3-256").unwrap();
    for (len, expected) in BLAKE3_VECTORS {
        let input = vector_input(len);
        let mut state = verifier.start();
        state.update(&input).unwrap();
        let digest = state.finish().unwrap();
        assert_eq!(hex(&digest), expected, "len {len}");
        // Iroh's hash of the same bytes, unchanged: no prefix or framing.
        assert_eq!(iroh_blobs::Hash::new(&input).as_bytes(), &digest);
        let id = ContentId::new(HashProfile::RawBlake3_256, digest);
        dispatch.verify_bytes(&id, &input).unwrap();
    }
    assert_eq!(hex(iroh_blobs::Hash::EMPTY.as_bytes()), BLAKE3_VECTORS[0].1);
}

#[test]
fn profile_tags_and_tokens_are_pinned() {
    assert_eq!(HashProfile::RawBlake3_256.token(), "raw-blake3-256");
    assert_eq!(HashProfile::RawBlake3_256.tag(), 0x01);
    assert_eq!(
        HashProfile::AtlasChunkSha256Utf8.token(),
        "atlas-chunk-sha256-utf8"
    );
    assert_eq!(HashProfile::AtlasChunkSha256Utf8.tag(), 0x02);
}

#[test]
fn legacy_artifact_sha256_is_not_a_content_profile() {
    let dispatch = ProfileDispatch::default();
    for token in [
        "sha256",
        "sha-256",
        "raw-sha256",
        "artifact-sha256",
        "legacy-artifact-sha256",
        "signed-artifact-sha256",
    ] {
        assert_eq!(
            dispatch.resolve_token(token).unwrap_err(),
            IdShapeError::UnsupportedProfile {
                profile: token.to_owned()
            }
        );
    }
}

#[test]
fn equal_sha256_digest_is_still_a_different_identity() {
    // The legacy preimage happens to be UTF-8 text, so the Atlas text
    // profile computes the same SHA-256 bytes as the artifact hash.
    // That makes them numerically equal, not interchangeable: the Atlas
    // content ID names chunk text, the artifact hash names a signed
    // statement, and nothing converts one into the other.
    let atlas = ContentId::of_bytes(
        HashProfile::AtlasChunkSha256Utf8,
        ARTIFACT_PREIMAGE.as_bytes(),
    )
    .unwrap();
    assert_eq!(hex(atlas.digest()), ARTIFACT_HASH);
    let raw =
        ContentId::of_bytes(HashProfile::RawBlake3_256, ARTIFACT_PREIMAGE.as_bytes()).unwrap();
    assert_ne!(hex(raw.digest()), ARTIFACT_HASH);
    // Under the raw profile, the artifact hash bytes do not verify the
    // preimage: profiles never translate.
    let as_raw = ContentId::new(HashProfile::RawBlake3_256, *atlas.digest());
    assert_eq!(
        ProfileDispatch::default()
            .verify_bytes(&as_raw, ARTIFACT_PREIMAGE.as_bytes())
            .unwrap_err()
            .rejection()
            .cloned(),
        Some(IdShapeError::DigestMismatch)
    );
}
