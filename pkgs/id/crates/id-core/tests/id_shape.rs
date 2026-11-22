//! Golden-vector and property tests for the ID shape discriminator
//! (slice A9113-shape-types, issue #10180; design A-9113.1/A-9113.2,
//! statements S3.1–S3.3).
//!
//! Oracle: **invariant** — a finite typed reference enum; every
//! known encoding maps to exactly one variant; unknown critical
//! tags return `UnsupportedShape`; canonical `encode(parse(x))`
//! equals `x` — checked over generated inputs, with the exact
//! failure classification asserted for unknown shape/version/
//! profile inputs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_core::id_shape::{
    ArrayId, ContentId, DerivedId, DerivedKind, HashProfile, Id, IdShape, IdShapeError,
    ParsePolicy, format_id, parse_id_bytes, parse_id_text,
};
use proptest::prelude::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

// ---------- golden digest vectors (independent of this code) ----------

#[test]
fn digest_vectors_match_the_profiles() {
    // BLAKE3 of the empty input (published BLAKE3 test vector).
    let raw = ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap();
    assert_eq!(
        hex(raw.digest()),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
    // SHA-256 of "hello" (published SHA-256 test vector), under the
    // Atlas UTF-8 text profile.
    let atlas = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, b"hello").unwrap();
    assert_eq!(
        hex(atlas.digest()),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    // The two profiles are distinct shapes of identity: the same
    // bytes under the other profile do not verify.
    assert_eq!(raw.verify(b"hello"), Err(IdShapeError::DigestMismatch),);
    assert!(raw.verify(b"").is_ok());
    assert!(atlas.verify(b"hello").is_ok());
}

#[test]
fn atlas_profile_rejects_non_utf8_input() {
    assert_eq!(
        ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, &[0xff, 0xfe]),
        Err(IdShapeError::InvalidProfileParameters {
            profile: HashProfile::AtlasChunkSha256Utf8,
            detail: "atlas-chunk-sha256-utf8 is defined over UTF-8 text",
        })
    );
}

// ---------- golden binary vectors (from the module-doc layout) ----------

fn golden_binary() -> Vec<(Id, &'static str)> {
    vec![
        (
            Id::Content(ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap()),
            "010101af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            Id::Content(ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, b"hello").unwrap()),
            "0101022cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        ),
        (Id::Array(ArrayId::new(vec![]).unwrap()), "020100000000"),
        (
            Id::Array(ArrayId::new(vec![None, Some(vec![])]).unwrap()),
            "020102000000000100000000",
        ),
        (
            Id::Derived(DerivedId::new(DerivedKind::Name, b"wiki/Home".to_vec()).unwrap()),
            "0301010900000077696b692f486f6d65",
        ),
    ]
}

#[test]
fn golden_binary_vectors_round_trip() {
    for (id, encoding) in golden_binary() {
        assert_eq!(hex(&id.to_bytes()), encoding, "encode {id:?}");
        let parsed = Id::parse_bytes(&unhex(encoding)).unwrap();
        assert_eq!(parsed, id, "decode {encoding}");
        assert_eq!(parsed.shape(), id.shape());
        assert_eq!(parsed.version(), 1);
    }
}

// ---------- golden text vectors (design #9117 forms) ----------

#[test]
fn golden_text_vectors_round_trip() {
    let cases: Vec<(Id, &str)> = vec![
        (
            Id::Content(ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap()),
            "content/raw-blake3-256.af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        // The array examples are the design's own (#9117 grammar).
        (Id::Array(ArrayId::new(vec![]).unwrap()), "array"),
        (Id::Array(ArrayId::new(vec![None]).unwrap()), "array/~"),
        (
            Id::Array(ArrayId::new(vec![Some(vec![])]).unwrap()),
            "array/b.",
        ),
        (
            Id::Array(ArrayId::new(vec![Some(b"/".to_vec())]).unwrap()),
            "array/b.%2F",
        ),
        (
            Id::Array(ArrayId::new(vec![None, Some(vec![0x00, 0xff])]).unwrap()),
            "array/~/b.%00%FF",
        ),
        (
            Id::Array(ArrayId::new(vec![Some(b"us president.md".to_vec())]).unwrap()),
            "array/b.us%20president.md",
        ),
        (
            Id::Derived(DerivedId::new(DerivedKind::Name, b"wiki/Home".to_vec()).unwrap()),
            "derived/name/v1.77696b692f486f6d65",
        ),
    ];
    for (id, text) in cases {
        assert_eq!(id.to_text(), text, "format {id:?}");
        assert_eq!(format_id(&id), text);
        assert_eq!(Id::parse_text(text).unwrap(), id, "parse {text}");
        assert_eq!(parse_id_text(text, &ParsePolicy::ALL).unwrap(), id);
    }
}

// ---------- failure classification ----------

#[test]
fn unknown_critical_tags_return_unsupported_shape() {
    for tag in [0x00u8, 0x04, 0x7f, 0xff] {
        assert_eq!(
            Id::parse_bytes(&[tag, 0x01]),
            Err(IdShapeError::UnsupportedShape { tag }),
            "tag 0x{tag:02x}"
        );
    }
}

#[test]
fn unknown_version_and_profile_and_kind_are_classified() {
    // Known shape, unknown version.
    assert_eq!(
        Id::parse_bytes(&[0x01, 0x02, 0x01]),
        Err(IdShapeError::UnsupportedVersion {
            shape: IdShape::Content,
            version: 2,
        })
    );
    // Known shape+version, unknown profile tag.
    let mut bytes = vec![0x01, 0x01, 0x09];
    bytes.extend_from_slice(&[0u8; 32]);
    assert_eq!(
        Id::parse_bytes(&bytes),
        Err(IdShapeError::UnsupportedProfile {
            profile: "tag 0x09".to_string(),
        })
    );
    // Unknown profile token in text form.
    assert_eq!(
        Id::parse_text(
            "content/sha-256.e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ),
        Err(IdShapeError::UnsupportedProfile {
            profile: "sha-256".to_string(),
        })
    );
    // Unknown derived kind tag.
    assert_eq!(
        Id::parse_bytes(&[0x03, 0x01, 0x09, 0x00, 0x00, 0x00, 0x00]),
        Err(IdShapeError::UnsupportedDerivedKind { tag: 0x09 })
    );
}

#[test]
fn malformed_inputs_are_classified() {
    // Truncated and trailing encodings.
    assert!(matches!(
        Id::parse_bytes(&[0x01, 0x01, 0x01, 0x00]),
        Err(IdShapeError::Malformed { .. })
    ));
    let mut trailing =
        Id::Content(ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap()).to_bytes();
    trailing.push(0x00);
    assert!(matches!(
        Id::parse_bytes(&trailing),
        Err(IdShapeError::Malformed { .. })
    ));
    // Uppercase digest hex is not the canonical text form.
    assert!(matches!(
        Id::parse_text(
            "content/raw-blake3-256.AF1349B9F5F9A1A6A0404DEA36DCC9499BCB25C9ADC112B7CC9A93CAE41F3262"
        ),
        Err(IdShapeError::Malformed { .. })
    ));
    // Non-canonical percent forms: lowercase escape, redundant
    // escape of an unreserved octet, raw byte that must be escaped.
    for text in ["array/b.%2f", "array/b.%41", "array/b.a b", "array/b.a+b"] {
        assert!(
            matches!(Id::parse_text(text), Err(IdShapeError::Malformed { .. })),
            "{text}"
        );
    }
}

#[test]
fn profile_policy_is_configured_dispatch() {
    let atlas = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, b"hello").unwrap();
    let raw_only = ParsePolicy::only(HashProfile::RawBlake3_256);
    assert_eq!(
        Id::parse_bytes_with(&Id::Content(atlas).to_bytes(), &raw_only),
        Err(IdShapeError::UnsupportedProfile {
            profile: "atlas-chunk-sha256-utf8".to_string(),
        })
    );
    assert_eq!(
        parse_id_bytes(&Id::Content(atlas).to_bytes(), &ParsePolicy::ALL).unwrap(),
        Id::Content(atlas)
    );
    assert!(Id::parse_text_with(&Id::Content(atlas).to_text(), &raw_only).is_err());
    assert!(Id::parse_text_with(&Id::Content(atlas).to_text(), &ParsePolicy::ALL).is_ok());
}

// ---------- properties over generated inputs ----------

fn arb_profile() -> impl Strategy<Value = HashProfile> {
    prop_oneof![
        Just(HashProfile::RawBlake3_256),
        Just(HashProfile::AtlasChunkSha256Utf8),
    ]
}

fn arb_id() -> impl Strategy<Value = Id> {
    let content = (arb_profile(), any::<[u8; 32]>())
        .prop_map(|(profile, digest)| Id::Content(ContentId::new(profile, digest)));
    let array = prop::collection::vec(
        prop::option::of(prop::collection::vec(any::<u8>(), 0..24)),
        0..8,
    )
    .prop_map(|slots| Id::Array(ArrayId::new(slots).unwrap()));
    let derived = (
        prop_oneof![
            Just(DerivedKind::Numeric),
            Just(DerivedKind::Stream),
            Just(DerivedKind::SignedWorld),
        ],
        prop::collection::vec(any::<u8>(), 0..48),
    )
        .prop_map(|(kind, payload)| Id::Derived(DerivedId::new(kind, payload).unwrap()));
    let derived_name = "[a-z0-9/._-]{0,24}".prop_map(|path| {
        Id::Derived(DerivedId::new(DerivedKind::Name, path.into_bytes()).unwrap())
    });
    prop_oneof![content, array, derived, derived_name]
}

proptest! {
    /// Every generated ID round-trips through both encodings, and
    /// dispatch (shape + variant) is unambiguous.
    #[test]
    fn generated_ids_round_trip(id in arb_id()) {
        let bytes = id.to_bytes();
        let parsed = Id::parse_bytes(&bytes).unwrap();
        prop_assert_eq!(&parsed, &id);
        prop_assert_eq!(parsed.shape(), id.shape());
        let text = id.to_text();
        let parsed_text = Id::parse_text(&text).unwrap();
        prop_assert_eq!(&parsed_text, &id);
        // The two encodings agree on identity.
        prop_assert_eq!(parsed_text.to_bytes(), bytes);
    }

    /// Canonicality: any byte string that parses at all re-encodes
    /// to itself — no second spelling of the same ID exists.
    #[test]
    fn accepted_bytes_are_canonical(bytes in prop::collection::vec(any::<u8>(), 0..96)) {
        if let Ok(id) = Id::parse_bytes(&bytes) {
            prop_assert_eq!(id.to_bytes(), bytes);
        }
    }

    /// Unknown critical shape tags always classify as
    /// UnsupportedShape, whatever follows them.
    #[test]
    fn unknown_tags_always_unsupported(
        tag in any::<u8>(),
        rest in prop::collection::vec(any::<u8>(), 0..40),
    ) {
        prop_assume!(tag != 0x01 && tag != 0x02 && tag != 0x03);
        let mut bytes = vec![tag];
        bytes.extend_from_slice(&rest);
        prop_assert_eq!(
            Id::parse_bytes(&bytes),
            Err(IdShapeError::UnsupportedShape { tag })
        );
    }

    /// Content IDs verify exactly the bytes they were computed
    /// from, under their own profile.
    #[test]
    fn content_ids_verify_their_bytes(
        profile in arb_profile(),
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        other in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        prop_assume!(bytes != other);
        if let Ok(id) = ContentId::of_bytes(profile, &bytes) {
            prop_assert!(id.verify(&bytes).is_ok());
            // `other` either digests to a different value (mismatch)
            // or cannot be digested under the profile at all (for
            // example non-UTF-8 bytes under the Atlas text profile),
            // which is its own classification.
            match profile.digest(&other) {
                Ok(_) => {
                    prop_assert_eq!(id.verify(&other), Err(IdShapeError::DigestMismatch));
                }
                Err(e) => {
                    prop_assert_eq!(id.verify(&other), Err(e));
                }
            }
        }
    }
}
