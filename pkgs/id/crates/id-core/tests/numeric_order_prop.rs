//! Property tests for the type-first numeric order (slice
//! A9122-numeric-order, issue #10245; design statement A-9122.1,
//! S4.69–S4.73).
//!
//! Oracle: **invariant**, against an exact arbitrary-precision
//! reference. For operands that fit, the reference scales both values
//! to a common exponent in `i128` and compares exactly; finite `f64`
//! values are also checked against the hardware total order. Mixed kinds
//! order by kind tag alone. Negatives order by descending magnitude,
//! then canonical zero, then positives (`-10 < -2 < 0 < 2 < 10`).
//! Comparison over exponents far beyond any materializable power
//! terminates, and an operand over the budget returns `resource_limit`.
//!
//! The crate-root exports of `numeric_value` and `numeric_order` belong
//! to slice 9110-02, so until that lands this test includes both modules
//! by path, the same files the exports will name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/numeric_value.rs"]
#[allow(dead_code)]
mod numeric_value;

#[path = "../src/numeric_order.rs"]
mod numeric_order;

use numeric_order::{compare, compare_with};
use numeric_value::{BigInt, NumericError, NumericLimits, NumericValue};
use proptest::prelude::*;
use std::cmp::Ordering;

fn big(text: &str) -> BigInt {
    BigInt::parse(text).unwrap()
}

fn int(v: i64) -> NumericValue {
    NumericValue::integer(BigInt::from_i64(v))
}

fn dec(c: i64, e: i64) -> NumericValue {
    NumericValue::decimal(&BigInt::from_i64(c), &BigInt::from_i64(e))
}

fn bin(v: i64, e: i64) -> NumericValue {
    NumericValue::binary_float(
        v < 0,
        &BigInt::from_i128(i128::from(v).abs()),
        &BigInt::from_i64(e),
    )
    .unwrap()
}

fn cmp(a: &NumericValue, b: &NumericValue) -> Ordering {
    compare(a, b).unwrap()
}

/// Exact reference: `ca * base^ea` against `cb * base^eb`, scaled to the
/// smaller exponent in `i128`. Callers keep values within range.
fn ref_scaled(ca: i128, ea: i64, cb: i128, eb: i64, base: i128) -> Ordering {
    let low = ea.min(eb);
    let scale = |c: i128, e: i64| c * base.pow(u32::try_from(e - low).unwrap());
    scale(ca, ea).cmp(&scale(cb, eb))
}

fn assert_strictly_increasing(values: &[NumericValue]) {
    for (i, a) in values.iter().enumerate() {
        for (j, b) in values.iter().enumerate() {
            assert_eq!(cmp(a, b), i.cmp(&j), "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn negative_zero_positive_vectors_in_every_kind() {
    assert_strictly_increasing(&[int(-10), int(-2), int(0), int(2), int(10)]);
    assert_strictly_increasing(&[dec(-10, 0), dec(-2, 0), dec(0, 0), dec(2, 0), dec(10, 0)]);
    assert_strictly_increasing(&[bin(-10, 0), bin(-2, 0), bin(0, 0), bin(2, 0), bin(10, 0)]);
    assert_strictly_increasing(&[
        NumericValue::from_f64(-10.0).unwrap(),
        NumericValue::from_f64(-2.0).unwrap(),
        NumericValue::from_f64(-0.0).unwrap(),
        NumericValue::from_f64(2.0).unwrap(),
        NumericValue::from_f64(10.0).unwrap(),
    ]);
}

#[test]
fn kinds_order_by_tag_with_no_coercion() {
    // Every integer < every decimal < every binary float.
    assert_strictly_increasing(&[
        int(-5),
        int(1000),
        dec(-1, 9),
        dec(3, -2),
        bin(-7, 40),
        bin(1, 0),
    ]);
    assert_eq!(cmp(&int(1), &dec(1, 0)), Ordering::Less);
    assert_eq!(cmp(&dec(1, 0), &bin(1, 0)), Ordering::Less);
}

#[test]
fn decimals_with_equal_effective_length_compare_digits() {
    // 0.15 < 0.2 < 0.25 < 1 < 1.5, all as decimals.
    assert_strictly_increasing(&[dec(15, -2), dec(2, -1), dec(25, -2), dec(1, 0), dec(15, -1)]);
    // 123 < 1231e-1 < 124 (equal length 3, digit order decides).
    assert_strictly_increasing(&[dec(123, 0), dec(1231, -1), dec(124, 0)]);
}

#[test]
fn huge_exponents_compare_without_materializing_powers() {
    let googolplex_like = format!("1{}", "0".repeat(3000));
    let e = big(&googolplex_like);
    let e_minus_one = e.add(&BigInt::from_i64(-1));
    let neg_e = big(&format!("-{googolplex_like}"));
    // 9e(E-1) < 1eE < 2eE, and the reciprocals sort below 1.
    let values = [
        NumericValue::decimal(&BigInt::from_i64(1), &neg_e),
        NumericValue::decimal(&BigInt::from_i64(1), &BigInt::zero()),
        NumericValue::decimal(&BigInt::from_i64(9), &e_minus_one),
        NumericValue::decimal(&BigInt::from_i64(1), &e),
        NumericValue::decimal(&BigInt::from_i64(2), &e),
    ];
    assert_strictly_increasing(&values);
    // Same for binary floats: 3 * 2^(E-2) < 1 * 2^E < 3 * 2^E.
    let binary = [
        NumericValue::binary_float(false, &BigInt::from_i64(3), &e.add(&BigInt::from_i64(-2)))
            .unwrap(),
        NumericValue::binary_float(false, &BigInt::from_i64(1), &e).unwrap(),
        NumericValue::binary_float(false, &BigInt::from_i64(3), &e).unwrap(),
    ];
    assert_strictly_increasing(&binary);
    // And reversed for negatives.
    let negatives = [
        NumericValue::decimal(&BigInt::from_i64(-2), &e),
        NumericValue::decimal(&BigInt::from_i64(-1), &e),
        NumericValue::decimal(&BigInt::from_i64(-9), &e_minus_one),
    ];
    assert_strictly_increasing(&negatives);
}

#[test]
fn over_budget_operand_is_a_resource_limit() {
    let wide = NumericValue::decimal(
        &BigInt::from_i64(7),
        &BigInt::from_be_magnitude(false, &[0xff; 9]),
    );
    let limits = NumericLimits {
        max_magnitude_bytes: 8,
    };
    let error = compare_with(&wide, &dec(1, 0), limits).unwrap_err();
    assert_eq!(
        error,
        NumericError::ResourceLimit {
            field: "exponent10",
            len: 9,
            limit: 8
        }
    );
    assert_eq!(error.code(), "resource_limit");
    // The budget applies even when the kind tag alone would decide.
    assert!(compare_with(&int(0), &wide, limits).is_err());
    assert_eq!(
        compare_with(
            &wide,
            &dec(1, 0),
            NumericLimits {
                max_magnitude_bytes: 9
            }
        ),
        Ok(Ordering::Greater)
    );
}

fn small_value() -> impl Strategy<Value = NumericValue> {
    prop_oneof![
        any::<i32>().prop_map(|v| int(i64::from(v))),
        (any::<i32>(), -12i64..12).prop_map(|(c, e)| dec(i64::from(c), e)),
        (any::<i32>(), -20i64..20).prop_map(|(s, e)| bin(i64::from(s), e)),
    ]
}

/// Exact reference over small values of any kind.
fn reference(a: &NumericValue, b: &NumericValue) -> Ordering {
    let by_kind = a.kind_tag().cmp(&b.kind_tag());
    if by_kind != Ordering::Equal {
        return by_kind;
    }
    match (a, b) {
        (NumericValue::Integer(x), NumericValue::Integer(y)) => {
            x.to_i128().unwrap().cmp(&y.to_i128().unwrap())
        }
        (NumericValue::Decimal(x), NumericValue::Decimal(y)) => ref_scaled(
            x.coefficient().to_i128().unwrap(),
            i64::try_from(x.exponent10().to_i128().unwrap()).unwrap(),
            y.coefficient().to_i128().unwrap(),
            i64::try_from(y.exponent10().to_i128().unwrap()).unwrap(),
            10,
        ),
        (NumericValue::BinaryFloat(x), NumericValue::BinaryFloat(y)) => {
            let signed = |f: &numeric_value::BinaryFloat| {
                let s = f.significand().to_i128().unwrap();
                if f.is_negative() { -s } else { s }
            };
            ref_scaled(
                signed(x),
                i64::try_from(x.exponent2().to_i128().unwrap()).unwrap(),
                signed(y),
                i64::try_from(y.exponent2().to_i128().unwrap()).unwrap(),
                2,
            )
        }
        _ => unreachable!(),
    }
}

proptest! {
    #[test]
    fn matches_exact_reference(a in small_value(), b in small_value()) {
        prop_assert_eq!(cmp(&a, &b), reference(&a, &b));
    }

    #[test]
    fn comparator_laws(a in small_value(), b in small_value(), c in small_value()) {
        // Reflexive equality agrees with value equality.
        prop_assert_eq!(cmp(&a, &a), Ordering::Equal);
        prop_assert_eq!(cmp(&a, &b) == Ordering::Equal, a == b);
        // Antisymmetry.
        prop_assert_eq!(cmp(&a, &b), cmp(&b, &a).reverse());
        // Transitivity.
        if cmp(&a, &b) != Ordering::Greater && cmp(&b, &c) != Ordering::Greater {
            prop_assert_ne!(cmp(&a, &c), Ordering::Greater);
        }
    }

    #[test]
    fn finite_f64_order_is_hardware_order(x in any::<u64>(), y in any::<u64>()) {
        let (x, y) = (f64::from_bits(x), f64::from_bits(y));
        prop_assume!(x.is_finite() && y.is_finite());
        let (a, b) = (NumericValue::from_f64(x).unwrap(), NumericValue::from_f64(y).unwrap());
        prop_assert_eq!(cmp(&a, &b), x.partial_cmp(&y).unwrap());
    }

    #[test]
    fn large_integers_order_like_their_digits(
        a in "-?[1-9][0-9]{0,60}",
        b in "-?[1-9][0-9]{0,60}",
    ) {
        // Reference: compare sign, then digit count, then digits.
        let key = |s: &str| -> (i8, usize, String) {
            let neg = s.starts_with('-');
            let digits = s.trim_start_matches('-').to_owned();
            (if neg { -1 } else { 1 }, digits.len(), digits)
        };
        let (ka, kb) = (key(&a), key(&b));
        let magnitude = ka.1.cmp(&kb.1).then_with(|| ka.2.cmp(&kb.2));
        let expected = match ka.0.cmp(&kb.0) {
            Ordering::Equal if ka.0 < 0 => magnitude.reverse(),
            Ordering::Equal => magnitude,
            other => other,
        };
        let (x, y) = (NumericValue::integer(big(&a)), NumericValue::integer(big(&b)));
        prop_assert_eq!(cmp(&x, &y), expected);
    }
}
