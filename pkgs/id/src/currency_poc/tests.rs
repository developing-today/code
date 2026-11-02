//! Scenario tests for the conversion-executor PoC: the #2063 §4
//! failure cases plus the posted-conversion acceptance check.
//!
//! These run against [`InMemoryLedger`], which implements the
//! TigerBeetle semantics the executor relies on (atomic linked
//! chains, idempotent transfer IDs). The same scenarios are the ones
//! to run against a real test cluster once a `LedgerBackend` binding
//! for the official Rust client lands; the harness that starts that
//! cluster lives in `super::harness` and has its own ignored smoke
//! test.

use super::executor::{ConversionExecutor, ConversionRequest, RateQuote};
use super::intent::{FileIntentStore, InMemoryIntentStore, IntentState, IntentStore};
use super::ledger::{
    Account, AccountFlags, AccountId, InMemoryLedger, LedgerBackend, Transfer, TransferError,
};
use super::recovery::{ReconcileOutcome, RecoveryPolicy, reconcile};

const SRC: u32 = 101;
const DST: u32 = 102;

const USER_SRC: AccountId = 1001;
const LP_SRC: AccountId = 1002;
const FEE: AccountId = 1003;
const LP_DST: AccountId = 2001;
const USER_DST: AccountId = 2002;
const ISSUER_SRC: AccountId = 9001;
const ISSUER_DST: AccountId = 9002;

fn account(id: AccountId, ledger: u32, guarded: bool) -> Account {
    Account {
        id,
        ledger,
        code: 0,
        flags: AccountFlags {
            debits_must_not_exceed_credits: guarded,
            credits_must_not_exceed_debits: false,
        },
    }
}

fn funding_transfer(
    id: u128,
    from: AccountId,
    to: AccountId,
    amount: u128,
    ledger: u32,
) -> Transfer {
    Transfer {
        id,
        debit_account_id: from,
        credit_account_id: to,
        amount,
        ledger,
        user_data_128: 0,
        code: 0,
        linked: false,
    }
}

/// A ledger with stub accounts (#2289 sanctions stubs): the user holds
/// 10_000 source units, the LP holds 50_000 destination units.
fn funded_ledger() -> InMemoryLedger {
    let mut ledger = InMemoryLedger::new();
    let results = ledger.create_accounts(&[
        account(ISSUER_SRC, SRC, false),
        account(ISSUER_DST, DST, false),
        account(USER_SRC, SRC, true),
        account(LP_SRC, SRC, false),
        account(FEE, SRC, false),
        account(LP_DST, DST, true),
        account(USER_DST, DST, false),
    ]);
    assert!(results.iter().all(|result| result.is_ok()));
    let results = ledger.create_transfers(&[
        funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC),
        funding_transfer(502, ISSUER_DST, LP_DST, 50_000, DST),
    ]);
    assert!(results.iter().all(|result| result.is_ok()));
    ledger
}

fn request() -> ConversionRequest {
    ConversionRequest {
        conversion_id: 777,
        transfer_ids: vec![601, 602, 603],
        user_source: USER_SRC,
        lp_source: LP_SRC,
        fee_account: Some(FEE),
        lp_dest: LP_DST,
        user_dest: USER_DST,
        source_ledger: SRC,
        dest_ledger: DST,
        amount_in: 1_000,
        fee_amount: 25,
        quote: RateQuote {
            numerator: 2,
            denominator: 1,
            authorisation_id: "auth-777".to_string(),
        },
    }
}

type TestExecutor = ConversionExecutor<InMemoryLedger, InMemoryIntentStore>;

fn executor() -> TestExecutor {
    ConversionExecutor::new(funded_ledger(), InMemoryIntentStore::new())
}

#[test]
fn posted_conversion_legs_findable_by_user_data_128() {
    let mut executor = executor();
    let record = executor.execute(&request()).expect("conversion posts");
    assert_eq!(record.state, IntentState::Posted);
    assert_eq!(record.amount_out, 2_000);

    // Acceptance: both legs (all three, with the fee leg) findable by
    // user_data_128 = conversion_id.
    let legs = executor.backend.transfers_with_user_data_128(777);
    assert_eq!(legs.len(), 3);
    assert!(legs.iter().all(|leg| leg.user_data_128 == 777));

    // Balances moved exactly once, on the right ledgers.
    assert_eq!(
        executor
            .backend
            .account_balance(USER_SRC)
            .unwrap()
            .spendable(),
        8_975
    );
    assert_eq!(
        executor
            .backend
            .account_balance(LP_SRC)
            .unwrap()
            .credits_posted,
        1_000
    );
    assert_eq!(
        executor
            .backend
            .account_balance(FEE)
            .unwrap()
            .credits_posted,
        25
    );
    assert_eq!(
        executor
            .backend
            .account_balance(USER_DST)
            .unwrap()
            .credits_posted,
        2_000
    );
    assert_eq!(
        executor
            .backend
            .account_balance(LP_DST)
            .unwrap()
            .spendable(),
        48_000
    );
}

#[test]
fn atomic_failure_moves_nothing() {
    // #2063 §4: one leg of a linked batch is forced to fail (the LP
    // leg exceeds the LP's guarded balance) — no balance moves and no
    // leg is stored.
    let mut executor = executor();
    let mut record = executor.prepare(&request()).expect("intent prepared");

    // Drain the LP destination account after the write-ahead intent,
    // so T3 must fail inside the batch.
    let drain = funding_transfer(900, LP_DST, USER_DST, 49_000, DST);
    assert!(executor.backend.create_transfers(&[drain])[0].is_ok());

    let before_user = executor.backend.account_balance(USER_SRC).unwrap();
    let before_lp_src = executor.backend.account_balance(LP_SRC).unwrap();
    assert!(executor.submit(&mut record).is_err());

    assert_eq!(
        executor.backend.account_balance(USER_SRC).unwrap(),
        before_user
    );
    assert_eq!(
        executor.backend.account_balance(LP_SRC).unwrap(),
        before_lp_src
    );
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty(),
        "no leg of a failed linked batch may be stored"
    );
    // The record stays Submitted (unresolved) for the recovery scanner.
    assert_eq!(record.state, IntentState::Submitted);
}

#[test]
fn linked_chain_failure_marks_other_legs_linked_event_failed() {
    let mut ledger = funded_ledger();
    let chain = vec![
        Transfer {
            id: 700,
            debit_account_id: USER_SRC,
            credit_account_id: LP_SRC,
            amount: 100,
            ledger: SRC,
            user_data_128: 42,
            code: 1,
            linked: true,
        },
        Transfer {
            id: 701,
            debit_account_id: LP_DST,
            credit_account_id: USER_DST,
            amount: 999_999,
            ledger: DST,
            user_data_128: 42,
            code: 1,
            linked: false,
        },
    ];
    let results = ledger.create_transfers(&chain);
    assert_eq!(results[0], Err(TransferError::LinkedEventFailed));
    assert_eq!(results[1], Err(TransferError::DebitsMustNotExceedCredits));
    assert!(ledger.transfers_with_user_data_128(42).is_empty());
}

#[test]
fn crash_after_submission_recovers_without_double_posting() {
    // #2063 §4: crash between submission and the Posted state write.
    // The legs are in the ledger; the record still says Submitted.
    // The scanner must mark it Posted by lookup and move nothing again.
    let mut executor = executor();
    let mut record = executor.prepare(&request()).expect("intent prepared");
    record.state = IntentState::Submitted;
    executor.store.put(&record).unwrap();
    let results = executor.backend.create_transfers(&record.legs);
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Ok(()) | Err(TransferError::Exists)))
    );

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("reconcile");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::AlreadyPosted)]);
    assert_eq!(
        executor
            .backend
            .account_balance(USER_DST)
            .unwrap()
            .credits_posted,
        2_000,
        "balances must reflect exactly one posting"
    );
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Posted
    );
}

#[test]
fn crash_before_submission_resubmits_same_ids() {
    // Crash after the write-ahead intent but before submission: no
    // legs in the ledger. The scanner resubmits the persisted legs
    // with their original IDs; a second scan finds nothing unresolved.
    let mut executor = executor();
    let record = executor.prepare(&request()).expect("intent prepared");
    assert_eq!(record.state, IntentState::Recorded);

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("reconcile");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    assert_eq!(executor.backend.transfers_with_user_data_128(777).len(), 3);
    assert_eq!(
        executor
            .backend
            .account_balance(USER_DST)
            .unwrap()
            .credits_posted,
        2_000
    );

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("second reconcile is a no-op");
    assert!(outcomes.is_empty());
}

#[test]
fn insufficient_lp_inventory_fails_preflight() {
    // #2063 §4: LP pre-flight — a conversion the LP cannot cover is
    // rejected before any intent is written or transfer submitted.
    let mut executor = executor();
    let mut big = request();
    big.amount_in = 40_000; // amount_out = 80_000 > LP's 50_000
    assert!(executor.prepare(&big).is_err());
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn duplicate_t1_and_fee_transfer_ids_fail_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut duplicate = request();
    duplicate.transfer_ids = vec![601, 601, 603];

    let error = executor
        .prepare(&duplicate)
        .expect_err("T1 and fee must not share a transfer ID");
    assert!(error.to_string().contains("duplicate transfer ID 601"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn duplicate_t1_and_t3_transfer_ids_fail_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut duplicate = request();
    duplicate.transfer_ids = vec![601, 602, 601];

    let error = executor
        .prepare(&duplicate)
        .expect_err("T1 and T3 must not share a transfer ID");
    assert!(error.to_string().contains("duplicate transfer ID 601"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn file_intent_store_write_ahead_survives_reopen() {
    // The write-ahead record is the crash-recovery contract: prepare
    // with a file store, drop everything, reopen, and reconcile.
    let dir = std::env::temp_dir().join(format!("id-currency-poc-intents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut backend = funded_ledger();
    {
        let store = FileIntentStore::open(dir.clone()).expect("open store");
        let mut executor = ConversionExecutor::new(funded_ledger(), store);
        // Prepare against a throwaway ledger; only the intent matters here.
        let record = executor.prepare(&request()).expect("intent prepared");
        assert_eq!(record.state, IntentState::Recorded);
    }
    let mut store = FileIntentStore::open(dir.clone()).expect("reopen store");
    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit)
        .expect("reconcile after reopen");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    assert_eq!(backend.transfers_with_user_data_128(777).len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Regression tests for the Codex review findings on PR #3873 ---

#[test]
fn zero_denominator_quote_is_rejected_not_a_panic() {
    let mut executor = executor();
    let mut bad = request();
    bad.quote.denominator = 0;
    assert!(executor.prepare(&bad).is_err());
    assert!(executor.store.get(777).unwrap().is_none());
}

#[test]
fn overflowing_quote_is_rejected_not_saturated() {
    let mut executor = executor();
    let mut bad = request();
    bad.quote.numerator = u128::MAX;
    bad.quote.denominator = 1;
    assert!(executor.prepare(&bad).is_err());
    assert!(executor.store.get(777).unwrap().is_none());
}

#[test]
fn duplicate_transfer_id_in_one_chain_is_rejected() {
    // Two legs sharing one transfer ID must not both stage balance
    // changes while only the first is journaled.
    let mut ledger = funded_ledger();
    let first = Transfer {
        id: 700,
        debit_account_id: ISSUER_SRC,
        credit_account_id: USER_SRC,
        amount: 100,
        ledger: SRC,
        user_data_128: 43,
        code: 1,
        linked: true,
    };
    let second = Transfer {
        id: 700,
        debit_account_id: ISSUER_SRC,
        credit_account_id: LP_SRC,
        amount: 200,
        ledger: SRC,
        user_data_128: 43,
        code: 1,
        linked: false,
    };
    let results = ledger.create_transfers(&[first, second]);
    assert!(results.iter().all(|result| result.is_err()));
    assert!(ledger.transfers_with_user_data_128(43).is_empty());
    assert_eq!(
        ledger.account_balance(USER_SRC).unwrap().spendable(),
        10_000,
        "no balance may move for a rejected duplicate-ID chain"
    );
}

#[test]
fn recovery_does_not_trust_transfer_id_existence_alone() {
    // A prepared no-fee conversion whose transfer IDs collide with
    // the fixture's funding transfers (501/502, different contents)
    // is Inconsistent, never AlreadyPosted.
    let mut executor = executor();
    let mut colliding = request();
    colliding.fee_amount = 0;
    colliding.fee_account = None;
    colliding.transfer_ids = vec![501, 502];
    let record = executor.prepare(&colliding).expect("intent prepared");
    assert_eq!(record.legs.len(), 2);

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("reconcile");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Inconsistent)]);
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Recorded,
        "a mismatched record stays unresolved"
    );
}

#[test]
fn conversion_id_reuse_cannot_replace_persisted_intent() {
    let mut executor = executor();
    let record = executor.execute(&request()).expect("conversion posts");

    // An identical retry reuses the original record, state included.
    let again = executor
        .prepare(&request())
        .expect("identical retry reuses the persisted intent");
    assert_eq!(again, record);

    // A conflicting retry is rejected and the original survives.
    let mut conflicting = request();
    conflicting.amount_in = 500;
    conflicting.transfer_ids = vec![611, 612, 613];
    assert!(executor.prepare(&conflicting).is_err());
    assert_eq!(executor.store.get(777).unwrap().unwrap(), record);
}

#[test]
fn recovery_scan_continues_after_a_rejected_resubmission() {
    let mut executor = executor();
    let _record_a = executor.prepare(&request()).expect("intent A prepared");
    let mut request_b = request();
    request_b.conversion_id = 888;
    request_b.transfer_ids = vec![604, 605, 606];
    request_b.amount_in = 100;
    let _record_b = executor.prepare(&request_b).expect("intent B prepared");

    // Drain the LP destination so A's resubmission (amount_out 2_000)
    // is rejected while B's (200) still posts.
    let drain = funding_transfer(900, LP_DST, USER_DST, 49_000, DST);
    assert!(executor.backend.create_transfers(&[drain])[0].is_ok());

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("one rejected resubmission must not abort the scan");
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.contains(&(777, ReconcileOutcome::ResubmissionRejected)));
    assert!(outcomes.contains(&(888, ReconcileOutcome::Resubmitted)));
    assert_eq!(executor.backend.transfers_with_user_data_128(888).len(), 3);
}
