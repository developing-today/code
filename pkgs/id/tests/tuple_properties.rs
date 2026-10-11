//! Property tests for the tuple encoding in `src/tuple.rs`.
//!
//! The proptest properties use the default case count, which `PROPTEST_CASES`
//! raises. The fixed-seed run draws the same cases on every machine, and
//! `ID_TUPLE_FUZZ_CASES` raises its count for long runs. Failures are not
//! written to `proptest-regressions/`: the minimal input goes into
//! `tests/regressions/tuple.txt` instead (see `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Value, check_pair, check_roundtrip, encode_tuple, tuple_order};
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
    fn test_tuples_roundtrip_and_sort_by_their_elements(
        xs in prop::collection::vec(value(), 0..4),
        ys in prop::collection::vec(value(), 0..4),
    ) {
        check_roundtrip(&xs).map_err(TestCaseError::fail)?;
        prop_assert_eq!(encode_tuple(&xs).cmp(&encode_tuple(&ys)), tuple_order(&xs, &ys));
    }
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
    let pairs = (value(), value());
    runner
        .run(&pairs, |(a, b)| {
            check_pair(&a, &b).map_err(TestCaseError::fail)
        })
        .expect("fuzz found a violation: append the minimal pair to tests/regressions/tuple.txt");
}
