//! Golden-vector and property tests for the array-address core
//! (slice A9118-array-codec, issue #10182; design statements S4.23–S4.28).
//!
//! Oracle: typed reference vectors with byte-for-byte canonical round
//! trip; values with different null/empty structure never collide.
//!
//! The crate-root export of `array_address` belongs to slice 9110-02, so
//! until that lands this test includes the module by path — the same file
//! the export will name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/array_address.rs"]
mod array_address;

use array_address::{ArrayAddress, DecodeError, FORMAT_VERSION};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn addr(slots: Vec<Option<Vec<u8>>>) -> ArrayAddress {
    ArrayAddress::new(slots)
}

fn bytes(text: &str) -> Option<Vec<u8>> {
    Some(text.as_bytes().to_vec())
}

/// Golden vectors: (value, canonical binary encoding as hex).
fn golden() -> Vec<(ArrayAddress, &'static str)> {
    vec![
        // [] — the zero-length address is valid.
        (addr(vec![]), "010000000000000000"),
        // [null]
        (addr(vec![None]), "01000000000000000100"),
        // [empty bytes] — distinct from [null].
        (
            addr(vec![Some(vec![])]),
            "010000000000000001010000000000000000",
        ),
        // [one zero byte]
        (
            addr(vec![Some(vec![0x00])]),
            "01000000000000000101000000000000000100",
        ),
        // [null, "a", empty bytes, null] — order and structure preserved.
        (
            addr(vec![None, bytes("a"), Some(vec![]), None]),
            "010000000000000004000100000000000000016101000000000000000000",
        ),
        // [0xff, 0x00, 0x7f] in one slot — arbitrary octets.
        (
            addr(vec![Some(vec![0xff, 0x00, 0x7f])]),
            "010000000000000001010000000000000003ff007f",
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
        let raw: Vec<u8> = (0..encoded_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded_hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(
            ArrayAddress::decode(&raw),
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
        assert_eq!(ArrayAddress::decode(&encoded), Ok(value.clone()));
        // encode(decode(bytes)) == bytes for canonical bytes
        let decoded = ArrayAddress::decode(&encoded).unwrap();
        assert_eq!(decoded.encode(), encoded);
    }
}

#[test]
fn null_empty_and_absent_structure_never_collides() {
    let empty_address = addr(vec![]);
    let null_slot = addr(vec![None]);
    let empty_bytes = addr(vec![Some(vec![])]);
    let encodings = [
        empty_address.encode(),
        null_slot.encode(),
        empty_bytes.encode(),
    ];
    for i in 0..encodings.len() {
        for j in i + 1..encodings.len() {
            assert_ne!(encodings[i], encodings[j], "encodings {i} and {j} collide");
        }
    }
    assert_ne!(null_slot, empty_bytes);
    // A trailing null slot is a different address from its prefix: an
    // omitted position does not exist.
    assert_ne!(addr(vec![bytes("a")]), addr(vec![bytes("a"), None]));
}

#[test]
fn ordering_is_componentwise_typed_then_shorter_prefix_first() {
    let chain = [
        addr(vec![]),
        addr(vec![None]),
        addr(vec![None, None]),
        addr(vec![Some(vec![])]),
        addr(vec![Some(vec![0x00])]),
        addr(vec![Some(vec![0x00]), None]),
        addr(vec![Some(vec![0x00, 0x00])]),
        addr(vec![Some(vec![0x01])]),
        addr(vec![Some(vec![0x01]), Some(vec![])]),
        addr(vec![bytes("a")]),
    ];
    for pair in chain.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{:?} must sort before {:?}",
            pair[0],
            pair[1]
        );
        assert_eq!(pair[0].cmp(&pair[1]), std::cmp::Ordering::Less);
    }
    // Null sorts before any bytes, including empty bytes.
    assert!(addr(vec![None]) < addr(vec![Some(vec![])]));
    // Bytes compare unsigned-lexicographically, prefix before extension.
    assert!(addr(vec![Some(vec![0x7f])]) < addr(vec![Some(vec![0x80])]));
    assert!(addr(vec![Some(vec![1, 2])]) < addr(vec![Some(vec![1, 2, 0])]));
}

#[test]
fn decode_rejects_noncanonical_or_broken_bytes() {
    // Empty input has no version byte.
    assert_eq!(ArrayAddress::decode(&[]), Err(DecodeError::Truncated));
    // Wrong version.
    let mut bad_version = addr(vec![None]).encode();
    bad_version[0] = FORMAT_VERSION + 1;
    assert_eq!(
        ArrayAddress::decode(&bad_version),
        Err(DecodeError::BadVersion(FORMAT_VERSION + 1))
    );
    // Trailing byte after a complete value.
    let mut trailing = addr(vec![None]).encode();
    trailing.push(0x00);
    assert_eq!(
        ArrayAddress::decode(&trailing),
        Err(DecodeError::TrailingBytes)
    );
    // Unknown slot tag.
    let mut unknown_tag = addr(vec![None]).encode();
    let last = unknown_tag.len() - 1;
    unknown_tag[last] = 0x02;
    assert_eq!(
        ArrayAddress::decode(&unknown_tag),
        Err(DecodeError::UnknownTag(0x02))
    );
    // Declared payload length overruns the input.
    let mut overrun = addr(vec![Some(vec![1, 2, 3])]).encode();
    overrun.truncate(overrun.len() - 1);
    assert_eq!(ArrayAddress::decode(&overrun), Err(DecodeError::Truncated));
    // Slot count larger than the input can hold.
    let mut big_count = vec![FORMAT_VERSION];
    big_count.extend_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        ArrayAddress::decode(&big_count),
        Err(DecodeError::Truncated)
    );
}

#[test]
fn uri_tokens_golden_and_round_trip() {
    let cases: Vec<(ArrayAddress, &str)> = vec![
        (addr(vec![]), ""),
        (addr(vec![None]), "~"),
        (addr(vec![Some(vec![])]), "b."),
        (addr(vec![bytes("a")]), "b.a"),
        (addr(vec![None, Some(vec![]), bytes("a b")]), "~/b./b.a%20b"),
        (addr(vec![Some(vec![0xff, 0x00])]), "b.%FF%00"),
        (addr(vec![Some(vec![b'~', b'.', b'_', b'-'])]), "b.~._-"),
    ];
    for (value, expected) in &cases {
        assert_eq!(&value.to_uri_tokens(), expected, "tokens for {value:?}");
        assert_eq!(ArrayAddress::from_uri_tokens(expected), Ok(value.clone()));
        // Display is the token form.
        assert_eq!(format!("{value}"), *expected);
    }
}

#[test]
fn uri_tokens_reject_malformed_input() {
    // Neither ~ nor b.-prefixed.
    assert_eq!(
        ArrayAddress::from_uri_tokens("x"),
        Err(DecodeError::BadToken)
    );
    assert_eq!(
        ArrayAddress::from_uri_tokens("b"),
        Err(DecodeError::BadToken)
    );
    assert_eq!(
        ArrayAddress::from_uri_tokens("~~"),
        Err(DecodeError::BadToken)
    );
    // Truncated / invalid percent escapes.
    assert_eq!(
        ArrayAddress::from_uri_tokens("b.%2"),
        Err(DecodeError::BadPercentEscape)
    );
    assert_eq!(
        ArrayAddress::from_uri_tokens("b.%zz"),
        Err(DecodeError::BadPercentEscape)
    );
    // A raw reserved octet (space) is not canonical token payload.
    assert_eq!(
        ArrayAddress::from_uri_tokens("b.a b"),
        Err(DecodeError::BadPercentEscape)
    );
    // Lowercase escapes are accepted on input but re-encode canonically.
    let parsed = ArrayAddress::from_uri_tokens("b.%ff").unwrap();
    assert_eq!(parsed.to_uri_tokens(), "b.%FF");
}
