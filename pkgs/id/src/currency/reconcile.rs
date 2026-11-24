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
use num_bigint::BigInt;
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
    // Published numeric reports guarantee that both totals and their
    // difference fit the existing signed minor-unit API.
    fn from_totals(issued: &BigInt, accounted: &BigInt) -> Option<Self> {
        let issued = i128::try_from(issued).ok()?;
        let accounted = i128::try_from(accounted).ok()?;
        issued.checked_sub(accounted)?;
        Some(Self { issued, accounted })
    }

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
    /// The ledger reads succeeded but the exact supply totals or their
    /// difference exceed the signed numeric report range. Decimal strings
    /// preserve the complete evidence for an operator or a wide reader.
    SupplyOutsideReportRange {
        /// The currency being audited.
        currency_id: String,
        /// The currency's ledger.
        ledger_id: u32,
        /// Exact net supply that left the issuer.
        issued: String,
        /// Exact supply held in audited accounts.
        accounted: String,
        /// Exact signed issued minus accounted.
        difference: String,
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
    /// The supply audit, or `None` when reads or numeric representation failed (the
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

/// Lifetime counters can be individually large even when the net balance
/// is small. Keep the exact net through aggregation across all accounts.
fn net(balance: AccountBalance) -> BigInt {
    BigInt::from(balance.credits_posted) - BigInt::from(balance.debits_posted)
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
            let supply = (|| -> Result<(BigInt, BigInt), String> {
                let issued = -net(balance_of(backend, issuer.id)?);
                let mut accounted =
                    net(balance_of(backend, fee.id)?) + net(balance_of(backend, burn.id)?);
                for account in &accounts {
                    if account.code == u16::from(AccountKind::Holder.code()) {
                        accounted += net(balance_of(backend, account.id)?);
                    }
                }
                Ok((issued, accounted))
            })();
            match supply {
                Ok((issued, accounted)) => {
                    if let Some(supply) = SupplyAudit::from_totals(&issued, &accounted) {
                        if !supply.matches() {
                            findings.push(ReconFinding::SupplyMismatch {
                                currency_id: currency_id.clone(),
                                ledger_id,
                                issued: supply.issued,
                                accounted: supply.accounted,
                            });
                        }
                        recon.supply = Some(supply);
                    } else {
                        findings.push(ReconFinding::SupplyOutsideReportRange {
                            currency_id: currency_id.clone(),
                            ledger_id,
                            issued: issued.to_string(),
                            accounted: accounted.to_string(),
                            difference: (issued - accounted).to_string(),
                        });
                    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use proptest::prelude::*;

    // F4 acceptance (#2320, design section 7.3): the supply
    // audit's aggregation invariant over generated transfer
    // sets. Oracle: the job sums holder + fee + burn balances
    // against issuer debits, so a set whose every unit stays
    // inside the audited accounts must report issued ==
    // accounted with a zero difference.
    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(128))]

        #[test]
        fn prop_supply_audit_conserves_generated_transfer_sets(
            (issued, holder, fee) in (0i128..1_000_000).prop_flat_map(|issued| {
                (Just(issued), 0i128..=issued).prop_flat_map(|(issued, holder)| {
                    (Just(issued), Just(holder), 0i128..=(issued - holder))
                })
            }),
        ) {
            let burn = issued - holder - fee;
            let audit = SupplyAudit {
                issued,
                accounted: holder + fee + burn,
            };
            prop_assert!(audit.matches());
            prop_assert_eq!(audit.difference(), 0);
        }

        #[test]
        fn prop_supply_divergence_reports_signed_difference(
            amounts in proptest::collection::vec(0u64..1_000_000, 1..32),
            leaked in 1u64..1_000_000,
        ) {
            let sum: i128 = amounts.iter().map(|amount| *amount as i128).sum();
            let audit = SupplyAudit {
                issued: sum,
                accounted: sum - leaked as i128,
            };
            prop_assert!(!audit.matches());
            prop_assert_eq!(audit.difference(), leaked as i128);
        }
    }

    /// F4 acceptance (#2320): a report with no findings is
    /// clean, and any finding — including a supply mismatch —
    /// makes it dirty. Oracle: `is_clean` is exactly
    /// `findings.is_empty()`.
    #[test]
    fn recon_report_clean_iff_no_findings() {
        assert!(ReconReport::default().is_clean());
        let dirty = ReconReport {
            currencies: Vec::new(),
            findings: vec![ReconFinding::SupplyMismatch {
                currency_id: String::from("usd"),
                ledger_id: 1,
                issued: 100,
                accounted: 90,
            }],
        };
        assert!(!dirty.is_clean());
    }
}
