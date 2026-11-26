//! Snapshot serving: the #5800 snapshot fetch boundary for public
//! shares — share snapshots served to readers with page-and-resume,
//! under the mandatory serving limits of the #5802 decision record
//! (D3 item 4, invariant 8).
//!
//! The enforcement core is [`crate::serving_limits`]: per-share caps,
//! the per-(share, reader) rate cap, and the per-owner aggregate
//! ceiling, composed by [`ServingGate`]. The adapter public-read
//! boundary ([`ServingGate`] wired into `query_public`) serves a
//! fetch whole or not at all, because that surface carries a literal
//! `LIMIT` but no offset — there is no resume position to hand back
//! there. This module is the boundary where resume exists: a
//! snapshot is an ordered cut of objects, a reader fetches it one
//! page at a time, and every capped page names its cap
//! ([`FetchCap`]) and hands back the position the reader resumes
//! from under #5800's chunk-resume rule
//! ([`FetchPage::resume_from`]).
//!
//! [`SnapshotServing`] holds one [`ServingGate`] across calls — the
//! gate's rate windows, aggregate budgets, and concurrency counts
//! are meaningful only held across fetches, so they live in this
//! serving-layer type, not in a stateless snapshot source. Per
//! reader fetch it runs the gate in the gate's own order — rate
//! check, page planning under the per-share snapshot caps,
//! aggregate charge — with the owner's concurrency slot
//! ([`ServingGate::begin_owner_fetch`] /
//! [`ServingGate::end_owner_fetch`]) held around page extraction,
//! as the core's docs prescribe. A refused fetch serves nothing and
//! charges nothing (the gate's own all-or-nothing charge); a capped
//! fetch serves exactly its page — never a partial object, never a
//! page presented as the whole snapshot.
//!
//! Objects are byte strings (`AsRef<[u8]>`): the serving layer
//! needs only each object's length to plan the page, and returns
//! the page as a slice of the snapshot in snapshot order. The
//! receiving side's #5800 §5 caps are the reader's own layer and
//! are untouched here, exactly as in the core.
//!
//! Time is injected as `now_ms`, as in the core, so every decision
//! is deterministic and unit-testable without sleeping.

use crate::serving_limits::{
    FetchPage, OwnerAggregateLimits, ServingGate, ServingRefusal, ShareServingLimits,
};

/// The serving context of one snapshot fetch: the opaque ids the
/// gate keys on, the owner's tuned limits for the share, and the
/// caller's clock.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotReadContext<'a> {
    /// The share's owner (the aggregate budget and the concurrency
    /// ceiling are charged to the owner, across all their shares).
    pub owner: &'a str,
    /// The share being read (the rate window is per share and
    /// reader).
    pub share: &'a str,
    /// The reader, by identity-or-address.
    pub reader: &'a str,
    /// The owner's tuned limits for this share, if any; the limits
    /// in force clamp these to the platform ceilings.
    pub owner_tuned_limits: Option<&'a ShareServingLimits>,
    /// Milliseconds on the caller's monotonic clock, as in the gate.
    pub now_ms: u64,
}

/// One served page of a snapshot cut: the gate's [`FetchPage`]
/// plan — start, counts, the cap that stopped the fetch if any,
/// and the #5800 resume position — together with the page's
/// objects, in snapshot order.
///
/// `fetch.capped_by` and `fetch.resume_from` are set together or
/// not at all: when they are set, this page is not the whole
/// snapshot and the reader resumes at `resume_from`; when they are
/// `None`, this page completes the snapshot — including when it
/// lands exactly on a cap with nothing left over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPage<'a, T> {
    /// The gate's plan for this fetch.
    pub fetch: FetchPage,
    /// The page's objects: `snapshot[fetch.start .. fetch.start +
    /// fetch.objects]`, in snapshot order.
    pub objects: &'a [T],
}

/// The snapshot serving layer for public shares: one
/// [`ServingGate`] held across reader fetches, wiring the
/// mandatory serving limits into the #5800 snapshot fetch path.
#[derive(Debug)]
pub struct SnapshotServing {
    gate: ServingGate,
    platform_ceilings: ShareServingLimits,
}

impl SnapshotServing {
    /// A serving layer enforcing the deployment's per-owner
    /// aggregate limits under the platform's per-share ceilings.
    #[must_use]
    pub fn new(owner_limits: OwnerAggregateLimits, platform_ceilings: ShareServingLimits) -> Self {
        Self {
            gate: ServingGate::new(owner_limits),
            platform_ceilings,
        }
    }

    /// Serve one reader fetch of one share's snapshot cut.
    ///
    /// - `snapshot` is the cut's objects in snapshot order; only
    ///   each object's byte length is read before the page is cut;
    /// - `from` is the object index to start at — 0 for a fresh
    ///   fetch, or a previous page's `resume_from` for a resumed
    ///   one.
    ///
    /// The owner's concurrency slot is taken first and held across
    /// page extraction, and released on every outcome once taken.
    /// A fetch the gate refuses serves nothing and charges
    /// nothing; a capped fetch returns its page with the cap named
    /// and the resume position set.
    ///
    /// # Errors
    ///
    /// The gate's stable named [`ServingRefusal`] — rate limited,
    /// owner aggregate exceeded, or owner concurrency exceeded.
    pub fn serve_page<T: AsRef<[u8]>>(
        &mut self,
        context: &SnapshotReadContext<'_>,
        snapshot: &[T],
        from: u64,
    ) -> Result<SnapshotPage<'_, T>, ServingRefusal> {
        let limits =
            ShareServingLimits::effective(context.owner_tuned_limits, &self.platform_ceilings);
        self.gate.begin_owner_fetch(context.owner)?;
        let sizes: Vec<u64> = snapshot
            .iter()
            .map(|object| u64::try_from(object.as_ref().len()).unwrap_or(u64::MAX))
            .collect();
        let planned = self.gate.serve_fetch(
            context.owner,
            context.share,
            context.reader,
            &sizes,
            from,
            &limits,
            context.now_ms,
        );
        self.gate.end_owner_fetch(context.owner);
        let fetch = planned?;
        // `fetch.start` echoes `from`, which a caller may set past
        // the end of the cut (an exhausted snapshot is a complete
        // empty page in the core); clamp the slice range to the
        // snapshot so an out-of-range resume serves empty rather
        // than panicking.
        let total = u64::try_from(snapshot.len()).unwrap_or(u64::MAX);
        let start = fetch.start.min(total);
        let end = start.saturating_add(fetch.objects).min(total);
        let start = usize::try_from(start).unwrap_or(snapshot.len());
        let end = usize::try_from(end).unwrap_or(snapshot.len());
        Ok(SnapshotPage {
            fetch,
            objects: &snapshot[start..end],
        })
    }

    /// Mark one of the owner's fetches as in flight outside a
    /// single [`SnapshotServing::serve_page`] call (for example a
    /// caller streaming a page's bytes itself). Pair every
    /// admission with [`SnapshotServing::end_owner_fetch`].
    ///
    /// # Errors
    ///
    /// [`ServingRefusal::OwnerConcurrencyExceeded`] when the
    /// owner's concurrent-fetch ceiling is reached.
    pub fn begin_owner_fetch(&mut self, owner: &str) -> Result<(), ServingRefusal> {
        self.gate.begin_owner_fetch(owner)
    }

    /// Mark one of the owner's in-flight fetches as finished.
    /// Ending a fetch that was never begun is a no-op.
    pub fn end_owner_fetch(&mut self, owner: &str) {
        self.gate.end_owner_fetch(owner);
    }

    /// Prune expired rate and aggregate windows. This bounds
    /// retained state and changes no admission decision.
    pub fn prune(&mut self, now_ms: u64) {
        self.gate.prune(now_ms);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::serving_limits::FetchCap;
    use proptest::prelude::*;

    fn share_limits(objects: u64, bytes: u64, rate: u32) -> ShareServingLimits {
        ShareServingLimits {
            max_objects_per_fetch: objects,
            max_bytes_per_fetch: bytes,
            max_requests_per_minute: rate,
        }
    }

    fn owner_limits(objects: u64, bytes: u64, concurrent: u32) -> OwnerAggregateLimits {
        OwnerAggregateLimits {
            max_objects_per_window: objects,
            max_bytes_per_window: bytes,
            window_ms: 60_000,
            max_concurrent_fetches: concurrent,
        }
    }

    fn generous_owner() -> OwnerAggregateLimits {
        owner_limits(u64::MAX, u64::MAX, 16)
    }

    fn ctx<'a>(limits: &'a ShareServingLimits) -> SnapshotReadContext<'a> {
        SnapshotReadContext {
            owner: "owner",
            share: "share",
            reader: "reader",
            owner_tuned_limits: Some(limits),
            now_ms: 0,
        }
    }

    fn snapshot(sizes: &[usize]) -> Vec<Vec<u8>> {
        sizes.iter().map(|&n| vec![7u8; n]).collect()
    }

    /// Follow resume positions from 0 to completion, collecting
    /// every served object. Oracle: the collected objects are the
    /// snapshot, exactly and in order.
    fn serve_all(
        serving: &mut SnapshotServing,
        context: &SnapshotReadContext<'_>,
        snap: &[Vec<u8>],
    ) -> Vec<Vec<u8>> {
        let mut collected = Vec::new();
        let mut from = 0u64;
        loop {
            let page = serving.serve_page(context, snap, from).unwrap();
            assert_eq!(page.fetch.start, from);
            assert_eq!(page.objects.len() as u64, page.fetch.objects);
            collected.extend(page.objects.iter().cloned());
            match page.fetch.resume_from {
                Some(next) => {
                    assert!(page.fetch.capped_by.is_some());
                    assert_eq!(next, from + page.fetch.objects);
                    from = next;
                }
                None => {
                    assert!(page.fetch.capped_by.is_none());
                    break;
                }
            }
        }
        collected
    }

    #[test]
    fn paging_by_resume_partitions_the_snapshot_exactly() {
        let limits = share_limits(2, 1_000, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let snap = snapshot(&[10, 20, 30, 40, 50]);
        let collected = serve_all(&mut serving, &ctx(&limits), &snap);
        assert_eq!(collected, snap);
    }

    #[test]
    fn byte_cap_names_the_cap_and_the_resume_position() {
        let limits = share_limits(100, 25, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let snap = snapshot(&[10, 10, 10]);
        let context = ctx(&limits);
        let p1 = serving.serve_page(&context, &snap, 0).unwrap();
        assert_eq!(p1.objects.len(), 2);
        assert_eq!(p1.fetch.bytes, 20);
        assert_eq!(p1.fetch.capped_by, Some(FetchCap::Bytes));
        assert_eq!(p1.fetch.resume_from, Some(2));
        let p2 = serving
            .serve_page(&context, &snap, p1.fetch.resume_from.unwrap())
            .unwrap();
        assert_eq!(p2.objects.len(), 1);
        assert_eq!(p2.fetch.capped_by, None);
        assert_eq!(p2.fetch.resume_from, None);
    }

    #[test]
    fn single_oversize_object_is_served_alone_not_dropped() {
        let limits = share_limits(100, 10, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let snap = snapshot(&[100, 5]);
        let context = ctx(&limits);
        let p1 = serving.serve_page(&context, &snap, 0).unwrap();
        assert_eq!(p1.objects.len(), 1);
        assert_eq!(p1.fetch.bytes, 100);
        assert_eq!(p1.fetch.capped_by, Some(FetchCap::Bytes));
        assert_eq!(p1.fetch.resume_from, Some(1));
        let collected = serve_all(&mut serving, &context, &snap);
        assert_eq!(collected, snap);
    }

    #[test]
    fn landing_exactly_on_a_cap_with_nothing_left_is_complete() {
        let limits = share_limits(2, 20, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let snap = snapshot(&[10, 10]);
        let page = serving.serve_page(&ctx(&limits), &snap, 0).unwrap();
        assert_eq!(page.objects.len(), 2);
        assert_eq!(page.fetch.capped_by, None);
        assert_eq!(page.fetch.resume_from, None);
    }

    #[test]
    fn empty_and_exhausted_snapshots_are_complete_empty_pages() {
        let limits = share_limits(10, 10, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let context = ctx(&limits);
        let empty: Vec<Vec<u8>> = Vec::new();
        let page = serving.serve_page(&context, &empty, 0).unwrap();
        assert!(page.objects.is_empty());
        assert_eq!(page.fetch.capped_by, None);
        let snap = snapshot(&[1, 2]);
        let page = serving.serve_page(&context, &snap, 5).unwrap();
        assert!(page.objects.is_empty());
        assert_eq!(page.fetch.capped_by, None);
        assert_eq!(page.fetch.resume_from, None);
    }

    #[test]
    fn rate_limit_refusal_is_named_and_serves_nothing() {
        let limits = share_limits(10, 1_000, 1);
        let mut serving = SnapshotServing::new(generous_owner(), ShareServingLimits::v1_defaults());
        let snap = snapshot(&[10]);
        let context = ctx(&limits);
        assert!(serving.serve_page(&context, &snap, 0).is_ok());
        let refusal = serving.serve_page(&context, &snap, 0).unwrap_err();
        assert_eq!(refusal.name(), "rate_limited");
    }

    #[test]
    fn owner_aggregate_refusal_is_named_and_charges_nothing() {
        // Aggregate ceiling: 3 objects per window. First page
        // charges 2; a second 2-object page would cross it and is
        // refused whole; a 1-object page still fits afterwards,
        // proving the refused charge applied nothing.
        let limits = share_limits(2, 1_000, 60);
        let mut serving = SnapshotServing::new(
            owner_limits(3, 1_000_000, 16),
            ShareServingLimits::v1_defaults(),
        );
        let snap = snapshot(&[10, 10, 10, 10]);
        let context = ctx(&limits);
        let p1 = serving.serve_page(&context, &snap, 0).unwrap();
        assert_eq!(p1.fetch.resume_from, Some(2));
        let refusal = serving.serve_page(&context, &snap, 2).unwrap_err();
        assert_eq!(refusal, ServingRefusal::OwnerAggregateExceeded);
        assert_eq!(refusal.name(), "owner_aggregate_exceeded");
        let last = serving.serve_page(&context, &snap, 3).unwrap();
        assert_eq!(last.objects.len(), 1);
    }

    #[test]
    fn owner_concurrency_ceiling_refuses_a_page_fetch() {
        let limits = share_limits(10, 1_000, 60);
        let mut serving = SnapshotServing::new(
            owner_limits(1_000, 1_000_000, 1),
            ShareServingLimits::v1_defaults(),
        );
        let snap = snapshot(&[10]);
        let context = ctx(&limits);
        serving.begin_owner_fetch("owner").unwrap();
        let refusal = serving.serve_page(&context, &snap, 0).unwrap_err();
        assert_eq!(refusal, ServingRefusal::OwnerConcurrencyExceeded);
        serving.end_owner_fetch("owner");
        assert!(serving.serve_page(&context, &snap, 0).is_ok());
    }

    #[test]
    fn owner_tuning_above_the_platform_ceiling_is_clamped() {
        // Owner tunes to 100 objects per fetch; the platform
        // ceiling held by the serving layer is 2, so pages cap
        // at 2 objects.
        let greedy = share_limits(100, 1_000_000, 60);
        let ceilings = share_limits(2, 1_000_000, 60);
        let mut serving = SnapshotServing::new(generous_owner(), ceilings);
        let snap = snapshot(&[10, 10, 10]);
        let page = serving.serve_page(&ctx(&greedy), &snap, 0).unwrap();
        assert_eq!(page.objects.len(), 2);
        assert_eq!(page.fetch.capped_by, Some(FetchCap::Objects));
        assert_eq!(page.fetch.resume_from, Some(2));
    }

    proptest! {
        #[test]
        fn served_pages_partition_arbitrary_snapshots(
            sizes in prop::collection::vec(0usize..500, 0..64),
            max_objects in 1u64..20,
            max_bytes in 1u64..2_000,
        ) {
            // Oracle: paging by resume positions serves every
            // object exactly once, in order; every non-final page
            // names a cap; page counts respect the caps except a
            // lone oversize object served alone.
            let limits = share_limits(max_objects, max_bytes, u32::MAX);
            let mut serving = SnapshotServing::new(
                owner_limits(u64::MAX, u64::MAX, 16),
                ShareServingLimits {
                    max_objects_per_fetch: u64::MAX,
                    max_bytes_per_fetch: u64::MAX,
                    max_requests_per_minute: u32::MAX,
                },
            );
            let snap = snapshot(&sizes);
            let context = SnapshotReadContext {
                owner: "owner",
                share: "share",
                reader: "reader",
                owner_tuned_limits: Some(&limits),
                now_ms: 0,
            };
            let collected = serve_all(&mut serving, &context, &snap);
            prop_assert_eq!(collected, snap);
        }
    }
}
