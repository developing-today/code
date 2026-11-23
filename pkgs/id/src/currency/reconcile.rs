//! F4: graph ↔ registry ↔ ledger reconciliation and the supply
//! audit (issue #2320, design section 7 of
//! `doc/2026-10-29T12-42-21Z_design_currency_control_plane`).
//!
//! Three stores hold three truths about a currency: the registry
//! says what a ledger *means*, the ledger holds the balances, and
//! the signed graph records the business events the ledger writes
//! were made for. This job is the checker section 7.3 specifies:
//!
//! 1. **Supply audit** — per currency, the issuer account's net
//!    debits (everything that left the issuer) are compared
//!    against the sum of holder balances plus the fee and burn
//!    account balances. Issuance is a debit from the issuer, so a
//!    negative issuer balance *is* the circulating supply; every
//!    unit of it must be sitting in a holder, fee, or burn account.
//! 2. **Linkage** — each recent transfer's `user_data_128` stamp
//!    (section 7.1) is resolved to its signed graph object through
//!    the gateway's [`GraphLog`]. A non-zero stamp that resolves
//!    to nothing is reported, never silently ignored.
//! 3. **Amount agreement** — where the resolved graph object is
//!    an amount-bearing value event (issue / transfer / burn),
//!    its amount is compared to the transfer's. On disagreement
//!    the ledger wins (it is the balance truth, section 7.2) and
//!    the graph object is flagged for review.
//!
//! The job is **report-only**: it never repairs, deletes, or
//! rewrites anything. Its output is the operator surface — a
//! [`ReconReport`] whose findings name the currency, the ledger,
//! and the offending transfer or graph object for each drift.
//!
//! A read the backend cannot serve (the `LedgerBackend`
//! enumeration seam errors) becomes a
//! [`ReconFinding::LedgerReadFailed`] for that currency instead
//! of aborting the run: a reconciliation report that hides its
//! own blindness would read as a match over no evidence.

use id_currency_registry::{AccountKind, CurrencyRecord, PLATFORM_OWNER_ID, account_spec};
use serde::{Deserialize, Serialize};

use crate::currency::gateway_poc::GraphLog;
use crate::currency::graph::{TokenBurn, TokenIssue, TokenTransfer};
use crate::currency_poc::ledger::{AccountBalance, LedgerBackend};

/// One currency's supply-audit outcome (design section 7.3
/// step 1). Amounts are signed minor units: `issued` is the
/// issuer account's debits minus its credits, `accounted` is
/// holder balances plus the fee and burn balances, each computed
/// as credits minus debits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyAudit {
    /// Net supply that left the issuer account.
    pub issued: i128,
    /// Supply accounted for by holder + fee + burn balances.
    pub accounted: i128,
}

impl SupplyAudit {
    /// Whether the two sides agree exactly.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.issued == self.accounted
    }

    /// `issued - accounted`: positive when supply left the
    /// audited accounts for somewhere the audit does not count,
    /// negative when more is held than the issuer ever issued.
    #[must_use]
    pub fn difference(&self) -> i128 {
        self.issued - self.accounted
    }
}

/// One drift the job found. Every variant names the currency and
/// its ledger so an operator can go straight to the record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconFinding {
    /// The supply audit's two sides disagree (section 7.3).
    SupplyMismatch {
        /// The currency's registry id.
        currency_id: String,
        /// The currency's ledger.
        ledger_id: u32,
        /// Net supply that left the issuer account.
        issued: i128,
        /// Supply accounted for by holder + fee + burn balances.
        accounted: i128,
    },
    /// A transfer carries a non-zero `user_data_128` stamp that
    /// resolves to no graph object (section 7.2: graph object
    /// missing for a ledger write). Reported, never ignored.
    UnresolvedGraphLink {
        /// The currency's registry id.
        currency_id: String,
        /// The currency's ledger.
        ledger_id: u32,
        /// The ledger transfer whose stamp does not resolve.
        transfer_id: u128,
        /// The stamp that resolved to nothing.
        user_data_128: u128,
    },
    /// A transfer and the graph object it is stamped with
    /// disagree on the amount (section 7.2). The ledger is the
    /// balance truth; the graph object named here is the one
    /// flagged for review.
    AmountDrift {
        /// The currency's registry id.
        currency_id: String,
        /// The currency's ledger.
        ledger_id: u32,
        /// The ledger transfer.
        transfer_id: u128,
        /// The graph object's id (hex), as receipts name it.
        graph_object: String,
        /// The amount the ledger moved.
        ledger_amount: u128,
        /// The amount the graph object records.
        graph_amount: u128,
    },
    /// A read the job needed failed on the backend, so part of
    /// this currency's audit did not run. The report carries the
    /// blindness instead of a false clean result.
    LedgerReadFailed {
        /// The currency's registry id.
        currency_id: String,
        /// The currency's ledger.
        ledger_id: u32,
        /// What was being read and why it failed.
        detail: String,
    },
}

/// The audit outcome for one currency, clean or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyRecon {
    /// The currency's registry id.
    pub currency_id: String,
    /// The currency's ledger.
    pub ledger_id: u32,
    /// The supply audit, or `None` when its reads failed (the
    /// failure itself is in the report's findings).
    pub supply: Option<SupplyAudit>,
    /// How many recent transfers the linkage check examined.
    pub transfers_checked: usize,
    /// Examined transfers carrying no graph stamp
    /// (`user_data_128 == 0`): outside the linkage rule, counted
    /// here so the report shows they were seen.
    pub unstamped_transfers: usize,
}

/// The full report: one section per currency plus every finding.
/// This struct is the operator surface of section 7.3 step 3.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconReport {
    /// Per-currency audit outcomes, in currency-key order.
    pub currencies: Vec<CurrencyRecon>,
    /// Every drift found, grouped by currency in the same order.
    pub findings: Vec<ReconFinding>,
}

impl ReconReport {
    /// Whether the run found no drift at all.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// The amount a graph body commits to, when it is one of the
/// amount-bearing value events (issue, transfer, burn). Other
/// bodies — a currency definition, an undecodable body — carry
/// no single amount to compare, and return `None`.
#[must_use]
pub fn graph_amount(body: &[u8]) -> Option<u128> {
    if let Ok(token) = TokenIssue::from_body(body) {
        return Some(token.amount);
    }
    if let Ok(token) = TokenTransfer::from_body(body) {
        return Some(token.amount);
    }
    if let Ok(token) = TokenBurn::from_body(body) {
        return Some(token.amount);
    }
    None
}

/// A balance as a signed net: credits posted minus debits
/// posted. Amounts beyond `i128` saturate; ledger amounts are
/// minor units and nowhere near that range in practice.
fn net(balance: AccountBalance) -> i128 {
    fn as_i128(amount: u128) -> i128 {
        i128::try_from(amount).unwrap_or(i128::MAX)
    }
    as_i128(balance.credits_posted) - as_i128(balance.debits_posted)
}

/// The posted balance of one account, or zero when the account
/// does not exist on the ledger: accounts are ensured lazily on
/// first use, so an absent fee or burn account has simply never
/// held anything. A backend *failure* is not absence — it
/// propagates as the job's error string.
fn balance_of<B: LedgerBackend>(backend: &B, account_id: u128) -> Result<AccountBalance, String> {
    backend
        .try_account_balance(account_id)
        .map(|found| found.unwrap_or_default())
        .map_err(|error| format!("balance read for account {account_id}: {error}"))
}

/// Audit one currency against the ledger and the graph log.
/// Returns the currency's report section and its findings.
///
/// `recent_limit` bounds the linkage check to the most recent
/// transfers on the currency's ledger (section 7.3 step 2 says
/// "each recent transfer"); the supply audit always reads the
/// full account set.
pub fn audit_currency<B: LedgerBackend>(
    record: &CurrencyRecord,
    backend: &B,
    graph: &GraphLog,
    recent_limit: usize,
) -> (CurrencyRecon, Vec<ReconFinding>) {
    let currency_id = record.currency_id.clone();
    let ledger_id = record.ledger_id;
    let mut findings = Vec::new();
    let mut recon = CurrencyRecon {
        currency_id: currency_id.clone(),
        ledger_id,
        supply: None,
        transfers_checked: 0,
        unstamped_transfers: 0,
    };

    // Supply audit (section 7.3 step 1): issuer net debits
    // against holder balances + fee + burn balances. The
    // per-currency system accounts are the factory's derived
    // ids; holder accounts are the ledger's own accounts of
    // holder kind (the factory's kind code, which the gateway's
    // cluster adapter stamps onto the ledger account).
    match backend.try_accounts_on_ledger(ledger_id) {
        Ok(accounts) => {
            let issuer = account_spec(ledger_id, AccountKind::Issuer, &record.issuer_id, "");
            let fee = account_spec(ledger_id, AccountKind::Fee, PLATFORM_OWNER_ID, "");
            let burn = account_spec(ledger_id, AccountKind::Burn, PLATFORM_OWNER_ID, "");
            let supply = (|| -> Result<SupplyAudit, String> {
                let issued = -net(balance_of(backend, issuer.id)?);
                let mut accounted =
                    net(balance_of(backend, fee.id)?) + net(balance_of(backend, burn.id)?);
                for account in &accounts {
                    if account.code == u16::from(AccountKind::Holder.code()) {
                        accounted += net(balance_of(backend, account.id)?);
                    }
                }
                Ok(SupplyAudit { issued, accounted })
            })();
            match supply {
                Ok(supply) => {
                    if !supply.matches() {
                        findings.push(ReconFinding::SupplyMismatch {
                            currency_id: currency_id.clone(),
                            ledger_id,
                            issued: supply.issued,
                            accounted: supply.accounted,
                        });
                    }
                    recon.supply = Some(supply);
                }
                Err(detail) => findings.push(ReconFinding::LedgerReadFailed {
                    currency_id: currency_id.clone(),
                    ledger_id,
                    detail,
                }),
            }
        }
        Err(error) => findings.push(ReconFinding::LedgerReadFailed {
            currency_id: currency_id.clone(),
            ledger_id,
            detail: format!("account enumeration for ledger {ledger_id}: {error}"),
        }),
    }

    // Linkage + amount agreement (sections 7.1, 7.2, 7.3
    // step 2) over the recent transfers on this ledger.
    match backend.try_transfers_on_ledger(ledger_id, recent_limit) {
        Ok(transfers) => {
            recon.transfers_checked = transfers.len();
            for transfer in &transfers {
                let stamp = transfer.user_data_128;
                if stamp == 0 {
                    recon.unstamped_transfers += 1;
                    continue;
                }
                match graph.resolve(stamp) {
                    None => findings.push(ReconFinding::UnresolvedGraphLink {
                        currency_id: currency_id.clone(),
                        ledger_id,
                        transfer_id: transfer.id,
                        user_data_128: stamp,
                    }),
                    Some((event, body)) => {
                        if let Some(graph_amount) = graph_amount(body) {
                            if graph_amount != transfer.amount {
                                findings.push(ReconFinding::AmountDrift {
                                    currency_id: currency_id.clone(),
                                    ledger_id,
                                    transfer_id: transfer.id,
                                    graph_object: event.id.to_hex(),
                                    ledger_amount: transfer.amount,
                                    graph_amount,
                                });
                            }
                        }
                    }
                }
            }
        }
        Err(error) => findings.push(ReconFinding::LedgerReadFailed {
            currency_id: currency_id.clone(),
            ledger_id,
            detail: format!("transfer enumeration for ledger {ledger_id}: {error}"),
        }),
    }

    (recon, findings)
}
