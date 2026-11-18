//! Invariant tests for the strict `id://` V1 parser (slice
//! A9117-uri-parser, issue #10260; design A-9117.1, S4.11–S4.18).
//!
//! Oracle: **invariant** — an RFC 3986 component parse followed by
//! exactly one field-specific decode and exact grammar validation. The
//! golden vectors and the malformed list are the ones in
//! `part-a-addressing/uri_codec_reference.py`, with the reference's
//! error classes. Generated values round-trip through the canonical text
//! form, every canonical text reparses to the same value, escaped `/` is
//! data and never a separator, nothing is decoded twice or
//! dot-normalized, an unknown critical version or shape is rejected, and
//! the authority never selects a resolver on its own.
//!
//! The crate-root export of `id_uri_parse` and `array_address` belongs
//! to slice 9110-02, so until that lands this test includes them by path;
//! `id_shape` is already exported and is re-exported under the name the
//! module uses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod id_shape {
    pub use id_core::id_shape::*;
}

#[path = "../src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../src/id_uri_parse.rs"]
mod id_uri_parse;

use array_address::ArrayAddress;
use id_shape::{ContentId, HashProfile};
use id_uri_parse::{IdUriV1, UriParseError, UriTarget, parse, resolve_context};
use proptest::prelude::*;

const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn array(alias: &str, namespace: &str, slots: Vec<Option<&[u8]>>) -> IdUriV1 {
    let slots = slots
        .into_iter()
        .map(|slot| slot.map(<[u8]>::to_vec))
        .collect();
    IdUriV1::new(alias, namespace, UriTarget::Array(ArrayAddress::new(slots))).unwrap()
}

fn digest(hex: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    out
}

fn class(uri: &str) -> &'static str {
    parse(uri).expect_err(uri).class()
}

#[test]
fn reference_array_vectors_parse_and_format_exactly() {
    let vectors: Vec<(&str, IdUriV1)> = vec![
        ("id://local/main/v1/array", array("local", "main", vec![])),
        (
            "id://local/main/v1/array/~",
            array("local", "main", vec![None]),
        ),
        (
            "id://local/main/v1/array/b.",
            array("local", "main", vec![Some(b"")]),
        ),
        (
            "id://local/main/v1/array/b.%2F",
            array("local", "main", vec![Some(b"/")]),
        ),
        (
            "id://local/wiki/v1/array/b.us%20president.md",
            array("local", "wiki", vec![Some(b"us president.md")]),
        ),
        (
            "id://local/%E9%9B%AA%2F%C3%A9/v1/array",
            array("local", "雪/é", vec![]),
        ),
    ];
    for (uri, expected) in vectors {
        let got = IdUriV1::parse(uri).unwrap();
        assert_eq!(got, expected, "{uri}");
        assert_eq!(got.format(), uri);
        assert_eq!(got.to_string(), uri);
    }
}

#[test]
fn reference_content_golden_parses_and_formats_exactly() {
    let uri = format!("id://local/main/v1/content/raw-blake3-256.{EMPTY_BLAKE3}");
    let expected = IdUriV1::new(
        "local",
        "main",
        UriTarget::Content(ContentId::new(
            HashProfile::RawBlake3_256,
            digest(EMPTY_BLAKE3),
        )),
    )
    .unwrap();
    let got = parse(&uri).unwrap();
    assert_eq!(got, expected);
    assert_eq!(got.format(), uri);
    // The golden digest is the content ID of the empty byte string.
    assert_eq!(
        ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap(),
        ContentId::new(HashProfile::RawBlake3_256, digest(EMPTY_BLAKE3))
    );
}

#[test]
fn reference_malformed_list_is_rejected_with_its_class() {
    let zeros63 = "0".repeat(63);
    let bad: Vec<(String, &str)> = vec![
        (
            "id:/local/main/v1/array".into(),
            "invalid_scheme_or_missing_authority",
        ),
        ("id://Local/main/v1/array".into(), "invalid_resolver_alias"),
        (
            "id://local/main/v2/array".into(),
            "unsupported_critical_version",
        ),
        (
            "id://local/main/v1/array/%2F".into(),
            "invalid_array_slot_token",
        ),
        (
            "id://local/main/v1/array/b.%".into(),
            "malformed_percent_escape",
        ),
        (
            "id://local/main/v1/array/b.%2F/".into(),
            "empty_or_missing_path_component",
        ),
        (
            "id://local/main/v1/array/b.%61".into(),
            "noncanonical_byte_slot",
        ),
        (
            "id://local/main/v1/array?".into(),
            "query_or_fragment_not_in_v1",
        ),
        (
            "id://local/main/v1/array#".into(),
            "query_or_fragment_not_in_v1",
        ),
        ("id://local/%FF/v1/array".into(), "invalid_namespace_utf8"),
        (
            "id://local/main/v1/array?x=1".into(),
            "query_or_fragment_not_in_v1",
        ),
        (
            "id://local/main/v1/array#x".into(),
            "query_or_fragment_not_in_v1",
        ),
        (
            "id://local/../v1/array".into(),
            "invalid_namespace_component",
        ),
        (
            format!("id://local/main/v1/content/unknown.{}", "0".repeat(64)),
            "unsupported_hash_profile",
        ),
        (
            format!("id://local/main/v1/content/raw-blake3-256.{zeros63}"),
            "invalid_digest_text",
        ),
        (
            format!("id://local/main/v1/content/raw-blake3-256.{zeros63}G"),
            "invalid_digest_text",
        ),
        (
            format!("id://local/main/v1/content/raw-blake3-256.{zeros63}A"),
            "invalid_digest_text",
        ),
    ];
    for (uri, expected) in bad {
        assert_eq!(class(&uri), expected, "{uri}");
    }
}

#[test]
fn component_and_grammar_edges_have_stable_classes() {
    let cases = [
        // Visible ASCII only.
        (
            "id://local/main/v1/array/b.a b",
            "raw_control_space_or_nonascii",
        ),
        (
            "id://local/main/v1/array/b.\t",
            "raw_control_space_or_nonascii",
        ),
        ("id://local/雪/v1/array", "raw_control_space_or_nonascii"),
        ("", "invalid_scheme_or_missing_authority"),
        (
            "http://local/main/v1/array",
            "invalid_scheme_or_missing_authority",
        ),
        (
            "id:local/main/v1/array",
            "invalid_scheme_or_missing_authority",
        ),
        ("id:///main/v1/array", "invalid_scheme_or_missing_authority"),
        ("id://?x", "invalid_scheme_or_missing_authority"),
        ("id://[local/main/v1/array", "uri_parse_error"),
        ("id://[::1]/main/v1/array", "uri_parse_error"),
        ("x://lo]cal/main/v1/array", "uri_parse_error"),
        // The authority is a bare lowercase alias: no user, port or host.
        ("id://user@local/main/v1/array", "invalid_resolver_alias"),
        ("id://local:80/main/v1/array", "invalid_resolver_alias"),
        ("id://-local/main/v1/array", "invalid_resolver_alias"),
        ("id://local", "path_must_be_absolute"),
        ("id://local/", "empty_or_missing_path_component"),
        ("id://local/main/v1", "empty_or_missing_path_component"),
        ("id://local//v1/array", "empty_or_missing_path_component"),
        (
            "id://local/main/v1//array",
            "empty_or_missing_path_component",
        ),
        (
            "id://local/main/v1/array//~",
            "empty_or_missing_path_component",
        ),
        // The namespace is decoded once and checked as text.
        ("id://local/%2E/v1/array", "invalid_namespace_component"),
        ("id://local/%2E%2E/v1/array", "invalid_namespace_component"),
        ("id://local/./v1/array", "invalid_namespace_component"),
        (
            "id://local/a:b/v1/array",
            "raw_non_unreserved_component_byte",
        ),
        ("id://local/%C3/v1/array", "invalid_namespace_utf8"),
        ("id://local/%G1/v1/array", "malformed_percent_escape"),
        // Critical version and shape.
        ("id://local/main/V1/array", "unsupported_critical_version"),
        ("id://local/main/v1.0/array", "unsupported_critical_version"),
        ("id://local/main/v1/Array", "unknown_critical_shape"),
        ("id://local/main/v1/derived", "unknown_critical_shape"),
        ("id://local/main/v1/content", "unknown_critical_shape"),
        ("id://local/main/v1/content/a/b", "unknown_critical_shape"),
        // Slot tokens.
        ("id://local/main/v1/array/.", "invalid_array_slot_token"),
        ("id://local/main/v1/array/..", "invalid_array_slot_token"),
        ("id://local/main/v1/array/~~", "invalid_array_slot_token"),
        ("id://local/main/v1/array/B.x", "invalid_array_slot_token"),
        ("id://local/main/v1/array/b.%2f", "noncanonical_byte_slot"),
        ("id://local/main/v1/array/b.%7E", "noncanonical_byte_slot"),
        (
            "id://local/main/v1/array/b.a+b",
            "raw_non_unreserved_component_byte",
        ),
        // Content tokens.
        (
            "id://local/main/v1/content/raw-blake3-256",
            "invalid_content_token",
        ),
        (
            "id://local/main/v1/content/RAW-BLAKE3-256.00",
            "unsupported_hash_profile",
        ),
        (
            "id://local/main/v1/content/atlas-chunk-sha256-utf8.00",
            "unsupported_hash_profile",
        ),
    ];
    for (uri, expected) in cases {
        assert_eq!(class(uri), expected, "{uri}");
    }
}

#[test]
fn escaped_separator_is_data_and_nothing_is_decoded_twice() {
    // `%2F` in a slot is one byte of data, never a segment break.
    let parsed = parse("id://local/main/v1/array/b.a%2Fb/~").unwrap();
    let UriTarget::Array(address) = parsed.target() else {
        panic!()
    };
    assert_eq!(address.slots(), &[Some(b"a/b".to_vec()), None]);
    // `%2525` decodes once to the three bytes `%25`, not to `%`.
    let parsed = parse("id://local/main/v1/array/b.%2525").unwrap();
    let UriTarget::Array(address) = parsed.target() else {
        panic!()
    };
    assert_eq!(address.slots(), &[Some(b"%25".to_vec())]);
    // Same for the namespace.
    assert_eq!(
        parse("id://local/%2525/v1/array").unwrap().namespace(),
        "%25"
    );
    // A literal dot byte stays literal in a slot; there is no dot-segment
    // processing anywhere.
    let parsed = parse("id://local/main/v1/array/b../b...").unwrap();
    let UriTarget::Array(address) = parsed.target() else {
        panic!()
    };
    assert_eq!(
        address.slots(),
        &[Some(b".".to_vec()), Some(b"..".to_vec())]
    );
}

#[test]
fn scheme_is_case_insensitive_and_formats_lowercase() {
    let parsed = parse("ID://local/main/v1/array").unwrap();
    assert_eq!(parsed, array("local", "main", vec![]));
    assert_eq!(parsed.format(), "id://local/main/v1/array");
}

#[test]
fn resolver_hint_never_selects_a_resolver() {
    let parsed = parse("id://local/main/v1/array/~/b.").unwrap();
    assert_eq!(parsed.resolver_hint(), "local");
    assert_eq!(
        resolve_context(parsed.resolver_hint(), "local"),
        Ok("local")
    );
    assert_eq!(
        resolve_context(parsed.resolver_hint(), "other"),
        Err(UriParseError::ResolverContextMismatch)
    );
    assert_eq!(
        resolve_context("Local", "local"),
        Err(UriParseError::InvalidResolverAlias)
    );
    assert_eq!(
        resolve_context("local", "Local"),
        Err(UriParseError::InvalidResolverAlias)
    );
}

#[test]
fn values_that_cannot_be_written_are_refused() {
    let empty = UriTarget::Array(ArrayAddress::empty());
    assert_eq!(
        IdUriV1::new("local", "", empty.clone())
            .unwrap_err()
            .class(),
        "empty_namespace"
    );
    for dots in [".", ".."] {
        assert_eq!(
            IdUriV1::new("local", dots, empty.clone())
                .unwrap_err()
                .class(),
            "dot_segment_namespace"
        );
    }
    assert_eq!(
        IdUriV1::new("Local", "main", empty.clone())
            .unwrap_err()
            .class(),
        "invalid_resolver_alias"
    );
    assert_eq!(
        IdUriV1::new("a".repeat(64), "main", empty)
            .unwrap_err()
            .class(),
        "invalid_resolver_alias"
    );
    let atlas = ContentId::new(HashProfile::AtlasChunkSha256Utf8, [0; 32]);
    assert_eq!(
        IdUriV1::new("local", "main", UriTarget::Content(atlas))
            .unwrap_err()
            .class(),
        "unsupported_hash_profile"
    );
}

#[test]
fn alias_length_bound_is_inclusive_at_63() {
    let alias = format!("a{}", "-".repeat(62));
    let uri = format!("id://{alias}/main/v1/array");
    assert_eq!(parse(&uri).unwrap().resolver_hint(), alias);
    let too_long = format!("id://a{}/main/v1/array", "-".repeat(63));
    assert_eq!(class(&too_long), "invalid_resolver_alias");
}

fn alias_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._-]{0,62}"
}

fn namespace_strategy() -> impl Strategy<Value = String> {
    any::<String>().prop_filter("formattable namespace", |namespace| {
        !namespace.is_empty() && namespace != "." && namespace != ".."
    })
}

fn slots_strategy() -> impl Strategy<Value = Vec<Option<Vec<u8>>>> {
    let slot = prop_oneof![
        Just(None),
        Just(Some(Vec::new())),
        Just(Some(b"/".to_vec())),
        Just(Some(b"%".to_vec())),
        Just(Some(b"~".to_vec())),
        Just(Some((0..=255).collect::<Vec<u8>>())),
        proptest::collection::vec(any::<u8>(), 0..17).prop_map(Some),
    ];
    proptest::collection::vec(slot, 0..10)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn array_values_round_trip_through_canonical_text(
        alias in alias_strategy(),
        namespace in namespace_strategy(),
        slots in slots_strategy(),
    ) {
        let value = IdUriV1::new(
            alias,
            namespace,
            UriTarget::Array(ArrayAddress::new(slots)),
        ).unwrap();
        let text = value.format();
        prop_assert!(text.bytes().all(|byte| (0x21..=0x7E).contains(&byte)));
        let reparsed = parse(&text).unwrap();
        prop_assert_eq!(&reparsed, &value);
        prop_assert_eq!(reparsed.format(), text);
    }

    #[test]
    fn content_values_round_trip_through_canonical_text(
        alias in alias_strategy(),
        namespace in namespace_strategy(),
        digest in any::<[u8; 32]>(),
    ) {
        let value = IdUriV1::new(
            alias,
            namespace,
            UriTarget::Content(ContentId::new(HashProfile::RawBlake3_256, digest)),
        ).unwrap();
        let text = value.format();
        let reparsed = parse(&text).unwrap();
        prop_assert_eq!(&reparsed, &value);
        prop_assert_eq!(reparsed.format(), text);
    }

    /// Any slot text the parser admits is already canonical: reformatting
    /// an accepted URI gives back the same string (modulo the namespace,
    /// which is fixed here), so one value has one slot spelling.
    #[test]
    fn accepted_slot_text_is_canonical(token in "[A-Za-z0-9._~%-]{0,12}") {
        let uri = format!("id://local/main/v1/array/b.{token}");
        if let Ok(parsed) = parse(&uri) {
            prop_assert_eq!(parsed.format(), uri);
        }
    }

    /// The parser never panics, and every rejection carries one of the
    /// reference classes.
    #[test]
    fn arbitrary_text_is_parsed_or_classified(text in "\\PC{0,64}") {
        if let Err(error) = parse(&text) {
            prop_assert!(!error.class().is_empty());
        }
        let prefixed = format!("id://local/{text}");
        let _ = parse(&prefixed);
    }

    /// A version segment other than `v1` is critical and always rejected.
    #[test]
    fn unknown_versions_are_rejected(version in "[a-z0-9._~-]{1,8}") {
        prop_assume!(version != "v1");
        let uri = format!("id://local/main/{version}/array");
        prop_assert_eq!(class(&uri), "unsupported_critical_version");
    }

    /// A shape segment other than `array` or `content` is critical and
    /// always rejected.
    #[test]
    fn unknown_shapes_are_rejected(shape in "[a-z0-9._~-]{1,10}") {
        prop_assume!(shape != "array" && shape != "content");
        let uri = format!("id://local/main/v1/{shape}");
        prop_assert_eq!(class(&uri), "unknown_critical_shape");
    }
}
