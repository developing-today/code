//! Ledger types and the backend seam for the conversion-executor PoC.
//!
//! The types mirror TigerBeetle's data model (see the #2040 backend
//! investigation, `doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend`):
//! a currency is a bare `ledger: u32`, amounts are `u128`, and the link
//! between a conversion and its transfers is stamped into
//! `user_data_128`, because TigerBeetle does not persist linked-chain
//! membership itself.
//!
//! [`LedgerBackend`] is the seam the executor works against.
//! [`InMemoryLedger`] implements the TigerBeetle semantics the PoC
//! depends on — atomic linked chains, idempotent transfer IDs, and
//! balance-invariant flags — so the scenario tests in `tests` run
//! anywhere. A production binding would implement the same trait over
//! the official TigerBeetle Rust client against the test cluster
//! managed by `super::harness`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// TigerBeetle account ID (128-bit).
pub type AccountId = u128;
/// TigerBeetle transfer ID (128-bit), generated and persisted by the
/// originating client before submission (write-ahead discipline).
pub type TransferId = u128;

/// Account flags the PoC relies on (a subset of TigerBeetle's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFlags {
    /// Reject a transfer whose posting would make debits exceed credits.
    pub debits_must_not_exceed_credits: bool,
    /// Reject a transfer whose posting would make credits exceed debits.
    pub credits_must_not_exceed_debits: bool,
}

/// A TigerBeetle-shaped account. `code` is the application chart-of-
/// accounts code (issuer / liquidity / user / fee, per #2063).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub ledger: u32,
    pub code: u16,
    pub flags: AccountFlags,
}

/// A TigerBeetle-shaped transfer. When `linked` is set on every leg of
/// a batch except the last, the batch is one linked chain: either all
/// legs post or none do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transfer {
    pub id: TransferId,
    pub debit_account_id: AccountId,
    pub credit_account_id: AccountId,
    pub amount: u128,
    pub ledger: u32,
    /// The conversion ID on every leg of a conversion (see module docs).
    pub user_data_128: u128,
    pub code: u16,
    pub linked: bool,
}

/// Posted balance totals for one account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccountBalance {
    pub debits_posted: u128,
    pub credits_posted: u128,
}

impl AccountBalance {
    /// Funds available to spend under a debits-must-not-exceed-credits
    /// reading: credits received minus debits already posted.
    pub fn spendable(&self) -> u128 {
        self.credits_posted.saturating_sub(self.debits_posted)
    }
}

/// Per-account creation failures (subset of TigerBeetle's result codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    /// An account with this ID already exists with identical fields —
    /// idempotent retry, treated as success by callers.
    Exists,
    /// An account with this ID exists but with different fields.
    ExistsWithDifferentFields,
    /// `ledger` must be non-zero.
    LedgerMustNotBeZero,
    /// An account ID must be non-zero.
    IdMustNotBeZero,
    /// An account ID must not be `u128::MAX`.
    IdMustNotBeIntMax,
    /// The two mutually exclusive account guard flags were both set.
    FlagsAreMutuallyExclusive,
    /// TigerBeetle returned a status this adapter does not model.
    UnsupportedStatus,
}

/// Per-transfer failures (subset of TigerBeetle's result codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    /// A transfer with this ID already exists with identical fields —
    /// idempotent retry, treated as success by callers. This is the
    /// property that makes crash-recovery resubmission safe.
    Exists,
    /// A transfer with this ID exists but with different fields.
    ExistsWithDifferentFields,
    /// The transfer ID belongs to a transfer that already failed:
    /// TigerBeetle remembers failed transfer IDs and refuses their
    /// resubmission (`id_already_failed`). A recovery resubmission
    /// meeting this is a rejected resubmission, not a posting.
    IdAlreadyFailed,
    /// Another leg of the same linked chain failed; this leg did not post.
    LinkedEventFailed,
    /// The batch ended while this leg's linked chain was still open:
    /// this leg is the final leg of the batch and still has
    /// `linked == true`, so the chain it promised never terminated
    /// (mirrors TigerBeetle's `linked_event_chain_open`).
    LinkedEventChainOpen,
    DebitAccountNotFound,
    CreditAccountNotFound,
    AccountsMustBeDifferent,
    AmountMustNotBeZero,
    /// A transfer's ledger must match both accounts' ledgers;
    /// cross-ledger transfers are forbidden (conversions are composed
    /// pairs of same-ledger transfers, per the #2040 investigation).
    LedgerMismatch,
    DebitsMustNotExceedCredits,
    CreditsMustNotExceedDebits,
    /// A transfer ID must be non-zero.
    IdMustNotBeZero,
    /// A transfer ID must not be `u128::MAX`.
    IdMustNotBeIntMax,
    /// A debit account ID must be non-zero.
    DebitAccountIdMustNotBeZero,
    /// A debit account ID must not be `u128::MAX`.
    DebitAccountIdMustNotBeIntMax,
    /// A credit account ID must be non-zero.
    CreditAccountIdMustNotBeZero,
    /// A credit account ID must not be `u128::MAX`.
    CreditAccountIdMustNotBeIntMax,
    /// The debit account has been closed.
    DebitAccountAlreadyClosed,
    /// The credit account has been closed.
    CreditAccountAlreadyClosed,
    /// TigerBeetle returned a status this adapter does not model.
    UnsupportedStatus,
}

/// The ledger operations the conversion executor needs.
pub trait LedgerBackend {
    fn create_accounts(&mut self, accounts: &[Account]) -> Vec<Result<(), AccountError>>;
    /// Submit a batch. The returned vector carries exactly one
    /// result per submitted transfer, in submission order — callers
    /// treat any other length as a backend failure, never as
    /// success (#4669). Contiguous runs ending at a transfer with
    /// `linked == false` form linked chains and post atomically.
    ///
    /// A batch whose final chain is unterminated — its last transfer
    /// still has `linked == true` — is rejected atomically for that
    /// chain: no leg of the open chain posts and no balance moves.
    /// Every leg of the open chain reports an error: the final leg
    /// reports [`TransferError::LinkedEventChainOpen`] and every
    /// earlier leg reports [`TransferError::LinkedEventFailed`].
    /// Terminated chains earlier in the same batch are unaffected,
    /// and the transfers of a rejected open chain may be resubmitted
    /// (properly terminated) in a later batch. A production binding
    /// must implement the same rule.
    ///
    /// `Exists` legs inside a linked chain (#6170): TigerBeetle
    /// breaks a linked chain on ANY non-`Created` status, so a chain
    /// that mixes an idempotent `Exists` leg with new legs posts
    /// NOTHING \u2014 the `Exists` leg reports
    /// [`TransferError::Exists`] and every other leg reports
    /// [`TransferError::LinkedEventFailed`]. The single exception is
    /// a chain whose legs are ALL identical resubmissions: it
    /// reports per-leg `Exists` and moves nothing (the surface the
    /// `tb_ledger` binding presents after verifying every stored
    /// leg of a resubmitted chain, PR #5072). A lone unlinked
    /// transfer is a chain of one: `Exists` stays `Exists`, a new
    /// transfer posts as before.
    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>>;
    fn lookup_transfer(&self, id: TransferId) -> Option<Transfer>;
    fn lookup_account(&self, id: AccountId) -> Option<Account>;
    fn account_balance(&self, id: AccountId) -> Option<AccountBalance>;
    /// All posted transfers stamped with the given `user_data_128`
    /// (the conversion lookup the acceptance criteria ask for).
    fn transfers_with_user_data_128(&self, value: u128) -> Vec<Transfer>;
}

/// In-memory backend implementing the TigerBeetle semantics listed in
/// the module docs. Used by the PoC scenario tests and usable as a
/// reference for a real client binding.
#[derive(Default)]
pub struct InMemoryLedger {
    accounts: HashMap<AccountId, Account>,
    balances: HashMap<AccountId, AccountBalance>,
    transfers: HashMap<TransferId, Transfer>,
    failed_transfers: HashMap<TransferId, TransferError>,
    transfer_order: Vec<TransferId>,
}

impl InMemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate one transfer against a scratch balance map; on success
    /// apply its balance effects to the scratch map. Nothing touches
    /// the real state until a whole chain validates.
    fn validate_and_stage(
        &self,
        transfer: &Transfer,
        staged: &mut HashMap<AccountId, AccountBalance>,
        staged_transfers: &HashMap<TransferId, Transfer>,
    ) -> Result<(), TransferError> {
        // A transfer ID previously recorded as failed is refused
        // with `IdAlreadyFailed`, matching the variant's contract
        // above and the live binding's status mapping (tb_ledger.rs)
        // -- never the stored original error, which goes stale once
        // its cause is fixed (#6628).
        if self.failed_transfers.contains_key(&transfer.id) {
            return Err(TransferError::IdAlreadyFailed);
        }

        if let Some(existing) = self.transfers.get(&transfer.id) {
            return if existing == transfer {
                Err(TransferError::Exists)
            } else {
                Err(TransferError::ExistsWithDifferentFields)
            };
        }
        // A transfer ID may appear at most once per chain: without
        // this check two legs sharing an ID would both stage balance
        // changes while only the first is journaled at commit.
        if let Some(staged_transfer) = staged_transfers.get(&transfer.id) {
            return if staged_transfer == transfer {
                Err(TransferError::Exists)
            } else {
                Err(TransferError::ExistsWithDifferentFields)
            };
        }
        if transfer.amount == 0 {
            return Err(TransferError::AmountMustNotBeZero);
        }
        if transfer.debit_account_id == transfer.credit_account_id {
            return Err(TransferError::AccountsMustBeDifferent);
        }
        let debit = self
            .accounts
            .get(&transfer.debit_account_id)
            .ok_or(TransferError::DebitAccountNotFound)?;
        let credit = self
            .accounts
            .get(&transfer.credit_account_id)
            .ok_or(TransferError::CreditAccountNotFound)?;
        if transfer.ledger == 0
            || transfer.ledger != debit.ledger
            || transfer.ledger != credit.ledger
        {
            return Err(TransferError::LedgerMismatch);
        }
        let debit_balance = staged
            .get(&transfer.debit_account_id)
            .copied()
            .unwrap_or_default();
        let credit_balance = staged
            .get(&transfer.credit_account_id)
            .copied()
            .unwrap_or_default();
        let new_debit = AccountBalance {
            debits_posted: debit_balance
                .debits_posted
                .checked_add(transfer.amount)
                .ok_or(TransferError::DebitsMustNotExceedCredits)?,
            ..debit_balance
        };
        if debit.flags.debits_must_not_exceed_credits
            && new_debit.debits_posted > new_debit.credits_posted
        {
            return Err(TransferError::DebitsMustNotExceedCredits);
        }
        let new_credit = AccountBalance {
            credits_posted: credit_balance
                .credits_posted
                .checked_add(transfer.amount)
                .ok_or(TransferError::CreditsMustNotExceedDebits)?,
            ..credit_balance
        };
        if credit.flags.credits_must_not_exceed_debits
            && new_credit.credits_posted > new_credit.debits_posted
        {
            return Err(TransferError::CreditsMustNotExceedDebits);
        }
        staged.insert(transfer.debit_account_id, new_debit);
        staged.insert(transfer.credit_account_id, new_credit);
        Ok(())
    }
}

impl LedgerBackend for InMemoryLedger {
    fn create_accounts(&mut self, accounts: &[Account]) -> Vec<Result<(), AccountError>> {
        accounts
            .iter()
            .map(|account| {
                if account.ledger == 0 {
                    return Err(AccountError::LedgerMustNotBeZero);
                }
                if let Some(existing) = self.accounts.get(&account.id) {
                    return if existing == account {
                        Err(AccountError::Exists)
                    } else {
                        Err(AccountError::ExistsWithDifferentFields)
                    };
                }
                self.accounts.insert(account.id, account.clone());
                self.balances.entry(account.id).or_default();
                Ok(())
            })
            .collect()
    }

    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>> {
        let mut results = Vec::with_capacity(batch.len());
        let mut chain_start = 0;
        while chain_start < batch.len() {
            // A chain runs until (and including) the first unlinked leg.
            let mut chain_end = chain_start;
            while batch[chain_end].linked && chain_end + 1 < batch.len() {
                chain_end += 1;
            }
            let chain = &batch[chain_start..=chain_end];
            // A chain that reaches the end of the batch with its last
            // leg still `linked` is unterminated: the linked flag
            // promises a following leg that never arrived. Reject the
            // whole chain atomically — nothing is validated, staged,
            // or stored — instead of committing it as if the end of
            // the batch terminated it.
            if chain[chain.len() - 1].linked {
                for (offset, _) in chain.iter().enumerate() {
                    results.push(Err(if offset == chain.len() - 1 {
                        TransferError::LinkedEventChainOpen
                    } else {
                        TransferError::LinkedEventFailed
                    }));
                }
                chain_start = chain_end + 1;
                continue;
            }
            let mut staged = self.balances.clone();
            let mut staged_transfers: HashMap<TransferId, Transfer> = HashMap::new();
            // Per-leg validation outcome. An idempotent `Exists` leg
            // stages nothing (its effects are already in the real
            // balances), but inside a linked chain it is NOT neutral:
            // TigerBeetle breaks a chain on any non-`Created` status
            // (#6170, and the trait doc above). The chain outcome is
            // decided after the loop from these per-leg outcomes:
            // all-`Exists` chains report per-leg `Exists`; a mixed
            // chain posts nothing, with the first `Exists` leg (or
            // the first hard failure, whichever comes first in chain
            // order) reporting its own outcome and every other leg
            // reporting `LinkedEventFailed`.
            let mut leg_outcomes: Vec<Result<(), TransferError>> = Vec::with_capacity(chain.len());
            let mut first_exists_offset: Option<usize> = None;
            let mut failure: Option<(usize, TransferError)> = None;
            for (offset, transfer) in chain.iter().enumerate() {
                match self.validate_and_stage(transfer, &mut staged, &staged_transfers) {
                    Ok(()) => {
                        staged_transfers.insert(transfer.id, transfer.clone());
                        leg_outcomes.push(Ok(()));
                    }
                    Err(TransferError::Exists) => {
                        if first_exists_offset.is_none() {
                            first_exists_offset = Some(offset);
                        }
                        leg_outcomes.push(Err(TransferError::Exists));
                    }
                    Err(error) => {
                        failure = Some((offset, error));
                        break;
                    }
                }
            }
            match failure {
                None => {
                    let exists_count = leg_outcomes
                        .iter()
                        .filter(|outcome| matches!(outcome, Err(TransferError::Exists)))
                        .count();
                    if exists_count > 0 && exists_count < chain.len() {
                        // Mixed chain (#6170): an `Exists` leg ahead of
                        // or behind new legs breaks the chain, exactly
                        // as TigerBeetle breaks it. Nothing commits:
                        // the staged balances and transfers are dropped,
                        // `Exists` legs keep their outcome, and every
                        // new leg reports `LinkedEventFailed`.
                        for outcome in leg_outcomes {
                            results.push(match outcome {
                                Err(TransferError::Exists) => Err(TransferError::Exists),
                                _ => Err(TransferError::LinkedEventFailed),
                            });
                        }
                    } else {
                        // No `Exists` leg (an ordinary posting chain)
                        // or nothing but `Exists` legs (a whole-chain
                        // identical resubmission: per-leg `Exists`,
                        // and since `Exists` legs stage nothing, the
                        // commit below moves no balances and stores no
                        // new transfers).
                        self.balances = staged;
                        for transfer in chain {
                            if !self.transfers.contains_key(&transfer.id) {
                                self.transfers.insert(transfer.id, transfer.clone());
                                self.transfer_order.push(transfer.id);
                            }
                        }
                        results.extend(leg_outcomes);
                    }
                }
                Some((failing_offset, error)) => {
                    // Record failed transfer IDs for IdAlreadyFailed behavior.
                    // When a transfer fails for its own reason (validation/balance
                    // errors), TigerBeetle remembers the ID and refuses resubmission.
                    // The stored error is kept for diagnosis; future
                    // submissions of the ID are refused with
                    // `IdAlreadyFailed`, not the stored error (#6628).
                    // Only record failures that are transfer-level errors (not chain-level failures).
                    if !matches!(
                        error,
                        TransferError::LinkedEventFailed | TransferError::LinkedEventChainOpen
                    ) {
                        // Record only the leg that actually failed
                        // (#6627): the chain's other legs did not fail
                        // and must remain submittable, and an ID that
                        // is already posted must never be marked
                        // failed -- `validate_and_stage` consults
                        // `failed_transfers` before `transfers`, so a
                        // failure cached against a posted ID would
                        // break its idempotent `Exists` retry.
                        let failing = &chain[failing_offset];
                        // But if an earlier leg already broke the
                        // chain with `Exists`, TigerBeetle never
                        // attempted this leg at all (the result
                        // selection below reports only
                        // `LinkedEventFailed` for it): caching its
                        // error would refuse a leg that was never
                        // tried, so only cache when this leg is the
                        // leg the chain actually broke on.
                        let chain_broke_at_earlier_exists = matches!(
                            first_exists_offset,
                            Some(exists_offset) if exists_offset < failing_offset
                        );
                        if !chain_broke_at_earlier_exists
                            && !self.transfers.contains_key(&failing.id)
                        {
                            self.failed_transfers.insert(failing.id, error);
                        }
                    }

                    // The chain broke at the first non-`Created` leg in
                    // chain order: an earlier `Exists` leg (TigerBeetle
                    // breaks on it before ever reaching the hard
                    // failure) reports `Exists`; otherwise the failing
                    // leg reports its own error. Every other leg
                    // reports `LinkedEventFailed`.
                    let (breaker_offset, breaker_error) = match first_exists_offset {
                        Some(exists_offset) if exists_offset < failing_offset => {
                            (exists_offset, TransferError::Exists)
                        }
                        _ => (failing_offset, error),
                    };
                    for (offset, _) in chain.iter().enumerate() {
                        results.push(Err(if offset == breaker_offset {
                            breaker_error
                        } else {
                            TransferError::LinkedEventFailed
                        }));
                    }
                }
            }
            chain_start = chain_end + 1;
        }
        results
    }

    fn lookup_transfer(&self, id: TransferId) -> Option<Transfer> {
        self.transfers.get(&id).cloned()
    }

    fn lookup_account(&self, id: AccountId) -> Option<Account> {
        self.accounts.get(&id).cloned()
    }

    fn account_balance(&self, id: AccountId) -> Option<AccountBalance> {
        self.balances.get(&id).copied()
    }

    fn transfers_with_user_data_128(&self, value: u128) -> Vec<Transfer> {
        self.transfer_order
            .iter()
            .filter_map(|id| self.transfers.get(id))
            .filter(|transfer| transfer.user_data_128 == value)
            .cloned()
            .collect()
    }
}
