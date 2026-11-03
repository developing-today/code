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
//! - Status mapping covers the subset of TigerBeetle result codes the
//!   PoC types model. An unmodelled status — or a transport-level
//!   client error, for which the trait has no channel — panics with
//!   the status/error named, rather than being silently folded into a
//!   wrong variant. The scenarios exercise only the mapped subset.
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
        other => panic!("tigerbeetle create_accounts returned unmodelled status {other:?}"),
    }
}

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
        other => panic!("tigerbeetle create_transfers returned unmodelled status {other:?}"),
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
