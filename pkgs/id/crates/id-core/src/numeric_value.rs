//! Arbitrary-precision typed numeric values and their canonical codec.
//!
//! Slice A9122-numeric-encoding (issue #10228), design statement A-9122.1
//! (`part-a-addressing/versioned-design.md` section "#9122 — Numeric
//! registry family", S4.64–S4.71). A numeric key is one of three kinds:
//!
//! ```text
//! NumericValue = Integer(BigInt)
//!              | Decimal(coefficient: BigInt, exponent10: BigInt)
//!              | BinaryFloat(sign, significand: BigInt, exponent2: BigInt)
//! ```
//!
//! There is no fixed maximum magnitude or precision: integers,
//! coefficients, significands and both exponents are unbounded
//! [`BigInt`]s. Every value is held in its one canonical form:
//!
//! * a decimal strips factors of ten from a nonzero coefficient and adds
//!   them to `exponent10` (`1200e-3` is held as `12e-1`);
//! * a binary float strips factors of two from a nonzero significand and
//!   adds them to `exponent2` (`12 * 2^0` is held as `3 * 2^2`);
//! * every signed or exponent-carrying zero canonicalizes to `+0` with a
//!   zero exponent, so `-0.0`, `0e7` and `-0 * 2^-3` all collapse.
//!
//! NaN and the infinities have no representation here: [`NumericValue::from_f64`]
//! rejects them with [`NumericError::NonFinite`], and the wire format below
//! cannot express them. (The design's optional IEEE-754 compatibility
//! profile, which would keep them as typed non-ordered values outside the
//! ordered registry, is not part of this slice.) Kinds never coerce into
//! one another: integer `1`, decimal `1e0` and binary float `1 * 2^0` are
//! three distinct values. Ordering is slice A9122-numeric-order; this
//! module only owns representation and the codec.
//!
//! Payload format (version [`PAYLOAD_VERSION`]), all integers big-endian.
//! It is the payload of a typed value whose type ID is [`TYPE_NUMERIC`]:
//!
//! ```text
//! payload := version u8 | kind u8 | body
//! kind    := 0x00 integer       body := bigint
//!          | 0x01 decimal       body := bigint coefficient | bigint exponent10
//!          | 0x02 binary float  body := sign u8 | bigint significand
//!                                       | bigint exponent2
//! bigint  := sign u8 | len u64 | magnitude (len bytes, big-endian)
//! sign    := 0x00 zero or positive | 0x01 negative
//! ```
//!
//! The format is canonical: every value has exactly one encoding, so
//! `decode(encode(x)) == x` for every value and `encode(decode(b)) == b`
//! for every accepted `b`. The decoder rejects a magnitude with a leading
//! zero byte, a negative zero, a negative significand, a decimal
//! coefficient still divisible by ten, an even nonzero significand, a zero
//! with a nonzero exponent or negative sign, unknown version or kind bytes,
//! truncation and trailing bytes. A magnitude longer than the configured
//! [`NumericLimits::max_magnitude_bytes`] rejects with the operational
//! [`NumericError::ResourceLimit`] before anything of that size is
//! allocated; that is a budget, not a semantic range rule.
//!
//! This module is intentionally dependency-free (pure `std`): the crate
//! root export and manifest wiring belong to slice 9110-02; until that
//! lands, the integration tests include this file by path.

use std::cmp::Ordering;
use std::fmt::{self, Write as _};

/// Type ID under which a numeric payload is carried as a typed value.
pub const TYPE_NUMERIC: &str = "id-value/numeric/v1";

/// Version byte of the numeric payload format defined here.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Kind byte of an integer value. Also its stable type tag.
pub const KIND_INTEGER: u8 = 0x00;
/// Kind byte of a decimal value. Also its stable type tag.
pub const KIND_DECIMAL: u8 = 0x01;
/// Kind byte of a binary float value. Also its stable type tag.
pub const KIND_BINARY_FLOAT: u8 = 0x02;

/// Sign byte: zero or positive.
pub const SIGN_NON_NEGATIVE: u8 = 0x00;
/// Sign byte: negative.
pub const SIGN_NEGATIVE: u8 = 0x01;

/// Default per-magnitude decode budget, in bytes (1 MiB, about 2.5
/// million decimal digits). Operational, not a domain limit.
pub const DEFAULT_MAX_MAGNITUDE_BYTES: usize = 1 << 20;

/// Base used to strip ten-factors nine digits at a time.
const BILLION: u32 = 1_000_000_000;

// ---------------------------------------------------------------------------
// BigInt
// ---------------------------------------------------------------------------

/// A signed arbitrary-precision integer.
///
/// Held as a sign and little-endian `u32` limbs with no high zero limbs;
/// zero has no limbs and is never negative, so derived equality and
/// hashing are value equality.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct BigInt {
    negative: bool,
    limbs: Vec<u32>,
}

impl BigInt {
    /// Zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            negative: false,
            limbs: Vec::new(),
        }
    }

    fn from_parts(negative: bool, mut limbs: Vec<u32>) -> Self {
        trim(&mut limbs);
        let negative = negative && !limbs.is_empty();
        Self { negative, limbs }
    }

    /// The integer equal to `value`.
    #[must_use]
    pub fn from_i128(value: i128) -> Self {
        Self::from_parts(value < 0, limbs_from_u128(value.unsigned_abs()))
    }

    /// The integer equal to `value`.
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        Self::from_i128(i128::from(value))
    }

    /// The integer equal to `value`.
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self::from_parts(false, limbs_from_u128(u128::from(value)))
    }

    /// The integer with this sign and big-endian magnitude bytes. Leading
    /// zero bytes are allowed here and ignored; a negative zero is zero.
    #[must_use]
    pub fn from_be_magnitude(negative: bool, magnitude: &[u8]) -> Self {
        let mut limbs = Vec::with_capacity(magnitude.len().div_ceil(4));
        for chunk in magnitude.rchunks(4) {
            let mut limb = 0u32;
            for &byte in chunk {
                limb = (limb << 8) | u32::from(byte);
            }
            limbs.push(limb);
        }
        Self::from_parts(negative, limbs)
    }

    /// Parses an optionally signed run of ASCII decimal digits
    /// (`-123`, `+7`, `0042`). Returns `None` for anything else,
    /// including the empty string and a bare sign.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (negative, digits) = match text.as_bytes().first() {
            Some(b'-') => (true, &text[1..]),
            Some(b'+') => (false, &text[1..]),
            _ => (false, text),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut limbs = Vec::new();
        for chunk in digits.as_bytes().chunks(9) {
            let mut value = 0u32;
            let mut scale = 1u32;
            for &digit in chunk {
                value = value * 10 + u32::from(digit - b'0');
                scale *= 10;
            }
            mul_small_add(&mut limbs, scale, value);
        }
        Some(Self::from_parts(negative, limbs))
    }

    /// Whether this is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Whether this is strictly negative.
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// The absolute value.
    #[must_use]
    pub fn abs(&self) -> Self {
        Self {
            negative: false,
            limbs: self.limbs.clone(),
        }
    }

    /// The value as an `i128`, when it fits.
    #[must_use]
    pub fn to_i128(&self) -> Option<i128> {
        if self.limbs.len() > 4 {
            return None;
        }
        let magnitude = self
            .limbs
            .iter()
            .rev()
            .fold(0u128, |acc, &limb| (acc << 32) | u128::from(limb));
        if self.negative {
            0i128.checked_sub_unsigned(magnitude)
        } else {
            i128::try_from(magnitude).ok()
        }
    }

    /// The magnitude as big-endian bytes with no leading zero byte
    /// (empty for zero).
    #[must_use]
    pub fn magnitude_be_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.limbs.len() * 4);
        for limb in self.limbs.iter().rev() {
            out.extend_from_slice(&limb.to_be_bytes());
        }
        let first = out.iter().position(|&b| b != 0).unwrap_or(out.len());
        out.drain(..first);
        out
    }

    /// `self + other`.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            return Self::from_parts(self.negative, add_mag(&self.limbs, &other.limbs));
        }
        match cmp_mag(&self.limbs, &other.limbs) {
            Ordering::Equal => Self::zero(),
            Ordering::Greater => {
                Self::from_parts(self.negative, sub_mag(&self.limbs, &other.limbs))
            }
            Ordering::Less => Self::from_parts(other.negative, sub_mag(&other.limbs, &self.limbs)),
        }
    }

    /// Number of trailing zero bits of the magnitude (0 for zero).
    fn trailing_zero_bits(&self) -> u64 {
        let mut bits = 0u64;
        for &limb in &self.limbs {
            if limb == 0 {
                bits += 32;
            } else {
                return bits + u64::from(limb.trailing_zeros());
            }
        }
        0
    }

    /// The magnitude shifted right by `bits`, keeping the sign.
    fn shr_magnitude(&self, bits: u64) -> Self {
        let whole = usize::try_from(bits / 32).unwrap_or(usize::MAX);
        if whole >= self.limbs.len() {
            return Self::zero();
        }
        let part = u32::try_from(bits % 32).unwrap_or(0);
        let source = &self.limbs[whole..];
        let limbs = if part == 0 {
            source.to_vec()
        } else {
            (0..source.len())
                .map(|i| {
                    let high = source.get(i + 1).copied().unwrap_or(0);
                    (source[i] >> part) | (high << (32 - part))
                })
                .collect()
        };
        Self::from_parts(self.negative, limbs)
    }

    /// The decimal digits of the value, with a leading `-` when negative.
    #[must_use]
    pub fn to_decimal_string(&self) -> String {
        if self.limbs.is_empty() {
            return "0".to_owned();
        }
        let mut chunks = Vec::new();
        let mut rest = self.limbs.clone();
        while !rest.is_empty() {
            let (quotient, remainder) = div_rem_small(&rest, BILLION);
            chunks.push(remainder);
            rest = quotient;
        }
        let mut out = String::with_capacity(chunks.len() * 9 + 1);
        if self.negative {
            out.push('-');
        }
        let mut iter = chunks.iter().rev();
        if let Some(first) = iter.next() {
            out.push_str(&first.to_string());
        }
        for chunk in iter {
            let _ = write!(out, "{chunk:09}");
        }
        out
    }
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_decimal_string())
    }
}

impl fmt::Debug for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BigInt({self})")
    }
}

fn trim(limbs: &mut Vec<u32>) {
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // limb arithmetic: values are masked or bounded below 2^32
fn limbs_from_u128(mut value: u128) -> Vec<u32> {
    let mut limbs = Vec::new();
    while value != 0 {
        limbs.push((value & 0xffff_ffff) as u32);
        value >>= 32;
    }
    limbs
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // limb arithmetic: values are masked or bounded below 2^32
fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for (i, &limb) in long.iter().enumerate() {
        let sum = u64::from(limb) + u64::from(short.get(i).copied().unwrap_or(0)) + carry;
        out.push((sum & 0xffff_ffff) as u32);
        carry = sum >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // limb arithmetic: values are masked or bounded below 2^32
/// `a - b` for magnitudes with `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &limb) in a.iter().enumerate() {
        let mut diff = i64::from(limb) - i64::from(b.get(i).copied().unwrap_or(0)) - borrow;
        if diff < 0 {
            diff += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out.push((diff & 0xffff_ffff) as u32);
    }
    trim(&mut out);
    out
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // limb arithmetic: values are masked or bounded below 2^32
fn div_rem_small(limbs: &[u32], divisor: u32) -> (Vec<u32>, u32) {
    let mut quotient = vec![0u32; limbs.len()];
    let mut remainder = 0u64;
    for i in (0..limbs.len()).rev() {
        let current = (remainder << 32) | u64::from(limbs[i]);
        quotient[i] = (current / u64::from(divisor)) as u32;
        remainder = current % u64::from(divisor);
    }
    trim(&mut quotient);
    (quotient, remainder as u32)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // limb arithmetic: values are masked or bounded below 2^32
fn mul_small_add(limbs: &mut Vec<u32>, factor: u32, addend: u32) {
    let mut carry = u64::from(addend);
    for limb in limbs.iter_mut() {
        let product = u64::from(*limb) * u64::from(factor) + carry;
        *limb = (product & 0xffff_ffff) as u32;
        carry = product >> 32;
    }
    if carry != 0 {
        limbs.push(carry as u32);
    }
}

/// Strips every factor of ten from a nonzero magnitude, returning the
/// stripped magnitude and how many factors were removed.
fn strip_tens(magnitude: &[u32]) -> (Vec<u32>, u64) {
    let mut rest = magnitude.to_vec();
    let mut count = 0u64;
    if rest.is_empty() {
        return (rest, 0);
    }
    loop {
        let (quotient, remainder) = div_rem_small(&rest, BILLION);
        if remainder != 0 {
            break;
        }
        rest = quotient;
        count += 9;
    }
    loop {
        let (quotient, remainder) = div_rem_small(&rest, 10);
        if remainder != 0 {
            break;
        }
        rest = quotient;
        count += 1;
    }
    (rest, count)
}

// ---------------------------------------------------------------------------
// NumericValue
// ---------------------------------------------------------------------------

/// A canonical decimal: `coefficient * 10^exponent10`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Decimal {
    coefficient: BigInt,
    exponent10: BigInt,
}

impl Decimal {
    /// The coefficient. Zero, or not divisible by ten.
    #[must_use]
    pub const fn coefficient(&self) -> &BigInt {
        &self.coefficient
    }

    /// The power of ten. Zero when the coefficient is zero.
    #[must_use]
    pub const fn exponent10(&self) -> &BigInt {
        &self.exponent10
    }
}

/// A canonical binary float: `(-1)^negative * significand * 2^exponent2`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BinaryFloat {
    negative: bool,
    significand: BigInt,
    exponent2: BigInt,
}

impl BinaryFloat {
    /// Whether the value is negative. Never true for zero.
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// The significand: non-negative, and odd unless zero.
    #[must_use]
    pub const fn significand(&self) -> &BigInt {
        &self.significand
    }

    /// The power of two. Zero when the significand is zero.
    #[must_use]
    pub const fn exponent2(&self) -> &BigInt {
        &self.exponent2
    }
}

/// A typed numeric key in canonical form. See the module docs.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum NumericValue {
    /// An unbounded integer.
    Integer(BigInt),
    /// A normalized decimal.
    Decimal(Decimal),
    /// A normalized binary float.
    BinaryFloat(BinaryFloat),
}

impl NumericValue {
    /// The integer `value`.
    #[must_use]
    pub const fn integer(value: BigInt) -> Self {
        Self::Integer(value)
    }

    /// The decimal `coefficient * 10^exponent10`, normalized: ten-factors
    /// move from the coefficient into the exponent, and zero becomes
    /// `0e0`.
    #[must_use]
    pub fn decimal(coefficient: &BigInt, exponent10: &BigInt) -> Self {
        if coefficient.is_zero() {
            return Self::Decimal(Decimal {
                coefficient: BigInt::zero(),
                exponent10: BigInt::zero(),
            });
        }
        let (stripped, count) = strip_tens(&coefficient.limbs);
        Self::Decimal(Decimal {
            coefficient: BigInt::from_parts(coefficient.negative, stripped),
            exponent10: exponent10.add(&BigInt::from_u64(count)),
        })
    }

    /// The binary float `(-1)^negative * significand * 2^exponent2`,
    /// normalized: two-factors move from the significand into the
    /// exponent, and every zero becomes `+0 * 2^0`.
    ///
    /// # Errors
    ///
    /// [`NumericError::NegativeSignificand`] when `significand` is
    /// negative; the sign is carried only by `negative`.
    pub fn binary_float(
        negative: bool,
        significand: &BigInt,
        exponent2: &BigInt,
    ) -> Result<Self, NumericError> {
        if significand.is_negative() {
            return Err(NumericError::NegativeSignificand);
        }
        if significand.is_zero() {
            return Ok(Self::BinaryFloat(BinaryFloat {
                negative: false,
                significand: BigInt::zero(),
                exponent2: BigInt::zero(),
            }));
        }
        let shift = significand.trailing_zero_bits();
        Ok(Self::BinaryFloat(BinaryFloat {
            negative,
            significand: significand.shr_magnitude(shift),
            exponent2: exponent2.add(&BigInt::from_u64(shift)),
        }))
    }

    /// The exact binary float equal to a finite `f64`. `-0.0` and `0.0`
    /// both give canonical `+0`.
    ///
    /// # Errors
    ///
    /// [`NumericError::NonFinite`] for NaN and the infinities, which the
    /// ordered numeric profile does not admit.
    pub fn from_f64(value: f64) -> Result<Self, NumericError> {
        if !value.is_finite() {
            return Err(NumericError::NonFinite);
        }
        let bits = value.to_bits();
        let negative = bits >> 63 == 1;
        let exponent_bits = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
        let fraction = bits & ((1u64 << 52) - 1);
        let (significand, exponent2) = if exponent_bits == 0 {
            (fraction, -1074i64)
        } else {
            (fraction | (1u64 << 52), exponent_bits - 1075)
        };
        Self::binary_float(
            negative,
            &BigInt::from_u64(significand),
            &BigInt::from_i64(exponent2),
        )
    }

    /// The stable kind tag: [`KIND_INTEGER`] < [`KIND_DECIMAL`] <
    /// [`KIND_BINARY_FLOAT`].
    #[must_use]
    pub const fn kind_tag(&self) -> u8 {
        match self {
            Self::Integer(_) => KIND_INTEGER,
            Self::Decimal(_) => KIND_DECIMAL,
            Self::BinaryFloat(_) => KIND_BINARY_FLOAT,
        }
    }

    /// Whether the value is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        match self {
            Self::Integer(value) => value.is_zero(),
            Self::Decimal(decimal) => decimal.coefficient.is_zero(),
            Self::BinaryFloat(float) => float.significand.is_zero(),
        }
    }

    /// The canonical payload bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![PAYLOAD_VERSION, self.kind_tag()];
        match self {
            Self::Integer(value) => put_bigint(&mut out, value),
            Self::Decimal(decimal) => {
                put_bigint(&mut out, &decimal.coefficient);
                put_bigint(&mut out, &decimal.exponent10);
            }
            Self::BinaryFloat(float) => {
                out.push(if float.negative {
                    SIGN_NEGATIVE
                } else {
                    SIGN_NON_NEGATIVE
                });
                put_bigint(&mut out, &float.significand);
                put_bigint(&mut out, &float.exponent2);
            }
        }
        out
    }

    /// Decodes canonical payload bytes under the default budget.
    ///
    /// # Errors
    ///
    /// See [`NumericValue::decode_with`].
    pub fn decode(bytes: &[u8]) -> Result<Self, NumericError> {
        Self::decode_with(bytes, NumericLimits::default())
    }

    /// Decodes canonical payload bytes under `limits`.
    ///
    /// # Errors
    ///
    /// A malformed or non-canonical payload rejects with the matching
    /// [`NumericError`]; a magnitude over `limits` rejects with
    /// [`NumericError::ResourceLimit`] before it is read.
    pub fn decode_with(bytes: &[u8], limits: NumericLimits) -> Result<Self, NumericError> {
        let mut reader = Reader { bytes, pos: 0 };
        let version = reader.u8()?;
        if version != PAYLOAD_VERSION {
            return Err(NumericError::UnknownVersion(version));
        }
        let value = match reader.u8()? {
            KIND_INTEGER => Self::Integer(reader.bigint("integer", limits)?),
            KIND_DECIMAL => {
                let coefficient = reader.bigint("coefficient", limits)?;
                let exponent10 = reader.bigint("exponent10", limits)?;
                if coefficient.is_zero() {
                    if !exponent10.is_zero() {
                        return Err(NumericError::NonCanonical(
                            "decimal zero has a nonzero exponent",
                        ));
                    }
                } else if div_rem_small(&coefficient.limbs, 10).1 == 0 {
                    return Err(NumericError::NonCanonical(
                        "decimal coefficient is divisible by ten",
                    ));
                }
                Self::Decimal(Decimal {
                    coefficient,
                    exponent10,
                })
            }
            KIND_BINARY_FLOAT => {
                let negative = match reader.u8()? {
                    SIGN_NON_NEGATIVE => false,
                    SIGN_NEGATIVE => true,
                    other => return Err(NumericError::BadSign(other)),
                };
                let significand = reader.bigint("significand", limits)?;
                if significand.is_negative() {
                    return Err(NumericError::NegativeSignificand);
                }
                let exponent2 = reader.bigint("exponent2", limits)?;
                if significand.is_zero() {
                    if negative {
                        return Err(NumericError::NegativeZero);
                    }
                    if !exponent2.is_zero() {
                        return Err(NumericError::NonCanonical(
                            "binary float zero has a nonzero exponent",
                        ));
                    }
                } else if significand.limbs[0] & 1 == 0 {
                    return Err(NumericError::NonCanonical(
                        "binary float significand is even",
                    ));
                }
                Self::BinaryFloat(BinaryFloat {
                    negative,
                    significand,
                    exponent2,
                })
            }
            other => return Err(NumericError::UnknownKind(other)),
        };
        if reader.pos != bytes.len() {
            return Err(NumericError::TrailingBytes);
        }
        Ok(value)
    }
}

fn put_bigint(out: &mut Vec<u8>, value: &BigInt) {
    out.push(if value.negative {
        SIGN_NEGATIVE
    } else {
        SIGN_NON_NEGATIVE
    });
    let magnitude = value.magnitude_be_bytes();
    out.extend_from_slice(&(magnitude.len() as u64).to_be_bytes());
    out.extend_from_slice(&magnitude);
}

/// Operational decode budget. Never a semantic range rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NumericLimits {
    /// Largest magnitude, in bytes, that one integer, coefficient,
    /// significand or exponent may carry.
    pub max_magnitude_bytes: usize,
}

impl Default for NumericLimits {
    fn default() -> Self {
        Self {
            max_magnitude_bytes: DEFAULT_MAX_MAGNITUDE_BYTES,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], NumericError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(NumericError::Truncated)?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, NumericError> {
        Ok(self.take(1)?[0])
    }

    fn u64(&mut self) -> Result<u64, NumericError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(raw))
    }

    fn bigint(
        &mut self,
        field: &'static str,
        limits: NumericLimits,
    ) -> Result<BigInt, NumericError> {
        let negative = match self.u8()? {
            SIGN_NON_NEGATIVE => false,
            SIGN_NEGATIVE => true,
            other => return Err(NumericError::BadSign(other)),
        };
        let declared = self.u64()?;
        let len = usize::try_from(declared)
            .ok()
            .filter(|&len| len <= limits.max_magnitude_bytes)
            .ok_or(NumericError::ResourceLimit {
                field,
                len: declared,
                limit: limits.max_magnitude_bytes,
            })?;
        let magnitude = self.take(len)?;
        if magnitude.first() == Some(&0) {
            return Err(NumericError::LeadingZero(field));
        }
        if negative && magnitude.is_empty() {
            return Err(NumericError::NegativeZero);
        }
        Ok(BigInt::from_be_magnitude(negative, magnitude))
    }
}

/// Why a numeric value was rejected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NumericError {
    /// The payload ended early.
    Truncated,
    /// Bytes remain after a complete value.
    TrailingBytes,
    /// The payload version byte is not [`PAYLOAD_VERSION`].
    UnknownVersion(u8),
    /// The kind byte names no numeric kind.
    UnknownKind(u8),
    /// A sign byte is neither 0x00 nor 0x01.
    BadSign(u8),
    /// A zero carries a negative sign.
    NegativeZero,
    /// A magnitude starts with a zero byte.
    LeadingZero(&'static str),
    /// A binary float significand is negative.
    NegativeSignificand,
    /// A value is not in its normalized form.
    NonCanonical(&'static str),
    /// NaN or an infinity was offered to the ordered profile.
    NonFinite,
    /// A magnitude exceeds the configured decode budget.
    ResourceLimit {
        /// Which field.
        field: &'static str,
        /// Its declared length in bytes.
        len: u64,
        /// The configured limit in bytes.
        limit: usize,
    },
}

impl NumericError {
    /// The design's error-envelope code: `resource_limit` for a budget
    /// overrun, `malformed` for everything else.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ResourceLimit { .. } => "resource_limit",
            _ => "malformed",
        }
    }
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("numeric payload is truncated"),
            Self::TrailingBytes => f.write_str("numeric payload has trailing bytes"),
            Self::UnknownVersion(v) => write!(f, "unknown numeric payload version {v:#04x}"),
            Self::UnknownKind(k) => write!(f, "unknown numeric kind {k:#04x}"),
            Self::BadSign(s) => write!(f, "bad sign byte {s:#04x}"),
            Self::NegativeZero => f.write_str("negative zero is not canonical"),
            Self::LeadingZero(field) => write!(f, "{field} magnitude has a leading zero byte"),
            Self::NegativeSignificand => f.write_str("binary float significand is negative"),
            Self::NonCanonical(why) => write!(f, "non-canonical numeric value: {why}"),
            Self::NonFinite => f.write_str("NaN and infinities are not ordered numeric values"),
            Self::ResourceLimit { field, len, limit } => {
                write!(
                    f,
                    "{field} magnitude of {len} bytes exceeds the {limit}-byte budget"
                )
            }
        }
    }
}

impl std::error::Error for NumericError {}
