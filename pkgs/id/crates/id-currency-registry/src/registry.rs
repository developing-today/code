//! The registry itself: currency records, issuer grants, and the
//! account index (design sections 2.2, 3, 4.2).
//!
//! `currency_issuer` and `account` are drafted tables (decision 4 on
//! issue #2317): the design names them and fixes their roles —
//! `currency_issuer` is where `register_currency` records the issuer
//! and `grant_minter` writes minters (section 4.2); `account` indexes
//! the deterministic factory accounts of section 3 — but does not
//! tabulate their columns, so the columns here and in migration
//! `0001` are the fleet draft for ratification.

use std::fmt;

use crate::LedgerKind;
use crate::allocator::Allocator;
use crate::store::{Store, StoreError};

/// Lifecycle status of a currency (design section 2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrencyStatus {
    /// Gateway may write.
    Active,
    /// Gateway refuses new writes.
    Frozen,
    /// No new writes; redemption window only.
    Sunset,
}

impl CurrencyStatus {
    /// The `currency.status` text for this status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::Sunset => "sunset",
        }
    }
}

/// A currency's supply policy (design section 2.2 `supply_policy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyPolicy {
    /// Whether minting is permitted at all.
    pub mintable: bool,
    /// Total supply cap in minor units, if capped.
    pub cap: Option<u128>,
    /// Off-registry policy document, if any.
    pub policy_uri: Option<String>,
}

/// One row of `currency` (design section 2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyRecord {
    /// The ledger this currency is (PK, FK to `ledger_allocation`).
    pub ledger_id: u32,
    /// Semantic identity: minted by the registry at registration,
    /// globally unique, never changes (decision 3, section 2.3).
    pub currency_id: String,
    /// Display name. Free-form; not unique.
    pub name: String,
    /// Short ticker. Unique per issuer, not globally (section 2.3).
    pub symbol: String,
    /// Decimal places. Registry-only; immutable once balances exist.
    pub scale: u8,
    /// The ID (account id) that owns issuance.
    pub issuer_id: String,
    /// Minting policy.
    pub supply_policy: SupplyPolicy,
    /// Lifecycle status.
    pub status: CurrencyStatus,
    /// The signed graph object that is this currency's canonical
    /// record (section 2.4); this row is a derived index over it.
    pub graph_object_id: Option<String>,
}

/// The input to [`Registry::register_currency`]. The `currency_id`
///
/// is proposed by the caller and *minted* by the registry: it
/// becomes the currency's identity only when registration records
/// it, and a duplicate proposal is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterCurrency {
    /// Proposed semantic id (e.g. `abc`).
    pub currency_id: String,
    /// Display name.
    pub name: String,
    /// Ticker; must be free in the issuer's namespace.
    pub symbol: String,
    /// Decimal places.
    pub scale: u8,
    /// The registering issuer's account id.
    pub issuer_id: String,
    /// Minting policy.
    pub supply_policy: SupplyPolicy,
    /// Signed graph object recording the registration (section 2.4).
    pub graph_object_id: Option<String>,
}

/// A role a `currency_issuer` row grants (design section 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuerRole {
    /// The currency's issuer (recorded at registration).
    Issuer,
    /// A granted minter (`grant_minter`).
    Minter,
}

impl IssuerRole {
    /// The `currency_issuer.role` text for this role.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issuer => "issuer",
            Self::Minter => "minter",
        }
    }
}

/// One row of `currency_issuer` (drafted from design section 4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuerGrant {
    /// The currency's ledger.
    pub ledger_id: u32,
    /// The account the role is granted to.
    pub account_id: String,
    /// The granted role.
    pub role: IssuerRole,
    /// Who made the grant (the issuer, for minters).
    pub granted_by: Option<String>,
}

/// The per-currency account kinds (design section 3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    /// Source of every mint.
    Issuer,
    /// Conversion-leg liquidity.
    Liquidity,
    /// Fee collection.
    Fee,
    /// Retirement destination.
    Burn,
    /// A user's holding account.
    Holder,
}

impl AccountKind {
    /// The `account.kind` text for this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issuer => "issuer",
            Self::Liquidity => "liquidity",
            Self::Fee => "fee",
            Self::Burn => "burn",
            Self::Holder => "holder",
        }
    }
}

/// One row of `account` (drafted from design section 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The derived 128-bit account id (hex text), section 3.1.
    pub account_id: String,
    /// The currency's ledger.
    pub ledger_id: u32,
    /// The account holder's id.
    pub owner_id: String,
    /// Which of the per-currency accounts this is.
    pub kind: AccountKind,
    /// Derivation salt; empty unless an owner has several accounts
    /// of one kind (section 3.1).
    pub salt: String,
}

/// The input to [`Registry::register_account`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAccount {
    /// The currency's semantic id.
    pub currency_id: String,
    /// The derived account id (hex text).
    pub account_id: String,
    /// The account holder's id.
    pub owner_id: String,
    /// Which per-currency account this is.
    pub kind: AccountKind,
    /// Derivation salt (default empty).
    pub salt: String,
}

/// A registry failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// No currency is registered under this id.
    UnknownCurrency(String),
    /// An issuer-only write (`grant_minter`, or a status
    /// transition) was attempted by an account that is not the
    /// currency's issuer (design section 4.2).
    NotIssuer {
        /// The currency whose issuer set was checked.
        currency_id: String,
        /// The account that attempted the grant.
        account_id: String,
    },
    /// A status transition the lifecycle does not allow.
    /// Sunset is terminal: a sunset currency cannot be frozen or
    /// otherwise moved. Repeating the transition that produced
    /// the current status is not an error — it succeeds without
    /// a write (see [`Registry::freeze_currency`]).
    InvalidTransition {
        /// The currency whose transition was refused.
        currency_id: String,
        /// The status the currency is in.
        from: CurrencyStatus,
        /// The status the caller tried to move it to.
        to: CurrencyStatus,
    },
    /// The store rejected the write.
    Store(StoreError),
    /// The allocator could not hand out a ledger.
    Alloc(crate::allocator::AllocError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCurrency(id) => write!(f, "no currency registered as {id}"),
            Self::NotIssuer {
                currency_id,
                account_id,
            } => {
                write!(f, "account {account_id} is not the issuer of {currency_id}")
            }
            Self::InvalidTransition {
                currency_id,
                from,
                to,
            } => {
                write!(
                    f,
                    "currency {currency_id} is {} and cannot become {}",
                    from.as_str(),
                    to.as_str()
                )
            }
            Self::Store(err) => write!(f, "{err}"),
            Self::Alloc(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<StoreError> for RegistryError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl From<crate::allocator::AllocError> for RegistryError {
    fn from(err: crate::allocator::AllocError) -> Self {
        Self::Alloc(err)
    }
}

/// The currency registry over a [`Store`].
#[derive(Debug)]
pub struct Registry<S: Store> {
    store: S,
}

impl<S: Store> Registry<S> {
    /// Create a registry over `store`.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Access the underlying store.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Register a currency: allocate its ledger from the currency
    /// range, mint its `currency_id`, and record the issuer grant —
    /// in that order, so a registration that fails on a duplicate
    /// `currency_id` or a taken symbol never burns a ledger id.
    pub fn register_currency(
        &mut self,
        input: RegisterCurrency,
    ) -> Result<CurrencyRecord, RegistryError> {
        // Check the uniqueness verdicts before spending a ledger id:
        // a rejected registration must not consume from the finite,
        // never-recycled u32 space.
        if self.store.currency_by_id(&input.currency_id).is_some() {
            return Err(StoreError::DuplicateCurrencyId(input.currency_id).into());
        }
        if self.store.symbol_taken(&input.issuer_id, &input.symbol) {
            return Err(StoreError::DuplicateSymbol {
                issuer_id: input.issuer_id,
                symbol: input.symbol,
            }
            .into());
        }
        let ledger_id = {
            let mut allocator = Allocator::new(&mut self.store);
            allocator.allocate(LedgerKind::Currency, input.currency_id.clone())?
        };
        let record = CurrencyRecord {
            ledger_id,
            currency_id: input.currency_id,
            name: input.name,
            symbol: input.symbol,
            scale: input.scale,
            issuer_id: input.issuer_id,
            supply_policy: input.supply_policy,
            status: CurrencyStatus::Active,
            graph_object_id: input.graph_object_id,
        };
        self.store.insert_currency(record.clone())?;
        self.store.insert_grant(IssuerGrant {
            ledger_id: record.ledger_id,
            account_id: record.issuer_id.clone(),
            role: IssuerRole::Issuer,
            granted_by: None,
        })?;
        Ok(record)
    }

    /// Grant the minter role for a currency. Only the currency's
    /// issuer may grant (design section 4.2).
    pub fn grant_minter(
        &mut self,
        currency_id: &str,
        minter_account_id: impl Into<String>,
        granted_by: impl Into<String>,
    ) -> Result<(), RegistryError> {
        let granted_by = granted_by.into();
        let currency = self
            .store
            .currency_by_id(currency_id)
            .ok_or_else(|| RegistryError::UnknownCurrency(currency_id.to_owned()))?;
        if currency.issuer_id != granted_by {
            return Err(RegistryError::NotIssuer {
                currency_id: currency_id.to_owned(),
                account_id: granted_by,
            });
        }
        self.store.insert_grant(IssuerGrant {
            ledger_id: currency.ledger_id,
            account_id: minter_account_id.into(),
            role: IssuerRole::Minter,
            granted_by: Some(granted_by),
        })?;
        Ok(())
    }

    /// Freeze a currency (design section 2.2): while frozen the
    /// gateway refuses new writes. Issuer-only. See
    /// [`Registry::set_status`] for the transition table.
    pub fn freeze_currency(
        &mut self,
        currency_id: &str,
        requested_by: impl Into<String>,
    ) -> Result<CurrencyRecord, RegistryError> {
        self.set_status(currency_id, CurrencyStatus::Frozen, requested_by)
    }

    /// Sunset a currency (design section 2.2): no new writes;
    /// redemption window only. Issuer-only, and terminal — a
    /// sunset currency cannot be frozen or otherwise moved. See
    /// [`Registry::set_status`] for the transition table.
    pub fn sunset_currency(
        &mut self,
        currency_id: &str,
        requested_by: impl Into<String>,
    ) -> Result<CurrencyRecord, RegistryError> {
        self.set_status(currency_id, CurrencyStatus::Sunset, requested_by)
    }

    /// The one status-transition write (issue #13342). Only the
    /// currency's issuer may move its status — the same
    /// issuer-only rule as `grant_minter` (design section 4.2).
    ///
    /// The transition table, decided here:
    ///
    /// - `Active -> Frozen` (freeze) and `Active -> Sunset`
    ///   (sunset) are the moves out of active.
    /// - `Frozen -> Sunset` is allowed: sunset stays reachable
    ///   from frozen, so freezing is never a trap short of the
    ///   terminal state.
    /// - Sunset is terminal: `Sunset -> Frozen` is refused with
    ///   [`RegistryError::InvalidTransition`].
    /// - Repeating the transition that produced the current
    ///   status (freeze on frozen, sunset on sunset) is an
    ///   idempotent success, not an error: the caller is told
    ///   the record as it stands, with no second write. This is
    ///   what lets the gateway service answer a retry whose
    ///   intent finalize was lost after the transition landed.
    fn set_status(
        &mut self,
        currency_id: &str,
        target: CurrencyStatus,
        requested_by: impl Into<String>,
    ) -> Result<CurrencyRecord, RegistryError> {
        let requested_by = requested_by.into();
        let currency = self
            .store
            .currency_by_id(currency_id)
            .ok_or_else(|| RegistryError::UnknownCurrency(currency_id.to_owned()))?
            .clone();
        if currency.issuer_id != requested_by {
            return Err(RegistryError::NotIssuer {
                currency_id: currency_id.to_owned(),
                account_id: requested_by,
            });
        }
        match (currency.status, target) {
            (from, to) if from == to => Ok(currency),
            (CurrencyStatus::Active, CurrencyStatus::Frozen | CurrencyStatus::Sunset)
            | (CurrencyStatus::Frozen, CurrencyStatus::Sunset) => {
                self.store.set_currency_status(currency.ledger_id, target)?;
                Ok(CurrencyRecord {
                    status: target,
                    ..currency
                })
            }
            (from, to) => Err(RegistryError::InvalidTransition {
                currency_id: currency_id.to_owned(),
                from,
                to,
            }),
        }
    }

    /// Record a factory account in the registry index (design
    /// section 3). The account id itself is derived by the factory
    /// (slice F2); this records that it exists for a currency.
    pub fn register_account(
        &mut self,
        input: RegisterAccount,
    ) -> Result<AccountRecord, RegistryError> {
        let currency = self
            .store
            .currency_by_id(&input.currency_id)
            .ok_or_else(|| RegistryError::UnknownCurrency(input.currency_id.clone()))?;
        let record = AccountRecord {
            account_id: input.account_id,
            ledger_id: currency.ledger_id,
            owner_id: input.owner_id,
            kind: input.kind,
            salt: input.salt,
        };
        self.store.insert_account(record.clone())?;
        Ok(record)
    }

    /// Resolve a `currency_id` to its record — the
    /// `currency_id -> ledger_id` lookup the gateway performs before
    /// any transfer (design section 2.3).
    pub fn resolve(&self, currency_id: &str) -> Option<&CurrencyRecord> {
        self.store.currency_by_id(currency_id)
    }
}
