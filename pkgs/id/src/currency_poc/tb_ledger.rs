//! [`LedgerBackend`] over the official TigerBeetle Rust client
//! (`tigerbeetle` crate, pinned to the harness's server version), so
//! the conversion-executor scenarios in `super::tests` can run against
//! a real test cluster (`super::harness::TestCluster`) and not only
//! the in-memory backend. This closes acceptance item 1 of #2289.
//!
//! Design notes:
//!
//! - The trait is synchronous; the client is async but runtime-
//!   agnostic (it runs its own off-thread event loop). The binding
//!   owns a private current-thread tokio runtime and `block_on`s each
//!   request. It must therefore be driven from plain synchronous code
//!   (as the PoC executor is) — never from inside a tokio runtime.
//! - TigerBeetle requires a non-zero `code` on accounts and transfers,
//!   while the PoC types use `0` as a supported code. Codes are sent
//!   shifted by one ([`wire_code`]): the shift is a bijection on
//!   `0..=u16::MAX - 1`, so codes `0` and `1` stay distinct on the
//!   wire and a retry with a different code is reported by
//!   TigerBeetle as `ExistsWithDifferentFields`, as the backend's
//!   field-sensitive idempotency contract requires. The binding also
//!   records the original code of every account/transfer it creates
//!   and restores it on lookups, so a looked-up leg still compares
//!   equal to the persisted intent leg the recovery scanner checks
//!   (`super::recovery`).
//! - Status mapping turns ordinary validation rejections, including
//!   IDs and flags accepted by the PoC types, into typed per-leg errors.
//!   TigerBeetle statuses that require fields this adapter does not
//!   expose are reported as `UnsupportedStatus`; transport-level client
//!   errors still have no channel in the trait and remain fatal.
//!
//! The binding is behind the `currency-poc-tb` feature so default
//! builds of the `id` crate do not link the client's native
//! `tb_client` library.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use anyhow::{Context, Result};
use tigerbeetle as tb;

use super::ledger::{
    Account, AccountBalance, AccountError, AccountFlags, LedgerBackend, Transfer, TransferError,
};

/// Encode a PoC `code` for the wire. TigerBeetle rejects code `0`,
/// so codes are sent shifted by one: `0 -> 1`, `1 -> 2`, ... The
/// shift keeps every supported code distinct — mapping only `0` to a
/// substitute would serialize codes `0` and `1` identically, and a
/// retry with the other code would report `Exists` instead of
/// `ExistsWithDifferentFields`. `u16::MAX` has no wire representation
/// under this scheme (its shift would wrap to `0`); the PoC's
/// chart-of-accounts codes are small, and asking for `u16::MAX` is a
/// programming error, not a runtime condition.
fn wire_code(code: u16) -> u16 {
    code.checked_add(1)
        .expect("code u16::MAX has no TigerBeetle wire representation")
}

/// Inverse of [`wire_code`], used to restore the original code on
/// lookups the registry does not know (e.g. records created by an
/// earlier binding instance). Wire codes are always non-zero.
fn poc_code(wire: u16) -> u16 {
    wire.checked_sub(1)
        .expect("tigerbeetle returned a zero wire code")
}

/// A [`LedgerBackend`] backed by a live TigerBeetle cluster through
/// the official Rust client. One client (and its runtime) per ledger;
/// the harness cluster it connects to is owned by the caller.
pub struct TigerBeetleLedger {
    client: tb::Client,
    runtime: tokio::runtime::Runtime,
    /// Original (pre-substitution) account codes by account ID.
    account_codes: Mutex<HashMap<u128, u16>>,
    /// Original (pre-substitution) transfer codes by transfer ID.
    transfer_codes: Mutex<HashMap<u128, u16>>,
}

impl TigerBeetleLedger {
    /// Connect to the cluster at `address` (`host:port`, e.g. a
    /// [`super::harness::TestCluster`]'s address). The harness formats
    /// its data file with cluster ID 0, so the client uses 0 too.
    pub fn connect(address: &str) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building the tigerbeetle binding runtime")?;
        let client = tb::Client::new(0, address)
            .map_err(|error| anyhow::anyhow!("tigerbeetle client init for {address}: {error:?}"))?;
        Ok(Self {
            client,
            runtime,
            account_codes: Mutex::new(HashMap::new()),
            transfer_codes: Mutex::new(HashMap::new()),
        })
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    fn account_code(&self, id: u128, wire: u16) -> u16 {
        self.account_codes
            .lock()
            .expect("account code registry poisoned")
            .get(&id)
            .copied()
            .unwrap_or_else(|| poc_code(wire))
    }

    fn transfer_code(&self, id: u128, wire: u16) -> u16 {
        self.transfer_codes
            .lock()
            .expect("transfer code registry poisoned")
            .get(&id)
            .copied()
            .unwrap_or_else(|| poc_code(wire))
    }
}

fn to_tb_account(account: &Account) -> tb::Account {
    let mut flags = tb::AccountFlags::empty();
    if account.flags.debits_must_not_exceed_credits {
        flags = flags | tb::AccountFlags::DebitsMustNotExceedCredits;
    }
    if account.flags.credits_must_not_exceed_debits {
        flags = flags | tb::AccountFlags::CreditsMustNotExceedDebits;
    }
    tb::Account {
        id: account.id,
        ledger: account.ledger,
        code: wire_code(account.code),
        flags,
        ..Default::default()
    }
}

fn from_tb_account(account: &tb::Account, code: u16) -> Account {
    Account {
        id: account.id,
        ledger: account.ledger,
        code,
        flags: AccountFlags {
            debits_must_not_exceed_credits: account.flags.0
                & tb::AccountFlags::DebitsMustNotExceedCredits.0
                != 0,
            credits_must_not_exceed_debits: account.flags.0
                & tb::AccountFlags::CreditsMustNotExceedDebits.0
                != 0,
        },
    }
}

fn to_tb_transfer(transfer: &Transfer) -> tb::Transfer {
    tb::Transfer {
        id: transfer.id,
        debit_account_id: transfer.debit_account_id,
        credit_account_id: transfer.credit_account_id,
        amount: transfer.amount,
        user_data_128: transfer.user_data_128,
        ledger: transfer.ledger,
        code: wire_code(transfer.code),
        flags: if transfer.linked {
            tb::TransferFlags::Linked
        } else {
            tb::TransferFlags::empty()
        },
        ..Default::default()
    }
}

fn from_tb_transfer(transfer: &tb::Transfer, code: u16) -> Transfer {
    Transfer {
        id: transfer.id,
        debit_account_id: transfer.debit_account_id,
        credit_account_id: transfer.credit_account_id,
        amount: transfer.amount,
        ledger: transfer.ledger,
        user_data_128: transfer.user_data_128,
        code,
        linked: transfer.flags.0 & tb::TransferFlags::Linked.0 != 0,
    }
}

#[deny(non_exhaustive_omitted_patterns)]
fn map_account_status(status: tb::CreateAccountStatus) -> Result<(), AccountError> {
    use tb::CreateAccountStatus as Status;
    match status {
        Status::Created => Ok(()),
        Status::Exists => Err(AccountError::Exists),
        Status::ExistsWithDifferentFlags
        | Status::ExistsWithDifferentUserData128
        | Status::ExistsWithDifferentUserData64
        | Status::ExistsWithDifferentUserData32
        | Status::ExistsWithDifferentLedger
        | Status::ExistsWithDifferentCode => Err(AccountError::ExistsWithDifferentFields),
        Status::LedgerMustNotBeZero => Err(AccountError::LedgerMustNotBeZero),
        Status::IdMustNotBeZero => Err(AccountError::IdMustNotBeZero),
        Status::IdMustNotBeIntMax => Err(AccountError::IdMustNotBeIntMax),
        Status::FlagsAreMutuallyExclusive => Err(AccountError::FlagsAreMutuallyExclusive),
        // These statuses require imported-event data, caller-controlled
        // timestamps/reserved fields, or pending balances, none of which
        // this adapter sends. Keep them typed rather than panicking if a
        // server or adapter change makes one observable.
        Status::LinkedEventFailed
        | Status::LinkedEventChainOpen
        | Status::ImportedEventExpected
        | Status::ImportedEventNotExpected
        | Status::TimestampMustBeZero
        | Status::ImportedEventTimestampOutOfRange
        | Status::ImportedEventTimestampMustNotAdvance
        | Status::ReservedField
        | Status::ReservedFlag
        | Status::DebitsPendingMustBeZero
        | Status::DebitsPostedMustBeZero
        | Status::CreditsPendingMustBeZero
        | Status::CreditsPostedMustBeZero
        | Status::CodeMustNotBeZero
        | Status::ImportedEventTimestampMustNotRegress => Err(AccountError::UnsupportedStatus),
        // The status enum is non-exhaustive. Unknown future statuses are
        // still returned through the typed result channel, never panicked.
        _ => Err(AccountError::UnsupportedStatus),
    }
}

#[deny(non_exhaustive_omitted_patterns)]
fn map_transfer_status(status: tb::CreateTransferStatus) -> Result<(), TransferError> {
    use tb::CreateTransferStatus as Status;
    match status {
        Status::Created => Ok(()),
        Status::Exists => Err(TransferError::Exists),
        Status::ExistsWithDifferentFlags
        | Status::ExistsWithDifferentPendingId
        | Status::ExistsWithDifferentTimeout
        | Status::ExistsWithDifferentDebitAccountId
        | Status::ExistsWithDifferentCreditAccountId
        | Status::ExistsWithDifferentAmount
        | Status::ExistsWithDifferentUserData128
        | Status::ExistsWithDifferentUserData64
        | Status::ExistsWithDifferentUserData32
        | Status::ExistsWithDifferentLedger
        | Status::ExistsWithDifferentCode => Err(TransferError::ExistsWithDifferentFields),
        Status::LinkedEventFailed => Err(TransferError::LinkedEventFailed),
        Status::LinkedEventChainOpen => Err(TransferError::LinkedEventChainOpen),
        // A transfer ID that already failed is remembered by
        // TigerBeetle; resubmitting it (recovery) is a regular
        // rejection, not a panic-worthy unmodelled status.
        Status::IdAlreadyFailed => Err(TransferError::IdAlreadyFailed),
        Status::DebitAccountNotFound => Err(TransferError::DebitAccountNotFound),
        Status::CreditAccountNotFound => Err(TransferError::CreditAccountNotFound),
        Status::AccountsMustBeDifferent => Err(TransferError::AccountsMustBeDifferent),
        Status::AccountsMustHaveTheSameLedger
        | Status::TransferMustHaveTheSameLedgerAsAccounts
        | Status::LedgerMustNotBeZero => Err(TransferError::LedgerMismatch),
        // TigerBeetle's exceeds_credits is the debit account's
        // debits_must_not_exceed_credits guard tripping; exceeds_debits
        // is the credit account's credits_must_not_exceed_debits guard.
        Status::ExceedsCredits => Err(TransferError::DebitsMustNotExceedCredits),
        Status::ExceedsDebits => Err(TransferError::CreditsMustNotExceedDebits),
        Status::OverflowsDebitsPending
        | Status::OverflowsDebitsPosted
        | Status::OverflowsDebits => Err(TransferError::DebitsMustNotExceedCredits),
        Status::OverflowsCreditsPending
        | Status::OverflowsCreditsPosted
        | Status::OverflowsCredits => Err(TransferError::CreditsMustNotExceedDebits),
        Status::IdMustNotBeZero => Err(TransferError::IdMustNotBeZero),
        Status::IdMustNotBeIntMax => Err(TransferError::IdMustNotBeIntMax),
        Status::DebitAccountIdMustNotBeZero => Err(TransferError::DebitAccountIdMustNotBeZero),
        Status::DebitAccountIdMustNotBeIntMax => Err(TransferError::DebitAccountIdMustNotBeIntMax),
        Status::CreditAccountIdMustNotBeZero => Err(TransferError::CreditAccountIdMustNotBeZero),
        Status::CreditAccountIdMustNotBeIntMax => {
            Err(TransferError::CreditAccountIdMustNotBeIntMax)
        }
        Status::DebitAccountAlreadyClosed => Err(TransferError::DebitAccountAlreadyClosed),
        Status::CreditAccountAlreadyClosed => Err(TransferError::CreditAccountAlreadyClosed),
        // These statuses require pending transfers, imported-event data,
        // caller-controlled timestamps/reserved fields, or flags that
        // this adapter does not expose. Return a typed error if one is
        // nevertheless observed rather than panicking.
        Status::ImportedEventExpected
        | Status::ImportedEventNotExpected
        | Status::TimestampMustBeZero
        | Status::ImportedEventTimestampOutOfRange
        | Status::ImportedEventTimestampMustNotAdvance
        | Status::ReservedFlag
        | Status::FlagsAreMutuallyExclusive
        | Status::PendingIdMustBeZero
        | Status::PendingIdMustNotBeZero
        | Status::PendingIdMustNotBeIntMax
        | Status::PendingIdMustBeDifferent
        | Status::TimeoutReservedForPendingTransfer
        | Status::ClosingTransferMustBePending
        | Status::CodeMustNotBeZero
        | Status::PendingTransferNotFound
        | Status::PendingTransferNotPending
        | Status::PendingTransferHasDifferentDebitAccountId
        | Status::PendingTransferHasDifferentCreditAccountId
        | Status::PendingTransferHasDifferentLedger
        | Status::PendingTransferHasDifferentCode
        | Status::ExceedsPendingTransferAmount
        | Status::PendingTransferHasDifferentAmount
        | Status::PendingTransferAlreadyPosted
        | Status::PendingTransferAlreadyVoided
        | Status::PendingTransferExpired
        | Status::ImportedEventTimestampMustNotRegress
        | Status::ImportedEventTimestampMustPostdateDebitAccount
        | Status::ImportedEventTimestampMustPostdateCreditAccount
        | Status::ImportedEventTimeoutMustBeZero
        | Status::OverflowsTimeout => Err(TransferError::UnsupportedStatus),
        // The status enum is non-exhaustive. Unknown future statuses are
        // still returned through the typed result channel, never panicked.
        _ => Err(TransferError::UnsupportedStatus),
    }
}

impl LedgerBackend for TigerBeetleLedger {
    fn create_accounts(&mut self, accounts: &[Account]) -> Vec<Result<(), AccountError>> {
        let wire: Vec<tb::Account> = accounts.iter().map(to_tb_account).collect();
        let results = self
            .block_on(self.client.create_accounts(&wire))
            .expect("tigerbeetle create_accounts request failed");
        assert_eq!(
            results.len(),
            accounts.len(),
            "tigerbeetle returned one result per account"
        );
        // Record original codes only for creations TigerBeetle
        // accepted (Created) or confirmed identical (Exists). A
        // rejected creation (e.g. ExistsWithDifferentFields) must not
        // overwrite the registry: the stored record still carries
        // the earlier code, and lookups restore from the registry.
        {
            let mut registry = self
                .account_codes
                .lock()
                .expect("account code registry poisoned");
            for (account, result) in accounts.iter().zip(results.iter()) {
                if matches!(
                    result.status,
                    tb::CreateAccountStatus::Created | tb::CreateAccountStatus::Exists
                ) {
                    registry.insert(account.id, account.code);
                }
            }
        }
        results
            .iter()
            .map(|result| map_account_status(result.status))
            .collect()
    }

    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>> {
        let wire: Vec<tb::Transfer> = batch.iter().map(to_tb_transfer).collect();
        let results = self
            .block_on(self.client.create_transfers(&wire))
            .expect("tigerbeetle create_transfers request failed");
        assert_eq!(
            results.len(),
            batch.len(),
            "tigerbeetle returned one result per transfer"
        );
        // Same rule as create_accounts: record original codes only
        // for accepted (Created) or identical (Exists) transfers, so
        // a rejected retry cannot rewrite what lookups report for
        // the stored transfer.
        {
            let mut registry = self
                .transfer_codes
                .lock()
                .expect("transfer code registry poisoned");
            for (transfer, result) in batch.iter().zip(results.iter()) {
                if matches!(
                    result.status,
                    tb::CreateTransferStatus::Created | tb::CreateTransferStatus::Exists
                ) {
                    registry.insert(transfer.id, transfer.code);
                }
            }
        }
        // Linked-chain retry (Codex review, PR #5072): TigerBeetle
        // breaks a linked chain on ANY non-Created status, so
        // resubmitting an already-posted chain reports `Exists` for
        // its first leg and `LinkedEventFailed` for every later leg —
        // while the in-memory backend reports `Exists` per leg and
        // the executor accepts the retry as idempotent success. When
        // a whole chain shows exactly that pattern AND every stored
        // leg verifies identical to the submitted leg, report
        // `Exists` for all of its legs. A chain that genuinely failed
        // never starts with `Exists`, and a leg whose stored fields
        // differ fails the verification, so both keep the raw mapping.
        let mut mapped: Vec<Result<(), TransferError>> = results
            .iter()
            .map(|result| map_transfer_status(result.status))
            .collect();
        let mut chain_start = 0;
        while chain_start < batch.len() {
            let mut chain_end = chain_start;
            while batch[chain_end].linked && chain_end + 1 < batch.len() {
                chain_end += 1;
            }
            let chain = &batch[chain_start..=chain_end];
            let raw = &results[chain_start..=chain_end];
            let already_posted_pattern = chain.len() > 1
                && !chain[chain.len() - 1].linked
                && matches!(raw[0].status, tb::CreateTransferStatus::Exists)
                && raw[1..]
                    .iter()
                    .all(|r| matches!(r.status, tb::CreateTransferStatus::LinkedEventFailed));
            if already_posted_pattern
                && chain
                    .iter()
                    .all(|leg| self.lookup_transfer(leg.id) == Some(leg.clone()))
            {
                for slot in &mut mapped[chain_start..=chain_end] {
                    *slot = Err(TransferError::Exists);
                }
            }
            chain_start = chain_end + 1;
        }
        mapped
    }

    fn lookup_transfer(&self, id: u128) -> Option<Transfer> {
        let found = self
            .block_on(self.client.lookup_transfers(&[id]))
            .expect("tigerbeetle lookup_transfers request failed");
        found
            .first()
            .map(|transfer| from_tb_transfer(transfer, self.transfer_code(id, transfer.code)))
    }

    fn lookup_account(&self, id: u128) -> Option<Account> {
        let found = self
            .block_on(self.client.lookup_accounts(&[id]))
            .expect("tigerbeetle lookup_accounts request failed");
        found
            .first()
            .map(|account| from_tb_account(account, self.account_code(id, account.code)))
    }

    fn account_balance(&self, id: u128) -> Option<AccountBalance> {
        let found = self
            .block_on(self.client.lookup_accounts(&[id]))
            .expect("tigerbeetle lookup_accounts request failed");
        found.first().map(|account| AccountBalance {
            debits_posted: account.debits_posted,
            credits_posted: account.credits_posted,
        })
    }

    fn transfers_with_user_data_128(&self, value: u128) -> Vec<Transfer> {
        // TigerBeetle treats a zero `user_data_128` filter as "no
        // filter" (QueryFilter documentation), so for value 0 the
        // query pages over every transfer; the exact post-filter
        // then keeps only the zero-stamped ones (the PoC's funding
        // transfers). The post-filter runs for every value, so the
        // result stays exact even if server filter semantics change.
        // Pages advance by timestamp (unique per transfer), so a
        // scan larger than one page is still complete.
        const PAGE: u32 = 8_189;
        let mut matched = Vec::new();
        let mut timestamp_min = 0u64;
        loop {
            let filter = tb::QueryFilter {
                user_data_128: value,
                timestamp_min,
                limit: PAGE,
                ..Default::default()
            };
            let found = self
                .block_on(self.client.query_transfers(filter))
                .expect("tigerbeetle query_transfers request failed");
            let page_full = found.len() == PAGE as usize;
            let last_timestamp = found.last().map(|transfer| transfer.timestamp);
            matched.extend(
                found
                    .iter()
                    .filter(|transfer| transfer.user_data_128 == value)
                    .map(|transfer| {
                        from_tb_transfer(transfer, self.transfer_code(transfer.id, transfer.code))
                    }),
            );
            match last_timestamp {
                Some(last) if page_full => timestamp_min = last.saturating_add(1),
                _ => break,
            }
        }
        matched
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{map_account_status, map_transfer_status, tb};

    #[test]
    fn every_pinned_account_status_maps_without_panicking() {
        use tb::CreateAccountStatus as S;

        let statuses = [
            S::Created,
            S::LinkedEventFailed,
            S::LinkedEventChainOpen,
            S::ImportedEventExpected,
            S::ImportedEventNotExpected,
            S::TimestampMustBeZero,
            S::ImportedEventTimestampOutOfRange,
            S::ImportedEventTimestampMustNotAdvance,
            S::ReservedField,
            S::ReservedFlag,
            S::IdMustNotBeZero,
            S::IdMustNotBeIntMax,
            S::ExistsWithDifferentFlags,
            S::ExistsWithDifferentUserData128,
            S::ExistsWithDifferentUserData64,
            S::ExistsWithDifferentUserData32,
            S::ExistsWithDifferentLedger,
            S::ExistsWithDifferentCode,
            S::Exists,
            S::FlagsAreMutuallyExclusive,
            S::DebitsPendingMustBeZero,
            S::DebitsPostedMustBeZero,
            S::CreditsPendingMustBeZero,
            S::CreditsPostedMustBeZero,
            S::LedgerMustNotBeZero,
            S::CodeMustNotBeZero,
            S::ImportedEventTimestampMustNotRegress,
        ];

        for status in statuses {
            assert!(
                std::panic::catch_unwind(|| map_account_status(status)).is_ok(),
                "account status {status:?} panicked",
            );
        }
    }

    #[test]
    fn every_pinned_transfer_status_maps_without_panicking() {
        use tb::CreateTransferStatus as S;

        let statuses = [
            S::Created,
            S::LinkedEventFailed,
            S::LinkedEventChainOpen,
            S::ImportedEventExpected,
            S::ImportedEventNotExpected,
            S::TimestampMustBeZero,
            S::ImportedEventTimestampOutOfRange,
            S::ImportedEventTimestampMustNotAdvance,
            S::ReservedFlag,
            S::IdMustNotBeZero,
            S::IdMustNotBeIntMax,
            S::ExistsWithDifferentFlags,
            S::ExistsWithDifferentPendingId,
            S::ExistsWithDifferentTimeout,
            S::ExistsWithDifferentDebitAccountId,
            S::ExistsWithDifferentCreditAccountId,
            S::ExistsWithDifferentAmount,
            S::ExistsWithDifferentUserData128,
            S::ExistsWithDifferentUserData64,
            S::ExistsWithDifferentUserData32,
            S::ExistsWithDifferentLedger,
            S::ExistsWithDifferentCode,
            S::Exists,
            S::IdAlreadyFailed,
            S::FlagsAreMutuallyExclusive,
            S::DebitAccountIdMustNotBeZero,
            S::DebitAccountIdMustNotBeIntMax,
            S::CreditAccountIdMustNotBeZero,
            S::CreditAccountIdMustNotBeIntMax,
            S::AccountsMustBeDifferent,
            S::PendingIdMustBeZero,
            S::PendingIdMustNotBeZero,
            S::PendingIdMustNotBeIntMax,
            S::PendingIdMustBeDifferent,
            S::TimeoutReservedForPendingTransfer,
            S::ClosingTransferMustBePending,
            S::LedgerMustNotBeZero,
            S::CodeMustNotBeZero,
            S::DebitAccountNotFound,
            S::CreditAccountNotFound,
            S::AccountsMustHaveTheSameLedger,
            S::TransferMustHaveTheSameLedgerAsAccounts,
            S::PendingTransferNotFound,
            S::PendingTransferNotPending,
            S::PendingTransferHasDifferentDebitAccountId,
            S::PendingTransferHasDifferentCreditAccountId,
            S::PendingTransferHasDifferentLedger,
            S::PendingTransferHasDifferentCode,
            S::ExceedsPendingTransferAmount,
            S::PendingTransferHasDifferentAmount,
            S::PendingTransferAlreadyPosted,
            S::PendingTransferAlreadyVoided,
            S::PendingTransferExpired,
            S::ImportedEventTimestampMustNotRegress,
            S::ImportedEventTimestampMustPostdateDebitAccount,
            S::ImportedEventTimestampMustPostdateCreditAccount,
            S::ImportedEventTimeoutMustBeZero,
            S::DebitAccountAlreadyClosed,
            S::CreditAccountAlreadyClosed,
            S::OverflowsDebitsPending,
            S::OverflowsCreditsPending,
            S::OverflowsDebitsPosted,
            S::OverflowsCreditsPosted,
            S::OverflowsDebits,
            S::OverflowsCredits,
            S::OverflowsTimeout,
            S::ExceedsCredits,
            S::ExceedsDebits,
        ];

        for status in statuses {
            assert!(
                std::panic::catch_unwind(|| map_transfer_status(status)).is_ok(),
                "transfer status {status:?} panicked",
            );
        }
    }

    #[test]
    fn reachable_validation_statuses_keep_their_typed_errors() {
        use super::super::ledger::{AccountError as A, TransferError as T};
        use tb::{CreateAccountStatus as AS, CreateTransferStatus as TS};

        assert_eq!(
            map_account_status(AS::IdMustNotBeZero),
            Err(A::IdMustNotBeZero)
        );
        assert_eq!(
            map_account_status(AS::IdMustNotBeIntMax),
            Err(A::IdMustNotBeIntMax)
        );
        assert_eq!(
            map_account_status(AS::FlagsAreMutuallyExclusive),
            Err(A::FlagsAreMutuallyExclusive),
        );
        assert_eq!(
            map_transfer_status(TS::IdMustNotBeZero),
            Err(T::IdMustNotBeZero)
        );
        assert_eq!(
            map_transfer_status(TS::IdMustNotBeIntMax),
            Err(T::IdMustNotBeIntMax)
        );
        assert_eq!(
            map_transfer_status(TS::DebitAccountIdMustNotBeZero),
            Err(T::DebitAccountIdMustNotBeZero),
        );
        assert_eq!(
            map_transfer_status(TS::DebitAccountIdMustNotBeIntMax),
            Err(T::DebitAccountIdMustNotBeIntMax),
        );
        assert_eq!(
            map_transfer_status(TS::CreditAccountIdMustNotBeZero),
            Err(T::CreditAccountIdMustNotBeZero),
        );
        assert_eq!(
            map_transfer_status(TS::CreditAccountIdMustNotBeIntMax),
            Err(T::CreditAccountIdMustNotBeIntMax),
        );
        assert_eq!(
            map_transfer_status(TS::DebitAccountAlreadyClosed),
            Err(T::DebitAccountAlreadyClosed),
        );
        assert_eq!(
            map_transfer_status(TS::CreditAccountAlreadyClosed),
            Err(T::CreditAccountAlreadyClosed),
        );
    }
}
