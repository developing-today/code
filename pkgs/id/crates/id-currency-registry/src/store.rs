//! Storage seam for the registry.
//!
//! The production store is Postgres (migration `0001`); its
//! constraints are the authority. [`Store`] states the operations
//! the allocator and registry need, and [`InMemoryStore`] implements
//! them with exactly the migration's uniqueness and range rules, so
//! behaviour proven against the in-memory store is the behaviour the
//! schema enforces (the schema side is proven separately against a
//! real Postgres by `scripts/test-migration.sh`).

use std::collections::BTreeMap;
use std::fmt;

use crate::LedgerKind;
use crate::registry::{AccountRecord, CurrencyRecord, IssuerGrant};

/// One row of `ledger_allocation` (design section 2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    /// The allocated ledger u32 (never 0, never recycled).
    pub ledger_id: u32,
    /// Why this u32 was spent.
    pub kind: LedgerKind,
    /// Currency id, system-account purpose, or tenant id.
    pub owner_ref: String,
}

/// A store failure. Each variant mirrors a constraint in migration
/// `0001`, named so a caller can tell a uniqueness rejection (a
/// verdict about the request) from an unknown reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The ledger id is already allocated; ids are never recycled.
    AlreadyAllocated(u32),
    /// A `currency_id` is already registered (globally unique,
    /// decision 3 on issue #2317).
    DuplicateCurrencyId(String),
    /// The `(issuer_id, symbol)` pair is already registered; symbol
    /// is unique per issuer, not globally (design section 2.3).
    DuplicateSymbol {
        /// The issuer whose namespace already holds the symbol.
        issuer_id: String,
        /// The conflicting symbol.
        symbol: String,
    },
    /// No allocation exists for this ledger id.
    UnknownLedger(u32),
    /// The ledger exists but is not a currency allocation.
    LedgerNotCurrency(u32),
    /// The derived account id is already recorded.
    DuplicateAccountId(String),
    /// An account already occupies this `(ledger, owner, kind, salt)`
    /// slot.
    DuplicateAccountSlot,
    /// This `(ledger, account, role)` grant already exists.
    DuplicateGrant,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyAllocated(id) => {
                write!(f, "ledger {id} is already allocated and never recycled")
            }
            Self::DuplicateCurrencyId(id) => {
                write!(f, "currency_id {id} is already registered")
            }
            Self::DuplicateSymbol { issuer_id, symbol } => {
                write!(f, "issuer {issuer_id} already registers symbol {symbol}")
            }
            Self::UnknownLedger(id) => write!(f, "ledger {id} is not allocated"),
            Self::LedgerNotCurrency(id) => {
                write!(f, "ledger {id} is not a currency allocation")
            }
            Self::DuplicateAccountId(id) => {
                write!(f, "account {id} is already recorded")
            }
            Self::DuplicateAccountSlot => {
                write!(
                    f,
                    "an account already occupies this (ledger, owner, kind, salt) slot"
                )
            }
            Self::DuplicateGrant => write!(f, "this issuer grant already exists"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The operations the allocator and registry require of a store.
///
/// Implementations must enforce the migration's constraints, not
/// just record rows: a store that accepts a duplicate `currency_id`
/// is not a registry store.
pub trait Store {
    /// Record a new allocation. Fails with
    /// [`StoreError::AlreadyAllocated`] if the id is taken — this is
    /// the never-recycle rule at the storage layer.
    fn insert_allocation(&mut self, allocation: Allocation) -> Result<(), StoreError>;

    /// Look up an allocation by ledger id.
    fn allocation(&self, ledger_id: u32) -> Option<&Allocation>;

    /// The highest allocated id inside `kind`'s range, if any. The
    /// allocator derives the next candidate from this, so allocation
    /// stays monotonic even across reservations made out of band.
    fn highest_in_range(&self, kind: LedgerKind) -> Option<u32>;

    /// Record a currency row for an allocated currency ledger.
    fn insert_currency(&mut self, record: CurrencyRecord) -> Result<(), StoreError>;

    /// Look up a currency by its semantic id.
    fn currency_by_id(&self, currency_id: &str) -> Option<&CurrencyRecord>;

    /// Look up a currency by its ledger id.
    fn currency_by_ledger(&self, ledger_id: u32) -> Option<&CurrencyRecord>;

    /// Whether `symbol` is already registered in `issuer_id`'s
    /// namespace. Lets the registry reject before spending a ledger
    /// id; [`Store::insert_currency`] still enforces the rule.
    fn symbol_taken(&self, issuer_id: &str, symbol: &str) -> bool;

    /// Record an issuer/minter grant.
    fn insert_grant(&mut self, grant: IssuerGrant) -> Result<(), StoreError>;

    /// Record a factory account index row.
    fn insert_account(&mut self, account: AccountRecord) -> Result<(), StoreError>;
}

/// In-memory [`Store`] enforcing the migration's constraints. Used
/// by tests and by any caller prototyping against the semantics
/// before the Postgres-backed store lands behind the same seam.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    allocations: BTreeMap<u32, Allocation>,
    currencies_by_id: BTreeMap<String, CurrencyRecord>,
    currencies_by_ledger: BTreeMap<u32, String>,
    grants: Vec<IssuerGrant>,
    accounts: Vec<AccountRecord>,
}

impl InMemoryStore {
    /// A new, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for InMemoryStore {
    fn insert_allocation(&mut self, allocation: Allocation) -> Result<(), StoreError> {
        if self.allocations.contains_key(&allocation.ledger_id) {
            return Err(StoreError::AlreadyAllocated(allocation.ledger_id));
        }
        self.allocations.insert(allocation.ledger_id, allocation);
        Ok(())
    }

    fn allocation(&self, ledger_id: u32) -> Option<&Allocation> {
        self.allocations.get(&ledger_id)
    }

    fn highest_in_range(&self, kind: LedgerKind) -> Option<u32> {
        self.allocations
            .keys()
            .filter(|id| kind.contains(**id))
            .max()
            .copied()
    }

    fn insert_currency(&mut self, record: CurrencyRecord) -> Result<(), StoreError> {
        match self.allocations.get(&record.ledger_id) {
            None => return Err(StoreError::UnknownLedger(record.ledger_id)),
            Some(allocation) if allocation.kind != LedgerKind::Currency => {
                return Err(StoreError::LedgerNotCurrency(record.ledger_id));
            }
            Some(_) => {}
        }
        if self.currencies_by_id.contains_key(&record.currency_id) {
            return Err(StoreError::DuplicateCurrencyId(record.currency_id));
        }
        if self.symbol_taken(&record.issuer_id, &record.symbol) {
            return Err(StoreError::DuplicateSymbol {
                issuer_id: record.issuer_id,
                symbol: record.symbol,
            });
        }
        self.currencies_by_ledger
            .insert(record.ledger_id, record.currency_id.clone());
        self.currencies_by_id
            .insert(record.currency_id.clone(), record);
        Ok(())
    }

    fn currency_by_id(&self, currency_id: &str) -> Option<&CurrencyRecord> {
        self.currencies_by_id.get(currency_id)
    }

    fn currency_by_ledger(&self, ledger_id: u32) -> Option<&CurrencyRecord> {
        self.currencies_by_ledger
            .get(&ledger_id)
            .and_then(|id| self.currencies_by_id.get(id))
    }

    fn symbol_taken(&self, issuer_id: &str, symbol: &str) -> bool {
        self.currencies_by_id
            .values()
            .any(|c| c.issuer_id == issuer_id && c.symbol == symbol)
    }

    fn insert_grant(&mut self, grant: IssuerGrant) -> Result<(), StoreError> {
        if self.currency_by_ledger(grant.ledger_id).is_none() {
            return Err(StoreError::UnknownLedger(grant.ledger_id));
        }
        if self.grants.iter().any(|g| {
            g.ledger_id == grant.ledger_id
                && g.account_id == grant.account_id
                && g.role == grant.role
        }) {
            return Err(StoreError::DuplicateGrant);
        }
        self.grants.push(grant);
        Ok(())
    }

    fn insert_account(&mut self, account: AccountRecord) -> Result<(), StoreError> {
        if self.currency_by_ledger(account.ledger_id).is_none() {
            return Err(StoreError::UnknownLedger(account.ledger_id));
        }
        if self
            .accounts
            .iter()
            .any(|a| a.account_id == account.account_id)
        {
            return Err(StoreError::DuplicateAccountId(account.account_id));
        }
        if self.accounts.iter().any(|a| {
            a.ledger_id == account.ledger_id
                && a.owner_id == account.owner_id
                && a.kind == account.kind
                && a.salt == account.salt
        }) {
            return Err(StoreError::DuplicateAccountSlot);
        }
        self.accounts.push(account);
        Ok(())
    }
}

/// A mutable reference to a store is itself a store. This is what
/// lets [`crate::Allocator`] and [`crate::Registry`] share one store
/// without either owning it.
impl<T: Store + ?Sized> Store for &mut T {
    fn insert_allocation(&mut self, allocation: Allocation) -> Result<(), StoreError> {
        (**self).insert_allocation(allocation)
    }

    fn allocation(&self, ledger_id: u32) -> Option<&Allocation> {
        (**self).allocation(ledger_id)
    }

    fn highest_in_range(&self, kind: LedgerKind) -> Option<u32> {
        (**self).highest_in_range(kind)
    }

    fn insert_currency(&mut self, record: CurrencyRecord) -> Result<(), StoreError> {
        (**self).insert_currency(record)
    }

    fn currency_by_id(&self, currency_id: &str) -> Option<&CurrencyRecord> {
        (**self).currency_by_id(currency_id)
    }

    fn currency_by_ledger(&self, ledger_id: u32) -> Option<&CurrencyRecord> {
        (**self).currency_by_ledger(ledger_id)
    }

    fn symbol_taken(&self, issuer_id: &str, symbol: &str) -> bool {
        (**self).symbol_taken(issuer_id, symbol)
    }

    fn insert_grant(&mut self, grant: IssuerGrant) -> Result<(), StoreError> {
        (**self).insert_grant(grant)
    }

    fn insert_account(&mut self, account: AccountRecord) -> Result<(), StoreError> {
        (**self).insert_account(account)
    }
}
