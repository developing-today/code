//! Golden-vector tests for the canonical CBOR profile
//! (slice A9114-cbor-profile, #10181) — evidence first.
//!
//! The vector table below is the cross-language oracle: the
//! hosted TypeScript suite (`hosted-worker/test/
//! canonical-cbor.test.ts`) asserts the *same* hex for the *same*
//! logical values, so byte equality across Rust and TypeScript
//! is checked by both suites against one fixed table, and every
//! ambiguous encoding in the rejection table must fail with the
//! named typed error in both implementations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_core::canonical_cbor::{
    CborError, RecordSpec, Value, decode_canonical, encode_canonical, validate_record,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn fixture_value(value: &serde_json::Value) -> Value {
    let kind = value["kind"].as_str().expect("fixture value kind");
    match kind {
        "null" => Value::Null,
        "bool" => Value::Bool(value["value"].as_bool().expect("fixture boolean")),
        "uint" => Value::UInt(
            value["value"]
                .as_str()
                .expect("fixture unsigned integer")
                .parse()
                .expect("fixture unsigned integer range"),
        ),
        "int" => Value::Int(
            value["value"]
                .as_str()
                .expect("fixture negative integer")
                .parse()
                .expect("fixture signed integer range"),
        ),
        "bytes" => Value::Bytes(unhex(value["value"].as_str().expect("fixture byte string"))),
        "text" => Value::Text(value["value"].as_str().expect("fixture text").to_owned()),
        "array" => Value::Array(
            value["value"]
                .as_array()
                .expect("fixture array")
                .iter()
                .map(fixture_value)
                .collect(),
        ),
        "map" => Value::Map(
            value["value"]
                .as_array()
                .expect("fixture map")
                .iter()
                .map(|pair| {
                    let pair = pair.as_array().expect("fixture map pair");
                    assert_eq!(pair.len(), 2, "fixture map pair has key and value");
                    (fixture_value(&pair[0]), fixture_value(&pair[1]))
                })
                .collect(),
        ),
        other => panic!("unknown fixture CBOR kind {other}"),
    }
}

fn golden_vectors() -> Vec<(String, Value, String)> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/hosted-second-version-g1.json"
    ))
    .expect("shared G1 fixture is valid JSON");
    fixture["canonical_cbor"]
        .as_array()
        .expect("shared canonical CBOR vectors")
        .iter()
        .map(|case| {
            (
                case["name"].as_str().expect("fixture vector name").to_owned(),
                fixture_value(&case["value"]),
                case["encoded_hex"]
                    .as_str()
                    .expect("fixture encoded bytes")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn golden_vectors_encode_and_decode_byte_exactly() {
    let vectors = golden_vectors();
    assert_eq!(vectors.len(), 27, "shared canonical CBOR fixture retains every vector");
    for (name, value, want_hex) in vectors {
        let encoded = encode_canonical(&value).unwrap();
        assert_eq!(hex(&encoded), want_hex, "encode {name}");
        let decoded = decode_canonical(&unhex(&want_hex)).unwrap();
        // Decoding yields the canonical in-memory form; for maps
        // that is key-sorted order regardless of construction
        // order, so compare re-encodings as well as values where
        // construction order already was canonical.
        assert_eq!(
            hex(&encode_canonical(&decoded).unwrap()),
            want_hex,
            "round-trip {name}"
        );
    }
}

#[test]
fn ambiguous_encodings_reject_with_typed_errors() {
    let cases: Vec<(&str, CborError)> = vec![
        // 23 encoded in the long form: not shortest.
        ("1817", CborError::NonShortestForm),
        // Indefinite-length array and map.
        ("9fff", CborError::IndefiniteLength),
        ("bfff", CborError::IndefiniteLength),
        // Duplicate key "a".
        ("a2616101616102", CborError::DuplicateKey),
        // Keys out of canonical order ("b" before "a").
        ("a2616202616101", CborError::KeysOutOfOrder),
        // Float 1.0: outside the closed value model.
        ("f93c00", CborError::UnsupportedType { byte: 0xf9 }),
        // Tag 0: outside the closed value model.
        ("c000", CborError::UnsupportedType { byte: 0xc0 }),
        // Undefined simple value.
        ("f7", CborError::UnsupportedType { byte: 0xf7 }),
        // Trailing bytes after a complete item.
        ("0001", CborError::TrailingBytes),
        // Truncated text string.
        ("6261", CborError::UnexpectedEof),
        // Invalid UTF-8 in a text string.
        ("61ff", CborError::InvalidUtf8),
    ];
    for (input, want) in cases {
        assert_eq!(
            decode_canonical(&unhex(input)).unwrap_err(),
            want,
            "reject {input}"
        );
    }
}

#[test]
fn hostile_declared_lengths_reject_without_allocating() {
    // Array header declaring u64::MAX items in a 9-byte input:
    // before the bound this attempted a multi-GB allocation
    // (capacity overflow / abort) instead of a typed error.
    let mut giant_array = vec![0x9b];
    giant_array.extend_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        decode_canonical(&giant_array),
        Err(CborError::UnexpectedEof)
    );

    // Same for a map header, and for a merely large-but-plausible
    // count that still exceeds the bytes left.
    let mut giant_map = vec![0xbb];
    giant_map.extend_from_slice(&(1u64 << 40).to_be_bytes());
    assert_eq!(decode_canonical(&giant_map), Err(CborError::UnexpectedEof));
    assert_eq!(
        decode_canonical(&unhex("9b0000000100000000")),
        Err(CborError::UnexpectedEof)
    );
    // A byte string length beyond `usize` (32-bit) or beyond the
    // remaining input rejects without slicing.
    let mut oversized_bytes = vec![0x5b];
    oversized_bytes.extend_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        decode_canonical(&oversized_bytes),
        Err(CborError::UnexpectedEof)
    );
}

#[test]
fn nesting_deeper_than_the_limit_rejects() {
    use id_core::canonical_cbor::MAX_NESTING_DEPTH;

    // One byte per level: without a depth bound a few KB of 0x81
    // would overflow the thread stack.
    let hostile = vec![0x81u8; MAX_NESTING_DEPTH as usize + 100];
    assert_eq!(decode_canonical(&hostile), Err(CborError::TooDeeplyNested));

    // Nesting exactly at the limit is still legal and round-trips:
    // MAX_NESTING_DEPTH arrays (one byte each) around a leaf.
    let mut allowed = vec![0x81u8; MAX_NESTING_DEPTH as usize];
    allowed.push(0x00);
    let decoded = decode_canonical(&allowed).unwrap();
    assert_eq!(hex(&encode_canonical(&decoded).unwrap()), hex(&allowed));

    // One level deeper than the limit rejects.
    let mut deeper = vec![0x81u8; MAX_NESTING_DEPTH as usize + 1];
    deeper.push(0x00);
    assert_eq!(decode_canonical(&deeper), Err(CborError::TooDeeplyNested));
}

/// `levels` nested arrays around a `0` leaf, built as a value.
fn nested_arrays(levels: usize) -> Value {
    let mut value = Value::UInt(0);
    for _ in 0..levels {
        value = Value::Array(vec![value]);
    }
    value
}

#[test]
fn encoder_applies_the_decoders_depth_limit() {
    use id_core::canonical_cbor::MAX_NESTING_DEPTH;

    // #11315: at the limit the encoder succeeds and the bytes decode back.
    let at_limit = nested_arrays(MAX_NESTING_DEPTH as usize);
    let bytes = encode_canonical(&at_limit).unwrap();
    assert_eq!(decode_canonical(&bytes).unwrap(), at_limit);

    // One level deeper is refused by the encoder with the decoder's error,
    // instead of producing bytes the decoder rejects.
    let deeper = nested_arrays(MAX_NESTING_DEPTH as usize + 1);
    assert_eq!(encode_canonical(&deeper), Err(CborError::TooDeeplyNested));

    // The same rule through a map value.
    let in_map = Value::Map(vec![(text("k"), nested_arrays(MAX_NESTING_DEPTH as usize))]);
    assert_eq!(encode_canonical(&in_map), Err(CborError::TooDeeplyNested));

    // Far deeper caller-built values fail fast instead of recursing without bound.
    assert_eq!(
        encode_canonical(&nested_arrays(10_000)),
        Err(CborError::TooDeeplyNested)
    );
}

#[test]
fn unknown_critical_fields_reject_and_others_tolerate() {
    let record = Value::Map(vec![
        (text("subject"), text("content:abc")),
        (text("future_proof"), Value::UInt(1)),
        (text("future_hint"), text("x")),
    ]);
    // A reader that knows "subject" and treats "future_proof" as
    // critical schema-wide: the record rejects.
    let strict = RecordSpec {
        known_fields: &["subject", "future_hint"],
        critical_fields: &["future_proof"],
    };
    assert_eq!(
        validate_record(&record, &strict).unwrap_err(),
        CborError::UnknownCriticalField {
            field: "future_proof".to_string(),
        }
    );
    // A reader that knows the critical field accepts the record;
    // the merely-unknown non-critical "future_hint" never blocks.
    let informed = RecordSpec {
        known_fields: &["subject", "future_proof"],
        critical_fields: &["future_proof"],
    };
    validate_record(&record, &informed).unwrap();
}
