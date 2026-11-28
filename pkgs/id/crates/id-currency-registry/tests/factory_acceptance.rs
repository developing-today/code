//! Acceptance tests for issue #2318 (design #2062 slice F2).
//!
//! The oracle for each test is a line of the issue's Acceptance
//! list or of design sections 3 / 4.4:
//!
//! - "The same `(currency, owner, kind, salt)` always derives the
//!   same account id, from any component, with no registry
//!   round-trip" — `derivation_matches_the_documented_layout`
//!   recomputes the id with an independent BLAKE3 construction
//!   straight from section 3.1's formula, and
//!   `derivation_is_deterministic_and_input_sensitive` checks
//!   stability and that every input is load-bearing.
//! - "Calling `ensure_account` twice creates one account and
//!   returns the same id" —
//!   `ensure_account_twice_creates_one_account` against an
//!   in-memory cluster that counts creates.
//! - "Flags are fixed at creation and documented; holder accounts
//!   reject overdrafts at the cluster" —
//!   `flags_match_the_section_3_2_table` checks the exact flag set
//!   per kind, including the holder constraint.
//! - Section 3.2's per-currency set and section 3.3's
//!   `ensure_currency_accounts` —
//!   `ensure_currency_accounts_creates_the_system_set_once`.
//! - Section 4.4's reserved transfer-code table —
//!   `transfer_code_ranges_match_section_4_4` at every boundary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id_currency_registry::factory::transfer_code::{self, TransferClass};
use id_currency_registry::factory::{
    AccountCluster, AccountSpec, ClusterError, CreateOutcome, Factory, PLATFORM_OWNER_ID,
    account_spec, derive_account_id,
};
use id_currency_registry::{AccountKind, CurrencyRecord, CurrencyStatus, SupplyPolicy};

/// An in-memory `TigerBeetle` stand-in: accounts keyed by id, a
/// create that reports `Exists` for a duplicate id (the cluster's
/// real verdict), and a create counter so tests can prove one
/// physical creation.
#[derive(Debug, Default)]
struct FakeCluster {
    accounts: BTreeMap<u128, AccountSpec>,
    create_calls: usize,
}

impl AccountCluster for FakeCluster {
    fn create_account(&mut self, spec: &AccountSpec) -> Result<CreateOutcome, ClusterError> {
        self.create_calls += 1;
        if self.accounts.contains_key(&spec.id) {
            return Ok(CreateOutcome::Exists);
        }
        self.accounts.insert(spec.id, spec.clone());
        Ok(CreateOutcome::Created)
    }
}

fn currency(ledger_id: u32, issuer_id: &str) -> CurrencyRecord {
    CurrencyRecord {
        ledger_id,
        currency_id: "cur-example".to_owned(),
        name: "Example token".to_owned(),
        symbol: "EXT".to_owned(),
        scale: 2,
        issuer_id: issuer_id.to_owned(),
        supply_policy: SupplyPolicy {
            mintable: true,
            cap: None,
            policy_uri: None,
        },
        status: CurrencyStatus::Active,
        graph_object_id: None,
    }
}

#[test]
fn derivation_matches_the_documented_layout() {
    // Oracle: design section 3.1's formula, rebuilt here from the
    // text — domain separator, ledger u32 LE, kind u8, then
    // length-prefixed owner and salt, first 16 digest bytes LE.
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"id-currency/account/v1");
    hasher.update(&0x0010_002Au32.to_le_bytes());
    hasher.update(&[5u8]); // holder
    let owner = "acct-alice";
    hasher.update(&(owner.len() as u32).to_le_bytes());
    hasher.update(owner.as_bytes());
    hasher.update(&0u32.to_le_bytes()); // empty salt
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    let expected = u128::from_le_bytes(bytes);

    let derived = derive_account_id(0x0010_002A, AccountKind::Holder, "acct-alice", "");
    assert_eq!(derived, expected);
    assert_ne!(derived, 0, "TigerBeetle id 0 is invalid");
}

#[test]
fn derivation_is_deterministic_and_input_sensitive() {
    let base = derive_account_id(7, AccountKind::Holder, "acct-alice", "");
    assert_eq!(
        base,
        derive_account_id(7, AccountKind::Holder, "acct-alice", "")
    );
    // Every input is load-bearing.
    assert_ne!(
        base,
        derive_account_id(8, AccountKind::Holder, "acct-alice", "")
    );
    assert_ne!(
        base,
        derive_account_id(7, AccountKind::Fee, "acct-alice", "")
    );
    assert_ne!(
        base,
        derive_account_id(7, AccountKind::Holder, "acct-bob", "")
    );
    assert_ne!(
        base,
        derive_account_id(7, AccountKind::Holder, "acct-alice", "escrow-1")
    );
    // Length prefixes keep the owner/salt boundary unambiguous.
    assert_ne!(
        derive_account_id(7, AccountKind::Holder, "ab", "c"),
        derive_account_id(7, AccountKind::Holder, "a", "bc"),
    );
}

#[test]
fn ensure_account_twice_creates_one_account() {
    let mut cluster = FakeCluster::default();
    let first =
        Factory::ensure_account(&mut cluster, 7, "acct-alice", AccountKind::Holder, "").unwrap();
    let second =
        Factory::ensure_account(&mut cluster, 7, "acct-alice", AccountKind::Holder, "").unwrap();
    assert_eq!(first, second);
    assert_eq!(cluster.accounts.len(), 1, "exactly one account created");
    assert_eq!(
        cluster.accounts[&first],
        account_spec(7, AccountKind::Holder, "acct-alice", ""),
    );
}

#[test]
fn flags_match_the_section_3_2_table() {
    let cases = [
        // (kind, debits_must_not_exceed_credits, history)
        (AccountKind::Issuer, false, false),
        (AccountKind::Liquidity, false, false),
        (AccountKind::Fee, true, false),
        (AccountKind::Burn, true, false),
        (AccountKind::Holder, true, true),
    ];
    for (kind, constrained, history) in cases {
        let flags = kind.flags();
        assert_eq!(
            flags.debits_must_not_exceed_credits, constrained,
            "{kind:?} overdraft constraint",
        );
        assert_eq!(flags.history, history, "{kind:?} history flag");
        assert!(
            !flags.credits_must_not_exceed_debits,
            "{kind:?} never constrains credits",
        );
    }
}

#[test]
fn ensure_currency_accounts_creates_the_system_set_once() {
    let record = currency(0x0010_002A, "acct-issuer");
    let mut cluster = FakeCluster::default();
    let accounts = Factory::ensure_currency_accounts(
        &mut cluster,
        &record,
        "acct-market-maker",
        &["buy", "sell"],
    )
    .unwrap();

    assert_eq!(
        accounts.issuer,
        derive_account_id(record.ledger_id, AccountKind::Issuer, "acct-issuer", ""),
    );
    assert_eq!(
        accounts.fee,
        derive_account_id(record.ledger_id, AccountKind::Fee, PLATFORM_OWNER_ID, ""),
    );
    assert_eq!(
        accounts.burn,
        derive_account_id(record.ledger_id, AccountKind::Burn, PLATFORM_OWNER_ID, ""),
    );
    assert_eq!(
        accounts.liquidity,
        vec![
            (
                "buy".to_owned(),
                derive_account_id(
                    record.ledger_id,
                    AccountKind::Liquidity,
                    "acct-market-maker",
                    "buy",
                ),
            ),
            (
                "sell".to_owned(),
                derive_account_id(
                    record.ledger_id,
                    AccountKind::Liquidity,
                    "acct-market-maker",
                    "sell",
                ),
            ),
        ],
    );
    assert_eq!(
        cluster.accounts.len(),
        5,
        "issuer + fee + burn + 2 liquidity"
    );

    // Idempotent: a second registration creates nothing new and
    // returns the same set.
    let again = Factory::ensure_currency_accounts(
        &mut cluster,
        &record,
        "acct-market-maker",
        &["buy", "sell"],
    )
    .unwrap();
    assert_eq!(again, accounts);
    assert_eq!(cluster.accounts.len(), 5);
}

#[test]
fn transfer_code_ranges_match_section_4_4() {
    let cases = [
        (0x0000, TransferClass::Unreserved),
        (0x0001, TransferClass::Issuance),
        (0x00FF, TransferClass::Issuance),
        (0x0100, TransferClass::HolderTransfer),
        (0x01FF, TransferClass::HolderTransfer),
        (0x0200, TransferClass::ConversionLeg),
        (0x02FF, TransferClass::ConversionLeg),
        (0x0300, TransferClass::Fee),
        (0x03FF, TransferClass::Fee),
        (0x0400, TransferClass::Correction),
        (0x04FF, TransferClass::Correction),
        (0x0500, TransferClass::Unreserved),
        (0xEFFF, TransferClass::Unreserved),
        (0xF000, TransferClass::PendingHoldRedemption),
        (0xFFFF, TransferClass::PendingHoldRedemption),
    ];
    for (code, class) in cases {
        assert_eq!(transfer_code::classify(code), class, "code {code:#06x}");
    }
}
