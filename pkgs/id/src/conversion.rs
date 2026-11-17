//! Production conversion pure layer: quote arithmetic and the
//! linked-batch builder (design #2063, Phase E of the phased plan in
//! `doc/2026-11-01_design_id2063.md` §4).
//!
//! This module is the non-PoC counterpart of
//! [`crate::currency_poc::executor`]'s quote and leg construction. It
//! has no TigerBeetle, database, or other side effects: given the
//! caller's fixed transfer IDs and the terms of a
//! [`crate::currency::RateAuthorisation`], it computes the converted
//! amount and fee and builds the single linked chain the gateway
//! submits. The durable executor/recovery layer (Phase F) persists the
//! intent first and submits exactly the batch built here.
//!
//! Contract points from #2063 §2–§3 that this layer owns:
//!
//! - Rates are exact positive integer rationals, never floating
//!   point. For rate `rate_num / rate_den` in destination major units
//!   per source major unit, source raw amount `x`, and ledger scales
//!   `sX` / `sY`: `y = floor(x * rate_num * 10^sY / (rate_den *
//!   10^sX))`, computed with a 256-bit intermediate so a result that
//!   fits `u128` is never lost to intermediate overflow (the PoC's
//!   `checked_mul` quote returns `None` for such inputs). Overflow of
//!   the *result* is a typed error, never a saturation.
//! - A zero destination amount after rounding is rejected
//!   ([`ConvertError::ZeroOutput`]), not treated as a free conversion.
//!   The rounding remainder stays with the LP.
//! - The fee is `floor(source_principal * fee_bps / 10_000)` in
//!   source units, submitted as a separate, separately audited
//!   source-ledger leg — never hidden in the rate.
//! - Every leg carries the same opaque 128-bit `conversion_id` in
//!   `user_data_128`, a distinct caller-supplied transfer ID, and a
//!   role-specific transfer code from the conversion range reserved
//!   by the #2062 control-plane design §4.4 (`0x0200–0x02FF`). #2063
//!   §2.1 places all three role codes in that range; the generic fee
//!   range (`0x0300–0x03FF`) is for standalone fees, not conversion
//!   legs.
//! - One conversion is one ordered linked chain: source leg, optional
//!   fee leg, destination leg. Every leg but the last is `linked`.

use crate::currency_poc::ledger::{AccountId, Transfer, TransferId};

/// Transfer code for the source leg (`U_X -> P_X`) of a conversion.
pub const CODE_CONVERSION_SOURCE: u16 = 0x0201;
/// Transfer code for the optional fee leg (`U_X -> fee_X`).
pub const CODE_CONVERSION_FEE: u16 = 0x0202;
/// Transfer code for the destination leg (`P_Y -> U_Y`).
pub const CODE_CONVERSION_DESTINATION: u16 = 0x0203;

/// Largest supported ledger scale (decimal digits). `10^38` is the
/// largest power of ten that fits a `u128`, so scale normalisation
/// stays exact across the full input range.
pub const MAX_SCALE: u32 = 38;

const BPS_DENOMINATOR: u128 = 10_000;
const MAX_FEE_BPS: u32 = 10_000;

/// Why a conversion quote or batch cannot be built. Every variant is
/// a rejection *before* any ledger write, per the failure table in
/// #2063 §2.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// The source amount is zero.
    ZeroAmount,
    /// The rate denominator is zero.
    ZeroRateDenominator,
    /// A ledger scale exceeds [`MAX_SCALE`].
    InvalidScale {
        /// The offending scale.
        scale: u32,
    },
    /// `fee_bps` exceeds 10_000 (a fee above the whole principal).
    InvalidFeeBps {
        /// The offending fee, in basis points.
        fee_bps: u32,
    },
    /// The source amount is below the authorisation's minimum.
    BelowMin {
        /// The requested source amount.
        amount: u128,
        /// The authorisation's `min_amount`.
        min: u128,
    },
    /// The source amount exceeds the authorisation's maximum.
    AboveMax {
        /// The requested source amount.
        amount: u128,
        /// The authorisation's `max_amount`.
        max: u128,
    },
    /// The converted amount rounds to zero at the destination scale.
    ZeroOutput,
    /// The true result does not fit a `u128`.
    Overflow,
    /// A leg would debit and credit the same account.
    SameAccount {
        /// Which leg: `"source"`, `"fee"`, or `"destination"`.
        leg: &'static str,
    },
    /// A positive fee was computed but no fee account was supplied.
    MissingFeeAccount,
    /// The caller supplied the wrong number of transfer IDs.
    TransferIdCount {
        /// Legs the batch needs (2 without a fee leg, 3 with).
        expected: usize,
        /// IDs actually supplied.
        got: usize,
    },
    /// Two legs would share one transfer ID; IDs are chosen once per
    /// leg and must be distinct.
    DuplicateTransferId {
        /// The repeated transfer ID.
        id: u128,
    },
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroAmount => write!(f, "source amount must not be zero"),
            Self::ZeroRateDenominator => write!(f, "rate denominator must not be zero"),
            Self::InvalidScale { scale } => {
                write!(f, "ledger scale {scale} exceeds maximum {MAX_SCALE}")
            }
            Self::InvalidFeeBps { fee_bps } => {
                write!(f, "fee_bps {fee_bps} exceeds {MAX_FEE_BPS}")
            }
            Self::BelowMin { amount, min } => {
                write!(f, "source amount {amount} is below the minimum {min}")
            }
            Self::AboveMax { amount, max } => {
                write!(f, "source amount {amount} exceeds the maximum {max}")
            }
            Self::ZeroOutput => {
                write!(
                    f,
                    "converted amount rounds to zero at the destination scale"
                )
            }
            Self::Overflow => write!(f, "conversion result overflows u128"),
            Self::SameAccount { leg } => {
                write!(f, "{leg} leg debits and credits the same account")
            }
            Self::MissingFeeAccount => {
                write!(f, "a positive fee requires a fee account")
            }
            Self::TransferIdCount { expected, got } => {
                write!(f, "batch needs {expected} transfer IDs, got {got}")
            }
            Self::DuplicateTransferId { id } => {
                write!(f, "transfer ID {id} is used by more than one leg")
            }
        }
    }
}

impl std::error::Error for ConvertError {}

/// A 256-bit unsigned intermediate for exact rational arithmetic.
/// Only the operations the conversion formula needs: full
/// `u128 * u128` products, a checked multiply by a `u128` factor, and
/// division. Not a general bignum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct U256 {
    hi: u128,
    lo: u128,
}

impl U256 {
    const ZERO: Self = Self { hi: 0, lo: 0 };

    fn from_u128(value: u128) -> Self {
        Self { hi: 0, lo: value }
    }

    /// The full product of two `u128`s; always fits 256 bits.
    fn mul_u128(a: u128, b: u128) -> Self {
        let a_lo = a & u128::from(u64::MAX);
        let a_hi = a >> 64;
        let b_lo = b & u128::from(u64::MAX);
        let b_hi = b >> 64;
        let p0 = a_lo * b_lo;
        let p1 = a_lo * b_hi;
        let p2 = a_hi * b_lo;
        let p3 = a_hi * b_hi;
        let (lo, carry1) = p0.overflowing_add(p1 << 64);
        let (lo, carry2) = lo.overflowing_add(p2 << 64);
        let hi = p3 + (p1 >> 64) + (p2 >> 64) + u128::from(carry1) + u128::from(carry2);
        Self { hi, lo }
    }

    /// `self * factor`, or `None` when the product needs more than
    /// 256 bits.
    fn mul_factor(self, factor: u128) -> Option<Self> {
        if factor == 0 {
            return Some(Self::ZERO);
        }
        let lo_product = Self::mul_u128(self.lo, factor);
        let hi = self.hi.checked_mul(factor)?.checked_add(lo_product.hi)?;
        Some(Self {
            hi,
            lo: lo_product.lo,
        })
    }

    fn bit_length(self) -> u32 {
        if self.hi != 0 {
            256 - self.hi.leading_zeros()
        } else {
            128 - self.lo.leading_zeros()
        }
    }

    /// `self << shift`; the caller guarantees the result fits (the
    /// shift is chosen so the top set bit stays in range).
    fn shl(self, shift: u32) -> Self {
        if shift == 0 {
            self
        } else if shift < 128 {
            Self {
                hi: (self.hi << shift) | (self.lo >> (128 - shift)),
                lo: self.lo << shift,
            }
        } else {
            Self {
                hi: self.lo << (shift - 128),
                lo: 0,
            }
        }
    }

    fn shr1(self) -> Self {
        Self {
            hi: self.hi >> 1,
            lo: (self.lo >> 1) | (self.hi << 127),
        }
    }

    /// `self - other`; the caller guarantees `self >= other`.
    fn sub(self, other: Self) -> Self {
        let (lo, borrow) = self.lo.overflowing_sub(other.lo);
        Self {
            hi: self.hi - other.hi - u128::from(borrow),
            lo,
        }
    }

    fn set_bit(&mut self, index: u32) {
        if index < 128 {
            self.lo |= 1u128 << index;
        } else {
            self.hi |= 1u128 << (index - 128);
        }
    }

    fn to_u128(self) -> Option<u128> {
        if self.hi == 0 { Some(self.lo) } else { None }
    }

    /// `(quotient, remainder)` of `self / divisor`; `divisor` must be
    /// non-zero. Shift-and-subtract long division: the shifted
    /// divisor never exceeds `self`, so no intermediate needs more
    /// than 256 bits.
    fn divmod(self, divisor: Self) -> (Self, Self) {
        debug_assert!(divisor != Self::ZERO);
        if self < divisor {
            return (Self::ZERO, self);
        }
        let shift = self.bit_length() - divisor.bit_length();
        let mut shifted = divisor.shl(shift);
        let mut quotient = Self::ZERO;
        let mut remainder = self;
        for index in (0..=shift).rev() {
            if remainder >= shifted {
                remainder = remainder.sub(shifted);
                quotient.set_bit(index);
            }
            shifted = shifted.shr1();
        }
        (quotient, remainder)
    }
}

fn pow10(exp: u32) -> u128 {
    debug_assert!(exp <= MAX_SCALE);
    10u128.pow(exp)
}

/// The converted destination amount for one conversion, per the
/// formula in the module docs: exact rational arithmetic, floored,
/// with the rounding remainder staying with the LP.
///
/// # Errors
///
/// [`ConvertError::ZeroRateDenominator`] for a zero rate denominator,
/// [`ConvertError::InvalidScale`] for a scale above [`MAX_SCALE`],
/// [`ConvertError::Overflow`] when the true result does not fit a
/// `u128`, and [`ConvertError::ZeroOutput`] when the result rounds
/// to zero.
pub fn convert_amount(
    amount_in: u128,
    rate_num: u128,
    rate_den: u128,
    source_scale: u32,
    dest_scale: u32,
) -> Result<u128, ConvertError> {
    if rate_den == 0 {
        return Err(ConvertError::ZeroRateDenominator);
    }
    if source_scale > MAX_SCALE {
        return Err(ConvertError::InvalidScale {
            scale: source_scale,
        });
    }
    if dest_scale > MAX_SCALE {
        return Err(ConvertError::InvalidScale { scale: dest_scale });
    }
    let mut numerator = U256::mul_u128(amount_in, rate_num);
    let denominator;
    if dest_scale >= source_scale {
        // Fold the scale factor into the numerator. If that product
        // overflows 256 bits, the true quotient still exceeds
        // u128::MAX (the denominator is at most u128::MAX), so
        // Overflow is the exact verdict, not an approximation.
        numerator = numerator
            .mul_factor(pow10(dest_scale - source_scale))
            .ok_or(ConvertError::Overflow)?;
        denominator = U256::from_u128(rate_den);
    } else {
        // The denominator product of two u128s always fits 256 bits.
        denominator = U256::mul_u128(rate_den, pow10(source_scale - dest_scale));
    }
    let (quotient, _) = numerator.divmod(denominator);
    let amount_out = quotient.to_u128().ok_or(ConvertError::Overflow)?;
    if amount_out == 0 {
        return Err(ConvertError::ZeroOutput);
    }
    Ok(amount_out)
}

/// The fee for one conversion: `floor(principal * fee_bps / 10_000)`
/// in source units (#2063 §2.2). Exact across the full `u128`
/// principal range.
///
/// # Errors
///
/// [`ConvertError::InvalidFeeBps`] when `fee_bps` exceeds 10_000.
pub fn fee_amount(principal: u128, fee_bps: u32) -> Result<u128, ConvertError> {
    if fee_bps > MAX_FEE_BPS {
        return Err(ConvertError::InvalidFeeBps { fee_bps });
    }
    let numerator = U256::mul_u128(principal, u128::from(fee_bps));
    let (quotient, _) = numerator.divmod(U256::from_u128(BPS_DENOMINATOR));
    // fee_bps <= 10_000, so the fee never exceeds the principal and
    // always fits; the error arm is unreachable but typed.
    quotient.to_u128().ok_or(ConvertError::Overflow)
}

/// Everything needed to build one conversion batch. Accounts and
/// ledger IDs are the TigerBeetle-shaped values the gateway resolved
/// at admission (#2063 §2.2); rate, fee, min, and max terms come from
/// the [`crate::currency::RateAuthorisation`] the quote is pinned
/// to. Transfer IDs are generated and persisted by the caller before
/// submission and are never regenerated here.
#[derive(Clone, Debug)]
pub struct ConversionBatchRequest {
    /// The opaque conversion ID stamped on every leg.
    pub conversion_id: u128,
    /// One pre-generated transfer ID per leg, in leg order (source,
    /// fee if any, destination).
    pub transfer_ids: Vec<TransferId>,
    /// The caller's source-ledger account (`U_X`).
    pub caller_source: AccountId,
    /// The LP's source-ledger account (`P_X`).
    pub lp_source: AccountId,
    /// The source-ledger fee account (`fee_X`); required when the
    /// computed fee is positive.
    pub fee_account: Option<AccountId>,
    /// The LP's destination-ledger account (`P_Y`).
    pub lp_dest: AccountId,
    /// The caller's destination-ledger account (`U_Y`).
    pub caller_dest: AccountId,
    /// TigerBeetle ledger ID of the source currency.
    pub source_ledger: u32,
    /// TigerBeetle ledger ID of the destination currency.
    pub dest_ledger: u32,
    /// Source amount in source-ledger raw units.
    pub amount_in: u128,
    /// Rate numerator from the authorisation.
    pub rate_num: u128,
    /// Rate denominator from the authorisation.
    pub rate_den: u128,
    /// Decimal scale of the source ledger.
    pub source_scale: u32,
    /// Decimal scale of the destination ledger.
    pub dest_scale: u32,
    /// Fee in basis points from the authorisation, if any.
    pub fee_bps: Option<u32>,
    /// Per-conversion minimum, in source raw units.
    pub min_amount: u128,
    /// Per-conversion maximum, in source raw units.
    pub max_amount: u128,
}

/// The amounts a built batch commits to, as quoted to the caller
/// before signing (#2063 §2.1): principal, fee, destination amount,
/// and their total source debit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversionAmounts {
    /// Source principal, in source raw units.
    pub amount_in: u128,
    /// Source fee, in source raw units (0 when no fee leg is built).
    pub fee: u128,
    /// Destination amount, in destination raw units.
    pub amount_out: u128,
    /// `amount_in + fee`: what the caller's source account is
    /// debited in total.
    pub total_source_debit: u128,
}

/// Builds the one linked chain for a conversion, or rejects the
/// request with the typed reason. Pure: no ledger or store access.
///
/// Validation order is fixed so callers see deterministic errors:
/// amount bounds, rate/scale/fee terms, account shape, fee-account
/// presence, then transfer-ID shape.
///
/// # Errors
///
/// Any [`ConvertError`] variant; see the variant docs.
pub fn build_conversion_batch(
    request: &ConversionBatchRequest,
) -> Result<(ConversionAmounts, Vec<Transfer>), ConvertError> {
    if request.amount_in == 0 {
        return Err(ConvertError::ZeroAmount);
    }
    if request.amount_in < request.min_amount {
        return Err(ConvertError::BelowMin {
            amount: request.amount_in,
            min: request.min_amount,
        });
    }
    if request.amount_in > request.max_amount {
        return Err(ConvertError::AboveMax {
            amount: request.amount_in,
            max: request.max_amount,
        });
    }
    let amount_out = convert_amount(
        request.amount_in,
        request.rate_num,
        request.rate_den,
        request.source_scale,
        request.dest_scale,
    )?;
    let fee = match request.fee_bps {
        Some(fee_bps) => fee_amount(request.amount_in, fee_bps)?,
        None => 0,
    };
    if request.caller_source == request.lp_source {
        return Err(ConvertError::SameAccount { leg: "source" });
    }
    if request.lp_dest == request.caller_dest {
        return Err(ConvertError::SameAccount { leg: "destination" });
    }
    if fee > 0 {
        match request.fee_account {
            None => return Err(ConvertError::MissingFeeAccount),
            Some(fee_account) if fee_account == request.caller_source => {
                return Err(ConvertError::SameAccount { leg: "fee" });
            }
            Some(_) => {}
        }
    }
    let leg_count = if fee > 0 { 3 } else { 2 };
    if request.transfer_ids.len() != leg_count {
        return Err(ConvertError::TransferIdCount {
            expected: leg_count,
            got: request.transfer_ids.len(),
        });
    }
    let mut seen = std::collections::HashSet::with_capacity(leg_count);
    for id in &request.transfer_ids {
        if !seen.insert(id) {
            return Err(ConvertError::DuplicateTransferId { id: *id });
        }
    }
    let total_source_debit = request
        .amount_in
        .checked_add(fee)
        .ok_or(ConvertError::Overflow)?;
    let mut ids = request.transfer_ids.iter();
    // The count check above guarantees one ID per leg built below.
    let mut next_id = move || ids.next().copied().unwrap_or(0);
    let mut legs = Vec::with_capacity(leg_count);
    legs.push(Transfer {
        id: next_id(),
        debit_account_id: request.caller_source,
        credit_account_id: request.lp_source,
        amount: request.amount_in,
        ledger: request.source_ledger,
        user_data_128: request.conversion_id,
        code: CODE_CONVERSION_SOURCE,
        linked: true,
    });
    if fee > 0 {
        if let Some(fee_account) = request.fee_account {
            legs.push(Transfer {
                id: next_id(),
                debit_account_id: request.caller_source,
                credit_account_id: fee_account,
                amount: fee,
                ledger: request.source_ledger,
                user_data_128: request.conversion_id,
                code: CODE_CONVERSION_FEE,
                linked: true,
            });
        }
    }
    legs.push(Transfer {
        id: next_id(),
        debit_account_id: request.lp_dest,
        credit_account_id: request.caller_dest,
        amount: amount_out,
        ledger: request.dest_ledger,
        user_data_128: request.conversion_id,
        code: CODE_CONVERSION_DESTINATION,
        linked: false,
    });
    let amounts = ConversionAmounts {
        amount_in: request.amount_in,
        fee,
        amount_out,
        total_source_debit,
    };
    Ok((amounts, legs))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The naive `u128` formula: the test oracle. On the bounded
    /// domains the properties below generate, its intermediate
    /// products cannot overflow, so it is exact — and it shares no
    /// code with the 256-bit implementation under test.
    fn naive_convert(
        amount_in: u128,
        rate_num: u128,
        rate_den: u128,
        source_scale: u32,
        dest_scale: u32,
    ) -> u128 {
        let numerator = amount_in * rate_num * 10u128.pow(dest_scale);
        let denominator = rate_den * 10u128.pow(source_scale);
        numerator / denominator
    }

    /// A test-only 512-bit unsigned integer in base 2^32: the wide
    /// oracle's arithmetic. It shares no code with the `U256` under
    /// test \u2014 different limb base (2^32, not 2^64 halves), a plain
    /// schoolbook multiply, and no division at all: the wide
    /// properties below verify the *floor characterisation* of the
    /// quotient (`den * out <= num < den * (out + 1)`) in exact
    /// arithmetic, which determines the quotient uniquely. 512 bits
    /// hold every product the full domain can form: a `u128 * u128`
    /// product folded by `10^38` stays below 2^383, and `den * (out
    /// + 1)` below 2^384.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Wide([u32; 16]);

    impl Wide {
        fn from_u128(value: u128) -> Self {
            let mut limbs = [0u32; 16];
            for (i, limb) in limbs.iter_mut().take(4).enumerate() {
                *limb = (value >> (32 * i)) as u32;
            }
            Self(limbs)
        }

        fn add_one(self) -> Self {
            let mut limbs = self.0;
            for limb in &mut limbs {
                let (next, carry) = limb.overflowing_add(1);
                *limb = next;
                if !carry {
                    break;
                }
            }
            Self(limbs)
        }

        /// `self * 2^128` (four whole limbs).
        fn shl128(self) -> Self {
            let mut limbs = [0u32; 16];
            limbs[4..].copy_from_slice(&self.0[..12]);
            Self(limbs)
        }

        /// Schoolbook multiplication in base 2^32, one output limb at
        /// a time with a `u128` accumulator (at most 16 products of
        /// two `u32`s plus a carry \u2014 far inside `u128`). Callers
        /// keep every product below 2^512 (see the type docs), so
        /// the carry out of the top limb is always zero.
        fn mul(self, other: Self) -> Self {
            let mut out = [0u32; 16];
            let mut carry = 0u128;
            for k in 0..16 {
                let mut sum = carry;
                for i in 0..=k {
                    sum += u128::from(self.0[i]) * u128::from(other.0[k - i]);
                }
                out[k] = sum as u32;
                carry = sum >> 32;
            }
            debug_assert_eq!(carry, 0, "wide product exceeds 512 bits");
            Self(out)
        }
    }

    impl PartialOrd for Wide {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Wide {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            for i in (0..16).rev() {
                match self.0[i].cmp(&other.0[i]) {
                    std::cmp::Ordering::Equal => {}
                    ord => return ord,
                }
            }
            std::cmp::Ordering::Equal
        }
    }

    /// The exact terms of the documented formula `y = floor(amount *
    /// rate_num * 10^dest_scale / (rate_den * 10^source_scale))` as
    /// `Wide` values: `(numerator, denominator)`. The oracle for the
    /// wide-domain properties \u2014 exact across the whole `u128`
    /// range, where the naive `u128` formula above would overflow.
    fn wide_terms(
        amount_in: u128,
        rate_num: u128,
        rate_den: u128,
        source_scale: u32,
        dest_scale: u32,
    ) -> (Wide, Wide) {
        let numerator = Wide::from_u128(amount_in)
            .mul(Wide::from_u128(rate_num))
            .mul(Wide::from_u128(10u128.pow(dest_scale)));
        let denominator = Wide::from_u128(rate_den).mul(Wide::from_u128(10u128.pow(source_scale)));
        (numerator, denominator)
    }

    /// Full-range `u128` strategy for the wide-domain properties:
    /// arbitrary values, powers of two and their neighbours, small
    /// values (zero included), and the top of the range.
    fn wide_u128() -> impl Strategy<Value = u128> {
        prop_oneof![
            any::<u128>(),
            (0u32..128).prop_map(|k| 1u128 << k),
            (0u32..128).prop_map(|k| (1u128 << k) - 1),
            (0u32..128).prop_map(|k| (1u128 << k) + 1),
            0u128..1_000,
            (0u128..1_000).prop_map(|s| u128::MAX - s),
        ]
    }

    #[test]
    fn converts_at_par_with_equal_scales() {
        assert_eq!(convert_amount(1_000, 3, 2, 0, 0), Ok(1_500));
        assert_eq!(convert_amount(1_000, 1, 3, 0, 0), Ok(333));
    }

    #[test]
    fn scale_normalisation_moves_the_decimal_point() {
        // 1.5 source units at scale 6, rate 2/1, into scale 2:
        // 1_500_000 * 2 * 10^2 / 10^6 = 300 (3.00 destination units).
        assert_eq!(convert_amount(1_500_000, 2, 1, 6, 2), Ok(300));
        // Same value the other way: scale 2 -> scale 6.
        assert_eq!(convert_amount(300, 1, 2, 2, 6), Ok(1_500_000));
    }

    #[test]
    fn zero_output_is_rejected_not_free() {
        // 1 raw unit at rate 1/1000 rounds to zero.
        assert_eq!(
            convert_amount(1, 1, 1_000, 0, 0),
            Err(ConvertError::ZeroOutput)
        );
        assert_eq!(convert_amount(0, 3, 2, 0, 0), Err(ConvertError::ZeroOutput));
    }

    #[test]
    fn malformed_terms_are_typed_errors() {
        assert_eq!(
            convert_amount(10, 1, 0, 0, 0),
            Err(ConvertError::ZeroRateDenominator)
        );
        assert_eq!(
            convert_amount(10, 1, 1, MAX_SCALE + 1, 0),
            Err(ConvertError::InvalidScale {
                scale: MAX_SCALE + 1
            })
        );
        assert_eq!(
            convert_amount(10, 1, 1, 0, MAX_SCALE + 1),
            Err(ConvertError::InvalidScale {
                scale: MAX_SCALE + 1
            })
        );
    }

    #[test]
    fn wide_intermediate_recovers_results_the_poc_quote_loses() {
        // amount * rate_num = 2^140 overflows u128, so a checked_mul
        // quote returns None; the true result 2^120 fits exactly.
        let amount = 1u128 << 100;
        let result = convert_amount(amount, 1u128 << 40, 1u128 << 20, 0, 0);
        assert_eq!(result, Ok(1u128 << 120));
    }

    #[test]
    fn true_overflow_is_an_error_not_a_saturation() {
        // True result is 2^140: does not fit u128.
        assert_eq!(
            convert_amount(1u128 << 100, 1u128 << 40, 1, 0, 0),
            Err(ConvertError::Overflow)
        );
        // Numerator overflows even 256 bits once the scale factor is
        // folded in; the denominator is 1, so the quotient overflows
        // too and Overflow stays the exact verdict.
        assert_eq!(
            convert_amount(u128::MAX, u128::MAX, 1, 0, MAX_SCALE),
            Err(ConvertError::Overflow)
        );
    }

    #[test]
    fn fee_is_floored_basis_points_of_principal() {
        assert_eq!(fee_amount(10_000, 25), Ok(25));
        assert_eq!(fee_amount(999, 25), Ok(2));
        assert_eq!(fee_amount(1_000, 0), Ok(0));
        assert_eq!(fee_amount(u128::MAX, 10_000), Ok(u128::MAX));
        assert_eq!(
            fee_amount(1_000, 10_001),
            Err(ConvertError::InvalidFeeBps { fee_bps: 10_001 })
        );
    }

    fn request() -> ConversionBatchRequest {
        ConversionBatchRequest {
            conversion_id: 42,
            transfer_ids: vec![11, 22],
            caller_source: 1,
            lp_source: 2,
            fee_account: None,
            lp_dest: 3,
            caller_dest: 4,
            source_ledger: 100,
            dest_ledger: 200,
            amount_in: 1_000,
            rate_num: 3,
            rate_den: 2,
            source_scale: 0,
            dest_scale: 0,
            fee_bps: None,
            min_amount: 10,
            max_amount: 10_000,
        }
    }

    #[test]
    fn builds_a_two_leg_linked_chain_without_a_fee() {
        let (amounts, legs) = build_conversion_batch(&request()).expect("valid request");
        assert_eq!(
            amounts,
            ConversionAmounts {
                amount_in: 1_000,
                fee: 0,
                amount_out: 1_500,
                total_source_debit: 1_000,
            }
        );
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0].id, 11);
        assert_eq!(legs[0].debit_account_id, 1);
        assert_eq!(legs[0].credit_account_id, 2);
        assert_eq!(legs[0].amount, 1_000);
        assert_eq!(legs[0].ledger, 100);
        assert_eq!(legs[0].code, CODE_CONVERSION_SOURCE);
        assert!(legs[0].linked);
        assert_eq!(legs[1].id, 22);
        assert_eq!(legs[1].debit_account_id, 3);
        assert_eq!(legs[1].credit_account_id, 4);
        assert_eq!(legs[1].amount, 1_500);
        assert_eq!(legs[1].ledger, 200);
        assert_eq!(legs[1].code, CODE_CONVERSION_DESTINATION);
        assert!(!legs[1].linked);
        assert!(legs.iter().all(|leg| leg.user_data_128 == 42));
    }

    #[test]
    fn builds_a_three_leg_chain_with_a_separate_fee_leg() {
        let mut req = request();
        req.transfer_ids = vec![11, 22, 33];
        req.fee_bps = Some(50); // 0.5% of 1_000 = 5
        req.fee_account = Some(9);
        let (amounts, legs) = build_conversion_batch(&req).expect("valid request");
        assert_eq!(amounts.fee, 5);
        assert_eq!(amounts.total_source_debit, 1_005);
        assert_eq!(legs.len(), 3);
        assert_eq!(legs[1].id, 22);
        assert_eq!(legs[1].debit_account_id, 1);
        assert_eq!(legs[1].credit_account_id, 9);
        assert_eq!(legs[1].amount, 5);
        assert_eq!(legs[1].ledger, 100);
        assert_eq!(legs[1].code, CODE_CONVERSION_FEE);
        assert!(legs[1].linked);
        assert_eq!(legs[2].id, 33);
        assert!(!legs[2].linked);
    }

    #[test]
    fn batch_validation_errors_are_typed() {
        let mut req = request();
        req.amount_in = 0;
        assert_eq!(build_conversion_batch(&req), Err(ConvertError::ZeroAmount));

        let mut req = request();
        req.amount_in = 5;
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::BelowMin { amount: 5, min: 10 })
        );

        let mut req = request();
        req.amount_in = 20_000;
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::AboveMax {
                amount: 20_000,
                max: 10_000
            })
        );

        let mut req = request();
        req.lp_source = req.caller_source;
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::SameAccount { leg: "source" })
        );

        let mut req = request();
        req.transfer_ids = vec![11];
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::TransferIdCount {
                expected: 2,
                got: 1
            })
        );

        let mut req = request();
        req.transfer_ids = vec![11, 11];
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::DuplicateTransferId { id: 11 })
        );

        let mut req = request();
        req.fee_bps = Some(50);
        assert_eq!(
            build_conversion_batch(&req),
            Err(ConvertError::MissingFeeAccount)
        );
    }

    proptest! {
        /// Oracle: the naive u128 formula (see `naive_convert`). On
        /// this domain — amounts < 2^64, rate terms < 2^32, scales
        /// <= 9 — its products stay below 2^126, so it is exact.
        #[test]
        fn convert_amount_matches_naive_oracle_on_bounded_domain(
            amount in 0u128..(1u128 << 64),
            rate_num in 1u128..(1u128 << 32),
            rate_den in 1u128..(1u128 << 32),
            source_scale in 0u32..=9,
            dest_scale in 0u32..=9,
        ) {
            let expected = naive_convert(amount, rate_num, rate_den, source_scale, dest_scale);
            let result = convert_amount(amount, rate_num, rate_den, source_scale, dest_scale);
            if expected == 0 {
                prop_assert_eq!(result, Err(ConvertError::ZeroOutput));
            } else {
                prop_assert_eq!(result, Ok(expected));
            }
        }

        /// Invariant oracle: the result is the unique value with
        /// `out * den * 10^ss <= amount * num * 10^ds < (out+1) * den
        /// * 10^ss`. On this domain every product fits u128, so the
        /// check itself is exact and shares no code with the
        /// implementation.
        #[test]
        fn convert_amount_satisfies_the_floor_invariant(
            amount in 1u128..(1u128 << 50),
            rate_num in 1u128..(1u128 << 50),
            rate_den in 1u128..(1u128 << 50),
            source_scale in 0u32..=6,
            dest_scale in 0u32..=6,
        ) {
            let Ok(out) = convert_amount(amount, rate_num, rate_den, source_scale, dest_scale)
            else {
                // ZeroOutput: the invariant's left side must fail
                // for out = 1 already, i.e. the true value is < 1.
                let numerator = amount * rate_num * 10u128.pow(dest_scale);
                let denominator = rate_den * 10u128.pow(source_scale);
                prop_assert!(numerator < denominator);
                return Ok(());
            };
            let numerator = amount * rate_num * 10u128.pow(dest_scale);
            let denominator = rate_den * 10u128.pow(source_scale);
            prop_assert!(out * denominator <= numerator);
            prop_assert!(numerator < (out + 1) * denominator);
        }

        /// Fee oracle: the same naive-formula arrangement as
        /// `convert_amount_matches_naive_oracle_on_bounded_domain`.
        #[test]
        fn fee_amount_matches_naive_oracle(
            principal in 0u128..(1u128 << 100),
            fee_bps in 0u32..=10_000,
        ) {
            let expected = principal * u128::from(fee_bps) / 10_000;
            prop_assert_eq!(fee_amount(principal, fee_bps), Ok(expected));
        }

        /// Wide oracle: the floor characterisation, checked in exact
        /// `Wide` arithmetic (see `wide_terms`). `convert_amount`
        /// returns the unique `out` with `den * out <= num < den *
        /// (out + 1)`; it returns ZeroOutput exactly when `num <
        /// den`, and Overflow exactly when `num >= den * 2^128` (the
        /// true quotient does not fit a `u128`) \u2014 including when
        /// the implementation's own scale fold overflows 256 bits,
        /// since the denominator there is at most `u128::MAX`. The
        /// oracle shares no code with the `U256` under test: base
        /// 2^32 schoolbook multiplication, and no division anywhere.
        #[test]
        fn convert_amount_matches_wide_oracle_on_full_domain(
            amount in wide_u128(),
            rate_num in wide_u128(),
            rate_den in wide_u128(),
            source_scale in 0u32..=MAX_SCALE,
            dest_scale in 0u32..=MAX_SCALE,
        ) {
            let result = convert_amount(amount, rate_num, rate_den, source_scale, dest_scale);
            if rate_den == 0 {
                prop_assert_eq!(result, Err(ConvertError::ZeroRateDenominator));
                return Ok(());
            }
            let (num, den) = wide_terms(amount, rate_num, rate_den, source_scale, dest_scale);
            match result {
                Ok(out) => {
                    let out = Wide::from_u128(out);
                    prop_assert!(den.mul(out) <= num, "out is above the true quotient");
                    prop_assert!(num < den.mul(out.add_one()), "out is below the true quotient");
                }
                Err(ConvertError::ZeroOutput) => {
                    prop_assert!(num < den, "ZeroOutput but the true quotient is >= 1");
                }
                Err(ConvertError::Overflow) => {
                    prop_assert!(num >= den.shl128(), "Overflow but the true quotient fits u128");
                }
                Err(other) => {
                    prop_assert!(false, "unexpected error on the wide domain: {other:?}");
                }
            }
        }

        /// The 256-bit-overflow scale-fold path, from generated
        /// inputs: amounts and rate terms above 2^100 with a scale
        /// fold of at least 10^21 push the folded numerator past
        /// 256 bits, and the verdict must be Overflow \u2014 confirmed
        /// against the wide oracle, not just asserted.
        #[test]
        fn convert_amount_scale_fold_overflow_from_generated_inputs(
            amount in (1u128 << 100)..=u128::MAX,
            rate_num in (1u128 << 100)..=u128::MAX,
            rate_den in 1u128..(1u128 << 32),
            source_scale in 0u32..=4,
            dest_scale in 25u32..=MAX_SCALE,
        ) {
            let result = convert_amount(amount, rate_num, rate_den, source_scale, dest_scale);
            prop_assert_eq!(result, Err(ConvertError::Overflow));
            let (num, den) = wide_terms(amount, rate_num, rate_den, source_scale, dest_scale);
            prop_assert!(num >= den.shl128());
        }

        /// Fee oracle on the full domain: the same floor
        /// characterisation in `Wide` arithmetic \u2014 `fee * 10_000
        /// <= principal * fee_bps < (fee + 1) * 10_000` \u2014 with
        /// principals across the whole `u128` range (the naive
        /// oracle above bounds them to 2^100 because its `u128`
        /// product would overflow).
        #[test]
        fn fee_amount_matches_wide_oracle_on_full_domain(
            principal in wide_u128(),
            fee_bps in 0u32..=20_000,
        ) {
            let result = fee_amount(principal, fee_bps);
            if fee_bps > MAX_FEE_BPS {
                prop_assert_eq!(result, Err(ConvertError::InvalidFeeBps { fee_bps }));
                return Ok(());
            }
            let fee = result.expect("fee_bps within range always quotes");
            let product = Wide::from_u128(principal).mul(Wide::from_u128(u128::from(fee_bps)));
            let fee = Wide::from_u128(fee);
            let bps_denominator = Wide::from_u128(BPS_DENOMINATOR);
            prop_assert!(fee.mul(bps_denominator) <= product);
            prop_assert!(product < fee.add_one().mul(bps_denominator));
        }
    }
}
