//! Property and golden-vector tests for arbitrary-precision typed
//! numeric values (slice A9122-numeric-encoding, issue #10228; design
//! statement A-9122.1, S4.64–S4.71).
//!
//! Oracle: **invariant**, against an exact reference model. For values
//! that fit, the reference is plain `i128` arithmetic: a decimal's
//! canonical form strips every factor of ten from the coefficient into
//! the exponent, a binary float's strips every factor of two from the
//! significand, and every zero is `+0` with a zero exponent. For values
//! that do not fit, the reference is constructive (a known odd or
//! ten-free core times a known power) and exact decimal-string
//! round-tripping. Every value round-trips through the canonical codec
//! byte-for-byte, non-canonical bytes reject with the stated failure
//! class, non-finite floats reject, and an over-budget magnitude rejects
//! as `resource_limit` without being allocated.
//!
//! The crate-root export of `numeric_value` belongs to slice 9110-02, so
//! until that lands this test includes the module by path, the same file
//! the export will name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/numeric_value.rs"]
mod numeric_value;

#[path = "../src/typed_value.rs"]
#[allow(dead_code)]
mod typed_value;

use numeric_value::{
    BigInt, KIND_BINARY_FLOAT, KIND_DECIMAL, KIND_INTEGER, NumericError, NumericLimits,
    NumericValue, TYPE_NUMERIC,
};
use proptest::prelude::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn big(text: &str) -> BigInt {
    BigInt::parse(text).unwrap()
}

/// Reference decimal normalization over `i128`.
fn ref_decimal(coefficient: i128, exponent: i128) -> (i128, i128) {
    if coefficient == 0 {
        return (0, 0);
    }
    let (mut c, mut e) = (coefficient, exponent);
    while c % 10 == 0 {
        c /= 10;
        e += 1;
    }
    (c, e)
}

/// Reference binary-float normalization over `i128`.
fn ref_binary(negative: bool, significand: u128, exponent: i128) -> (bool, u128, i128) {
    if significand == 0 {
        return (false, 0, 0);
    }
    let shift = significand.trailing_zeros();
    (negative, significand >> shift, exponent + i128::from(shift))
}

fn parts_decimal(value: &NumericValue) -> (i128, i128) {
    match value {
        NumericValue::Decimal(d) => (
            d.coefficient().to_i128().unwrap(),
            d.exponent10().to_i128().unwrap(),
        ),
        other => panic!("not a decimal: {other:?}"),
    }
}

fn parts_binary(value: &NumericValue) -> (bool, u128, i128) {
    match value {
        NumericValue::BinaryFloat(b) => (
            b.is_negative(),
            u128::try_from(b.significand().to_i128().unwrap()).unwrap(),
            b.exponent2().to_i128().unwrap(),
        ),
        other => panic!("not a binary float: {other:?}"),
    }
}

/// Exact reconstruction of `sig * 2^exp` as an `f64`, stepping the
/// power so no intermediate over- or underflows before the (finite,
/// representable) result.
fn ldexp(significand: f64, mut exponent: i128) -> f64 {
    let mut x = significand;
    while exponent != 0 {
        let step = exponent.clamp(-1000, 1000);
        x *= 2f64.powi(i32::try_from(step).unwrap());
        exponent -= step;
    }
    x
}

fn assert_round_trip(value: &NumericValue) {
    let bytes = value.encode();
    let back = NumericValue::decode(&bytes).unwrap();
    assert_eq!(&back, value);
    assert_eq!(back.encode(), bytes);
}

// ---------------------------------------------------------------------------
// Golden vectors
// ---------------------------------------------------------------------------

#[test]
fn golden_vectors() {
    let cases = [
        (
            NumericValue::integer(BigInt::zero()),
            "01 00 00 0000000000000000",
        ),
        (
            NumericValue::integer(BigInt::from_i64(-258)),
            "01 00 01 0000000000000002 0102",
        ),
        (
            NumericValue::decimal(&big("1200"), &big("-3")),
            "01 01 00 0000000000000001 0c 01 0000000000000001 01",
        ),
        (
            NumericValue::from_f64(1.5).unwrap(),
            "01 02 00 00 0000000000000001 03 01 0000000000000001 01",
        ),
        (
            NumericValue::from_f64(-0.0).unwrap(),
            "01 02 00 00 0000000000000000 00 0000000000000000",
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(hex(&value.encode()), hex(&unhex(expected)), "{value:?}");
        assert_eq!(NumericValue::decode(&unhex(expected)).unwrap(), value);
    }
}

#[test]
fn kinds_never_coerce() {
    let integer = NumericValue::integer(BigInt::from_i64(1));
    let decimal = NumericValue::decimal(&BigInt::from_i64(1), &BigInt::zero());
    let float = NumericValue::from_f64(1.0).unwrap();
    assert_ne!(integer, decimal);
    assert_ne!(decimal, float);
    assert_ne!(integer, float);
    assert_ne!(integer.encode(), decimal.encode());
    assert_eq!(integer.kind_tag(), KIND_INTEGER);
    assert_eq!(decimal.kind_tag(), KIND_DECIMAL);
    assert_eq!(float.kind_tag(), KIND_BINARY_FLOAT);
    assert!(KIND_INTEGER < KIND_DECIMAL && KIND_DECIMAL < KIND_BINARY_FLOAT);
}

#[test]
fn every_zero_is_canonical_positive_zero() {
    let decimal_zero = NumericValue::decimal(&BigInt::zero(), &big("12345678901234567890"));
    assert_eq!(parts_decimal(&decimal_zero), (0, 0));
    let negative_zero = NumericValue::binary_float(true, &BigInt::zero(), &BigInt::from_i64(-7));
    assert_eq!(parts_binary(&negative_zero.unwrap()), (false, 0, 0));
    assert_eq!(
        NumericValue::from_f64(-0.0).unwrap(),
        NumericValue::from_f64(0.0).unwrap()
    );
    assert!(!BigInt::parse("-0").unwrap().is_negative());
    assert_eq!(BigInt::from_be_magnitude(true, &[0, 0]), BigInt::zero());
}

#[test]
fn non_finite_floats_reject() {
    for value in [f64::NAN, -f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = NumericValue::from_f64(value).unwrap_err();
        assert_eq!(error, NumericError::NonFinite);
        assert_eq!(error.code(), "malformed");
    }
}

#[test]
fn extreme_f64_values_are_exact() {
    for value in [
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::EPSILON,
    ] {
        let numeric = NumericValue::from_f64(value).unwrap();
        let (negative, significand, exponent) = parts_binary(&numeric);
        assert_eq!(significand % 2, 1);
        let magnitude = ldexp(significand as f64, exponent);
        assert_eq!(if negative { -magnitude } else { magnitude }, value);
        assert_round_trip(&numeric);
    }
}

#[test]
fn negative_significand_rejects() {
    assert_eq!(
        NumericValue::binary_float(false, &BigInt::from_i64(-3), &BigInt::zero()).unwrap_err(),
        NumericError::NegativeSignificand
    );
}

// ---------------------------------------------------------------------------
// Decoder rejections (failure classification)
// ---------------------------------------------------------------------------

#[test]
fn non_canonical_bytes_reject() {
    let cases = [
        // decimal coefficient 10: still divisible by ten
        (
            "01 01 00 0000000000000001 0a 00 0000000000000000",
            NumericError::NonCanonical("decimal coefficient is divisible by ten"),
        ),
        // decimal zero with exponent 1
        (
            "01 01 00 0000000000000000 00 0000000000000001 01",
            NumericError::NonCanonical("decimal zero has a nonzero exponent"),
        ),
        // binary float with even significand 2
        (
            "01 02 00 00 0000000000000001 02 00 0000000000000000",
            NumericError::NonCanonical("binary float significand is even"),
        ),
        // binary float zero with exponent 1
        (
            "01 02 00 00 0000000000000000 00 0000000000000001 01",
            NumericError::NonCanonical("binary float zero has a nonzero exponent"),
        ),
        // binary float negative zero
        (
            "01 02 01 00 0000000000000000 00 0000000000000000",
            NumericError::NegativeZero,
        ),
        // binary float negative significand
        (
            "01 02 00 01 0000000000000001 03 00 0000000000000000",
            NumericError::NegativeSignificand,
        ),
        // integer negative zero
        ("01 00 01 0000000000000000", NumericError::NegativeZero),
        // integer magnitude with a leading zero byte
        (
            "01 00 00 0000000000000002 0001",
            NumericError::LeadingZero("integer"),
        ),
        // bad sign byte
        ("01 00 02 0000000000000000", NumericError::BadSign(2)),
        // unknown version and kind
        ("02 00 00 0000000000000000", NumericError::UnknownVersion(2)),
        ("01 03", NumericError::UnknownKind(3)),
        // trailing byte
        ("01 00 00 0000000000000000 00", NumericError::TrailingBytes),
    ];
    for (bytes, expected) in cases {
        let error = NumericValue::decode(&unhex(bytes)).unwrap_err();
        assert_eq!(error, expected, "{bytes}");
        assert_eq!(error.code(), "malformed");
    }
}

#[test]
fn every_strict_prefix_is_truncated() {
    let value = NumericValue::decimal(&big("-98765432109876543210123"), &big("-400"));
    let bytes = value.encode();
    for len in 0..bytes.len() {
        assert_eq!(
            NumericValue::decode(&bytes[..len]).unwrap_err(),
            NumericError::Truncated,
            "prefix {len}"
        );
    }
}

#[test]
fn over_budget_magnitude_is_a_resource_limit() {
    let value = NumericValue::integer(BigInt::from_be_magnitude(false, &[0xff; 9]));
    let bytes = value.encode();
    let limits = NumericLimits {
        max_magnitude_bytes: 8,
    };
    let error = NumericValue::decode_with(&bytes, limits).unwrap_err();
    assert_eq!(
        error,
        NumericError::ResourceLimit {
            field: "integer",
            len: 9,
            limit: 8
        }
    );
    assert_eq!(error.code(), "resource_limit");
    assert_eq!(
        NumericValue::decode_with(
            &bytes,
            NumericLimits {
                max_magnitude_bytes: 9
            }
        )
        .unwrap(),
        value
    );

    // An adversarial exponent length is refused from its header alone:
    // nothing near u64::MAX bytes is ever allocated or read.
    let mut huge = unhex("01 01 00 0000000000000001 07 00");
    huge.extend_from_slice(&u64::MAX.to_be_bytes());
    let error = NumericValue::decode(&huge).unwrap_err();
    assert!(matches!(
        error,
        NumericError::ResourceLimit {
            field: "exponent10",
            len: u64::MAX,
            ..
        }
    ));
    assert_eq!(error.code(), "resource_limit");
}

#[test]
fn large_values_are_not_rejected_by_any_semantic_maximum() {
    let digits = "9".repeat(5000);
    let exponent = format!("-{}", "7".repeat(400));
    let value = NumericValue::decimal(&big(&digits), &big(&exponent));
    assert_round_trip(&value);
    match &value {
        NumericValue::Decimal(d) => {
            assert_eq!(d.coefficient().to_decimal_string(), digits);
            assert_eq!(d.exponent10().to_decimal_string(), exponent);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn numeric_payload_rides_in_a_typed_value() {
    use typed_value::{TypeId, TypedValue};
    let payload = NumericValue::decimal(&big("-314159"), &big("-5")).encode();
    let typed = TypedValue::opaque(TypeId::registered(TYPE_NUMERIC), payload.clone());
    let back = TypedValue::decode(&typed.encode()).unwrap();
    assert_eq!(back.type_id().name_string(), TYPE_NUMERIC);
    assert_eq!(back.payload(), payload.as_slice());
    assert_eq!(
        NumericValue::decode(back.payload()).unwrap(),
        NumericValue::decimal(&big("-314159"), &big("-5"))
    );
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn bigint_matches_i128(a in any::<i64>(), b in any::<i64>()) {
        let (x, y) = (BigInt::from_i64(a), BigInt::from_i64(b));
        prop_assert_eq!(x.to_i128(), Some(i128::from(a)));
        prop_assert_eq!(x.add(&y).to_i128(), Some(i128::from(a) + i128::from(b)));
        prop_assert_eq!(x.to_decimal_string(), a.to_string());
        prop_assert_eq!(x.abs().to_i128(), Some(i128::from(a).abs()));
        prop_assert_eq!(x.is_zero(), a == 0);
        prop_assert_eq!(BigInt::parse(&a.to_string()), Some(x));
    }

    #[test]
    fn bigint_i128_extremes(a in any::<i128>()) {
        let x = BigInt::from_i128(a);
        prop_assert_eq!(x.to_i128(), Some(a));
        prop_assert_eq!(x.to_decimal_string(), a.to_string());
    }

    #[test]
    fn bigint_large_round_trips(
        negative in any::<bool>(),
        magnitude in proptest::collection::vec(any::<u8>(), 0..96),
        other in proptest::collection::vec(any::<u8>(), 0..96),
    ) {
        let x = BigInt::from_be_magnitude(negative, &magnitude);
        let first = magnitude.iter().position(|&b| b != 0).unwrap_or(magnitude.len());
        prop_assert_eq!(x.magnitude_be_bytes(), magnitude[first..].to_vec());
        prop_assert_eq!(x.magnitude_len(), magnitude.len() - first);
        prop_assert_eq!(BigInt::parse(&x.to_decimal_string()), Some(x.clone()));
        let y = BigInt::from_be_magnitude(!negative, &other);
        // (x + y) + (-y) == x and addition commutes.
        let minus_y = BigInt::from_be_magnitude(negative, &other);
        prop_assert_eq!(x.add(&y).add(&minus_y), x.clone());
        prop_assert_eq!(x.add(&y), y.add(&x));
    }

    #[test]
    fn decimal_normalizes_like_the_reference(c in any::<i64>(), e in any::<i64>()) {
        let value = NumericValue::decimal(&BigInt::from_i64(c), &BigInt::from_i64(e));
        prop_assert_eq!(parts_decimal(&value), ref_decimal(i128::from(c), i128::from(e)));
        prop_assert_eq!(value.is_zero(), c == 0);
        assert_round_trip(&value);
    }

    #[test]
    fn decimal_strips_large_ten_powers(
        core in "[1-9][0-9]{0,40}[1-9]",
        negative in any::<bool>(),
        zeros in 0usize..300,
        e in any::<i64>(),
    ) {
        let text = format!("{}{core}{}", if negative { "-" } else { "" }, "0".repeat(zeros));
        let value = NumericValue::decimal(&big(&text), &BigInt::from_i64(e));
        match &value {
            NumericValue::Decimal(d) => {
                let expected_core = format!("{}{core}", if negative { "-" } else { "" });
                prop_assert_eq!(d.coefficient().to_decimal_string(), expected_core);
                prop_assert_eq!(
                    d.exponent10().to_i128(),
                    Some(i128::from(e) + i128::try_from(zeros).unwrap())
                );
            }
            other => prop_assert!(false, "{other:?}"),
        }
        assert_round_trip(&value);
    }

    #[test]
    fn binary_float_normalizes_like_the_reference(
        negative in any::<bool>(),
        s in any::<u64>(),
        e in any::<i64>(),
    ) {
        let value =
            NumericValue::binary_float(negative, &BigInt::from_u64(s), &BigInt::from_i64(e)).unwrap();
        prop_assert_eq!(
            parts_binary(&value),
            ref_binary(negative, u128::from(s), i128::from(e))
        );
        assert_round_trip(&value);
    }

    #[test]
    fn binary_float_strips_large_two_powers(
        core in proptest::collection::vec(any::<u8>(), 1..40),
        low in 0u8..128,
        zero_bytes in 0usize..64,
        e in any::<i64>(),
    ) {
        // core | (2*low + 1) is odd; appending zero bytes multiplies by 256^n.
        let mut odd = core.clone();
        odd.push(low * 2 + 1);
        let mut shifted = odd.clone();
        shifted.extend(std::iter::repeat_n(0u8, zero_bytes));
        let value = NumericValue::binary_float(
            false,
            &BigInt::from_be_magnitude(false, &shifted),
            &BigInt::from_i64(e),
        )
        .unwrap();
        match &value {
            NumericValue::BinaryFloat(b) => {
                prop_assert_eq!(b.significand(), &BigInt::from_be_magnitude(false, &odd));
                prop_assert_eq!(
                    b.exponent2().to_i128(),
                    Some(i128::from(e) + 8 * i128::try_from(zero_bytes).unwrap())
                );
            }
            other => prop_assert!(false, "{other:?}"),
        }
        assert_round_trip(&value);
    }

    #[test]
    fn finite_f64_is_exact(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        prop_assume!(x.is_finite());
        let value = NumericValue::from_f64(x).unwrap();
        let (negative, significand, exponent) = parts_binary(&value);
        if x == 0.0 {
            prop_assert_eq!((negative, significand, exponent), (false, 0, 0));
        } else {
            prop_assert_eq!(significand % 2, 1);
            let magnitude = ldexp(significand as f64, exponent);
            prop_assert_eq!(if negative { -magnitude } else { magnitude }, x);
        }
        assert_round_trip(&value);
    }

    #[test]
    fn non_finite_f64_rejects(bits in any::<u64>()) {
        let x = f64::from_bits(bits | 0x7ff0_0000_0000_0000);
        prop_assert_eq!(NumericValue::from_f64(x), Err(NumericError::NonFinite));
    }

    #[test]
    fn decoder_never_panics_and_accepts_only_canonical(
        bytes in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        if let Ok(value) = NumericValue::decode(&bytes) {
            prop_assert_eq!(value.encode(), bytes);
        }
    }
}
