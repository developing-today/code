//! Deterministic account factory — slice F2 of the #2062 design
//! (issue #2318, design section 3 and section 4.4).
//!
//! `TigerBeetle` accounts are created explicitly with a 128-bit id
//! and a ledger, and their flags are fixed at creation. The factory
//! makes account identity *derivable* and account creation
//! *idempotent*:
//!
//! - [`derive_account_id`] computes the account id for
//!   `(ledger, kind, owner, salt)` from BLAKE3 alone — any component
//!   can compute it with no registry round-trip, which is what makes
//!   creation idempotent and the factory stateless (section 3.1).
//! - [`Factory::ensure_account`] issues one `create_accounts`
//!   through the [`AccountCluster`] seam and treats the cluster's
//!   "exists" verdict as success: no read-modify-write, no lock.
//! - [`Factory::ensure_currency_accounts`] creates the per-currency
//!   system account set of section 3.2 (issuer, fee, burn, and one
//!   liquidity account per direction).
//! - [`transfer_code`] carries the reserved transfer-`code` table
//!   of section 4.4, so ledger history stays self-describing.
//!
//! The production cluster binding is the gateway's `TigerBeetle`
//! client (design section 4: the gateway is the only client). This
//! crate states the factory semantics over the seam, exactly as
//! slice F1 stated the registry semantics over its store seam, so
//! the behaviour is testable without a cluster.

use std::fmt;

use crate::registry::{AccountKind, CurrencyRecord};

/// Versioned domain separator for account-id derivation
/// (section 3.1). A future derivation scheme bumps the version and
/// coexists with this one.
pub const ACCOUNT_DOMAIN_SEP: &str = "id-currency/account/v1";

/// The platform owner id for the fee and burn accounts
/// (section 3.2: their owner is "platform").
pub const PLATFORM_OWNER_ID: &str = "platform";

impl AccountKind {
    /// The `kind(u8)` byte in the derivation input (section 3.1).
    /// The values are part of the v1 derivation scheme: changing
    /// one changes every derived id for that kind.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Issuer => 1,
            Self::Liquidity => 2,
            Self::Fee => 3,
            Self::Burn => 4,
            Self::Holder => 5,
        }
    }

    /// The creation-fixed `TigerBeetle` flags for this kind
    /// (section 3.2). Flags are fixed at creation and never change
    /// afterwards, so this table is the whole policy:
    ///
    /// - issuer and liquidity are unconstrained — issuance is a
    ///   debit from the issuer, so a negative issuer balance *is*
    ///   the circulating supply, and a supply cap is a registry
    ///   check, not a ledger invariant;
    /// - fee and burn are constrained
    ///   (`debits_must_not_exceed_credits`) so they can never spend
    ///   what they have not collected;
    /// - holder is constrained the same way — an overspend is
    ///   rejected by the cluster even if the gateway's logic is
    ///   wrong — and carries `history`, because audit matters for
    ///   holder accounts.
    #[must_use]
    pub const fn flags(self) -> AccountFlags {
        match self {
            Self::Issuer | Self::Liquidity => AccountFlags {
                debits_must_not_exceed_credits: false,
                credits_must_not_exceed_debits: false,
                history: false,
            },
            Self::Fee | Self::Burn => AccountFlags {
                debits_must_not_exceed_credits: true,
                credits_must_not_exceed_debits: false,
                history: false,
            },
            Self::Holder => AccountFlags {
                debits_must_not_exceed_credits: true,
                credits_must_not_exceed_debits: false,
                history: true,
            },
        }
    }
}

/// The creation-fixed account flags the factory sets, mirroring the
/// `TigerBeetle` account flags the design names (section 3.2).
/// (`linked` is a batching flag, not an account property the
/// factory decides, so it is not represented here.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountFlags {
    /// Reject a transfer that would make debits exceed credits
    /// (the no-overdraft guard).
    pub debits_must_not_exceed_credits: bool,
    /// Reject a transfer that would make credits exceed debits.
    pub credits_must_not_exceed_debits: bool,
    /// Keep this account's individual transfer history.
    pub history: bool,
}

/// Everything a `create_accounts` call needs for one account: the
/// derived id, its ledger, and its creation-fixed flags, plus the
/// derivation inputs for audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSpec {
    /// The derived 128-bit account id (section 3.1).
    pub id: u128,
    /// The currency's ledger.
    pub ledger_id: u32,
    /// Which of the per-currency accounts this is.
    pub kind: AccountKind,
    /// The account holder's id (an account id, section 4.1b of the
    /// design: a holder is an account, not a device).
    pub owner_id: String,
    /// Derivation salt; empty unless an owner has several accounts
    /// of one kind (section 3.1).
    pub salt: String,
    /// The creation-fixed flags for `kind`.
    pub flags: AccountFlags,
}

/// Derive the account id for `(ledger_id, kind, owner_id, salt)`
/// (section 3.1):
///
/// ```text
/// BLAKE3( domain_sep ‖ ledger_id(u32, LE) ‖ kind(u8)
///         ‖ owner_id(len-prefixed UTF-8) ‖ salt(len-prefixed) )[0..16]
/// ```
///
/// The length prefixes (u32 LE byte counts) are what make the
/// encoding unambiguous: `(owner="ab", salt="c")` and
/// `(owner="a", salt="bc")` derive different ids. The first 16
/// digest bytes are read little-endian as the `u128` id.
#[must_use]
pub fn derive_account_id(ledger_id: u32, kind: AccountKind, owner_id: &str, salt: &str) -> u128 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ACCOUNT_DOMAIN_SEP.as_bytes());
    hasher.update(&ledger_id.to_le_bytes());
    hasher.update(&[kind.code()]);
    hash_len_prefixed(&mut hasher, owner_id.as_bytes());
    hash_len_prefixed(&mut hasher, salt.as_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0u8; 16];
    id_bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(id_bytes)
}

fn hash_len_prefixed(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    // Byte counts of UTF-8 ids and salts are far below u32::MAX;
    // the cast cannot truncate in practice, and the derivation is
    // defined over the u32 LE count either way.
    #[allow(clippy::cast_possible_truncation)]
    let len = bytes.len() as u32;
    hasher.update(&len.to_le_bytes());
    hasher.update(bytes);
}

/// The spec for the account `(ledger_id, kind, owner_id, salt)`
/// would create: derived id plus the kind's creation-fixed flags.
#[must_use]
pub fn account_spec(ledger_id: u32, kind: AccountKind, owner_id: &str, salt: &str) -> AccountSpec {
    AccountSpec {
        id: derive_account_id(ledger_id, kind, owner_id, salt),
        ledger_id,
        kind,
        owner_id: owner_id.to_owned(),
        salt: salt.to_owned(),
        flags: kind.flags(),
    }
}

/// The cluster's verdict for one `create_accounts` item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    /// The account did not exist and was created.
    Created,
    /// The account already existed with this id; the factory
    /// treats this as success (section 3.1).
    Exists,
}

/// A cluster failure that is *not* a verdict about the request
/// (transport, timeout, cluster unavailable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterError {
    /// Human-readable detail from the cluster binding.
    pub message: String,
}

impl fmt::Display for ClusterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cluster error: {}", self.message)
    }
}

impl std::error::Error for ClusterError {}

/// The cluster seam: the one `create_accounts` operation the
/// factory needs. The production implementation is the gateway's
/// `TigerBeetle` client; tests substitute an in-memory cluster.
pub trait AccountCluster {
    /// Create one account, reporting [`CreateOutcome::Exists`]
    /// (not an error) when an account with `spec.id` is already
    /// present.
    ///
    /// # Errors
    ///
    /// Returns [`ClusterError`] on a transport or cluster failure.
    fn create_account(&mut self, spec: &AccountSpec) -> Result<CreateOutcome, ClusterError>;
}

/// A factory failure: the cluster could not be reached, so no
/// verdict about the account exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryError {
    /// The underlying cluster call failed.
    Cluster(ClusterError),
}

impl fmt::Display for FactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cluster(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for FactoryError {}

impl From<ClusterError> for FactoryError {
    fn from(err: ClusterError) -> Self {
        Self::Cluster(err)
    }
}

/// The per-currency system account set (section 3.2), as derived
/// account ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyAccounts {
    /// The issuer account (owner: the currency's issuer).
    pub issuer: u128,
    /// The fee account (owner: platform).
    pub fee: u128,
    /// The burn account (owner: platform).
    pub burn: u128,
    /// One liquidity account per direction, as
    /// `(direction, account_id)` in the order requested (owner:
    /// the liquidity provider; the direction is the salt).
    pub liquidity: Vec<(String, u128)>,
}

/// The deterministic account factory (section 3.3). Stateless: all
/// identity is derived, so one instance serves every currency.
#[derive(Debug, Default, Clone, Copy)]
pub struct Factory;

impl Factory {
    /// `ensure_account(currency, owner, kind, salt) -> account_id`
    /// (section 3.3): derive the id, issue the create, and return
    /// the id whether the cluster created the account or reported
    /// it already exists. Safe to call on every write path.
    ///
    /// `ledger_id` is the currency's ledger (the registry is the
    /// authority for which u32 a currency is, slice F1).
    ///
    /// # Errors
    ///
    /// Returns [`FactoryError`] when the cluster call fails; the
    /// caller retries — a retry derives the same id and is safe.
    pub fn ensure_account(
        cluster: &mut impl AccountCluster,
        ledger_id: u32,
        owner_id: &str,
        kind: AccountKind,
        salt: &str,
    ) -> Result<u128, FactoryError> {
        let spec = account_spec(ledger_id, kind, owner_id, salt);
        cluster.create_account(&spec)?;
        Ok(spec.id)
    }

    /// `ensure_currency_accounts(currency) -> { issuer, fee, burn,
    /// liquidity[] }` (section 3.3): create the per-currency system
    /// accounts of section 3.2 at registration. Idempotent, like
    /// [`Factory::ensure_account`].
    ///
    /// Owners: the issuer account belongs to `currency.issuer_id`;
    /// fee and burn belong to the platform ([`PLATFORM_OWNER_ID`]);
    /// each liquidity account belongs to `liquidity_provider_id`
    /// with its direction string as the derivation salt (one
    /// account per (currency, direction), section 3.2).
    ///
    /// # Errors
    ///
    /// Returns [`FactoryError`] when any cluster call fails; the
    /// accounts already created stay created, and a retry creates
    /// only the missing remainder (each create is idempotent).
    pub fn ensure_currency_accounts(
        cluster: &mut impl AccountCluster,
        currency: &CurrencyRecord,
        liquidity_provider_id: &str,
        liquidity_directions: &[&str],
    ) -> Result<CurrencyAccounts, FactoryError> {
        let issuer = Self::ensure_account(
            cluster,
            currency.ledger_id,
            &currency.issuer_id,
            AccountKind::Issuer,
            "",
        )?;
        let fee = Self::ensure_account(
            cluster,
            currency.ledger_id,
            PLATFORM_OWNER_ID,
            AccountKind::Fee,
            "",
        )?;
        let burn = Self::ensure_account(
            cluster,
            currency.ledger_id,
            PLATFORM_OWNER_ID,
            AccountKind::Burn,
            "",
        )?;
        let mut liquidity = Vec::with_capacity(liquidity_directions.len());
        for direction in liquidity_directions {
            let id = Self::ensure_account(
                cluster,
                currency.ledger_id,
                liquidity_provider_id,
                AccountKind::Liquidity,
                direction,
            )?;
            liquidity.push(((*direction).to_owned(), id));
        }
        Ok(CurrencyAccounts {
            issuer,
            fee,
            burn,
            liquidity,
        })
    }
}

/// The reserved transfer-`code` table (design section 4.4): a
/// reserved, documented range per transfer class, so ledger
/// history is self-describing.
pub mod transfer_code {
    /// What a transfer `code` means, by reserved range.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum TransferClass {
        /// Issuance (mint): `0x0001..=0x00FF`.
        Issuance,
        /// Holder transfer: `0x0100..=0x01FF`.
        HolderTransfer,
        /// Conversion leg (paired; #2063): `0x0200..=0x02FF`.
        ConversionLeg,
        /// Fee: `0x0300..=0x03FF`.
        Fee,
        /// Correction / refund: `0x0400..=0x04FF`.
        Correction,
        /// Reserved for pending-hold redemption (#2065):
        /// `0xF000..=0xFFFF`.
        PendingHoldRedemption,
        /// Not in any reserved range (including 0, which
        /// `TigerBeetle` leaves unspecified).
        Unreserved,
    }

    /// First issuance (mint) code.
    pub const ISSUANCE_START: u16 = 0x0001;
    /// Last issuance (mint) code.
    pub const ISSUANCE_END: u16 = 0x00FF;
    /// First holder-transfer code.
    pub const HOLDER_TRANSFER_START: u16 = 0x0100;
    /// Last holder-transfer code.
    pub const HOLDER_TRANSFER_END: u16 = 0x01FF;
    /// First conversion-leg code.
    pub const CONVERSION_LEG_START: u16 = 0x0200;
    /// Last conversion-leg code.
    pub const CONVERSION_LEG_END: u16 = 0x02FF;
    /// First fee code.
    pub const FEE_START: u16 = 0x0300;
    /// Last fee code.
    pub const FEE_END: u16 = 0x03FF;
    /// First correction / refund code.
    pub const CORRECTION_START: u16 = 0x0400;
    /// Last correction / refund code.
    pub const CORRECTION_END: u16 = 0x04FF;
    /// First code reserved for pending-hold redemption.
    pub const PENDING_HOLD_REDEMPTION_START: u16 = 0xF000;
    /// Last code reserved for pending-hold redemption.
    pub const PENDING_HOLD_REDEMPTION_END: u16 = 0xFFFF;

    /// The class a transfer `code` belongs to, by the reserved
    /// ranges of design section 4.4.
    #[must_use]
    pub const fn classify(code: u16) -> TransferClass {
        match code {
            ISSUANCE_START..=ISSUANCE_END => TransferClass::Issuance,
            HOLDER_TRANSFER_START..=HOLDER_TRANSFER_END => TransferClass::HolderTransfer,
            CONVERSION_LEG_START..=CONVERSION_LEG_END => TransferClass::ConversionLeg,
            FEE_START..=FEE_END => TransferClass::Fee,
            CORRECTION_START..=CORRECTION_END => TransferClass::Correction,
            PENDING_HOLD_REDEMPTION_START..=PENDING_HOLD_REDEMPTION_END => {
                TransferClass::PendingHoldRedemption
            }
            _ => TransferClass::Unreserved,
        }
    }
}
