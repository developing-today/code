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
use super::intent::{FileIntentStore, InMemoryIntentStore, IntentRecord, IntentState, IntentStore};
use super::ledger::{
    Account, AccountError, AccountFlags, AccountId, InMemoryLedger, LedgerBackend, Transfer,
    TransferError,
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
fn failed_chain_records_only_the_failing_leg() {
    // #6627 (oracle: the failing leg's own error is the only outcome
    // the chain may cache): when a linked chain fails on one leg,
    // only that leg's ID is recorded as failed -- a valid sibling
    // leg remains submittable afterwards.
    let mut ledger = funded_ledger();
    let sibling = Transfer {
        linked: true,
        ..funding_transfer(700, USER_SRC, LP_SRC, 100, SRC)
    };
    let failing = funding_transfer(701, LP_DST, USER_DST, 999_999, DST);
    let results = ledger.create_transfers(&[sibling.clone(), failing]);
    assert_eq!(results[0], Err(TransferError::LinkedEventFailed));
    assert_eq!(results[1], Err(TransferError::DebitsMustNotExceedCredits));
    // The sibling leg was never itself invalid: resubmitted on its
    // own (unlinked) it posts.
    let retry = Transfer {
        linked: false,
        ..sibling
    };
    assert_eq!(ledger.create_transfers(&[retry]), vec![Ok(())]);
    assert!(ledger.lookup_transfer(700).is_some());
}

#[test]
fn conflicting_submission_does_not_poison_a_posted_transfer() {
    // #6627 (oracle: a posted transfer's byte-identical retry always
    // returns `Exists`): a submission reusing a posted ID with
    // different fields is rejected with `ExistsWithDifferentFields`,
    // and that rejection must not be cached against the posted ID.
    let mut ledger = funded_ledger();
    let posted = funding_transfer(700, USER_SRC, LP_SRC, 100, SRC);
    assert_eq!(ledger.create_transfers(&[posted.clone()]), vec![Ok(())]);
    let conflicting = funding_transfer(700, USER_SRC, LP_SRC, 200, SRC);
    assert_eq!(
        ledger.create_transfers(&[conflicting]),
        vec![Err(TransferError::ExistsWithDifferentFields)]
    );
    assert_eq!(
        ledger.create_transfers(&[posted]),
        vec![Err(TransferError::Exists)]
    );
}

#[test]
fn resubmitted_failed_transfer_returns_id_already_failed_after_cause_fixed() {
    // #6628 (oracle: the `IdAlreadyFailed` variant's doc comment and
    // the live binding's `Status::IdAlreadyFailed` mapping in
    // tb_ledger.rs): a transfer that failed is remembered by ID, and
    // resubmitting that ID returns `IdAlreadyFailed` -- even after
    // the original failure's cause has been fixed, it must not
    // return the stale stored error (here `DebitAccountNotFound`).
    let mut ledger = InMemoryLedger::new();
    let transfer = funding_transfer(700, USER_SRC, LP_SRC, 100, SRC);
    // Neither account exists yet: the transfer fails and its ID is
    // recorded as failed.
    assert_eq!(
        ledger.create_transfers(&[transfer.clone()]),
        vec![Err(TransferError::DebitAccountNotFound)]
    );
    // Fix the cause: create both accounts and fund the debit side,
    // so the same transfer would now post if it were a fresh ID.
    let results = ledger.create_accounts(&[
        account(ISSUER_SRC, SRC, false),
        account(USER_SRC, SRC, false),
        account(LP_SRC, SRC, false),
    ]);
    assert!(results.iter().all(|result| result.is_ok()));
    assert_eq!(
        ledger.create_transfers(&[funding_transfer(701, ISSUER_SRC, USER_SRC, 1_000, SRC)]),
        vec![Ok(())]
    );
    // Resubmission of the failed ID is still refused, with
    // `IdAlreadyFailed`, and nothing posts.
    assert_eq!(
        ledger.create_transfers(&[transfer]),
        vec![Err(TransferError::IdAlreadyFailed)]
    );
    assert!(ledger.lookup_transfer(700).is_none());
}

#[test]
fn single_unterminated_linked_transfer_rejected() {
    // #4121: a lone transfer with `linked: true` promises a following
    // leg; the batch ending there leaves the chain open, so nothing
    // posts and no balance moves.
    let mut ledger = funded_ledger();
    let before = ledger.account_balance(USER_SRC).unwrap();
    let open = Transfer {
        linked: true,
        ..funding_transfer(700, USER_SRC, LP_SRC, 100, SRC)
    };
    let results = ledger.create_transfers(&[open]);
    assert_eq!(results, vec![Err(TransferError::LinkedEventChainOpen)]);
    assert!(ledger.lookup_transfer(700).is_none());
    assert_eq!(ledger.account_balance(USER_SRC).unwrap(), before);
}

#[test]
fn multi_leg_unterminated_chain_rejected_atomically() {
    // #4121: every leg of an open chain reports an error — the final
    // leg `LinkedEventChainOpen`, the earlier legs
    // `LinkedEventFailed` — and no leg is stored.
    let mut ledger = funded_ledger();
    let before_user = ledger.account_balance(USER_SRC).unwrap();
    let before_lp = ledger.account_balance(LP_SRC).unwrap();
    let chain = vec![
        Transfer {
            linked: true,
            ..funding_transfer(710, USER_SRC, LP_SRC, 100, SRC)
        },
        Transfer {
            linked: true,
            ..funding_transfer(711, USER_SRC, LP_SRC, 200, SRC)
        },
    ];
    let results = ledger.create_transfers(&chain);
    assert_eq!(
        results,
        vec![
            Err(TransferError::LinkedEventFailed),
            Err(TransferError::LinkedEventChainOpen),
        ]
    );
    assert!(ledger.lookup_transfer(710).is_none());
    assert!(ledger.lookup_transfer(711).is_none());
    assert_eq!(ledger.account_balance(USER_SRC).unwrap(), before_user);
    assert_eq!(ledger.account_balance(LP_SRC).unwrap(), before_lp);
}

#[test]
fn unterminated_chain_does_not_block_terminated_chains() {
    // #4121: the rejection is per-chain. A terminated chain earlier in
    // the same batch still posts, and a properly terminated chain
    // submitted in a later batch posts normally — including one that
    // reuses the rejected open chain's transfers, terminated.
    let mut ledger = funded_ledger();
    let batch = vec![
        Transfer {
            linked: true,
            ..funding_transfer(720, USER_SRC, LP_SRC, 100, SRC)
        },
        funding_transfer(721, USER_SRC, LP_SRC, 50, SRC),
        Transfer {
            linked: true,
            ..funding_transfer(722, USER_SRC, LP_SRC, 200, SRC)
        },
    ];
    let results = ledger.create_transfers(&batch);
    assert_eq!(results[0], Ok(()));
    assert_eq!(results[1], Ok(()));
    assert_eq!(results[2], Err(TransferError::LinkedEventChainOpen));
    assert!(ledger.lookup_transfer(720).is_some());
    assert!(ledger.lookup_transfer(721).is_some());
    assert!(ledger.lookup_transfer(722).is_none());

    let retry = ledger.create_transfers(&[funding_transfer(722, USER_SRC, LP_SRC, 200, SRC)]);
    assert_eq!(retry, vec![Ok(())]);
    assert!(ledger.lookup_transfer(722).is_some());
}

#[test]
fn linked_chain_with_exists_head_leg_posts_nothing_new() {
    // #6170: parity with TigerBeetle (and the tb_ledger compensation
    // contract, PR #5072). TigerBeetle breaks a linked chain on ANY
    // non-Created status: a mixed chain whose head leg is an
    // idempotent `Exists` resubmission and whose later legs are new
    // posts NOTHING new \u2014 the head reports `Exists`, the new legs
    // report `LinkedEventFailed`, and no balances move. The
    // whole-chain identical resubmit is the one exception: it stays
    // per-leg `Exists` (the surface the tb_ledger binding presents
    // after verifying every stored leg), and also moves nothing.
    let mut ledger = funded_ledger();
    let leg601 = Transfer {
        linked: true,
        ..funding_transfer(601, USER_SRC, LP_SRC, 100, SRC)
    };
    let leg602 = funding_transfer(602, LP_SRC, FEE, 100, SRC);
    let results = ledger.create_transfers(&[leg601.clone(), leg602.clone()]);
    assert_eq!(results, vec![Ok(()), Ok(())]);
    let user_after_chain = ledger.account_balance(USER_SRC).unwrap();
    let lp_after_chain = ledger.account_balance(LP_SRC).unwrap();
    let fee_after_chain = ledger.account_balance(FEE).unwrap();
    assert_eq!(user_after_chain.debits_posted, 100);
    assert_eq!(lp_after_chain.credits_posted, 100);
    assert_eq!(fee_after_chain.credits_posted, 100);

    // Mixed chain: leg 601 byte-identical (an `Exists` resubmission),
    // legs 603/604 genuinely new.
    let leg603 = Transfer {
        linked: true,
        ..funding_transfer(603, LP_SRC, USER_SRC, 50, SRC)
    };
    let leg604 = funding_transfer(604, USER_SRC, FEE, 50, SRC);
    let results = ledger.create_transfers(&[leg601.clone(), leg603, leg604]);
    assert_eq!(
        results,
        vec![
            Err(TransferError::Exists),
            Err(TransferError::LinkedEventFailed),
            Err(TransferError::LinkedEventFailed),
        ]
    );
    assert!(ledger.lookup_transfer(603).is_none());
    assert!(ledger.lookup_transfer(604).is_none());
    assert_eq!(ledger.account_balance(USER_SRC).unwrap(), user_after_chain);
    assert_eq!(ledger.account_balance(LP_SRC).unwrap(), lp_after_chain);
    assert_eq!(ledger.account_balance(FEE).unwrap(), fee_after_chain);

    // Control: the whole-chain identical resubmit stays all-`Exists`
    // and moves nothing.
    let results = ledger.create_transfers(&[leg601, leg602]);
    assert_eq!(
        results,
        vec![Err(TransferError::Exists), Err(TransferError::Exists)]
    );
    assert_eq!(ledger.account_balance(USER_SRC).unwrap(), user_after_chain);
    assert_eq!(ledger.account_balance(LP_SRC).unwrap(), lp_after_chain);
    assert_eq!(ledger.account_balance(FEE).unwrap(), fee_after_chain);
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
fn user_and_lp_source_must_be_distinct_before_persisting_intent() {
    let mut executor = executor();
    // Give the aliased source enough funds so an ordinary balance
    // failure cannot mask the same-account transfer defect.
    let funding = funding_transfer(902, ISSUER_SRC, LP_SRC, 10_000, SRC);
    assert_eq!(executor.backend.create_transfers(&[funding]), vec![Ok(())]);

    let mut aliased = request();
    aliased.user_source = LP_SRC;

    let error = executor
        .prepare(&aliased)
        .expect_err("T1 must reject a same-account transfer before writing an intent");
    assert!(error.to_string().contains("user source and LP source"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn fee_and_user_source_must_be_distinct_before_persisting_intent() {
    let mut executor = executor();
    let mut aliased = request();
    aliased.fee_account = Some(USER_SRC);

    let error = executor
        .prepare(&aliased)
        .expect_err("the fee leg must not debit and credit the same account");
    assert!(error.to_string().contains("user source and fee"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn lp_and_user_destination_must_be_distinct_before_persisting_intent() {
    let mut executor = executor();
    // Fund the aliased destination so the ordinary LP inventory check
    // cannot mask the same-account transfer defect.
    let funding = funding_transfer(901, ISSUER_DST, USER_DST, 50_000, DST);
    assert_eq!(executor.backend.create_transfers(&[funding]), vec![Ok(())]);

    let mut aliased = request();
    aliased.lp_dest = USER_DST;
    let error = executor
        .prepare(&aliased)
        .expect_err("T3 must reject a same-account transfer before writing an intent");
    assert!(
        error
            .to_string()
            .contains("LP destination and user destination")
    );
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn conversion_without_fee_still_posts() {
    let mut executor = executor();
    let mut no_fee = request();
    no_fee.transfer_ids = vec![604, 605];
    no_fee.fee_account = None;
    no_fee.fee_amount = 0;

    let record = executor
        .execute(&no_fee)
        .expect("distinct accounts must still allow a conversion without a fee leg");
    assert_eq!(record.state, IntentState::Posted);
    assert_eq!(record.legs.len(), 2);
    assert_eq!(executor.backend.transfers_with_user_data_128(777).len(), 2);
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
fn fee_without_account_fails_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut no_fee_account = request();
    no_fee_account.fee_account = None;

    let error = executor
        .prepare(&no_fee_account)
        .expect_err("a fee with no fee account must fail preflight");
    assert!(error.to_string().contains("requires a fee_account"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn too_few_transfer_ids_fail_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut short = request();
    short.transfer_ids = vec![601, 602];

    let error = executor
        .prepare(&short)
        .expect_err("two IDs cannot cover T1, the fee leg, and T3");
    assert!(error.to_string().contains("expected 3 IDs, got 2"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn extra_transfer_ids_fail_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut long = request();
    long.transfer_ids = vec![601, 602, 603, 604];

    let error = executor
        .prepare(&long)
        .expect_err("extra transfer IDs must not be silently dropped");
    assert!(error.to_string().contains("expected 3 IDs, got 4"));
    assert!(executor.store.get(777).unwrap().is_none());
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
}

#[test]
fn extra_transfer_ids_without_fee_fail_preflight_before_persisting_intent() {
    let mut executor = executor();
    let mut long = request();
    long.fee_amount = 0;
    long.fee_account = None;
    long.transfer_ids = vec![601, 602, 603];

    let error = executor
        .prepare(&long)
        .expect_err("a no-fee conversion has only two legs, so a third ID is extra");
    assert!(error.to_string().contains("expected 2 IDs, got 3"));
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

/// A bare `Recorded` intent with the given legs, bypassing the
/// executor's preflight so malformed leg vectors (#4133) can be
/// persisted the way a corrupt or legacy record would present them.
fn bare_record(conversion_id: u128, legs: Vec<Transfer>) -> IntentRecord {
    IntentRecord {
        conversion_id,
        legs,
        user_source: USER_SRC,
        lp_source: LP_SRC,
        fee_account: None,
        lp_dest: LP_DST,
        user_dest: USER_DST,
        source_ledger: SRC,
        dest_ledger: DST,
        amount_in: 0,
        amount_out: 0,
        fee_amount: 0,
        authorisation_id: "auth-bare".to_string(),
        state: IntentState::Recorded,
    }
}

#[test]
fn recovery_rejects_empty_leg_vector() {
    // An empty leg list satisfies the naive matched == len check
    // vacuously (0 == 0); it must never become Posted/AlreadyPosted
    // under either policy, and the record stays unresolved.
    let mut backend = funded_ledger();
    let mut store = InMemoryIntentStore::new();
    store.put(&bare_record(999, Vec::new())).unwrap();

    for policy in [RecoveryPolicy::Resubmit, RecoveryPolicy::Abandon] {
        let outcomes = reconcile(&mut backend, &mut store, policy).expect("reconcile");
        assert_eq!(outcomes, vec![(999, ReconcileOutcome::Inconsistent)]);
        assert_eq!(
            store.get(999).unwrap().unwrap().state,
            IntentState::Recorded,
            "an empty-leg record stays unresolved"
        );
    }
}

#[test]
fn recovery_rejects_duplicate_leg_ids_backed_by_one_transfer() {
    // The same leg twice, with only the one distinct transfer in the
    // ledger: per-entry counting would report 2 matched of 2 and mark
    // the record Posted off a single posting.
    let leg = funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC);
    let mut backend = funded_ledger();
    let mut store = InMemoryIntentStore::new();
    store
        .put(&bare_record(998, vec![leg.clone(), leg]))
        .expect("persist duplicate-leg record");

    let outcomes =
        reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit).expect("reconcile");
    assert_eq!(outcomes, vec![(998, ReconcileOutcome::Inconsistent)]);
    assert_eq!(
        store.get(998).unwrap().unwrap().state,
        IntentState::Recorded,
        "a duplicate-leg record stays unresolved"
    );
}

// --- #4125: one corrupt or foreign entry must not abort recovery ---

/// Seed a file store holding the standard Recorded intent (777) the
/// way the write-ahead test does, in a fresh temp directory.
fn file_store_with_recorded_intent(
    tag: &str,
) -> (PathBufForTests, FileIntentStore, InMemoryLedger) {
    let dir = std::env::temp_dir().join(format!(
        "id-currency-poc-intents-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileIntentStore::open(dir.clone()).expect("open store");
    let mut executor = ConversionExecutor::new(funded_ledger(), store);
    let record = executor.prepare(&request()).expect("intent prepared");
    assert_eq!(record.state, IntentState::Recorded);
    let backend = funded_ledger();
    let store = executor.store;
    (dir, store, backend)
}

type PathBufForTests = std::path::PathBuf;

#[test]
fn recovery_continues_past_a_corrupt_sibling_file() {
    let (dir, mut store, mut backend) = file_store_with_recorded_intent("corrupt");
    let corrupt = dir.join("deadbeef.json");
    std::fs::write(&corrupt, b"{not json").expect("write corrupt sibling");

    // The listing itself no longer aborts, and reports the bad entry.
    let listed = store.unresolved().expect("listing skips the corrupt file");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].conversion_id, 777);
    let unreadable = store.unreadable_entries();
    assert_eq!(unreadable.len(), 1);
    assert_eq!(unreadable[0].path, corrupt);

    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit)
        .expect("reconcile past the corrupt sibling");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    assert_eq!(backend.transfers_with_user_data_128(777).len(), 3);
    assert!(
        corrupt.exists(),
        "the scanner must not delete the corrupt entry an operator still needs to see"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_continues_past_a_json_named_subdirectory() {
    let (dir, mut store, mut backend) = file_store_with_recorded_intent("subdir");
    let subdir = dir.join("cafe.json");
    std::fs::create_dir(&subdir).expect("create *.json subdirectory");

    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit)
        .expect("reconcile past the *.json subdirectory");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    assert_eq!(backend.transfers_with_user_data_128(777).len(), 3);
    let unreadable = store.unreadable_entries();
    assert_eq!(unreadable.len(), 1);
    assert_eq!(unreadable[0].path, subdir);
    assert!(subdir.is_dir(), "the subdirectory stays in place");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- #4578: one filesystem identity contract for the intent store ---

fn empty_file_store(tag: &str) -> (PathBufForTests, FileIntentStore) {
    let dir = std::env::temp_dir().join(format!(
        "id-currency-poc-intents-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileIntentStore::open(dir.clone()).expect("open store");
    (dir, store)
}

fn canonical_name(conversion_id: u128) -> String {
    format!("{conversion_id:032x}.json")
}

fn json_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("read store dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
}

#[test]
fn duplicate_copies_of_one_intent_reconcile_exactly_once() {
    // The same record stored three times (canonical + two stray
    // copies, e.g. after an operator backup/restore) is ONE intent:
    // the listing yields it once and recovery reconciles it once.
    let (dir, mut store, mut backend) = file_store_with_recorded_intent("dup4578");
    let canonical = dir.join(canonical_name(777));
    let bytes = std::fs::read(&canonical).expect("read canonical intent");
    std::fs::write(dir.join("copy-a.json"), &bytes).expect("write copy a");
    std::fs::write(dir.join("copy-b.json"), &bytes).expect("write copy b");

    let listed = store.unresolved().expect("listing with duplicates");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].conversion_id, 777);
    assert!(
        store.unreadable_entries().is_empty(),
        "identical duplicates are not unreadable entries"
    );

    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit)
        .expect("reconcile with duplicates");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    assert_eq!(backend.transfers_with_user_data_128(777).len(), 3);

    // The state-advancing put canonicalized the store: the stray
    // copies of the superseded record are gone, one file remains.
    assert_eq!(json_files(&dir), vec![canonical]);
    assert_eq!(
        store
            .get(777)
            .expect("get after reconcile")
            .map(|r| r.state),
        Some(IntentState::Posted)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn misnamed_record_is_found_by_get_and_blocks_conflicting_prepare() {
    // A parseable record stored ONLY under a non-canonical name is
    // still the conversion's record: get() must find it, so
    // prepare()'s never-replace guard cannot be bypassed into
    // writing a conflicting canonical twin.
    let (dir, mut store) = empty_file_store("misnamed4578");
    let original = bare_record(
        9,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&original).expect("seed put");
    let canonical = dir.join(canonical_name(9));
    let misnamed = dir.join("misnamed.json");
    std::fs::rename(&canonical, &misnamed).expect("rename to a non-canonical name");

    assert_eq!(store.get(9).expect("get misnamed"), Some(original.clone()));

    let mut executor = ConversionExecutor::new(funded_ledger(), store);
    let mut conflicting = request();
    conflicting.conversion_id = 9;
    conflicting.transfer_ids = vec![701, 702, 703];
    conflicting.amount_in = 5_000;
    let error = executor
        .prepare(&conflicting)
        .expect_err("conflicting prepare must be refused");
    assert!(
        error.to_string().contains("already has a persisted intent"),
        "unexpected prepare error: {error}"
    );
    // No canonical twin was written; the misnamed record is intact.
    assert_eq!(json_files(&dir), vec![misnamed]);
    assert_eq!(
        executor.store.get(9).expect("get after refusal"),
        Some(original)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn misnamed_only_record_refuses_conflicting_put_and_canonicalizes_matching_put() {
    let (dir, mut store) = empty_file_store("put4578");
    let original = bare_record(
        9,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&original).expect("seed put");
    let canonical = dir.join(canonical_name(9));
    let misnamed = dir.join("stray.json");
    std::fs::rename(&canonical, &misnamed).expect("rename to a non-canonical name");

    // A conflicting put over the misnamed record fails closed and
    // writes nothing.
    let conflicting = bare_record(
        9,
        vec![funding_transfer(502, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    let error = store
        .put(&conflicting)
        .expect_err("conflicting put must be refused");
    assert!(
        error.to_string().contains("refusing to write conversion 9"),
        "unexpected put error: {error}"
    );
    assert_eq!(json_files(&dir), vec![misnamed.clone()]);

    // A state-advancing put of the SAME record succeeds and
    // canonicalizes: the record now lives only at its canonical
    // path.
    let mut advanced = original.clone();
    advanced.state = IntentState::Posted;
    store.put(&advanced).expect("state-advancing put");
    assert_eq!(json_files(&dir), vec![canonical]);
    assert_eq!(store.get(9).expect("get canonicalized"), Some(advanced));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn conflicting_copies_fail_closed_and_are_reported() {
    // Two DIFFERENT records stored under one conversion_id: the
    // identity is ambiguous. get()/put() fail closed, the listing
    // skips the conversion and reports both copies, and other
    // conversions reconcile normally.
    let (dir, mut store, mut backend) = file_store_with_recorded_intent("conflict4578");
    let record_a = bare_record(
        11,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&record_a).expect("seed canonical copy");
    let canonical_11 = dir.join(canonical_name(11));
    let record_b = bare_record(
        11,
        vec![funding_transfer(502, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    let (other_dir, mut other_store) = empty_file_store("conflict4578-b");
    other_store.put(&record_b).expect("seed other copy");
    let other_copy = dir.join("other.json");
    std::fs::copy(other_dir.join(canonical_name(11)), &other_copy).expect("plant conflicting copy");
    let _ = std::fs::remove_dir_all(&other_dir);

    let get_error = store.get(11).expect_err("ambiguous get must fail closed");
    assert!(
        get_error.to_string().contains("conflicting records"),
        "unexpected get error: {get_error}"
    );
    let mut advanced_a = record_a.clone();
    advanced_a.state = IntentState::Posted;
    assert!(
        store.put(&advanced_a).is_err(),
        "ambiguous put must fail closed"
    );

    let listed = store
        .unresolved()
        .expect("listing with a conflicted conversion");
    assert_eq!(
        listed.iter().map(|r| r.conversion_id).collect::<Vec<_>>(),
        vec![777],
        "the conflicted conversion is skipped, the good one is listed"
    );
    let reported: Vec<_> = store
        .unreadable_entries()
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert!(
        reported.contains(&canonical_11),
        "canonical copy reported: {reported:?}"
    );
    assert!(
        reported.contains(&other_copy),
        "stray copy reported: {reported:?}"
    );

    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Resubmit)
        .expect("reconcile past the conflicted conversion");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Resubmitted)]);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- #4578 / PR #4622 Codex re-review: casing aliases of the
// --- canonical name (P1/P2) ---

#[test]
fn uppercase_extension_alias_is_scanned_and_blocks_conflicting_put() {
    // A record stored under the canonical stem with an UPPERCASE
    // extension is still the conversion's record (Codex P1 on
    // PR #4622): the scan must not skip it, or get() reads the
    // conversion as absent and a conflicting put overwrites it.
    let (dir, mut store) = empty_file_store("upext4578");
    let original = bare_record(
        10,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&original).expect("seed put");
    let alias = dir.join(format!("{:032x}.JSON", 10));
    std::fs::rename(dir.join(canonical_name(10)), &alias).expect("rename to .JSON alias");

    assert_eq!(
        store.get(10).expect("get via .JSON alias"),
        Some(original.clone())
    );
    let listed = store.unresolved().expect("listing via .JSON alias");
    assert_eq!(listed, vec![original.clone()]);

    let conflicting = bare_record(
        10,
        vec![funding_transfer(601, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    let error = store
        .put(&conflicting)
        .expect_err("conflicting put over the .JSON alias must fail closed");
    assert!(
        error
            .to_string()
            .contains("refusing to write conversion 10"),
        "unexpected put error: {error}"
    );
    // The alias still holds the original record, byte for byte.
    assert_eq!(store.get(10).expect("get after refusal"), Some(original));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_uppercase_canonical_spelling_fails_get_closed() {
    // A corrupt file at the canonical name spelled with UPPERCASE
    // hex digits is still the conversion's canonical file (Codex
    // P2 on PR #4622): get() must fail closed on it, not read the
    // conversion as absent and let prepare() overwrite it.
    let (dir, store) = empty_file_store("uphex4578");
    let alias = dir.join(format!("{:032X}.json", 10));
    std::fs::write(&alias, b"{ not an intent record").expect("plant corrupt alias");

    let error = store
        .get(10)
        .expect_err("get with an unreadable uppercase canonical file must fail closed");
    assert!(
        error.to_string().contains("is unreadable"),
        "unexpected get error: {error}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- #4578 / PR #4622 Codex findings: canonical occupancy (P1) and
// --- state-only divergence after an interrupted canonicalization (P2)

#[test]
fn canonical_file_holding_another_conversions_record_blocks_get_and_put() {
    // Conversion 9's only record sits under conversion 10's canonical
    // file name (a misnamed write / operator restore). Conversion 10
    // must NOT read as absent (that would pass prepare()'s
    // never-replace guard) and must NOT be written over the occupant.
    let (dir, mut store) = empty_file_store("occupant4578");
    let occupant = bare_record(
        9,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&occupant).expect("seed occupant");
    let misplaced = dir.join(canonical_name(10));
    std::fs::rename(dir.join(canonical_name(9)), &misplaced).expect("misplace occupant");
    let occupant_bytes = std::fs::read(&misplaced).expect("read occupant bytes");

    let get_error = store.get(10).expect_err("occupied get must fail closed");
    assert!(
        get_error
            .to_string()
            .contains("currently stores conversion 9"),
        "unexpected get error: {get_error}"
    );
    let intruder = bare_record(
        10,
        vec![funding_transfer(601, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    let put_error = store
        .put(&intruder)
        .expect_err("occupied put must fail closed");
    assert!(
        put_error
            .to_string()
            .contains("currently stores conversion 9"),
        "unexpected put error: {put_error}"
    );
    assert_eq!(
        std::fs::read(&misplaced).expect("re-read occupant"),
        occupant_bytes,
        "the occupant's only record is byte-identical after both refusals"
    );
    // The occupant itself is unaffected: content identity still finds
    // it, and the listing yields it exactly once.
    assert_eq!(store.get(9).expect("get occupant"), Some(occupant));
    let listed = store.unresolved().expect("listing with an occupant");
    assert_eq!(
        listed.iter().map(|r| r.conversion_id).collect::<Vec<_>>(),
        vec![9]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn state_only_divergent_copies_resolve_to_the_most_advanced_state() {
    // A state-advancing put writes the new canonical record BEFORE
    // removing superseded copies; a crash in between leaves copies
    // that differ only in state. That residue is one record at its
    // most advanced state — readable and listed once — not an
    // ambiguous identity that bricks the conversion.
    let (dir, mut store) = empty_file_store("states4578");
    let recorded = bare_record(
        11,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&recorded).expect("seed recorded");
    // Plant the post-crash residue by hand: the canonical file holds
    // the Submitted copy, a stray still holds the Recorded copy.
    let mut submitted = recorded.clone();
    submitted.state = IntentState::Submitted;
    let (other_dir, mut other_store) = empty_file_store("states4578-b");
    other_store.put(&submitted).expect("seed submitted");
    std::fs::copy(
        other_dir.join(canonical_name(11)),
        dir.join(canonical_name(11)),
    )
    .expect("plant advanced canonical copy");
    let _ = std::fs::remove_dir_all(&other_dir);
    std::fs::copy(dir.join(canonical_name(11)), dir.join("stray.json")).expect("plant stray copy");
    // Downgrade the stray to the Recorded copy's bytes: rewrite it
    // from a fresh store holding only the Recorded record.
    let (old_dir, mut old_store) = empty_file_store("states4578-c");
    old_store.put(&recorded).expect("seed recorded copy");
    std::fs::copy(old_dir.join(canonical_name(11)), dir.join("stray.json"))
        .expect("plant recorded stray copy");
    let _ = std::fs::remove_dir_all(&old_dir);

    assert_eq!(store.get(11).expect("get diverged copies"), Some(submitted));
    let listed = store.unresolved().expect("listing diverged copies");
    assert_eq!(listed.len(), 1, "one conversion, listed once");
    assert_eq!(listed[0].state, IntentState::Submitted);
    // And a state-advancing put still canonicalizes the residue away.
    let mut posted = recorded.clone();
    posted.state = IntentState::Posted;
    store.put(&posted).expect("advancing put over the residue");
    assert_eq!(json_files(&dir), vec![dir.join(canonical_name(11))]);
    assert_eq!(store.get(11).expect("get canonicalized"), Some(posted));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- #4578 / PR #4622 Codex re-review: alias cleanup (P1) and
// --- canonically blocked identities in recovery listing (P2)

#[test]
fn put_never_deletes_a_filesystem_alias_of_the_canonical_file() {
    // A stray path that NAMES the canonical file (here a symlink to
    // it; on a case-insensitive filesystem the uppercase-hex spelling
    // of the canonical name is the same situation) must never be a
    // cleanup target: after the state-advancing put renames the new
    // record over the canonical path, the alias resolves to the NEW
    // record, so removing it would delete the record just persisted
    // while `put` reports success.
    let (dir, mut store) = empty_file_store("alias4578");
    let recorded = bare_record(
        10,
        vec![funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&recorded).expect("seed recorded");
    let canonical = dir.join(canonical_name(10));
    let alias = dir.join(format!("{:032X}.json", 10));
    std::os::unix::fs::symlink(&canonical, &alias).expect("plant alias of canonical");

    let mut submitted = recorded.clone();
    submitted.state = IntentState::Submitted;
    store
        .put(&submitted)
        .expect("state-advancing put over an alias");

    assert_eq!(
        store.get(10).expect("get after alias put"),
        Some(submitted),
        "the persisted record survives the cleanup pass"
    );
    assert!(
        alias.exists(),
        "the alias itself is left in place, never deleted as a stray copy"
    );
    // The listing still collapses the alias to the one conversion.
    let listed = store.unresolved().expect("listing with an alias");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].conversion_id, 10);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn canonically_blocked_conversion_is_skipped_and_reported_during_listing() {
    // Conversion 1's only record sits in stray.json, while conversion
    // 1's canonical file stores conversion 2's record: get(1)/put(1)
    // fail closed on the occupancy, so a listing that yielded
    // conversion 1 would hand recovery a record whose
    // state-advancing put errors out and aborts the whole scan. The
    // listing must skip and report the blocked conversion instead,
    // and still reconcile the unblocked one.
    let (dir, mut store) = empty_file_store("blocked4578");
    let blocked = bare_record(
        1,
        vec![funding_transfer(9501, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&blocked).expect("seed blocked record");
    let stray = dir.join("stray.json");
    std::fs::rename(dir.join(canonical_name(1)), &stray).expect("strand blocked record");
    let occupant = bare_record(
        2,
        vec![funding_transfer(9502, ISSUER_SRC, USER_SRC, 10_000, SRC)],
    );
    store.put(&occupant).expect("seed occupant record");
    std::fs::rename(dir.join(canonical_name(2)), dir.join(canonical_name(1)))
        .expect("occupy conversion 1's canonical file");

    let listed = store
        .unresolved()
        .expect("listing with a blocked conversion");
    assert_eq!(
        listed.iter().map(|r| r.conversion_id).collect::<Vec<_>>(),
        vec![2],
        "only the unblocked conversion is listed"
    );
    let skipped = store.unreadable_entries();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].path, dir.join(canonical_name(1)));
    assert!(
        skipped[0].reason.contains("blocked") && skipped[0].reason.contains("conversion 2"),
        "unexpected skip reason: {}",
        skipped[0].reason
    );

    // Recovery reconciles the unblocked conversion and never touches
    // the blocked conversion's stranded copy.
    let mut backend = funded_ledger();
    let outcomes = reconcile(&mut backend, &mut store, RecoveryPolicy::Abandon)
        .expect("reconcile past the blocked conversion");
    assert_eq!(outcomes, vec![(2, ReconcileOutcome::Abandoned)]);
    assert!(stray.exists(), "the blocked record's copy stays on disk");
    let _ = std::fs::remove_dir_all(&dir);
}
// ---- Live-cluster scenarios (#2289 acceptance item 1) ----
//
// The scenarios above run against InMemoryLedger; the ones below run
// the same acceptance paths against a real single-replica cluster
// (super::harness::TestCluster) through the official-client binding
// (super::tb_ledger::TigerBeetleLedger). Ignored by default: they need
// the pinned TigerBeetle binary. Run with:
//   TIGERBEETLE_POC_BINARY=<path> cargo test -p id --features currency-poc-tb \
//     currency_poc::tests::live -- --ignored

#[cfg(feature = "currency-poc-tb")]
use super::harness::TestCluster;
#[cfg(feature = "currency-poc-tb")]
use super::tb_ledger::TigerBeetleLedger;

#[cfg(feature = "currency-poc-tb")]
fn funded_tb_ledger(address: &str) -> TigerBeetleLedger {
    let mut ledger = TigerBeetleLedger::connect(address).expect("connect to live cluster");
    let results = ledger.create_accounts(&[
        account(ISSUER_SRC, SRC, false),
        account(ISSUER_DST, DST, false),
        account(USER_SRC, SRC, true),
        account(LP_SRC, SRC, false),
        account(FEE, SRC, false),
        account(LP_DST, DST, true),
        account(USER_DST, DST, false),
    ]);
    assert!(
        results.iter().all(|result| result.is_ok()),
        "live account creation failed: {results:?}"
    );
    let results = ledger.create_transfers(&[
        funding_transfer(501, ISSUER_SRC, USER_SRC, 10_000, SRC),
        funding_transfer(502, ISSUER_DST, LP_DST, 50_000, DST),
    ]);
    assert!(
        results.iter().all(|result| result.is_ok()),
        "live funding transfers failed: {results:?}"
    );
    ledger
}

#[cfg(feature = "currency-poc-tb")]
type LiveExecutor = ConversionExecutor<TigerBeetleLedger, InMemoryIntentStore>;

#[cfg(feature = "currency-poc-tb")]
fn live_executor(port: u16) -> (TestCluster, LiveExecutor) {
    let cluster = TestCluster::start(port).expect("live cluster should start");
    let ledger = funded_tb_ledger(&cluster.address);
    (
        cluster,
        ConversionExecutor::new(ledger, InMemoryIntentStore::new()),
    )
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_posted_conversion_legs_findable_by_user_data_128() {
    let (_cluster, mut executor) = live_executor(30021);
    let record = executor.execute(&request()).expect("conversion posts");
    assert_eq!(record.state, IntentState::Posted);
    assert_eq!(record.amount_out, 2_000);

    let legs = executor.backend.transfers_with_user_data_128(777);
    assert_eq!(legs.len(), 3);
    assert!(legs.iter().all(|leg| leg.user_data_128 == 777));

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

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_atomic_failure_moves_nothing() {
    let (_cluster, mut executor) = live_executor(30022);
    let mut record = executor.prepare(&request()).expect("intent prepared");

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
    assert_eq!(record.state, IntentState::Submitted);
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_failed_conversion_recovery_reports_resubmission_rejected() {
    let (_cluster, mut executor) = live_executor(30025);
    let mut record = executor.prepare(&request()).expect("intent prepared");

    // Same insufficient-liquidity failure as
    // live_atomic_failure_moves_nothing: the linked batch fails, no
    // leg is stored, and the intent stays Submitted.
    let drain = funding_transfer(900, LP_DST, USER_DST, 49_000, DST);
    assert!(executor.backend.create_transfers(&[drain])[0].is_ok());
    assert!(executor.submit(&mut record).is_err());
    assert_eq!(record.state, IntentState::Submitted);

    // TigerBeetle remembers the failed transfer IDs, so the
    // recovery resubmission is refused (id_already_failed). That
    // must surface as ResubmissionRejected for this record — not a
    // panic in the binding — and the record stays unresolved.
    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Resubmit,
    )
    .expect("reconcile");
    assert_eq!(
        outcomes,
        vec![(777, ReconcileOutcome::ResubmissionRejected)]
    );
    let stored = executor
        .store
        .get(777)
        .expect("read intent")
        .expect("intent still stored");
    assert_eq!(stored.state, IntentState::Submitted);
    assert!(
        executor
            .backend
            .transfers_with_user_data_128(777)
            .is_empty()
    );
    assert_eq!(
        executor
            .backend
            .account_balance(USER_DST)
            .unwrap()
            .credits_posted,
        49_000,
        "only the drain posted; the failed conversion moved nothing"
    );
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_zero_user_data_128_query_matches_exactly() {
    let (_cluster, mut executor) = live_executor(30026);
    let record = executor.execute(&request()).expect("conversion posts");
    assert_eq!(record.state, IntentState::Posted);

    // Funding transfers are stamped user_data_128 = 0. TigerBeetle
    // treats a zero query filter as "no filter", so the binding
    // must post-filter exactly: only the two funding transfers
    // match, never the conversion legs stamped 777.
    let zero = executor.backend.transfers_with_user_data_128(0);
    assert_eq!(zero.len(), 2);
    assert!(zero.iter().all(|transfer| transfer.user_data_128 == 0));
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_codes_zero_and_one_stay_distinct() {
    let (_cluster, mut executor) = live_executor(30027);
    let backend = &mut executor.backend;

    // Accounts: code 0 is created; the same ID retried with code 1
    // is a different record, not an idempotent Exists.
    let account_zero = account(424_242, SRC, false);
    assert_eq!(account_zero.code, 0);
    assert!(backend.create_accounts(&[account_zero.clone()])[0].is_ok());
    let mut account_one = account_zero.clone();
    account_one.code = 1;
    assert_eq!(
        backend.create_accounts(&[account_one])[0],
        Err(AccountError::ExistsWithDifferentFields)
    );
    // The rejected retry must not rewrite what lookups report.
    assert_eq!(backend.lookup_account(424_242).unwrap().code, 0);

    // Transfers: the same rule on a funded pair.
    let transfer_zero = funding_transfer(910, ISSUER_SRC, USER_SRC, 10, SRC);
    assert!(backend.create_transfers(&[transfer_zero.clone()])[0].is_ok());
    let mut transfer_one = transfer_zero;
    transfer_one.code = 1;
    assert_eq!(
        backend.create_transfers(&[transfer_one])[0],
        Err(TransferError::ExistsWithDifferentFields)
    );
    assert_eq!(backend.lookup_transfer(910).unwrap().code, 0);
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_crash_after_submission_recovers_without_double_posting() {
    let (_cluster, mut executor) = live_executor(30023);
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
}

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_crash_before_submission_resubmits_same_ids() {
    let (_cluster, mut executor) = live_executor(30024);
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

#[cfg(feature = "currency-poc-tb")]
#[test]
#[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
fn live_same_request_twice_posts_once() {
    // #4678: an identical execute() retry against a Posted record is
    // refused explicitly — terminal intents are final, so the retry
    // posts nothing a second time. The PR #5072 binding behaviour
    // this test originally pinned (resubmitting an already-posted
    // linked chain is an idempotent success at the backend:
    // TigerBeetle reports Exists for the first leg and
    // LinkedEventFailed for the rest, which the binding translates
    // back to per-leg Exists after verifying the stored legs) is
    // exercised directly through the backend below instead.
    let (_cluster, mut executor) = live_executor(30028);
    let first = executor.execute(&request()).expect("first execution posts");
    assert_eq!(first.state, IntentState::Posted);
    let balances_after_first = (
        executor.backend.account_balance(USER_SRC).unwrap(),
        executor.backend.account_balance(USER_DST).unwrap(),
        executor.backend.account_balance(LP_DST).unwrap(),
    );

    let error = executor
        .execute(&request())
        .expect_err("same-request retry of a Posted conversion is refused (#4678)");
    assert!(
        format!("{error:#}").contains("Posted"),
        "retry error names the terminal state: {error:#}"
    );
    let resubmitted = executor.backend.create_transfers(&first.legs);
    assert!(
        resubmitted
            .iter()
            .all(|result| matches!(result, Ok(()) | Err(TransferError::Exists))),
        "backend resubmission of the posted chain stays idempotent: {resubmitted:?}"
    );
    assert_eq!(
        executor.backend.account_balance(USER_SRC).unwrap(),
        balances_after_first.0,
        "retry must not move balances a second time"
    );
    assert_eq!(
        executor.backend.account_balance(USER_DST).unwrap(),
        balances_after_first.1
    );
    assert_eq!(
        executor.backend.account_balance(LP_DST).unwrap(),
        balances_after_first.2
    );
    assert_eq!(executor.backend.transfers_with_user_data_128(777).len(), 3);
}

// --- #4678: terminal intent states are final -------------------------

#[test]
fn submit_refuses_abandoned_record() {
    // A record the recovery scanner abandoned must never reach the
    // ledger via a direct submit: the refusal happens before any
    // state write or ledger call.
    let mut executor = executor();
    let mut record = executor.prepare(&request()).expect("prepare");
    record.state = IntentState::Abandoned;
    executor.store.put(&record).expect("persist Abandoned");

    let error = executor
        .submit(&mut record)
        .expect_err("submit must refuse an Abandoned record");
    let message = format!("{error:#}");
    assert!(
        message.contains("777") && message.contains("Abandoned"),
        "error names the conversion and its state: {message}"
    );
    assert_eq!(record.state, IntentState::Abandoned);
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Abandoned,
        "the persisted record stays Abandoned"
    );
    for leg_id in [601, 602, 603] {
        assert!(
            executor.backend.lookup_transfer(leg_id).is_none(),
            "no conversion leg may be posted for an Abandoned intent"
        );
    }
}

#[test]
fn abandoned_intent_stays_abandoned_on_identical_execute_retry() {
    // The #4678 resurrection path: prepare persists the intent, the
    // scanner abandons it, and an identical execute() retry used to
    // post the legs anyway. The retry must fail, the state must stay
    // Abandoned, and no leg may reach the ledger.
    let mut executor = executor();
    let req = request();
    executor.prepare(&req).expect("prepare");

    let outcomes = reconcile(
        &mut executor.backend,
        &mut executor.store,
        RecoveryPolicy::Abandon,
    )
    .expect("reconcile");
    assert_eq!(outcomes, vec![(777, ReconcileOutcome::Abandoned)]);
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Abandoned
    );

    let error = executor
        .execute(&req)
        .expect_err("an identical retry must not resurrect an Abandoned intent");
    assert!(
        format!("{error:#}").contains("Abandoned"),
        "retry error names the terminal state: {error:#}"
    );
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Abandoned,
        "the persisted record stays Abandoned after the retry"
    );
    for leg_id in [601, 602, 603] {
        assert!(
            executor.backend.lookup_transfer(leg_id).is_none(),
            "no conversion leg may be posted by the retry"
        );
    }
}

#[test]
fn execute_retry_on_posted_record_fails_explicitly() {
    // Already-Posted execute behavior is deliberate: an identical
    // retry fails explicitly (submit refuses terminal records) and
    // changes nothing — no resubmission, no state change.
    let mut executor = executor();
    let req = request();
    let posted = executor.execute(&req).expect("first execute");
    assert_eq!(posted.state, IntentState::Posted);

    let error = executor
        .execute(&req)
        .expect_err("a retry of a Posted conversion must fail explicitly");
    assert!(
        format!("{error:#}").contains("Posted"),
        "retry error names the terminal state: {error:#}"
    );
    assert_eq!(
        executor.store.get(777).unwrap().unwrap().state,
        IntentState::Posted,
        "the persisted record stays Posted after the retry"
    );
}
