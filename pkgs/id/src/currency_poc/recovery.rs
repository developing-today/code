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
//!
//! A record whose leg vector is empty or repeats a transfer ID is
//! malformed (#4133): it is reported `Inconsistent` too, never
//! `Posted` — an empty vector would satisfy the full-match check
//! vacuously, and a repeated ID would count one ledger transfer
//! once per entry.

use std::collections::HashSet;

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
    /// Resubmission of this record was rejected by the ledger; the
    /// record stays unresolved and the scan continued with the
    /// remaining records instead of aborting.
    ResubmissionRejected,
}

/// Reconcile every unresolved record in `store` against `backend`.
/// Returns one `(conversion_id, outcome)` pair per unresolved record.
pub fn reconcile<B: LedgerBackend, S: IntentStore>(
    backend: &mut B,
    store: &mut S,
    policy: RecoveryPolicy,
) -> Result<Vec<(u128, ReconcileOutcome)>> {
    let mut outcomes = Vec::new();
    let records = store.unresolved().context("listing unresolved intents")?;
    // Entries the listing skipped (#4125) stay on disk for an
    // operator; surface each one distinctly here instead of letting
    // it pass silently, and reconcile everything readable regardless.
    for unreadable in store.unreadable_entries() {
        eprintln!(
            "[recovery] skipping unreadable intent entry {}: {}",
            unreadable.path.display(),
            unreadable.reason
        );
    }
    for mut record in records {
        // A leg vector is the scanner's only statement of what must
        // be in the ledger, so it must name at least one transfer and
        // each transfer at most once (#4133): an empty vector would
        // satisfy `matched == record.legs.len()` vacuously (0 == 0),
        // and a repeated ID would count one ledger transfer once per
        // entry. Either shape is malformed — report it Inconsistent
        // and leave the record unresolved rather than guessing.
        let mut leg_ids = HashSet::with_capacity(record.legs.len());
        let malformed =
            record.legs.is_empty() || !record.legs.iter().all(|leg| leg_ids.insert(leg.id));
        if malformed {
            outcomes.push((record.conversion_id, ReconcileOutcome::Inconsistent));
            continue;
        }

        // Existence by ID is not proof of posting: a looked-up
        // transfer must equal the persisted leg, otherwise the ID
        // collided with an unrelated transfer.
        let mut present = 0usize;
        let mut matched = 0usize;
        for leg in &record.legs {
            if let Some(found) = backend.lookup_transfer(leg.id) {
                present += 1;
                if found == *leg {
                    matched += 1;
                }
            }
        }
        let outcome = if matched == record.legs.len() {
            record.state = IntentState::Posted;
            store
                .put(&record)
                .context("persist Posted after reconcile")?;
            ReconcileOutcome::AlreadyPosted
        } else if present == 0 {
            match policy {
                RecoveryPolicy::Resubmit => {
                    let results = backend.create_transfers(&record.legs);
                    // A short or long result vector violates the backend
                    // contract; never infer that the unreported legs posted.
                    if results.len() != record.legs.len() {
                        outcomes
                            .push((record.conversion_id, ReconcileOutcome::ResubmissionRejected));
                        continue;
                    }
                    let failed = results
                        .iter()
                        .find(|result| !matches!(result, Ok(()) | Err(TransferError::Exists)));
                    if failed.is_some() {
                        // One rejected resubmission must not abort
                        // recovery of every later record: leave this
                        // record unresolved, report it, and continue.
                        outcomes
                            .push((record.conversion_id, ReconcileOutcome::ResubmissionRejected));
                        continue;
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
