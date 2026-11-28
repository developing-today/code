//! Golden vectors for the content-address core identity (slice
//! A9120-content-id, issue #10227; design A-9113.2 and A-9120.1,
//! S3.4–S3.10 and S4.19–S4.22).
//!
//! Oracle: **golden** — the pinned Iroh `Hash::new` implementation and
//! published BLAKE3 bytes. The `raw-blake3-256` content ID of a byte
//! string equals BLAKE3 of exactly those bytes and Iroh's hash of them;
//! Iroh's binary and text forms read and write unchanged (checked
//! against Iroh itself and an independent base32 reference); and asking
//! for an Iroh form of an ID under another profile is an explicit
//! profile mismatch.
//!
//! The crate-root export of `content_id` belongs to slice 9110-02, so
//! until that lands this test includes it, and `id_shape`, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/content_id.rs"]
mod content_id;

use content_id::{
    ContentIdError, ContentTextForm, IROH_PROFILE, format_content_text, from_iroh_hash,
    parse_content_text, parse_legacy_bytes, raw_content_id, to_iroh_hash, to_legacy_bytes,
};
use id_shape::{ContentId, DerivedId, DerivedKind, HashProfile, Id, ParsePolicy};
use proptest::prelude::*;
use std::str::FromStr as _;

const ALL: ParsePolicy = ParsePolicy::ALL;
const ATLAS: HashProfile = HashProfile::AtlasChunkSha256Utf8;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// RFC 4648 base32 without padding: an independent reference for the
/// base32 text Iroh accepts.
fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The BLAKE3 test-vector input: byte `i` is `i % 251`.
fn vector_input(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

#[test]
fn raw_profile_matches_published_blake3_and_iroh() {
    let cases: [(Vec<u8>, &str); 5] = [
        (
            vector_input(0),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            vector_input(1),
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
        ),
        (
            vector_input(1023),
            "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
        ),
        (
            vector_input(1024),
            "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
        ),
        (
            b"abc".to_vec(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        ),
    ];
    for (input, digest) in cases {
        let id = raw_content_id(&input);
        assert_eq!(id.profile(), IROH_PROFILE);
        assert_eq!(hex(id.digest()), digest, "len {}", input.len());
        // Iroh's own hash of the same bytes, with no prefix or framing.
        let iroh = iroh_blobs::Hash::new(&input);
        assert_eq!(from_iroh_hash(&iroh), id);
        assert_eq!(to_iroh_hash(&id).unwrap(), iroh);
        assert_eq!(
            ContentId::of_bytes(HashProfile::RawBlake3_256, &input).unwrap(),
            id
        );
    }
    assert_eq!(
        from_iroh_hash(&iroh_blobs::Hash::EMPTY),
        raw_content_id(b"")
    );
}

#[test]
fn profiled_form_carries_the_profile_token() {
    let id = raw_content_id(b"");
    let text = format_content_text(&id, ContentTextForm::Profiled).unwrap();
    assert_eq!(
        text,
        "content/raw-blake3-256.af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
    assert_eq!(
        parse_content_text(&text, ContentTextForm::Profiled, ALL).unwrap(),
        id
    );
    // The Atlas profile has its own token and digest, never the raw one.
    let atlas = ContentId::of_bytes(ATLAS, b"abc").unwrap();
    assert_eq!(
        format_content_text(&atlas, ContentTextForm::Profiled).unwrap(),
        "content/atlas-chunk-sha256-utf8.ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn iroh_legacy_text_forms_are_unchanged() {
    let id = raw_content_id(b"abc");
    let iroh = iroh_blobs::Hash::new(b"abc");
    // Written exactly as Iroh writes it: 64 lowercase hex.
    let written = format_content_text(&id, ContentTextForm::IrohLegacy).unwrap();
    assert_eq!(written, iroh.to_string());
    assert_eq!(written, hex(id.digest()));
    // Read in every spelling Iroh reads.
    let b32 = base32(id.digest());
    assert_eq!(b32.len(), 52);
    for text in [written.clone(), b32.clone(), b32.to_ascii_lowercase()] {
        assert_eq!(
            parse_content_text(&text, ContentTextForm::IrohLegacy, ALL).unwrap(),
            id,
            "{text}"
        );
        assert_eq!(iroh_blobs::Hash::from_str(&text).unwrap(), iroh);
    }
    // The legacy form is not the profiled form, and vice versa (the
    // profiled text is 87 characters, a length at which Iroh's own
    // parser would panic).
    assert_eq!(
        parse_content_text(&written, ContentTextForm::Profiled, ALL)
            .unwrap_err()
            .class(),
        "malformed"
    );
    let profiled = format_content_text(&id, ContentTextForm::Profiled).unwrap();
    assert_eq!(
        parse_content_text(&profiled, ContentTextForm::IrohLegacy, ALL)
            .unwrap_err()
            .class(),
        "malformed"
    );
}

#[test]
fn iroh_legacy_binary_form_is_the_bare_digest() {
    let id = raw_content_id(b"abc");
    let bytes = to_legacy_bytes(&id).unwrap();
    assert_eq!(&bytes, iroh_blobs::Hash::new(b"abc").as_bytes());
    assert_eq!(parse_legacy_bytes(&bytes, ALL).unwrap(), id);
    for len in [0, 31, 33, 34] {
        assert_eq!(
            parse_legacy_bytes(&vec![0; len], ALL).unwrap_err().class(),
            "malformed",
            "len {len}"
        );
    }
}

#[test]
fn other_profiles_have_no_iroh_form() {
    let atlas = ContentId::of_bytes(ATLAS, b"abc").unwrap();
    let mismatch = ContentIdError::ProfileMismatch {
        expected: HashProfile::RawBlake3_256,
        found: ATLAS,
    };
    assert_eq!(to_iroh_hash(&atlas).unwrap_err(), mismatch);
    assert_eq!(to_legacy_bytes(&atlas).unwrap_err(), mismatch);
    assert_eq!(
        format_content_text(&atlas, ContentTextForm::IrohLegacy).unwrap_err(),
        mismatch
    );
    assert_eq!(mismatch.class(), "profile_mismatch");
}

#[test]
fn policy_that_excludes_raw_rejects_every_raw_form() {
    let only_atlas = ParsePolicy::only(ATLAS);
    let id = raw_content_id(b"abc");
    let legacy = format_content_text(&id, ContentTextForm::IrohLegacy).unwrap();
    let profiled = format_content_text(&id, ContentTextForm::Profiled).unwrap();
    for error in [
        parse_content_text(&legacy, ContentTextForm::IrohLegacy, only_atlas).unwrap_err(),
        parse_content_text(&profiled, ContentTextForm::Profiled, only_atlas).unwrap_err(),
        parse_legacy_bytes(id.digest(), only_atlas).unwrap_err(),
    ] {
        assert_eq!(error.class(), "unsupported_profile");
    }
}

#[test]
fn profiled_form_of_another_shape_is_not_a_content_id() {
    let derived = Id::Derived(DerivedId::new(DerivedKind::Name, b"n".to_vec()).unwrap()).to_text();
    assert_eq!(
        parse_content_text(&derived, ContentTextForm::Profiled, ALL)
            .unwrap_err()
            .class(),
        "unsupported_shape"
    );
}

proptest! {
    #[test]
    fn content_id_is_iroh_hash_of_exact_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let id = raw_content_id(&bytes);
        prop_assert_eq!(id, from_iroh_hash(&iroh_blobs::Hash::new(&bytes)));
        prop_assert_eq!(*id.digest(), *blake3::hash(&bytes).as_bytes());
    }

    #[test]
    fn every_form_round_trips(digest in any::<[u8; 32]>()) {
        let id = ContentId::new(HashProfile::RawBlake3_256, digest);
        for form in [ContentTextForm::Profiled, ContentTextForm::IrohLegacy] {
            let text = format_content_text(&id, form).unwrap();
            prop_assert_eq!(parse_content_text(&text, form, ALL).unwrap(), id);
        }
        prop_assert_eq!(parse_legacy_bytes(&to_legacy_bytes(&id).unwrap(), ALL).unwrap(), id);
        let b32 = base32(&digest);
        prop_assert_eq!(
            parse_content_text(&b32, ContentTextForm::IrohLegacy, ALL).unwrap(),
            id
        );
    }

    #[test]
    fn legacy_text_accepts_exactly_what_iroh_accepts(
        text in prop_oneof!["[0-9a-zA-Z=]{64}", "[0-9a-zA-Z=]{52}", "[0-9a-fA-F]{64}", "[a-zA-Z2-7]{52}"],
    ) {
        // Iroh's parser is only called at the two lengths it accepts;
        // at any other length it panics instead of returning an error.
        let ours = parse_content_text(&text, ContentTextForm::IrohLegacy, ALL).ok();
        let iroh = iroh_blobs::Hash::from_str(&text).ok().map(|h| from_iroh_hash(&h));
        prop_assert_eq!(ours, iroh);
    }

    #[test]
    fn other_lengths_reject_without_panicking(text in "\\PC{0,120}") {
        prop_assume!(text.len() != 64 && text.len() != 52);
        let error = parse_content_text(&text, ContentTextForm::IrohLegacy, ALL).unwrap_err();
        prop_assert_eq!(error.class(), "malformed");
    }

    #[test]
    fn near_miss_legacy_spellings_agree_with_iroh(digest in any::<[u8; 32]>(), index in 0usize..64) {
        let id = ContentId::new(HashProfile::RawBlake3_256, digest);
        let hex_text = format_content_text(&id, ContentTextForm::IrohLegacy).unwrap();
        // Uppercase one hex digit.
        let mut upper = hex_text.clone().into_bytes();
        upper[index] = upper[index].to_ascii_uppercase();
        let upper = String::from_utf8(upper).unwrap();
        let ours = parse_content_text(&upper, ContentTextForm::IrohLegacy, ALL).ok();
        let iroh = iroh_blobs::Hash::from_str(&upper).ok().map(|h| from_iroh_hash(&h));
        prop_assert_eq!(ours, iroh);
        // One character short is rejected, where Iroh itself would panic.
        let short = parse_content_text(&hex_text[..63], ContentTextForm::IrohLegacy, ALL);
        prop_assert_eq!(short.unwrap_err().class(), "malformed");
    }
}
