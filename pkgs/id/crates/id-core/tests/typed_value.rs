//! Golden-vector and invariant tests for tagged typed values
//! (slice A9118-typed-values, issue #10203; design statement A-9118.2,
//! S4.34–S4.40).
//!
//! Oracle: **invariant** — the type tag participates in equality
//! (identical payload under distinct types is unequal); declared codec
//! reference vectors round-trip byte-for-byte; an unknown non-critical
//! type is preserved opaquely; an unknown critical type rejects; there
//! is no implicit stringification.
//!
//! The crate-root export of `typed_value` belongs to slice 9110-02, so
//! until that lands this test includes the module by path — the same file
//! the export will name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/typed_value.rs"]
mod typed_value;

use typed_value::{CodecProfile, TypeId, TypeName, TypedValue, ValueError};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// Declared codec reference vectors: (value, canonical encoding as hex).
/// The hex was produced from the wire-format spec in the module docs by
/// an independent script, not by the implementation under test.
fn golden() -> Vec<(TypedValue, &'static str)> {
    vec![
        (
            TypedValue::null(),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001069642d76616c75652f6e756c6c2f76310000000000000000",
        ),
        (
            TypedValue::bytes(vec![]),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001169642d76616c75652f62797465732f76310000000000000000",
        ),
        (
            TypedValue::text(""),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001069642d76616c75652f746578742f76310000000000000000",
        ),
        (
            TypedValue::integer(0),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001369642d76616c75652f696e74656765722f763100000000000000080000000000000000",
        ),
        (
            TypedValue::integer(1),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001369642d76616c75652f696e74656765722f763100000000000000080000000000000001",
        ),
        (
            TypedValue::text("1"),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001069642d76616c75652f746578742f7631000000000000000131",
        ),
        (
            TypedValue::tuple(vec![]),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001169642d76616c75652f7475706c652f763100000000000000080000000000000000",
        ),
        (
            TypedValue::tuple(vec![TypedValue::integer(1), TypedValue::text("a")]),
            "02000000000000001169642d76616c75652d636f6465632f763100000000000000001169642d76616c75652f7475706c652f76310000000000000056000000000000000200000000000000001369642d76616c75652f696e74656765722f76310000000000000008000000000000000100000000000000001069642d76616c75652f746578742f7631000000000000000161",
        ),
        (
            TypedValue::opaque(
                TypeId::registered("x-example/unknown/v9"),
                vec![0xde, 0xad, 0xbe, 0xef],
            ),
            "02000000000000001169642d76616c75652d636f6465632f7631000000000000000014782d6578616d706c652f756e6b6e6f776e2f76390000000000000004deadbeef",
        ),
        (
            TypedValue::opaque(TypeId::code(42), vec![0x00, 0xff]),
            "02000000000000001169642d76616c75652d636f6465632f763101000000000000002a000000000000000200ff",
        ),
    ]
}

#[test]
fn golden_vectors_encode_byte_for_byte() {
    for (value, expected_hex) in golden() {
        assert_eq!(hex(&value.encode()), expected_hex, "encode {value:?}");
    }
}

#[test]
fn golden_vectors_decode_to_the_typed_value() {
    for (value, encoded_hex) in golden() {
        assert_eq!(
            TypedValue::decode(&unhex(encoded_hex)),
            Ok(value),
            "decode {encoded_hex}"
        );
    }
}

#[test]
fn round_trip_both_directions_for_all_golden_values() {
    for (value, _) in golden() {
        let encoded = value.encode();
        // decode(encode(x)) == x
        assert_eq!(TypedValue::decode(&encoded), Ok(value.clone()));
        // encode(decode(bytes)) == bytes for canonical bytes
        let decoded = TypedValue::decode(&encoded).unwrap();
        assert_eq!(decoded.encode(), encoded);
    }
}

#[test]
fn type_tag_participates_in_equality() {
    // Integer 1 and text "1": the design's canonical unequal pair.
    let int_one = TypedValue::integer(1);
    let text_one = TypedValue::text("1");
    assert_ne!(int_one, text_one);
    assert_ne!(hex(&int_one.encode()), hex(&text_one.encode()));
    // Same payload bytes under two distinct registered types are unequal.
    let left = TypedValue::opaque(TypeId::registered("x-example/a/v1"), vec![0x31]);
    let right = TypedValue::opaque(TypeId::registered("x-example/b/v1"), vec![0x31]);
    assert_eq!(left.payload(), right.payload());
    assert_ne!(left, right);
    assert_ne!(left.type_id(), right.type_id());
    // Same name under two profiles is a different type.
    let p1 = TypedValue::opaque(
        TypeId {
            profile: "profile-one/v1".to_string(),
            name: TypeName::Registered("t/v1".to_string()),
        },
        vec![],
    );
    let p2 = TypedValue::opaque(
        TypeId {
            profile: "profile-two/v1".to_string(),
            name: TypeName::Registered("t/v1".to_string()),
        },
        vec![],
    );
    assert_ne!(p1, p2);
    // Registered name and numeric code never collide.
    let named = TypedValue::opaque(TypeId::registered("42"), vec![]);
    let coded = TypedValue::opaque(TypeId::code(42), vec![]);
    assert_ne!(named, coded);
}

#[test]
fn null_empty_bytes_empty_tuple_zero_int_and_empty_text_are_distinct_vectors() {
    let values = vec![
        TypedValue::null(),
        TypedValue::bytes(vec![]),
        TypedValue::tuple(vec![]),
        TypedValue::integer(0),
        TypedValue::text(""),
    ];
    let encodings: Vec<Vec<u8>> = values.iter().map(TypedValue::encode).collect();
    for i in 0..encodings.len() {
        for j in (i + 1)..encodings.len() {
            assert_ne!(encodings[i], encodings[j], "vectors {i} and {j} collide");
            assert_ne!(values[i], values[j], "values {i} and {j} compare equal");
        }
    }
    // Hash follows equality: distinct values hash distinctly here, and
    // equal values hash equally.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hashes = Vec::new();
    for value in &values {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hashes.push(hasher.finish());
    }
    for i in 0..hashes.len() {
        for j in (i + 1)..hashes.len() {
            assert_ne!(hashes[i], hashes[j]);
        }
    }
    let mut hasher = DefaultHasher::new();
    TypedValue::integer(1).hash(&mut hasher);
    let first = hasher.finish();
    let mut hasher = DefaultHasher::new();
    TypedValue::integer(1).hash(&mut hasher);
    assert_eq!(first, hasher.finish());
}

#[test]
fn typed_accessors_reject_the_wrong_type() {
    assert_eq!(TypedValue::integer(1).as_integer(), Ok(1));
    assert_eq!(TypedValue::text("hi").as_text(), Ok("hi"));
    assert_eq!(TypedValue::bytes(vec![7]).as_bytes(), Ok([7].as_slice()));
    assert!(TypedValue::null().is_null());
    assert!(!TypedValue::integer(0).is_null());
    assert_eq!(
        TypedValue::integer(1).as_text(),
        Err(ValueError::WrongType {
            expected: "text",
            found: "id-value/integer/v1".to_string(),
        })
    );
    assert_eq!(
        TypedValue::text("1").as_integer(),
        Err(ValueError::WrongType {
            expected: "integer",
            found: "id-value/text/v1".to_string(),
        })
    );
    assert_eq!(
        TypedValue::null().as_bytes(),
        Err(ValueError::WrongType {
            expected: "bytes",
            found: "id-value/null/v1".to_string(),
        })
    );
    assert_eq!(TypedValue::tuple(vec![]).as_tuple(), Ok(Vec::new()));
    assert_eq!(
        TypedValue::integer(0).as_tuple(),
        Err(ValueError::WrongType {
            expected: "tuple",
            found: "id-value/integer/v1".to_string(),
        })
    );
}

#[test]
fn no_implicit_stringification() {
    // There is deliberately no `From<TypedValue> for String`, no
    // `Display` that renders a bare payload, and no accessor that
    // coerces across types: the integer payload is not readable as
    // text even though text "1" exists as a different value.
    let int_one = TypedValue::integer(1);
    assert!(int_one.as_text().is_err());
    // The Debug form always carries the type tag, never a bare payload.
    let rendered = format!("{int_one:?}");
    assert!(rendered.contains("id-value/integer/v1"), "{rendered}");
}

#[test]
fn unknown_non_critical_type_is_preserved_opaque() {
    let encoded = unhex(
        "02000000000000001169642d76616c75652d636f6465632f7631000000000000000014782d6578616d706c652f756e6b6e6f776e2f76390000000000000004deadbeef",
    );
    let permissive = CodecProfile::permissive();
    let value = TypedValue::decode_strict(&permissive, &encoded).unwrap();
    assert_eq!(value.type_id(), &TypeId::registered("x-example/unknown/v9"));
    assert_eq!(value.payload(), [0xde, 0xad, 0xbe, 0xef]);
    // Opaque bytes survive the round trip exactly.
    assert_eq!(value.encode(), encoded);
    // Typed accessors refuse to interpret the opaque payload.
    assert!(value.as_text().is_err());
    assert!(value.as_integer().is_err());
}

#[test]
fn unknown_critical_type_rejects() {
    let encoded = unhex(
        "02000000000000001169642d76616c75652d636f6465632f7631000000000000000014782d6578616d706c652f756e6b6e6f776e2f76390000000000000004deadbeef",
    );
    let profile = CodecProfile::new(&["x-example/unknown/v9"]);
    assert_eq!(
        TypedValue::decode_strict(&profile, &encoded),
        Err(ValueError::UnknownCriticalType {
            type_id: "x-example/unknown/v9".to_string(),
        })
    );
    // Built-in types are always known, so they decode even under a
    // profile that marks only the foreign type critical.
    assert_eq!(
        TypedValue::decode_strict(&profile, &TypedValue::integer(1).encode()),
        Ok(TypedValue::integer(1))
    );
    // Criticality applies inside tuples too.
    let nested = TypedValue::tuple(vec![TypedValue::opaque(
        TypeId::registered("x-example/unknown/v9"),
        vec![0x00],
    )]);
    assert_eq!(
        TypedValue::decode_strict(&profile, &nested.encode()),
        Err(ValueError::UnknownCriticalType {
            type_id: "x-example/unknown/v9".to_string(),
        })
    );
    // …while the permissive profile preserves the whole tuple opaquely.
    let permissive = CodecProfile::permissive();
    let decoded = TypedValue::decode_strict(&permissive, &nested.encode()).unwrap();
    assert_eq!(decoded, nested);
}

#[test]
fn strict_decode_enforces_the_profile_boundary() {
    let encoded = TypedValue::integer(1).encode();
    let other = CodecProfile {
        id: "some-other-profile/v7",
        critical: &[],
    };
    assert_eq!(
        TypedValue::decode_strict(&other, &encoded),
        Err(ValueError::ProfileMismatch {
            expected: "some-other-profile/v7".to_string(),
            found: "id-value-codec/v1".to_string(),
        })
    );
}

#[test]
fn decode_validates_known_type_payload_shapes() {
    // Text carrying invalid UTF-8 rejects: no silent lossy coercion.
    let mut bad_text = TypedValue::text("ok").encode();
    let last = bad_text.len() - 1;
    bad_text[last] = 0xff;
    assert_eq!(TypedValue::decode(&bad_text), Err(ValueError::InvalidUtf8));
    // Integer payload must be exactly 8 bytes.
    let mut short_int = TypedValue::integer(1).encode();
    // Payload length lives in the 8 bytes before the payload.
    let len_pos = short_int.len() - 16;
    short_int[len_pos..len_pos + 8].copy_from_slice(&7u64.to_be_bytes());
    short_int.pop();
    assert_eq!(
        TypedValue::decode(&short_int),
        Err(ValueError::MalformedPayload {
            type_id: "id-value/integer/v1".to_string(),
        })
    );
    // Null must carry an empty payload.
    let mut fat_null = TypedValue::null().encode();
    let len_pos = fat_null.len() - 8;
    fat_null[len_pos..len_pos + 8].copy_from_slice(&1u64.to_be_bytes());
    fat_null.push(0x00);
    assert_eq!(
        TypedValue::decode(&fat_null),
        Err(ValueError::MalformedPayload {
            type_id: "id-value/null/v1".to_string(),
        })
    );
    // Tuple payload must parse as count-prefixed typed elements.
    let mut bad_tuple = TypedValue::tuple(vec![]).encode();
    bad_tuple.pop();
    assert_eq!(TypedValue::decode(&bad_tuple), Err(ValueError::Truncated));
}

#[test]
fn decode_rejects_broken_wire_bytes() {
    // Empty input has no version byte.
    assert_eq!(TypedValue::decode(&[]), Err(ValueError::Truncated));
    // Wrong version.
    let mut bad_version = TypedValue::null().encode();
    bad_version[0] = 0xff;
    assert_eq!(
        TypedValue::decode(&bad_version),
        Err(ValueError::BadVersion(0xff))
    );
    // Trailing byte after a complete value.
    let mut trailing = TypedValue::null().encode();
    trailing.push(0x00);
    assert_eq!(
        TypedValue::decode(&trailing),
        Err(ValueError::TrailingBytes)
    );
    // Unknown type-tag byte.
    let mut unknown_tag = TypedValue::null().encode();
    // Version (1) + profile len (8) + profile (17) = tag position 26.
    unknown_tag[26] = 0x02;
    assert_eq!(
        TypedValue::decode(&unknown_tag),
        Err(ValueError::UnknownTag(0x02))
    );
    // Empty registered name is not a type.
    let empty_name = TypedValue::opaque(TypeId::registered(""), vec![]).encode();
    assert_eq!(
        TypedValue::decode(&empty_name),
        Err(ValueError::EmptyTypeName)
    );
    // Truncated payload.
    let mut truncated = TypedValue::integer(1).encode();
    truncated.truncate(truncated.len() - 3);
    assert_eq!(TypedValue::decode(&truncated), Err(ValueError::Truncated));
}
