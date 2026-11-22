//! Type-first total order over canonical numeric values.
//!
//! Slice A9122-numeric-order (issue #10245), design statement A-9122.1
//! (`part-a-addressing/versioned-design.md` section "#9122 — Numeric
//! registry family", S4.69–S4.73). Values compare by stable kind tag
//! first (`Integer < Decimal < BinaryFloat`, see
//! [`NumericValue::kind_tag`]) and only then numerically within their
//! kind. There is no cross-kind coercion: integer `1000` sorts before
//! decimal `-1e9`.
//!
//! Within a kind, negatives sort by descending absolute magnitude, the
//! canonical zero follows every negative, and positives sort by
//! ascending magnitude: `-10 < -2 < 0 < 2 < 10`.
//!
//! Magnitudes are compared without materializing `10^exponent` or
//! `2^exponent`. A nonzero canonical decimal `c * 10^e` with `d` digits
//! lies in `[10^(d+e-1), 10^(d+e))`, so the effective length `d + e`
//! (an exact [`BigInt`] sum) decides unless equal; then the digit
//! strings are compared left-aligned with virtual trailing zeros. A
//! binary float `s * 2^e` uses bit length the same way. Work is linear
//! in the operands' encoded size (quadratic for decimal digit
//! extraction) and never in the exponent's value.
//!
//! Every magnitude involved, exponents included, is first checked
//! against [`NumericLimits::max_magnitude_bytes`]; an operand over that
//! operational budget yields [`NumericError::ResourceLimit`] rather than
//! an ordering.
//!
//! This module is intentionally dependency-free (pure `std`) and builds
//! only on the public API of `numeric_value`. The crate-root export
//! belongs to slice 9110-02; until that lands, the integration tests
//! include both files by path.

use std::cmp::Ordering;

use crate::numeric_value::{BigInt, NumericError, NumericLimits, NumericValue};

/// Compares two numeric values under the default budget.
///
/// # Errors
///
/// [`NumericError::ResourceLimit`] when an operand exceeds the budget.
pub fn compare(a: &NumericValue, b: &NumericValue) -> Result<Ordering, NumericError> {
    compare_with(a, b, NumericLimits::default())
}

/// Compares two numeric values under `limits`.
///
/// # Errors
///
/// [`NumericError::ResourceLimit`] when an operand exceeds the budget.
pub fn compare_with(
    a: &NumericValue,
    b: &NumericValue,
    limits: NumericLimits,
) -> Result<Ordering, NumericError> {
    check_budget(a, limits)?;
    check_budget(b, limits)?;
    let by_kind = a.kind_tag().cmp(&b.kind_tag());
    if by_kind != Ordering::Equal {
        return Ok(by_kind);
    }
    Ok(match (a, b) {
        (NumericValue::Integer(x), NumericValue::Integer(y)) => cmp_bigint(x, y),
        (NumericValue::Decimal(x), NumericValue::Decimal(y)) => signed(
            sign_of(x.coefficient().is_negative(), x.coefficient().is_zero()),
            sign_of(y.coefficient().is_negative(), y.coefficient().is_zero()),
            || {
                cmp_decimal_magnitude(
                    x.coefficient(),
                    x.exponent10(),
                    y.coefficient(),
                    y.exponent10(),
                )
            },
        ),
        (NumericValue::BinaryFloat(x), NumericValue::BinaryFloat(y)) => signed(
            sign_of(x.is_negative(), x.significand().is_zero()),
            sign_of(y.is_negative(), y.significand().is_zero()),
            || {
                cmp_binary_magnitude(
                    x.significand(),
                    x.exponent2(),
                    y.significand(),
                    y.exponent2(),
                )
            },
        ),
        // Equal kind tags imply equal variants.
        _ => Ordering::Equal,
    })
}

fn check_budget(value: &NumericValue, limits: NumericLimits) -> Result<(), NumericError> {
    let parts: [(&'static str, Option<&BigInt>); 2] = match value {
        NumericValue::Integer(x) => [("integer", Some(x)), ("integer", None)],
        NumericValue::Decimal(d) => [
            ("coefficient", Some(d.coefficient())),
            ("exponent10", Some(d.exponent10())),
        ],
        NumericValue::BinaryFloat(f) => [
            ("significand", Some(f.significand())),
            ("exponent2", Some(f.exponent2())),
        ],
    };
    for (field, part) in parts {
        let Some(part) = part else { continue };
        let len = part.magnitude_len();
        if len > limits.max_magnitude_bytes {
            return Err(NumericError::ResourceLimit {
                field,
                len: u64::try_from(len).unwrap_or(u64::MAX),
                limit: limits.max_magnitude_bytes,
            });
        }
    }
    Ok(())
}

/// -1, 0 or 1.
const fn sign_of(negative: bool, zero: bool) -> i8 {
    if zero {
        0
    } else if negative {
        -1
    } else {
        1
    }
}

/// Orders by sign; for two values of the same nonzero sign, by
/// magnitude, reversed for negatives.
fn signed(a: i8, b: i8, magnitude: impl FnOnce() -> Ordering) -> Ordering {
    match a.cmp(&b) {
        Ordering::Equal if a == 0 => Ordering::Equal,
        Ordering::Equal if a < 0 => magnitude().reverse(),
        Ordering::Equal => magnitude(),
        other => other,
    }
}

/// Exact integer order.
fn cmp_bigint(a: &BigInt, b: &BigInt) -> Ordering {
    signed(
        sign_of(a.is_negative(), a.is_zero()),
        sign_of(b.is_negative(), b.is_zero()),
        || cmp_magnitude_bytes(&a.magnitude_be_bytes(), &b.magnitude_be_bytes()),
    )
}

/// Compares minimal big-endian magnitudes.
fn cmp_magnitude_bytes(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// `|ca| * 10^ea` against `|cb| * 10^eb`, both coefficients nonzero.
fn cmp_decimal_magnitude(ca: &BigInt, ea: &BigInt, cb: &BigInt, eb: &BigInt) -> Ordering {
    let da = ca.abs().to_decimal_string();
    let db = cb.abs().to_decimal_string();
    let la = ea.add(&BigInt::from_u64(da.len() as u64));
    let lb = eb.add(&BigInt::from_u64(db.len() as u64));
    cmp_bigint(&la, &lb).then_with(|| cmp_left_aligned(da.as_bytes(), db.as_bytes(), b'0'))
}

/// `sa * 2^ea` against `sb * 2^eb`, both significands nonzero.
fn cmp_binary_magnitude(sa: &BigInt, ea: &BigInt, sb: &BigInt, eb: &BigInt) -> Ordering {
    let ba = bits_msb_first(&sa.magnitude_be_bytes());
    let bb = bits_msb_first(&sb.magnitude_be_bytes());
    let la = ea.add(&BigInt::from_u64(ba.len() as u64));
    let lb = eb.add(&BigInt::from_u64(bb.len() as u64));
    cmp_bigint(&la, &lb).then_with(|| cmp_left_aligned(&ba, &bb, 0))
}

/// Compares two digit strings aligned at their most significant digit,
/// the shorter extended with `pad` digits.
fn cmp_left_aligned(a: &[u8], b: &[u8], pad: u8) -> Ordering {
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(pad);
        let y = b.get(i).copied().unwrap_or(pad);
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

/// The bits of a minimal big-endian magnitude, most significant first,
/// starting at the leading one bit.
fn bits_msb_first(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for byte in bytes {
        for shift in (0..8).rev() {
            bits.push((byte >> shift) & 1);
        }
    }
    let first = bits.iter().position(|&bit| bit == 1).unwrap_or(bits.len());
    bits.drain(..first);
    bits
}
