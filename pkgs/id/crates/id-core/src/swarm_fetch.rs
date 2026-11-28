//! PoC fetch-loop driver for the #2261 multi-provider retrieval scheduler.
//!
//! [`swarm`](crate::swarm) is the scheduling *policy* (which holder serves
//! which chunk-group range, in what order). This module is the fetch-loop
//! half: it turns [`Scheduler`](crate::swarm::Scheduler) assignments into
//! transport fetches, translates transport outcomes back into
//! [`SchedEvent`](crate::swarm::SchedEvent)s, and records per-range
//! provenance. The default `get` path is untouched — this driver is the
//! opt-in experimental PoC wiring, review-gated per the #2261 Adapt
//! decision (experimental PoC only; not the permanent retrieval feature
//! without the review round).
//!
//! Outcome mapping (documented heuristic, pinned by tests):
//! - transport success (bytes verified against the blob root hash) → `Verified`;
//! - verification/hash-mismatch failure → `BadGroup` (discard, requeue, deprioritise);
//! - anything else (refusal, reset, timeout, dead holder) → `Missing`
//!   (requeue elsewhere; the scheduler's `uncovered()` still reports the
//!   distinct nobody-home case when no holder can serve a range).

use std::future::Future;

use crate::swarm::{ByteRange, SchedEvent, Scheduler};

/// Size of one bao chunk in bytes: [`CHUNK_GROUP_BYTES`](crate::swarm::CHUNK_GROUP_BYTES)
/// is 16 chunks per group.
pub const CHUNK_BYTES: u64 = 1024;

/// Outcome of fetching one assigned byte range from one holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcome {
    /// Bytes arrived and verified against the blob root hash.
    Verified,
    /// Bytes arrived but failed verification: discard, requeue, deprioritise.
    BadGroup,
    /// Holder could not serve the range (refused, reset, gone): requeue elsewhere.
    Unavailable,
}

/// Aggregate stats for one driven fetch.
#[derive(Debug, Default)]
pub struct DriveStats {
    /// Assignment waves issued.
    pub waves: u64,
    /// Ranges verified, by group count.
    pub verified_groups: u64,
    /// Bytes fetched more than once (endgame duplicates + refetches).
    pub duplicated_bytes: u64,
    /// True when every group verified.
    pub complete: bool,
}

/// Drive `sched` to completion (or stall) using `fetch` for transport.
///
/// `fetch(holder, range)` performs one range fetch and reports its
/// outcome. Ranges already verified by an endgame duplicate win are
/// skipped (the loser's in-flight fetch still counts as a duplicate
/// when the transport reports bytes for it — transports that report
/// delivered bytes let the driver account them; see
/// [`drive_with_accounting`]).
pub async fn drive_to_completion<F, Fut>(
    sched: &mut Scheduler,
    blob_size: u64,
    mut fetch: F,
) -> DriveStats
where
    F: FnMut(usize, ByteRange) -> Fut,
    Fut: Future<Output = FetchOutcome>,
{
    let (stats, _) = drive_with_accounting(sched, blob_size, |h, r| {
        let fut = fetch(h, r);
        async move { (fut.await, 0u64) }
    })
    .await;
    stats
}

/// Like [`drive_to_completion`], but the fetch closure additionally
/// reports delivered payload bytes so endgame duplicates and refetches
/// are accounted in [`DriveStats::duplicated_bytes`].
pub async fn drive_with_accounting<F, Fut>(
    sched: &mut Scheduler,
    blob_size: u64,
    mut fetch: F,
) -> (DriveStats, Vec<String>)
where
    F: FnMut(usize, ByteRange) -> Fut,
    Fut: Future<Output = (FetchOutcome, u64)>,
{
    let mut stats = DriveStats::default();
    let mut seen_bytes: u64 = 0;
    // Guard against a transport that never makes progress.
    for _ in 0..(sched_group_count(blob_size) + 2) * 4 + 8 {
        if sched.is_complete() {
            break;
        }
        if sched.uncovered().is_some() {
            break;
        }
        let assignments = sched.assignments();
        if assignments.is_empty() {
            break;
        }
        stats.waves += 1;
        for a in &assignments {
            for r in &a.ranges {
                let (outcome, delivered) = fetch(a.holder, *r).await;
                seen_bytes += delivered;
                let event = match outcome {
                    FetchOutcome::Verified => SchedEvent::Verified {
                        holder: a.holder,
                        range: *r,
                    },
                    FetchOutcome::BadGroup => SchedEvent::BadGroup {
                        holder: a.holder,
                        range: *r,
                    },
                    FetchOutcome::Unavailable => SchedEvent::Missing {
                        holder: a.holder,
                        range: *r,
                    },
                };
                sched.apply(event);
            }
        }
    }
    stats.complete = sched.is_complete();
    stats.verified_groups = sched.provenance.len() as u64;
    // Duplicated bytes = delivered payload beyond one full blob copy.
    stats.duplicated_bytes = seen_bytes.saturating_sub(blob_size);
    (stats, sched.provenance.clone())
}

fn sched_group_count(blob_size: u64) -> u64 {
    blob_size.div_ceil(crate::swarm::CHUNK_GROUP_BYTES)
}

/// Convert a scheduler [`ByteRange`] to bao chunk numbers
/// `(first_chunk, one_past_last_chunk)` for a range request.
pub fn byte_range_to_chunks(range: ByteRange) -> (u64, u64) {
    (
        range.start / CHUNK_BYTES,
        range.end.div_ceil(CHUNK_BYTES),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::holders::HeldRanges;

    fn two_complete(blob_size: u64) -> Scheduler {
        Scheduler::new(
            blob_size,
            vec![
                ("a".to_string(), HeldRanges::Complete),
                ("b".to_string(), HeldRanges::Complete),
            ],
        )
    }

    #[tokio::test]
    async fn verified_completes_and_names_both_holders() {
        let blob_size = 4 * crate::swarm::CHUNK_GROUP_BYTES;
        let mut s = two_complete(blob_size);
        let (stats, provenance) = drive_with_accounting(&mut s, blob_size, |_, _| async {
            (FetchOutcome::Verified, crate::swarm::CHUNK_GROUP_BYTES)
        })
        .await;
        assert!(stats.complete);
        assert_eq!(stats.duplicated_bytes, 0);
        let holders: std::collections::BTreeSet<_> = provenance
            .iter()
            .filter_map(|l| l.rsplit(" <- holder ").next())
            .collect();
        assert!(holders.len() >= 2, "{provenance:?}");
    }

    #[tokio::test]
    async fn bad_group_requeues_to_other_holder() {
        let blob_size = 2 * crate::swarm::CHUNK_GROUP_BYTES;
        let mut s = two_complete(blob_size);
        let mut failed = false;
        let (stats, _) = drive_with_accounting(&mut s, blob_size, |h, r| {
            let out = if h == 0 && !failed {
                failed = true;
                (FetchOutcome::BadGroup, 0)
            } else {
                (FetchOutcome::Verified, r.end - r.start)
            };
            async move { out }
        })
        .await;
        assert!(stats.complete);
    }

    #[tokio::test]
    async fn unavailable_fails_over_and_counts_duplicates() {
        let blob_size = 2 * crate::swarm::CHUNK_GROUP_BYTES;
        let mut s = two_complete(blob_size);
        // Holder 1 always unavailable; every byte must come from holder 0,
        // and each failed attempt's delivered bytes (none here) plus the
        // re-fetch keeps accounting consistent.
        let (stats, _) = drive_with_accounting(&mut s, blob_size, |h, r| async move {
            if h == 1 {
                (FetchOutcome::Unavailable, 0)
            } else {
                (FetchOutcome::Verified, r.end - r.start)
            }
        })
        .await;
        assert!(stats.complete);
        // Endgame duplicate: report full-blob bytes twice, accounting
        // records exactly one extra copy.
        let mut s2 = two_complete(blob_size);
        let (stats2, _) = drive_with_accounting(&mut s2, blob_size, |_, r| async move {
            (FetchOutcome::Verified, (r.end - r.start) * 2)
        })
        .await;
        assert!(stats2.complete);
        assert_eq!(stats2.duplicated_bytes, blob_size);
    }

    #[test]
    fn chunk_mapping_is_group_aligned() {
        let (first, last) =
            byte_range_to_chunks(ByteRange::new(0, crate::swarm::CHUNK_GROUP_BYTES));
        assert_eq!((first, last), (0, 16));
    }
}
