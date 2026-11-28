//! Property tests for the tuple encoding in `src/tuple.rs`.
//!
//! The proptest properties use the default case count, which `PROPTEST_CASES`
//! raises. The fixed-seed run draws the same cases on every machine, and
//! `ID_TUPLE_FUZZ_CASES` raises its count for long runs. Failures are not
//! written to `proptest-regressions/`: the minimal input goes into
//! `tests/regressions/tuple.txt` instead (see `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    Value, check_order_and_prefix, check_pair, check_roundtrip, encode_tuple, tuple_order,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};

/// Seed for the fixed-seed run (`ChaCha` needs 32 bytes). Changing it changes
/// every generated case.
const FUZZ_SEED: &[u8; 32] = b"id-tuple-fuzz-seed-v1-----------";

/// Cases in the fixed-seed run unless `ID_TUPLE_FUZZ_CASES` says otherwise.
const DEFAULT_FUZZ_CASES: u32 = 2_000;

/// Strings weighted toward NUL and the bytes that follow it in the escape.
fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            4 => Just('\0'),
            2 => Just('\u{1}'),
            2 => Just('a'),
            1 => Just('\u{ff}'),
            1 => any::<char>(),
        ],
        0..12,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// Byte strings weighted toward `0x00` and `0xFF`, the escape pair.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            4 => Just(0u8),
            2 => Just(0xFFu8),
            1 => Just(1u8),
            2 => any::<u8>(),
        ],
        0..12,
    )
}

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![text().prop_map(Value::Text), bytes().prop_map(Value::Bytes),]
}

/// Pairs where the second value extends the first. A field that ends early shows
/// on these, since one value's encoding is then a prefix of the other's.
fn sharing_prefix() -> impl Strategy<Value = (Value, Value)> {
    prop_oneof![
        (text(), text()).prop_map(|(base, tail)| {
            let joined = format!("{base}{tail}");
            (Value::Text(base), Value::Text(joined))
        }),
        (bytes(), bytes()).prop_map(|(base, tail)| {
            let mut longer = base.clone();
            longer.extend(tail);
            (Value::Bytes(base), Value::Bytes(longer))
        }),
    ]
}

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn test_strings_keep_order_prefix_and_roundtrip(a in text(), b in text()) {
        check_pair(&Value::Text(a), &Value::Text(b)).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn test_byte_strings_keep_order_prefix_and_roundtrip(a in bytes(), b in bytes()) {
        check_pair(&Value::Bytes(a), &Value::Bytes(b)).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn test_mixed_elements_keep_order_prefix_and_roundtrip(a in value(), b in value()) {
        check_pair(&a, &b).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn test_values_sharing_a_prefix_keep_order_prefix_and_roundtrip(pair in sharing_prefix()) {
        check_pair(&pair.0, &pair.1).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn test_tuples_roundtrip_and_sort_by_their_elements(
        xs in prop::collection::vec(value(), 0..4),
        ys in prop::collection::vec(value(), 0..4),
    ) {
        check_roundtrip(&xs).map_err(TestCaseError::fail)?;
        prop_assert_eq!(encode_tuple(&xs).cmp(&encode_tuple(&ys)), tuple_order(&xs, &ys));
    }
}

/// The type tag and the raw bytes of an element, before escaping.
fn raw(value: &Value) -> (u8, Vec<u8>) {
    match value {
        Value::Bytes(bytes) => (1, bytes.clone()),
        Value::Text(text) => (2, text.as_bytes().to_vec()),
    }
}

/// The original bug: a field ends at its first 0x00, so `a` is a prefix of `a\0b`.
fn single_nul(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    [vec![tag], data, vec![0]].concat()
}

/// No escaping at all: a 0x00 inside the data reads as the terminator.
fn no_escape(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    [vec![tag], data, vec![0, 0]].concat()
}

/// Escapes a leading 0x00 and no other, so a later 0x00 ends the field early.
fn escape_first_only(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    let mut out = vec![tag];
    for (index, byte) in data.iter().enumerate() {
        if *byte == 0 && index == 0 {
            out.extend([0, 0xFF]);
        } else {
            out.push(*byte);
        }
    }
    out.extend([0, 0]);
    out
}

/// Type tags swapped, so byte strings sort after strings.
fn swapped_tags(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    let mut out = vec![if tag == 1 { 2 } else { 1 }];
    for byte in data {
        if byte == 0 {
            out.extend([0, 0xFF]);
        } else {
            out.push(byte);
        }
    }
    out.extend([0, 0]);
    out
}

/// A length prefix in place of a terminator, so a longer value can sort first.
fn length_prefix(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    let length = u32::try_from(data.len()).expect("a test value is short");
    [vec![tag], length.to_be_bytes().to_vec(), data].concat()
}

/// A valid alternative: a 0x00 is escaped as 0x00 0x01. It must pass.
fn escape_01(value: &Value) -> Vec<u8> {
    let (tag, data) = raw(value);
    let mut out = vec![tag];
    for byte in data {
        if byte == 0 {
            out.extend([0, 1]);
        } else {
            out.push(byte);
        }
    }
    out.extend([0, 0]);
    out
}

fn real_encoding(value: &Value) -> Vec<u8> {
    encode_tuple(std::slice::from_ref(value))
}

/// The shape of a tuple encoder under test.
type Encoder = fn(&Value) -> Vec<u8>;

/// Known-bad encoders, one per class, and the controls that must pass. The
/// flag says whether the encoder is bad.
fn known_encoders() -> [(&'static str, Encoder, bool); 7] {
    [
        ("single NUL terminator", single_nul, true),
        ("no escaping", no_escape, true),
        ("first NUL only escaped", escape_first_only, true),
        ("swapped type tags", swapped_tags, true),
        ("length prefix", length_prefix, true),
        ("control: NUL as 00 01", escape_01, false),
        ("control: the real encoder", real_encoding, false),
    ]
}

/// Whether `encode` fails the pair checks for a pair that `strategy` draws
/// within `cases`, over the fixed seed.
fn caught_within(
    encode: Encoder,
    strategy: impl Strategy<Value = (Value, Value)>,
    cases: u32,
) -> bool {
    let config = Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, FUZZ_SEED);
    let mut runner = TestRunner::new_with_rng(config, rng);
    runner
        .run(&strategy, |(a, b)| {
            check_order_and_prefix(encode, &a, &b).map_err(TestCaseError::fail)
        })
        .is_err()
}

/// Each known-bad encoder fails within the default proptest case count, and each
/// control passes. A property that needs a larger run to notice a class of bug
/// would miss it in an ordinary run, so this checks the default count.
#[test]
fn test_each_known_bad_encoder_is_caught_within_the_default_case_count() {
    let mut wrong = Vec::new();
    for (name, encode, bad) in known_encoders() {
        let caught = caught_within(encode, value_pairs(), 256)
            || caught_within(encode, sharing_prefix(), 256);
        if caught != bad {
            wrong.push(format!("{name}: caught {caught}, expected {bad}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Pairs of any values, as the properties draw them.
fn value_pairs() -> impl Strategy<Value = (Value, Value)> {
    (value(), value())
}

/// The same pair checks over a fixed seed, with a case count that
/// `ID_TUPLE_FUZZ_CASES` can raise for a long run.
#[test]
fn test_fixed_seed_fuzz_finds_no_violation() {
    let cases = match std::env::var("ID_TUPLE_FUZZ_CASES") {
        Ok(text) => text
            .parse::<u32>()
            .expect("ID_TUPLE_FUZZ_CASES must be a case count"),
        Err(_) => DEFAULT_FUZZ_CASES,
    };
    let config = Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, FUZZ_SEED);
    let mut runner = TestRunner::new_with_rng(config, rng);
    let pairs = prop_oneof![value_pairs(), sharing_prefix()];
    runner
        .run(&pairs, |(a, b)| {
            check_pair(&a, &b).map_err(TestCaseError::fail)
        })
        .expect("fuzz found a violation: append the minimal pair to tests/regressions/tuple.txt");
}
