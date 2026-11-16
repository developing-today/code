//! Currency control-plane registry — slice F1 of the #2062 design
//! (issue #2317).
//!
//! The design (`doc/2026-10-29T12-42-21Z_design_currency_control_plane`,
//! sections 2 and 6) makes the registry the authority for *which u32
//! means what*: `TigerBeetle` ledgers are bare non-zero `u32`s with no
//! metadata, so the meaning of a ledger — name, symbol, scale, issuer,
//! supply policy, status — lives here, and the ledger-u32 allocator
//! here is the only place a ledger value is assigned.
//!
//! This crate carries:
//!
//! - the Postgres migration that creates the registry tables
//!   ([`MIGRATION_0001_UP`] / [`MIGRATION_0001_DOWN`], decision 1 on
//!   issue #2317: the registry store is Postgres; ledger ids are
//!   `bigint` with a `0..=4294967295` check, decision 2);
//! - the allocator and registry semantics over a [`store::Store`]
//!   seam ([`allocator`], [`registry`]), with an in-memory store
//!   ([`store::InMemoryStore`]) that enforces the same constraints
//!   the migration enforces, so the acceptance behaviour is testable
//!   without a database. The SQL behaviour itself is verified
//!   against a real Postgres by `scripts/test-migration.sh`.
//!
//! Ratified range split (design section 6, decision 2):
//! `0x00000001..=0x000FFFFF` is tenancy/system, `0x00100000..` is
//! currencies. Allocation is monotonic and never recycled; ledger 0
//! is invalid in `TigerBeetle` and never allocated.

pub mod allocator;
pub mod factory;
pub mod registry;
pub mod store;

pub use allocator::{AllocError, Allocator};
pub use factory::{
    ACCOUNT_DOMAIN_SEP, AccountCluster, AccountFlags, AccountSpec, ClusterError, CreateOutcome,
    CurrencyAccounts, Factory, FactoryError, PLATFORM_OWNER_ID, account_spec, derive_account_id,
};
pub use registry::{
    AccountKind, AccountRecord, CurrencyRecord, CurrencyStatus, IssuerGrant, IssuerRole,
    RegisterAccount, RegisterCurrency, Registry, RegistryError, SupplyPolicy,
};
pub use store::{InMemoryStore, Store, StoreError};

/// The first migration: the four registry tables, the allocator
/// sequences and function, and the never-delete guard.
pub const MIGRATION_0001_UP: &str = include_str!("../migrations/0001_currency_registry.up.sql");

/// The reverse of [`MIGRATION_0001_UP`].
pub const MIGRATION_0001_DOWN: &str = include_str!("../migrations/0001_currency_registry.down.sql");

/// Largest valid `TigerBeetle` ledger value.
pub const LEDGER_ID_MAX: u32 = u32::MAX;

/// First ledger id of the currency range (design section 6 split).
pub const CURRENCY_RANGE_START: u32 = 0x0010_0000;

/// Last ledger id of the shared tenancy/system range.
pub const SYSTEM_RANGE_END: u32 = 0x000F_FFFF;

/// Why a ledger u32 is spent (design section 2.1 `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LedgerKind {
    /// A currency ledger.
    Currency,
    /// A platform system ledger.
    System,
    /// A tenant-partitioning ledger.
    Tenancy,
}

impl LedgerKind {
    /// The `ledger_allocation.kind` text for this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Currency => "currency",
            Self::System => "system",
            Self::Tenancy => "tenancy",
        }
    }

    /// The inclusive range this kind may be allocated from.
    #[must_use]
    pub const fn range(self) -> (u32, u32) {
        match self {
            Self::Currency => (CURRENCY_RANGE_START, LEDGER_ID_MAX),
            Self::System | Self::Tenancy => (1, SYSTEM_RANGE_END),
        }
    }

    /// Whether `ledger_id` lies inside this kind's range.
    #[must_use]
    pub const fn contains(self, ledger_id: u32) -> bool {
        let (start, end) = self.range();
        ledger_id >= start && ledger_id <= end
    }
}
