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
//! - Status mapping covers every variant of the pinned crate
//!   explicitly (#6108): `map_account_status` and
//!   `map_transfer_status` name each status of the pinned
//!   TigerBeetle crate's enums; the enums are `#[non_exhaustive]`,
//!   so a wildcard arm remains for future variants only, mapping
//!   them to `UnmodelledStatus` — and the variant tables in the
//!   tests are the inventory to extend when the pin moves. Every
//!   status reachable
//!   from inputs the PoC types accept maps to a specific typed error;
//!   the families the binding can never emit (imported events,
//!   pending transfers, reserved fields/flags, explicit timestamps)
//!   are enumerated arm-by-arm and map to `UnmodelledStatus` with a
//!   comment saying why they are unreachable. Transport failures from
//!   creation requests become per-item backend errors, and
//!   lookup/query failures propagate through the fallible trait
//!   methods instead of being mistaken for missing records.
//! - The in-memory backend (`super::ledger::InMemoryLedger`) rejects
//!   the same zero/int-max ID shapes with the same typed errors, so
//!   scenario tests mean the same thing on both backends (#6108).
//!   Statuses with no in-memory analogue (closed accounts, initial
//!   balances, flag exclusivity) exist only on this binding: the PoC
//!   types cannot express the state that produces them.
//!
//! The binding is behind the `currency-poc-tb` feature so default
//! builds of the `id` crate do not link the client's native
//! `tb_client` library.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};

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
/// under this scheme (its shift would wrap to `0`), so a create request
/// containing it is rejected with `CodeOutOfRange` before reaching the server.
fn wire_code(code: u16) -> Option<u16> {
    code.checked_add(1)
}

/// Inverse of [`wire_code`], used to restore the original code on
/// lookups the registry does not know (e.g. records created by an
/// earlier binding instance). Wire codes are always non-zero.
fn poc_code(wire: u16) -> Result<u16> {
    wire.checked_sub(1)
        .context("tigerbeetle returned a zero wire code")
}

fn lock_registry<'a, T>(registry: &'a Mutex<T>, name: &'static str) -> Option<MutexGuard<'a, T>> {
    match registry.lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            tracing::error!(
                registry = name,
                %error,
                "TigerBeetle code registry mutex poisoned; falling back to wire codes"
            );
            None
        }
    }
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

    fn account_code(&self, id: u128, wire: u16) -> Result<u16> {
        let code = lock_registry(&self.account_codes, "account")
            .and_then(|registry| registry.get(&id).copied());
        match code {
            Some(code) => Ok(code),
            None => poc_code(wire),
        }
    }

    fn transfer_code(&self, id: u128, wire: u16) -> Result<u16> {
        let code = lock_registry(&self.transfer_codes, "transfer")
            .and_then(|registry| registry.get(&id).copied());
        match code {
            Some(code) => Ok(code),
            None => poc_code(wire),
        }
    }
}

fn to_tb_account(account: &Account) -> std::result::Result<tb::Account, AccountError> {
    let code = wire_code(account.code).ok_or(AccountError::CodeOutOfRange)?;
    let mut flags = tb::AccountFlags::empty();
    if account.flags.debits_must_not_exceed_credits {
        flags = flags | tb::AccountFlags::DebitsMustNotExceedCredits;
    }
    if account.flags.credits_must_not_exceed_debits {
        flags = flags | tb::AccountFlags::CreditsMustNotExceedDebits;
    }
    Ok(tb::Account {
        id: account.id,
        ledger: account.ledger,
        code,
        flags,
        ..Default::default()
    })
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

fn to_tb_transfer(transfer: &Transfer) -> std::result::Result<tb::Transfer, TransferError> {
    let code = wire_code(transfer.code).ok_or(TransferError::CodeOutOfRange)?;
    Ok(tb::Transfer {
        id: transfer.id,
        debit_account_id: transfer.debit_account_id,
        credit_account_id: transfer.credit_account_id,
        amount: transfer.amount,
        user_data_128: transfer.user_data_128,
        ledger: transfer.ledger,
        code,
        flags: if transfer.linked {
            tb::TransferFlags::Linked
        } else {
            tb::TransferFlags::empty()
        },
        ..Default::default()
    })
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

fn unmodelled_account(status: tb::CreateAccountStatus) -> AccountError {
    tracing::error!(?status, "TigerBeetle returned an unmodelled account status");
    AccountError::UnmodelledStatus
}

fn unmodelled_transfer(status: tb::CreateTransferStatus) -> TransferError {
    tracing::error!(
        ?status,
        "TigerBeetle returned an unmodelled transfer status"
    );
    TransferError::UnmodelledStatus
}

/// Map a TigerBeetle account-creation status to the PoC's typed
/// errors. Every variant of the pinned crate is matched explicitly
/// (#6108); the enum is `#[non_exhaustive]`, so a wildcard arm is
/// still required — it exists only for variants a future crate bump
/// adds, maps them to `UnmodelledStatus`, and is inventoried by the
/// variant table in the tests: when the pin moves, extend the
/// explicit arms and the table together. Statuses the binding can
/// never emit are enumerated explicitly and keep the
/// `UnmodelledStatus` outcome.
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
        Status::LinkedEventFailed => Err(AccountError::LinkedEventFailed),
        Status::LinkedEventChainOpen => Err(AccountError::LinkedEventChainOpen),
        Status::IdMustNotBeZero => Err(AccountError::IdMustNotBeZero),
        Status::IdMustNotBeIntMax => Err(AccountError::IdMustNotBeIntMax),
        Status::CodeMustNotBeZero => Err(AccountError::CodeMustNotBeZero),
        Status::FlagsAreMutuallyExclusive => Err(AccountError::FlagsAreMutuallyExclusive),
        // A created account must start from zero balances.
        Status::DebitsPendingMustBeZero
        | Status::DebitsPostedMustBeZero
        | Status::CreditsPendingMustBeZero
        | Status::CreditsPostedMustBeZero => Err(AccountError::InitialBalanceMustBeZero),
        // Imported events: the binding never sets the `imported`
        // flag or an event timestamp, so a cluster can only return
        // these for requests this binding did not make.
        Status::ImportedEventExpected
        | Status::ImportedEventNotExpected
        | Status::ImportedEventTimestampOutOfRange
        | Status::ImportedEventTimestampMustNotAdvance
        | Status::ImportedEventTimestampMustNotRegress => Err(unmodelled_account(status)),
        // The binding never sets an account timestamp.
        Status::TimestampMustBeZero => Err(unmodelled_account(status)),
        // The binding never sets reserved fields or reserved flags.
        Status::ReservedField | Status::ReservedFlag => Err(unmodelled_account(status)),
        // `#[non_exhaustive]`: variants a future crate bump adds.
        // They keep the UnmodelledStatus outcome (and log) until
        // they are enumerated above — see the variant table test.
        _ => Err(unmodelled_account(status)),
    }
}

/// Map a TigerBeetle transfer-creation status to the PoC's typed
/// errors. Every variant of the pinned crate is matched explicitly
/// (#6108), like [`map_account_status`]; the wildcard arm is the
/// `#[non_exhaustive]` contract for future variants only.
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
        Status::IdMustNotBeZero => Err(TransferError::IdMustNotBeZero),
        Status::IdMustNotBeIntMax => Err(TransferError::IdMustNotBeIntMax),
        Status::DebitAccountIdMustNotBeZero => Err(TransferError::DebitAccountIdMustNotBeZero),
        Status::DebitAccountIdMustNotBeIntMax => Err(TransferError::DebitAccountIdMustNotBeIntMax),
        Status::CreditAccountIdMustNotBeZero => Err(TransferError::CreditAccountIdMustNotBeZero),
        Status::CreditAccountIdMustNotBeIntMax => {
            Err(TransferError::CreditAccountIdMustNotBeIntMax)
        }
        Status::FlagsAreMutuallyExclusive => Err(TransferError::FlagsAreMutuallyExclusive),
        Status::CodeMustNotBeZero => Err(TransferError::CodeMustNotBeZero),
        Status::DebitAccountNotFound => Err(TransferError::DebitAccountNotFound),
        Status::CreditAccountNotFound => Err(TransferError::CreditAccountNotFound),
        Status::DebitAccountAlreadyClosed => Err(TransferError::DebitAccountAlreadyClosed),
        Status::CreditAccountAlreadyClosed => Err(TransferError::CreditAccountAlreadyClosed),
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
        Status::OverflowsTimeout => Err(TransferError::OverflowsTimeout),
        // Pending transfers: the PoC `Transfer` type has no
        // `pending_id` and the binding never sets the pending,
        // post-pending, or void-pending flags, so it cannot emit a
        // transfer these statuses describe.
        Status::PendingIdMustBeZero
        | Status::PendingIdMustNotBeZero
        | Status::PendingIdMustNotBeIntMax
        | Status::PendingIdMustBeDifferent
        | Status::TimeoutReservedForPendingTransfer
        | Status::ClosingTransferMustBePending
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
        | Status::PendingTransferExpired => Err(unmodelled_transfer(status)),
        // Imported events: the binding never sets the `imported`
        // flag or an event timestamp.
        Status::ImportedEventExpected
        | Status::ImportedEventNotExpected
        | Status::ImportedEventTimestampOutOfRange
        | Status::ImportedEventTimestampMustNotAdvance
        | Status::ImportedEventTimestampMustNotRegress
        | Status::ImportedEventTimestampMustPostdateDebitAccount
        | Status::ImportedEventTimestampMustPostdateCreditAccount
        | Status::ImportedEventTimeoutMustBeZero => Err(unmodelled_transfer(status)),
        // The binding never sets a transfer timestamp.
        Status::TimestampMustBeZero => Err(unmodelled_transfer(status)),
        // The binding never sets reserved flags.
        Status::ReservedFlag => Err(unmodelled_transfer(status)),
        // `#[non_exhaustive]`: variants a future crate bump adds.
        // They keep the UnmodelledStatus outcome (and log) until
        // they are enumerated above — see the variant table test.
        _ => Err(unmodelled_transfer(status)),
    }
}

impl LedgerBackend for TigerBeetleLedger {
    fn create_accounts(&mut self, accounts: &[Account]) -> Vec<Result<(), AccountError>> {
        let wire: Vec<tb::Account> = match accounts
            .iter()
            .map(to_tb_account)
            .collect::<std::result::Result<_, _>>()
        {
            Ok(wire) => wire,
            Err(error) => return vec![Err(error); accounts.len()],
        };
        let results = match self.block_on(self.client.create_accounts(&wire)) {
            Ok(results) => results,
            Err(error) => {
                tracing::error!(%error, "TigerBeetle create_accounts request failed");
                return vec![Err(AccountError::BackendFailure); accounts.len()];
            }
        };
        if results.len() != accounts.len() {
            tracing::error!(
                expected = %accounts.len(),
                actual = %results.len(),
                "TigerBeetle returned an unexpected account status count"
            );
            return vec![Err(AccountError::BackendFailure); accounts.len()];
        }
        // Record original codes only for creations TigerBeetle
        // accepted (Created) or confirmed identical (Exists). A
        // rejected creation (e.g. ExistsWithDifferentFields) must not
        // overwrite the registry: the stored record still carries
        // the earlier code, and lookups restore from the registry.
        {
            if let Some(mut registry) = lock_registry(&self.account_codes, "account") {
                for (account, result) in accounts.iter().zip(results.iter()) {
                    if matches!(
                        result.status,
                        tb::CreateAccountStatus::Created | tb::CreateAccountStatus::Exists
                    ) {
                        registry.insert(account.id, account.code);
                    }
                }
            }
        }
        results
            .iter()
            .map(|result| map_account_status(result.status))
            .collect()
    }

    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>> {
        let wire: Vec<tb::Transfer> = match batch
            .iter()
            .map(to_tb_transfer)
            .collect::<std::result::Result<_, _>>()
        {
            Ok(wire) => wire,
            Err(error) => return vec![Err(error); batch.len()],
        };
        let results = match self.block_on(self.client.create_transfers(&wire)) {
            Ok(results) => results,
            Err(error) => {
                tracing::error!(%error, "TigerBeetle create_transfers request failed");
                return vec![Err(TransferError::BackendFailure); batch.len()];
            }
        };
        if results.len() != batch.len() {
            tracing::error!(
                expected = %batch.len(),
                actual = %results.len(),
                "TigerBeetle returned an unexpected transfer status count"
            );
            return vec![Err(TransferError::BackendFailure); batch.len()];
        }
        // Same rule as create_accounts: record original codes only
        // for accepted (Created) or identical (Exists) transfers, so
        // a rejected retry cannot rewrite what lookups report for
        // the stored transfer.
        {
            if let Some(mut registry) = lock_registry(&self.transfer_codes, "transfer") {
                for (transfer, result) in batch.iter().zip(results.iter()) {
                    if matches!(
                        result.status,
                        tb::CreateTransferStatus::Created | tb::CreateTransferStatus::Exists
                    ) {
                        registry.insert(transfer.id, transfer.code);
                    }
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
            let stored_legs_match = already_posted_pattern
                && chain
                    .iter()
                    .all(|leg| match self.try_lookup_transfer(leg.id) {
                        Ok(Some(found)) => found == *leg,
                        Ok(None) => false,
                        Err(error) => {
                            tracing::error!(
                                transfer_id = %leg.id,
                                %error,
                                "TigerBeetle idempotency verification lookup failed"
                            );
                            false
                        }
                    });
            if stored_legs_match {
                for slot in &mut mapped[chain_start..=chain_end] {
                    *slot = Err(TransferError::Exists);
                }
            }
            chain_start = chain_end + 1;
        }
        mapped
    }

    fn try_lookup_transfer(&self, id: u128) -> Result<Option<Transfer>> {
        let found = self
            .block_on(self.client.lookup_transfers(&[id]))
            .context("tigerbeetle lookup_transfers request failed")?;
        let Some(transfer) = found.first() else {
            return Ok(None);
        };
        let code = self.transfer_code(id, transfer.code)?;
        Ok(Some(from_tb_transfer(transfer, code)))
    }

    fn try_lookup_account(&self, id: u128) -> Result<Option<Account>> {
        let found = self
            .block_on(self.client.lookup_accounts(&[id]))
            .context("tigerbeetle lookup_accounts request failed")?;
        let Some(account) = found.first() else {
            return Ok(None);
        };
        let code = self.account_code(id, account.code)?;
        Ok(Some(from_tb_account(account, code)))
    }

    fn try_account_balance(&self, id: u128) -> Result<Option<AccountBalance>> {
        let found = self
            .block_on(self.client.lookup_accounts(&[id]))
            .context("tigerbeetle lookup_accounts request failed")?;
        Ok(found.first().map(|account| AccountBalance {
            debits_posted: account.debits_posted,
            credits_posted: account.credits_posted,
        }))
    }

    fn try_transfers_with_user_data_128(&self, value: u128) -> Result<Vec<Transfer>> {
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
                .context("tigerbeetle query_transfers request failed")?;
            let page_full = found.len() == PAGE as usize;
            let last_timestamp = found.last().map(|transfer| transfer.timestamp);
            for transfer in found
                .iter()
                .filter(|transfer| transfer.user_data_128 == value)
            {
                let code = self.transfer_code(transfer.id, transfer.code)?;
                matched.push(from_tb_transfer(transfer, code));
            }
            match last_timestamp {
                Some(last) if page_full => timestamp_min = last.saturating_add(1),
                _ => break,
            }
        }
        Ok(matched)
    }
}

#[cfg(test)]
mod tests {
    use super::{map_account_status, map_transfer_status, poc_code, wire_code};
    use crate::currency_poc::ledger::{AccountError, TransferError};
    use tigerbeetle as tb;

    #[test]
    fn wire_code_rejects_the_unrepresentable_maximum() {
        assert_eq!(wire_code(0), Some(1));
        assert_eq!(wire_code(u16::MAX), None);
    }

    #[test]
    fn zero_wire_code_is_reported_as_an_error() {
        assert!(poc_code(0).is_err());
    }

    /// Oracle (#6108 acceptance): every `CreateAccountStatus`
    /// variant of the pinned tigerbeetle crate maps to its specific
    /// typed error, except the enumerated unreachable families
    /// (imported events, explicit timestamps, reserved fields/flags)
    /// which map to `UnmodelledStatus` — and no variant panics. The
    /// table lists every variant by name; it is the inventory a
    /// crate bump must extend (the enums are `#[non_exhaustive]`,
    /// so a new variant cannot break compilation — it lands in the
    /// mapper's commented wildcard until it is recorded here).
    #[test]
    fn account_status_mapping_covers_every_variant() {
        use tb::CreateAccountStatus as S;
        let expected: &[(S, Result<(), AccountError>)] = &[
            (S::Created, Ok(())),
            (S::Exists, Err(AccountError::Exists)),
            (
                S::ExistsWithDifferentFlags,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData128,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData64,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData32,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentLedger,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentCode,
                Err(AccountError::ExistsWithDifferentFields),
            ),
            (
                S::LedgerMustNotBeZero,
                Err(AccountError::LedgerMustNotBeZero),
            ),
            (S::LinkedEventFailed, Err(AccountError::LinkedEventFailed)),
            (
                S::LinkedEventChainOpen,
                Err(AccountError::LinkedEventChainOpen),
            ),
            (S::IdMustNotBeZero, Err(AccountError::IdMustNotBeZero)),
            (S::IdMustNotBeIntMax, Err(AccountError::IdMustNotBeIntMax)),
            (S::CodeMustNotBeZero, Err(AccountError::CodeMustNotBeZero)),
            (
                S::FlagsAreMutuallyExclusive,
                Err(AccountError::FlagsAreMutuallyExclusive),
            ),
            (
                S::DebitsPendingMustBeZero,
                Err(AccountError::InitialBalanceMustBeZero),
            ),
            (
                S::DebitsPostedMustBeZero,
                Err(AccountError::InitialBalanceMustBeZero),
            ),
            (
                S::CreditsPendingMustBeZero,
                Err(AccountError::InitialBalanceMustBeZero),
            ),
            (
                S::CreditsPostedMustBeZero,
                Err(AccountError::InitialBalanceMustBeZero),
            ),
            // Unreachable families: enumerated in the mapper with
            // their reasons; the outcome stays UnmodelledStatus.
            (
                S::ImportedEventExpected,
                Err(AccountError::UnmodelledStatus),
            ),
            (
                S::ImportedEventNotExpected,
                Err(AccountError::UnmodelledStatus),
            ),
            (
                S::ImportedEventTimestampOutOfRange,
                Err(AccountError::UnmodelledStatus),
            ),
            (
                S::ImportedEventTimestampMustNotAdvance,
                Err(AccountError::UnmodelledStatus),
            ),
            (
                S::ImportedEventTimestampMustNotRegress,
                Err(AccountError::UnmodelledStatus),
            ),
            (S::TimestampMustBeZero, Err(AccountError::UnmodelledStatus)),
            (S::ReservedField, Err(AccountError::UnmodelledStatus)),
            (S::ReservedFlag, Err(AccountError::UnmodelledStatus)),
        ];
        for (status, want) in expected {
            assert_eq!(&map_account_status(*status), want, "status {status:?}");
        }
    }

    /// Oracle (#6108 acceptance): the transfer-side twin of
    /// `account_status_mapping_covers_every_variant` — every
    /// `CreateTransferStatus` variant of the pinned crate is listed
    /// with its expected outcome, the pending-transfer and
    /// imported-event families being the enumerated
    /// `UnmodelledStatus` set, and no variant panics.
    #[test]
    fn transfer_status_mapping_covers_every_variant() {
        use tb::CreateTransferStatus as S;
        let typed: &[(S, Result<(), TransferError>)] = &[
            (S::Created, Ok(())),
            (S::Exists, Err(TransferError::Exists)),
            (
                S::ExistsWithDifferentFlags,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentPendingId,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentTimeout,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentDebitAccountId,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentCreditAccountId,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentAmount,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData128,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData64,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentUserData32,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentLedger,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (
                S::ExistsWithDifferentCode,
                Err(TransferError::ExistsWithDifferentFields),
            ),
            (S::LinkedEventFailed, Err(TransferError::LinkedEventFailed)),
            (
                S::LinkedEventChainOpen,
                Err(TransferError::LinkedEventChainOpen),
            ),
            (S::IdAlreadyFailed, Err(TransferError::IdAlreadyFailed)),
            (S::IdMustNotBeZero, Err(TransferError::IdMustNotBeZero)),
            (S::IdMustNotBeIntMax, Err(TransferError::IdMustNotBeIntMax)),
            (
                S::DebitAccountIdMustNotBeZero,
                Err(TransferError::DebitAccountIdMustNotBeZero),
            ),
            (
                S::DebitAccountIdMustNotBeIntMax,
                Err(TransferError::DebitAccountIdMustNotBeIntMax),
            ),
            (
                S::CreditAccountIdMustNotBeZero,
                Err(TransferError::CreditAccountIdMustNotBeZero),
            ),
            (
                S::CreditAccountIdMustNotBeIntMax,
                Err(TransferError::CreditAccountIdMustNotBeIntMax),
            ),
            (
                S::FlagsAreMutuallyExclusive,
                Err(TransferError::FlagsAreMutuallyExclusive),
            ),
            (S::CodeMustNotBeZero, Err(TransferError::CodeMustNotBeZero)),
            (
                S::DebitAccountNotFound,
                Err(TransferError::DebitAccountNotFound),
            ),
            (
                S::CreditAccountNotFound,
                Err(TransferError::CreditAccountNotFound),
            ),
            (
                S::DebitAccountAlreadyClosed,
                Err(TransferError::DebitAccountAlreadyClosed),
            ),
            (
                S::CreditAccountAlreadyClosed,
                Err(TransferError::CreditAccountAlreadyClosed),
            ),
            (
                S::AccountsMustBeDifferent,
                Err(TransferError::AccountsMustBeDifferent),
            ),
            (
                S::AccountsMustHaveTheSameLedger,
                Err(TransferError::LedgerMismatch),
            ),
            (
                S::TransferMustHaveTheSameLedgerAsAccounts,
                Err(TransferError::LedgerMismatch),
            ),
            (S::LedgerMustNotBeZero, Err(TransferError::LedgerMismatch)),
            (
                S::ExceedsCredits,
                Err(TransferError::DebitsMustNotExceedCredits),
            ),
            (
                S::ExceedsDebits,
                Err(TransferError::CreditsMustNotExceedDebits),
            ),
            (
                S::OverflowsDebitsPending,
                Err(TransferError::DebitsMustNotExceedCredits),
            ),
            (
                S::OverflowsDebitsPosted,
                Err(TransferError::DebitsMustNotExceedCredits),
            ),
            (
                S::OverflowsDebits,
                Err(TransferError::DebitsMustNotExceedCredits),
            ),
            (
                S::OverflowsCreditsPending,
                Err(TransferError::CreditsMustNotExceedDebits),
            ),
            (
                S::OverflowsCreditsPosted,
                Err(TransferError::CreditsMustNotExceedDebits),
            ),
            (
                S::OverflowsCredits,
                Err(TransferError::CreditsMustNotExceedDebits),
            ),
            (S::OverflowsTimeout, Err(TransferError::OverflowsTimeout)),
        ];
        for (status, want) in typed {
            assert_eq!(&map_transfer_status(*status), want, "status {status:?}");
        }
        // Unreachable families: enumerated in the mapper with their
        // reasons; the outcome stays UnmodelledStatus.
        let unmodelled: &[S] = &[
            S::PendingIdMustBeZero,
            S::PendingIdMustNotBeZero,
            S::PendingIdMustNotBeIntMax,
            S::PendingIdMustBeDifferent,
            S::TimeoutReservedForPendingTransfer,
            S::ClosingTransferMustBePending,
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
            S::ImportedEventExpected,
            S::ImportedEventNotExpected,
            S::ImportedEventTimestampOutOfRange,
            S::ImportedEventTimestampMustNotAdvance,
            S::ImportedEventTimestampMustNotRegress,
            S::ImportedEventTimestampMustPostdateDebitAccount,
            S::ImportedEventTimestampMustPostdateCreditAccount,
            S::ImportedEventTimeoutMustBeZero,
            S::TimestampMustBeZero,
            S::ReservedFlag,
        ];
        for status in unmodelled {
            assert_eq!(
                map_transfer_status(*status),
                Err(TransferError::UnmodelledStatus),
                "status {status:?}"
            );
        }
    }
}
