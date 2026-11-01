//! Recovery scanner: reconciles unresolved intent records against
//! the ledger by pre-generated transfer ID.
//!
//! For every `Recorded` / `Submitted` record (#2289):
//! - every leg present in the ledger ⇒ the conversion POSTED (the
//!   crash happened after submission but before the state write);
//! - no leg present ⇒ the submission never landed; resubmit the same
//!   byte-identical legs (same IDs — idempotent by construction) or
//!   abandon the record, per [`RecoveryPolicy`];
//! - some legs present ⇒ impossible for an atomic linked chain, so it
//!   is reported as [`ReconcileOutcome::Inconsistent`] for a human
//!   rather than guessed at.

use anyhow::{Context, Result};

use super::intent::{IntentState, IntentStore};
use super::ledger::{LedgerBackend, TransferError};

/// What the scanner does with a record whose legs never landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryPolicy {
    /// Resubmit the persisted legs with their original transfer IDs.
    Resubmit,
    /// Mark the record `Abandoned` and move on.
    Abandon,
}

/// The reconciliation result for one intent record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// All legs were already in the ledger; state advanced to Posted.
    AlreadyPosted,
    /// No legs were present; the scanner resubmitted them and they posted.
    Resubmitted,
    /// No legs were present; the record was abandoned by policy.
    Abandoned,
    /// A partial leg set was found — not possible for an atomic linked
    /// chain; left unresolved for investigation.
    Inconsistent,
}

/// Reconcile every unresolved record in `store` against `backend`.
/// Returns one `(conversion_id, outcome)` pair per unresolved record.
pub fn reconcile<B: LedgerBackend, S: IntentStore>(
    backend: &mut B,
    store: &mut S,
    policy: RecoveryPolicy,
) -> Result<Vec<(u128, ReconcileOutcome)>> {
    let mut outcomes = Vec::new();
    for mut record in store.unresolved().context("listing unresolved intents")? {
        let present = record
            .legs
            .iter()
            .filter(|leg| backend.lookup_transfer(leg.id).is_some())
            .count();
        let outcome = if present == record.legs.len() {
            record.state = IntentState::Posted;
            store
                .put(&record)
                .context("persist Posted after reconcile")?;
            ReconcileOutcome::AlreadyPosted
        } else if present == 0 {
            match policy {
                RecoveryPolicy::Resubmit => {
                    let results = backend.create_transfers(&record.legs);
                    let failed = results
                        .iter()
                        .find(|result| !matches!(result, Ok(()) | Err(TransferError::Exists)));
                    if let Some(error) = failed {
                        anyhow::bail!(
                            "recovery resubmission of conversion {} failed: {error:?}",
                            record.conversion_id
                        );
                    }
                    record.state = IntentState::Posted;
                    store
                        .put(&record)
                        .context("persist Posted after resubmission")?;
                    ReconcileOutcome::Resubmitted
                }
                RecoveryPolicy::Abandon => {
                    record.state = IntentState::Abandoned;
                    store
                        .put(&record)
                        .context("persist Abandoned after reconcile")?;
                    ReconcileOutcome::Abandoned
                }
            }
        } else {
            ReconcileOutcome::Inconsistent
        };
        outcomes.push((record.conversion_id, outcome));
    }
    Ok(outcomes)
}
