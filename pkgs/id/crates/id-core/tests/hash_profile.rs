//! Golden-vector and invariant tests for static/configured hash-profile
//! dispatch (slice A9113-profile-dispatch, issue #10198; design
//! A-9113.2, S3.2–S3.10).
//!
//! Oracle: **invariant** — a profile token selects exactly one compiled
//! verifier; every unknown or policy-excluded token rejects as
//! `unsupported_profile` before any content is read; verification is
//! recomputation under the ID's own profile only (no translation map:
//! a digest under one profile never verifies as another); streaming in
//! arbitrary chunks gives the one-shot digest; invalid UTF-8 under the
//! Atlas text profile rejects as invalid profile parameters; and an
//! over-budget stream rejects as a resource limit.
//!
//! The crate-root export of `hash_profile` belongs to slice 9110-02, so
//! until that lands this test includes it, and the `id_shape` module it
//! builds on, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
mod hash_profile;

use hash_profile::{COMPILED_VERIFIERS, ProfileDispatch, VerifyError};
use id_shape::{ContentId, HashProfile, IdShapeError, ParsePolicy};
use proptest::prelude::*;
use std::io::{self, Read};

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn digest_of(profile: HashProfile, content: &[u8]) -> [u8; 32] {
    let mut state = ProfileDispatch::default().resolve(profile).unwrap().start();
    state.update(content).unwrap();
    state.finish().unwrap()
}

/// A reader that hands out its bytes in the given chunk sizes and
/// counts how many reads were made.
struct Chunked<'a> {
    data: &'a [u8],
    sizes: Vec<usize>,
    reads: usize,
}

impl Read for Chunked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.data.is_empty() {
            return Ok(0);
        }
        let want = self
            .sizes
            .get(self.reads - 1)
            .copied()
            .unwrap_or(usize::MAX)
            .max(1);
        let n = want.min(buf.len()).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

/// A reader that must never be read.
struct Untouchable;

impl Read for Untouchable {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("content was read before the profile was resolved");
    }
}

fn rejection(result: Result<hash_profile::Verified, VerifyError>) -> IdShapeError {
    match result {
        Err(VerifyError::Rejected(error)) => error,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn profile_token_golden_vectors() {
    let cases = [
        (
            "raw-blake3-256",
            HashProfile::RawBlake3_256,
            1u8,
            &b""[..],
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            "raw-blake3-256",
            HashProfile::RawBlake3_256,
            1,
            b"abc",
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        ),
        (
            "atlas-chunk-sha256-utf8",
            HashProfile::AtlasChunkSha256Utf8,
            2,
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "atlas-chunk-sha256-utf8",
            HashProfile::AtlasChunkSha256Utf8,
            2,
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    ];
    let dispatch = ProfileDispatch::default();
    for (token, profile, tag, content, digest) in cases {
        let verifier = dispatch.resolve_token(token).unwrap();
        assert_eq!(verifier.profile(), profile);
        assert_eq!(verifier.token(), token);
        assert_eq!(profile.tag(), tag);
        assert_eq!(hex(&digest_of(profile, content)), digest);
        // The streaming digest agrees with id_shape's one-shot digest.
        assert_eq!(
            digest_of(profile, content),
            profile.digest(content).unwrap()
        );
        let id = ContentId::of_bytes(profile, content).unwrap();
        let outcome = dispatch.verify_bytes(&id, content).unwrap();
        assert_eq!(outcome.profile, profile);
        assert_eq!(outcome.bytes, content.len() as u64);
    }
}

#[test]
fn each_compiled_profile_has_exactly_one_verifier() {
    for profile in HashProfile::ALL {
        let matching = COMPILED_VERIFIERS
            .iter()
            .filter(|verifier| verifier.profile() == profile)
            .count();
        assert_eq!(matching, 1, "{profile}");
    }
    assert_eq!(COMPILED_VERIFIERS.len(), HashProfile::ALL.len());
}

#[test]
fn unsupported_tokens_reject_exactly() {
    let dispatch = ProfileDispatch::default();
    for token in [
        "",
        "RAW-BLAKE3-256",
        "raw-blake3",
        "raw-blake3-256 ",
        "raw-blake3-2566",
        "sha256",
        "atlas-chunk-sha256",
        "atlas-chunk-sha256-utf8/v2",
    ] {
        assert_eq!(
            dispatch.resolve_token(token).unwrap_err(),
            IdShapeError::UnsupportedProfile {
                profile: token.to_owned()
            },
            "{token:?}"
        );
    }
}

#[test]
fn excluded_profile_rejects_before_reading_content() {
    let only_raw = ProfileDispatch::new(ParsePolicy::only(HashProfile::RawBlake3_256), 1 << 20);
    let atlas_id = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, b"chunk").unwrap();
    assert_eq!(
        rejection(only_raw.verify_content(&atlas_id, Untouchable)),
        IdShapeError::UnsupportedProfile {
            profile: "atlas-chunk-sha256-utf8".to_owned()
        }
    );
    assert!(!only_raw.policy().admits(HashProfile::AtlasChunkSha256Utf8));
    assert!(only_raw.resolve_token("atlas-chunk-sha256-utf8").is_err());
    assert!(only_raw.resolve_token("raw-blake3-256").is_ok());
}

#[test]
fn no_translation_between_profiles() {
    // The same content under two profiles gives two IDs; neither
    // digest verifies under the other profile's tag.
    let content = b"the same bytes";
    let raw = ContentId::of_bytes(HashProfile::RawBlake3_256, content).unwrap();
    let atlas = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, content).unwrap();
    let swapped_raw = ContentId::new(HashProfile::RawBlake3_256, *atlas.digest());
    let swapped_atlas = ContentId::new(HashProfile::AtlasChunkSha256Utf8, *raw.digest());
    let dispatch = ProfileDispatch::default();
    assert!(dispatch.verify_bytes(&raw, content).is_ok());
    assert!(dispatch.verify_bytes(&atlas, content).is_ok());
    for id in [swapped_raw, swapped_atlas] {
        assert_eq!(
            rejection(dispatch.verify_bytes(&id, content)),
            IdShapeError::DigestMismatch
        );
    }
}

#[test]
fn atlas_profile_rejects_invalid_text_as_profile_parameters() {
    let dispatch = ProfileDispatch::default();
    let id = ContentId::new(HashProfile::AtlasChunkSha256Utf8, [0; 32]);
    for bad in [&b"\xff"[..], b"ok\xc3", b"\xe2\x82", b"a\xed\xa0\x80b"] {
        assert!(
            matches!(
                rejection(dispatch.verify_bytes(&id, bad)),
                IdShapeError::InvalidProfileParameters {
                    profile: HashProfile::AtlasChunkSha256Utf8,
                    ..
                }
            ),
            "{bad:?}"
        );
    }
    // The raw profile hashes the same bytes without judging them.
    let raw = ContentId::of_bytes(HashProfile::RawBlake3_256, b"\xff").unwrap();
    assert!(dispatch.verify_bytes(&raw, b"\xff").is_ok());
}

#[test]
fn over_budget_stream_is_a_resource_limit() {
    let content = vec![7u8; 100];
    let id = ContentId::of_bytes(HashProfile::RawBlake3_256, &content).unwrap();
    let tight = ProfileDispatch::new(ParsePolicy::ALL, 99);
    assert!(matches!(
        rejection(tight.verify_bytes(&id, &content)),
        IdShapeError::ResourceLimit { .. }
    ));
    let exact = ProfileDispatch::new(ParsePolicy::ALL, 100);
    assert_eq!(exact.verify_bytes(&id, &content).unwrap().bytes, 100);
}

#[test]
fn read_errors_surface_as_io() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disk on fire"))
        }
    }
    let id = ContentId::of_bytes(HashProfile::RawBlake3_256, b"x").unwrap();
    assert!(matches!(
        ProfileDispatch::default().verify_content(&id, Broken),
        Err(VerifyError::Io(_))
    ));
}

proptest! {
    #[test]
    fn chunked_stream_matches_one_shot(
        content in proptest::collection::vec(any::<u8>(), 0..4096),
        sizes in proptest::collection::vec(1usize..300, 0..40),
    ) {
        let id = ContentId::of_bytes(HashProfile::RawBlake3_256, &content).unwrap();
        let reader = Chunked { data: &content, sizes, reads: 0 };
        let verified = ProfileDispatch::default().verify_content(&id, reader).unwrap();
        prop_assert_eq!(verified.bytes, content.len() as u64);
    }

    #[test]
    fn utf8_split_mid_code_point_still_verifies(
        text in "\\PC{0,200}",
        sizes in proptest::collection::vec(1usize..5, 0..200),
    ) {
        let bytes = text.as_bytes();
        let id = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, bytes).unwrap();
        let reader = Chunked { data: bytes, sizes, reads: 0 };
        prop_assert!(ProfileDispatch::default().verify_content(&id, reader).is_ok());
    }

    #[test]
    fn atlas_accepts_exactly_valid_utf8(
        bytes in proptest::collection::vec(any::<u8>(), 0..64),
        sizes in proptest::collection::vec(1usize..4, 0..64),
    ) {
        let id = ContentId::new(HashProfile::AtlasChunkSha256Utf8, digest_of(HashProfile::RawBlake3_256, &bytes));
        let reader = Chunked { data: &bytes, sizes, reads: 0 };
        let outcome = ProfileDispatch::default().verify_content(&id, reader);
        let error = outcome.unwrap_err().rejection().cloned().unwrap();
        if std::str::from_utf8(&bytes).is_ok() {
            // Valid text gets as far as the digest comparison.
            prop_assert_eq!(error, IdShapeError::DigestMismatch);
        } else {
            let is_parameter_error = matches!(error, IdShapeError::InvalidProfileParameters { .. });
            prop_assert!(is_parameter_error);
        }
    }

    #[test]
    fn any_other_token_is_unsupported(token in "\\PC{0,40}") {
        prop_assume!(HashProfile::from_token(&token).is_none());
        let error = ProfileDispatch::default().resolve_token(&token).unwrap_err();
        prop_assert_eq!(error, IdShapeError::UnsupportedProfile { profile: token });
    }

    #[test]
    fn corrupted_content_never_verifies(
        content in proptest::collection::vec(any::<u8>(), 1..512),
        index in any::<usize>(),
        flip in 1u8..=255,
    ) {
        let id = ContentId::of_bytes(HashProfile::RawBlake3_256, &content).unwrap();
        let mut corrupted = content;
        let i = index % corrupted.len();
        corrupted[i] ^= flip;
        prop_assert_eq!(
            rejection(ProfileDispatch::default().verify_bytes(&id, &corrupted)),
            IdShapeError::DigestMismatch
        );
    }
}
