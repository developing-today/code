//! Acceptance tests for issue #2317 (design #2062 slice F1).
//!
//! The oracle for each test is a line of the issue's Acceptance list
//! or of design sections 2/6:
//!
//! - "A migration creates the four tables with the uniqueness
//!   constraints the design names" — the migration text is checked
//!   here for the four tables and constraints, and proven against a
//!   real Postgres by `scripts/test-migration.sh`.
//! - "An allocator API hands out non-zero u32s monotonically and
//!   refuses to recycle" — the allocator tests below.
//! - "A test proves two issuers may both register symbol `ABC` but
//!   a duplicate `currency_id` is rejected" —
//!   `two_issuers_may_both_register_symbol_abc` and
//!   `duplicate_currency_id_is_rejected`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_currency_registry::{
    AccountKind, AllocError, Allocator, CURRENCY_RANGE_START, CurrencyStatus, InMemoryStore,
    LedgerKind, MIGRATION_0001_UP, RegisterAccount, RegisterCurrency, Registry, RegistryError,
    SYSTEM_RANGE_END, StoreError, SupplyPolicy,
};

fn register_input(currency_id: &str, issuer_id: &str, symbol: &str) -> RegisterCurrency {
    RegisterCurrency {
        currency_id: currency_id.to_owned(),
        name: format!("{symbol} token"),
        symbol: symbol.to_owned(),
        scale: 2,
        issuer_id: issuer_id.to_owned(),
        supply_policy: SupplyPolicy {
            mintable: true,
            cap: None,
            policy_uri: None,
        },
        graph_object_id: Some(format!("graph-{currency_id}")),
    }
}

#[test]
fn allocator_hands_out_nonzero_monotonic_currency_ledgers() {
    let mut allocator = Allocator::new(InMemoryStore::new());
    let first = allocator.allocate(LedgerKind::Currency, "abc").unwrap();
    let second = allocator.allocate(LedgerKind::Currency, "def").unwrap();
    let third = allocator.allocate(LedgerKind::Currency, "ghi").unwrap();
    assert_eq!(first, CURRENCY_RANGE_START);
    assert!(first != 0 && second != 0 && third != 0);
    assert!(first < second && second < third, "allocation is monotonic");
}

#[test]
fn allocator_keeps_the_ratified_range_split() {
    let mut allocator = Allocator::new(InMemoryStore::new());
    let tenancy = allocator.allocate(LedgerKind::Tenancy, "tenant-1").unwrap();
    let system = allocator.allocate(LedgerKind::System, "fees").unwrap();
    let currency = allocator.allocate(LedgerKind::Currency, "abc").unwrap();
    assert_eq!(tenancy, 1);
    assert!((1..=SYSTEM_RANGE_END).contains(&system));
    assert!(currency >= CURRENCY_RANGE_START);
}

#[test]
fn allocator_refuses_to_recycle() {
    let mut allocator = Allocator::new(InMemoryStore::new());
    let spent = allocator.allocate(LedgerKind::Currency, "abc").unwrap();
    // Reserving the spent id again is a recycle attempt: refused.
    let err = allocator
        .reserve_specific(LedgerKind::Currency, spent, "someone-else")
        .unwrap_err();
    assert_eq!(err, AllocError::Store(StoreError::AlreadyAllocated(spent)));
    // And sequential allocation never reissues it either.
    let next = allocator.allocate(LedgerKind::Currency, "def").unwrap();
    assert_ne!(next, spent);
    assert!(next > spent);
}

#[test]
fn reserve_specific_validates_the_range() {
    let mut allocator = Allocator::new(InMemoryStore::new());
    // A currency reservation inside the system range is out of range.
    let err = allocator
        .reserve_specific(LedgerKind::Currency, 7, "abc")
        .unwrap_err();
    assert_eq!(
        err,
        AllocError::OutOfRange {
            kind: LedgerKind::Currency,
            ledger_id: 7
        }
    );
    // Ledger 0 is in nobody's range: it can never be allocated.
    let err = allocator
        .reserve_specific(LedgerKind::System, 0, "zero")
        .unwrap_err();
    assert_eq!(
        err,
        AllocError::OutOfRange {
            kind: LedgerKind::System,
            ledger_id: 0
        }
    );
}

#[test]
fn two_issuers_may_both_register_symbol_abc() {
    let mut registry = Registry::new(InMemoryStore::new());
    let alice = registry
        .register_currency(register_input("alice-abc", "alice", "ABC"))
        .unwrap();
    let bob = registry
        .register_currency(register_input("bob-abc", "bob", "ABC"))
        .unwrap();
    assert_eq!(alice.symbol, "ABC");
    assert_eq!(bob.symbol, "ABC");
    assert_ne!(alice.ledger_id, bob.ledger_id);
    assert!(alice.ledger_id >= CURRENCY_RANGE_START);
    assert_eq!(alice.status, CurrencyStatus::Active);
    // The graph linkage (design section 2.4) is recorded at minting.
    assert_eq!(alice.graph_object_id.as_deref(), Some("graph-alice-abc"));
}

#[test]
fn duplicate_currency_id_is_rejected() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    // A different issuer proposing the same currency_id: rejected,
    // because currency_id is globally unique (decision 3).
    let err = registry
        .register_currency(register_input("abc", "bob", "XYZ"))
        .unwrap_err();
    assert_eq!(
        err,
        RegistryError::Store(StoreError::DuplicateCurrencyId("abc".to_owned()))
    );
}

#[test]
fn duplicate_symbol_for_the_same_issuer_is_rejected() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("alice-abc", "alice", "ABC"))
        .unwrap();
    let err = registry
        .register_currency(register_input("alice-abc-2", "alice", "ABC"))
        .unwrap_err();
    assert_eq!(
        err,
        RegistryError::Store(StoreError::DuplicateSymbol {
            issuer_id: "alice".to_owned(),
            symbol: "ABC".to_owned()
        })
    );
}

#[test]
fn rejected_registration_does_not_burn_a_ledger_id() {
    let mut registry = Registry::new(InMemoryStore::new());
    let first = registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    let _ = registry.register_currency(register_input("abc", "bob", "XYZ"));
    let _ = registry.register_currency(register_input("alice-2", "alice", "ABC"));
    let second = registry
        .register_currency(register_input("def", "bob", "DEF"))
        .unwrap();
    assert_eq!(second.ledger_id, first.ledger_id + 1);
}

#[test]
fn only_the_issuer_can_grant_minters() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    registry.grant_minter("abc", "alice-ops", "alice").unwrap();
    let err = registry
        .grant_minter("abc", "mallory", "mallory")
        .unwrap_err();
    assert_eq!(
        err,
        RegistryError::NotIssuer {
            currency_id: "abc".to_owned(),
            account_id: "mallory".to_owned()
        }
    );
}

#[test]
fn account_index_records_factory_accounts() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    let account = registry
        .register_account(RegisterAccount {
            currency_id: "abc".to_owned(),
            account_id: "0123456789abcdef0123456789abcdef".to_owned(),
            owner_id: "alice".to_owned(),
            kind: AccountKind::Issuer,
            salt: String::new(),
        })
        .unwrap();
    assert_eq!(account.kind, AccountKind::Issuer);
    // The same (ledger, owner, kind, salt) slot cannot be recorded
    // twice under a different account id.
    let err = registry
        .register_account(RegisterAccount {
            currency_id: "abc".to_owned(),
            account_id: "fedcba9876543210fedcba9876543210".to_owned(),
            owner_id: "alice".to_owned(),
            kind: AccountKind::Issuer,
            salt: String::new(),
        })
        .unwrap_err();
    assert_eq!(err, RegistryError::Store(StoreError::DuplicateAccountSlot));
}

#[test]
fn migration_creates_the_four_tables_with_the_named_constraints() {
    let sql = MIGRATION_0001_UP.to_lowercase();
    for table in [
        "create table ledger_allocation",
        "create table currency",
        "create table currency_issuer",
        "create table account",
    ] {
        assert!(sql.contains(table), "migration is missing {table}");
    }
    // Decision 2: bigint ledger id with the 0..4294967295 check.
    assert!(sql.contains("bigint primary key"));
    assert!(sql.contains("between 0 and 4294967295"));
    // Decision 3: currency_id globally unique; symbol per issuer.
    assert!(sql.contains("currency_id     text not null unique"));
    assert!(sql.contains("unique (issuer_id, symbol)"));
    // Design section 2.4: the graph linkage column exists.
    assert!(sql.contains("graph_object_id"));
    // Design section 6: the allocator entry point exists.
    assert!(sql.contains("create function allocate_ledger"));
}

// Status transitions (issue #13342). The oracle is the issue's
// Acceptance list: "Registry tests: transition happy paths,
// non-issuer refusal, repeat-transition behavior as documented."

#[test]
fn issuer_can_freeze_then_sunset_and_sunset_is_terminal() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    // Sunset is reachable from frozen: freeze first.
    let frozen = registry.freeze_currency("abc", "alice").unwrap();
    assert_eq!(frozen.status, CurrencyStatus::Frozen);
    assert_eq!(
        registry.resolve("abc").unwrap().status,
        CurrencyStatus::Frozen,
        "the store carries the new status"
    );
    let sunset = registry.sunset_currency("abc", "alice").unwrap();
    assert_eq!(sunset.status, CurrencyStatus::Sunset);
    // Sunset is terminal: freezing a sunset currency is a typed
    // refusal naming the from/to statuses.
    let err = registry.freeze_currency("abc", "alice").unwrap_err();
    assert_eq!(
        err,
        RegistryError::InvalidTransition {
            currency_id: "abc".to_owned(),
            from: CurrencyStatus::Sunset,
            to: CurrencyStatus::Frozen,
        }
    );
}

#[test]
fn issuer_can_sunset_directly_from_active() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    let sunset = registry.sunset_currency("abc", "alice").unwrap();
    assert_eq!(sunset.status, CurrencyStatus::Sunset);
    assert_eq!(
        registry.resolve("abc").unwrap().status,
        CurrencyStatus::Sunset
    );
}

#[test]
fn only_the_issuer_can_transition_status() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    for attempt in [
        registry.freeze_currency("abc", "mallory"),
        registry.sunset_currency("abc", "mallory"),
    ] {
        assert_eq!(
            attempt.unwrap_err(),
            RegistryError::NotIssuer {
                currency_id: "abc".to_owned(),
                account_id: "mallory".to_owned(),
            }
        );
    }
    // The refusals moved nothing.
    assert_eq!(
        registry.resolve("abc").unwrap().status,
        CurrencyStatus::Active
    );
}

#[test]
fn repeat_transitions_are_idempotent_successes() {
    let mut registry = Registry::new(InMemoryStore::new());
    registry
        .register_currency(register_input("abc", "alice", "ABC"))
        .unwrap();
    // Freeze on frozen answers with the record as it stands.
    let first = registry.freeze_currency("abc", "alice").unwrap();
    let repeat = registry.freeze_currency("abc", "alice").unwrap();
    assert_eq!(repeat, first);
    // Sunset on sunset likewise.
    let first = registry.sunset_currency("abc", "alice").unwrap();
    let repeat = registry.sunset_currency("abc", "alice").unwrap();
    assert_eq!(repeat, first);
    assert_eq!(repeat.status, CurrencyStatus::Sunset);
}

#[test]
fn transitions_on_an_unknown_currency_are_refused() {
    let mut registry = Registry::new(InMemoryStore::new());
    assert_eq!(
        registry.freeze_currency("nope", "alice").unwrap_err(),
        RegistryError::UnknownCurrency("nope".to_owned())
    );
    assert_eq!(
        registry.sunset_currency("nope", "alice").unwrap_err(),
        RegistryError::UnknownCurrency("nope".to_owned())
    );
}
