//! Golden and idempotence tests for the canonical `id://` V1 formatter
//! (slice A9117-uri-format, issue #10273; design A-9117.1, S4.11–S4.18).
//!
//! Oracle: the reference's required examples format to exact bytes;
//! `parse(format(value)) == value` for generated values; and
//! `format(parse(canonical)) == canonical` byte for byte. The resolver
//! hint must equal the caller's selected local resolver or the check
//! fails explicitly as `resolver_context_mismatch`.
//!
//! The crate-root export of `id_uri_format`, `id_uri_parse` and
//! `array_address` belongs to slice 9110-02, so until that lands this
//! test includes them by path; `id_shape` is already exported and is
//! re-exported under the name the modules use.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod id_shape {
    pub use id_core::id_shape::*;
}

#[path = "../src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../src/id_uri_parse.rs"]
#[allow(dead_code)]
mod id_uri_parse;

#[path = "../src/id_uri_format.rs"]
mod id_uri_format;

use std::fmt::Write as _;

use array_address::ArrayAddress;
use id_shape::{ContentId, HashProfile};
use id_uri_format::{format, format_array, format_content, resolve_context};
use id_uri_parse::{IdUriV1, UriTarget, parse};
use proptest::prelude::*;

const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn address(slots: &[Option<&[u8]>]) -> ArrayAddress {
    ArrayAddress::new(slots.iter().map(|slot| slot.map(<[u8]>::to_vec)).collect())
}

#[test]
fn required_examples_format_exactly() {
    let cases: [(&str, ArrayAddress, &str); 6] = [
        ("main", address(&[]), "id://local/main/v1/array"),
        ("main", address(&[None]), "id://local/main/v1/array/~"),
        ("main", address(&[Some(b"")]), "id://local/main/v1/array/b."),
        (
            "main",
            address(&[Some(b"/")]),
            "id://local/main/v1/array/b.%2F",
        ),
        (
            "wiki",
            address(&[Some(b"us president.md")]),
            "id://local/wiki/v1/array/b.us%20president.md",
        ),
        (
            "雪/é",
            address(&[]),
            "id://local/%E9%9B%AA%2F%C3%A9/v1/array",
        ),
    ];
    for (namespace, value, expected) in cases {
        assert_eq!(format_array("local", namespace, &value).unwrap(), expected);
    }
    let empty = ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap();
    assert_eq!(
        format_content("local", "main", &empty).unwrap(),
        format!("id://local/main/v1/content/raw-blake3-256.{EMPTY_BLAKE3}")
    );
}

#[test]
fn escapes_are_uppercase_and_unreserved_octets_stay_literal() {
    let every_byte: Vec<u8> = (0..=255).collect();
    let text = format_array("local", "main", &address(&[Some(&every_byte)])).unwrap();
    let payload = text.strip_prefix("id://local/main/v1/array/b.").unwrap();
    let mut expected = String::new();
    for byte in every_byte {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            expected.push(char::from(byte));
        } else {
            write!(expected, "%{byte:02X}").unwrap();
        }
    }
    assert_eq!(payload, expected);
    // No escape ever uses lowercase hex.
    assert!(
        payload
            .split('%')
            .skip(1)
            .all(|escape| escape.bytes().take(2).all(|b| !b.is_ascii_lowercase()))
    );
}

#[test]
fn null_and_empty_slots_format_differently_and_in_order() {
    let text = format_array("local", "main", &address(&[Some(b""), None, Some(b"~")])).unwrap();
    assert_eq!(text, "id://local/main/v1/array/b./~/b.~");
}

#[test]
fn values_that_cannot_be_written_are_refused() {
    let empty = ArrayAddress::empty();
    let class = |result: Result<String, id_uri_parse::UriParseError>| result.unwrap_err().class();
    assert_eq!(
        class(format_array("Local", "main", &empty)),
        "invalid_resolver_alias"
    );
    assert_eq!(
        class(format_array("", "main", &empty)),
        "invalid_resolver_alias"
    );
    assert_eq!(class(format_array("local", "", &empty)), "empty_namespace");
    assert_eq!(
        class(format_array("local", ".", &empty)),
        "dot_segment_namespace"
    );
    assert_eq!(
        class(format_array("local", "..", &empty)),
        "dot_segment_namespace"
    );
    let atlas = ContentId::new(HashProfile::AtlasChunkSha256Utf8, [0; 32]);
    assert_eq!(
        class(format_content("local", "main", &atlas)),
        "unsupported_hash_profile"
    );
}

#[test]
fn resolver_hint_mismatch_is_an_explicit_error() {
    let text = format_array("local", "main", &address(&[None, Some(b"")])).unwrap();
    let parsed = parse(&text).unwrap();
    assert_eq!(
        resolve_context(parsed.resolver_hint(), "local"),
        Ok("local")
    );
    let error = resolve_context(parsed.resolver_hint(), "other").unwrap_err();
    assert_eq!(error.class(), "resolver_context_mismatch");
    assert_eq!(error.to_string(), "resolver_context_mismatch");
    assert_eq!(
        resolve_context("local", "-bad").unwrap_err().class(),
        "invalid_resolver_alias"
    );
}

#[test]
fn display_and_method_agree_with_format() {
    let value = IdUriV1::new(
        "local",
        "main",
        UriTarget::Array(address(&[None, Some(b"x y")])),
    )
    .unwrap();
    assert_eq!(value.format(), format(&value));
    assert_eq!(value.to_string(), format(&value));
}

fn slot_strategy() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        Just(None),
        proptest::collection::vec(any::<u8>(), 0..17).prop_map(Some),
    ]
}

fn value_strategy() -> impl Strategy<Value = IdUriV1> {
    let alias = "[a-z0-9][a-z0-9._-]{0,62}";
    let namespace =
        any::<String>().prop_filter("writable", |ns| !ns.is_empty() && ns != "." && ns != "..");
    let target = prop_oneof![
        proptest::collection::vec(slot_strategy(), 0..8)
            .prop_map(|slots| UriTarget::Array(ArrayAddress::new(slots))),
        any::<[u8; 32]>().prop_map(|digest| {
            UriTarget::Content(ContentId::new(HashProfile::RawBlake3_256, digest))
        }),
    ];
    (alias, namespace, target)
        .prop_map(|(alias, namespace, target)| IdUriV1::new(alias, namespace, target).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn parse_of_format_is_the_value(value in value_strategy()) {
        let text = format(&value);
        prop_assert_eq!(parse(&text).unwrap(), value);
    }

    #[test]
    fn format_of_parse_is_byte_identical_for_canonical_text(value in value_strategy()) {
        let canonical = format(&value);
        let reparsed = parse(&canonical).unwrap();
        prop_assert_eq!(&format(&reparsed), &canonical);
        // Formatting is idempotent through any number of round trips.
        prop_assert_eq!(format(&parse(&format(&reparsed)).unwrap()), canonical);
    }

    #[test]
    fn formatted_text_is_visible_ascii_with_no_delimiters(value in value_strategy()) {
        let text = format(&value);
        prop_assert!(text.bytes().all(|byte| (0x21..=0x7E).contains(&byte)));
        prop_assert!(!text.contains(['?', '#']));
        // The only empty segment is the authority marker.
        prop_assert_eq!(text.matches("//").count(), 1);
    }
}
