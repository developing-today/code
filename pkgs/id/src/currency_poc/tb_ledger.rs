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
//!   while the PoC types use `0` as the default code. A zero code is
//!   sent as [`ZERO_CODE_SUBSTITUTE`]; the binding records the original
//!   code of every account/transfer it creates and restores it on
//!   lookups, so a looked-up leg still compares equal to the persisted
//!   intent leg the recovery scanner checks (`super::recovery`).
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

/// Substitute wire code for the PoC's default code `0` (TigerBeetle
/// rejects code 0). Restored to `0` on lookups; see module docs.
const ZERO_CODE_SUBSTITUTE: u16 = 1;

fn wire_code(code: u16) -> u16 {
    if code == 0 {
        ZERO_CODE_SUBSTITUTE
    } else {
        code
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

    fn account_code(&self, id: u128, wire: u16) -> u16 {
        self.account_codes
            .lock()
            .expect("account code registry poisoned")
            .get(&id)
            .copied()
            .unwrap_or(wire)
    }

    fn transfer_code(&self, id: u128, wire: u16) -> u16 {
        self.transfer_codes
            .lock()
            .expect("transfer code registry poisoned")
            .get(&id)
            .copied()
            .unwrap_or(wire)
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
        {
            let mut registry = self
                .account_codes
                .lock()
                .expect("account code registry poisoned");
            for account in accounts {
                registry.insert(account.id, account.code);
            }
        }
        let wire: Vec<tb::Account> = accounts.iter().map(to_tb_account).collect();
        let results = self
            .block_on(self.client.create_accounts(&wire))
            .expect("tigerbeetle create_accounts request failed");
        assert_eq!(
            results.len(),
            accounts.len(),
            "tigerbeetle returned one result per account"
        );
        results
            .iter()
            .map(|result| map_account_status(result.status))
            .collect()
    }

    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>> {
        {
            let mut registry = self
                .transfer_codes
                .lock()
                .expect("transfer code registry poisoned");
            for transfer in batch {
                registry.insert(transfer.id, transfer.code);
            }
        }
        let wire: Vec<tb::Transfer> = batch.iter().map(to_tb_transfer).collect();
        let results = self
            .block_on(self.client.create_transfers(&wire))
            .expect("tigerbeetle create_transfers request failed");
        assert_eq!(
            results.len(),
            batch.len(),
            "tigerbeetle returned one result per transfer"
        );
        results
            .iter()
            .map(|result| map_transfer_status(result.status))
            .collect()
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
        let filter = tb::QueryFilter {
            user_data_128: value,
            limit: 8_189,
            ..Default::default()
        };
        let found = self
            .block_on(self.client.query_transfers(filter))
            .expect("tigerbeetle query_transfers request failed");
        found
            .iter()
            .map(|transfer| {
                from_tb_transfer(transfer, self.transfer_code(transfer.id, transfer.code))
            })
            .collect()
    }
}
