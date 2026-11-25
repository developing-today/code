//! Public-share serving limits: the mandatory serving caps of the
//! #5802 decision record (D3 item 4), the enforcement core that
//! applies them, and the stable, named outcomes a capped or refused
//! fetch returns (#5802 invariant 8: caps fail loudly — no public
//! fetch silently truncates, silently serves stale-as-current, or
//! silently widens scope to satisfy a request).
//!
//! Three layers, per the record:
//!
//! - **per share** — a maximum snapshot size (objects and bytes) the
//!   share will serve per reader fetch, and a request-rate cap per
//!   share ([`ShareServingLimits`], [`plan_fetch`], [`RateLimiter`]);
//! - **per owner aggregate** — a ceiling across all of an owner's
//!   public shares, so one viral share cannot starve the owner's
//!   other serving ([`OwnerAggregateLimits`],
//!   [`OwnerAggregateTracker`]);
//! - **receiving side** — #5800 §5 caps are the reader's own layer,
//!   enforced by the reader at snapshot-chunk and update boundaries.
//!   They are unchanged by this module and not implemented here.
//!
//! A capped snapshot fetch says it was capped: [`plan_fetch`] returns
//! a [`FetchPage`] whose `capped_by` names the cap that stopped it and
//! whose `resume_from` is the position the reader resumes from under
//! #5800's chunk-resume rule (the next object index within the
//! (grant, epoch) cut). Pages partition the snapshot exactly — every
//! object is served by exactly one page, in order — so a cap can
//! never drop an object silently. Caps are enforced at object
//! boundaries: a single object larger than the byte cap is served
//! alone in its own fetch (objects are atomic; splitting one would be
//! the silent truncation the invariant forbids), and that page still
//! reports itself capped when more objects remain.
//!
//! This module is the core only, on the `id-core::abuse` pattern:
//! nothing is wired into a serving path here. The wiring belongs to
//! the sibling #5802 slices that build the path — the public
//! principal and grants (#14780), the publish/unpublish actions and
//! publication record (#14781), and the adapter public-read path
//! (#14784) call [`ServingGate::serve_fetch`] per reader fetch and
//! [`OwnerAggregateTracker::begin_fetch`] /
//! [`OwnerAggregateTracker::end_fetch`] around byte emission.
//!
//! Time is injected as `now_ms` (milliseconds on any monotonic clock
//! the caller chooses) so every decision is deterministic and
//! unit-testable without sleeping. Retained state is bounded by
//! pruning: callers run [`RateLimiter::prune`] and
//! [`OwnerAggregateTracker::prune`] periodically; an expired window is
//! indistinguishable from a never-seen key, so eviction changes no
//! admission decision.
//!
//! Keys are opaque strings — share id, owner id, and reader
//! identity-or-address — so this core does not depend on the grants
//! model's types, which land with #14780.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// V1 default snapshot object cap per reader fetch (#5802 D3 item 4:
/// 10,000 objects). Configuration, not an invariant.
pub const DEFAULT_MAX_OBJECTS_PER_FETCH: u64 = 10_000;

/// V1 default snapshot byte cap per reader fetch (#5802 D3 item 4:
/// 100 MB). Configuration, not an invariant.
pub const DEFAULT_MAX_BYTES_PER_FETCH: u64 = 100 * 1024 * 1024;

/// V1 default per-share request-rate cap (#5802 D3 item 4: 60
/// requests/minute per reader identity-or-address). Configuration,
/// not an invariant.
pub const DEFAULT_MAX_REQUESTS_PER_MINUTE: u32 = 60;

/// Length of the fixed rate-limit window: one minute, matching the
/// record's requests-per-minute framing.
pub const RATE_WINDOW_MS: u64 = 60_000;

/// Per-share serving limits: the caps one public share enforces on
/// each reader. Values are configuration — the owner tunes them
/// downward from the defaults and the platform sets the ceilings
/// ([`ShareServingLimits::effective`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareServingLimits {
    /// Maximum objects served in one reader fetch.
    pub max_objects_per_fetch: u64,
    /// Maximum bytes served in one reader fetch.
    pub max_bytes_per_fetch: u64,
    /// Maximum fetch requests per minute from one reader
    /// (identity-or-address) against this share.
    pub max_requests_per_minute: u32,
}

impl ShareServingLimits {
    /// The v1 starting defaults from the decision record.
    #[must_use]
    pub const fn v1_defaults() -> Self {
        Self {
            max_objects_per_fetch: DEFAULT_MAX_OBJECTS_PER_FETCH,
            max_bytes_per_fetch: DEFAULT_MAX_BYTES_PER_FETCH,
            max_requests_per_minute: DEFAULT_MAX_REQUESTS_PER_MINUTE,
        }
    }

    /// The limits in force for a share: the owner's tuned values (or
    /// the v1 defaults when the owner has not tuned), clamped
    /// field-by-field to the platform ceilings. An owner value above
    /// a ceiling is clamped to the ceiling — the platform's ceilings
    /// are the hard bound on every value, so no owner tuning can
    /// widen a share past them.
    #[must_use]
    pub fn effective(owner_tuned: Option<&Self>, platform_ceilings: &Self) -> Self {
        let base = owner_tuned.copied().unwrap_or_else(Self::v1_defaults);
        Self {
            max_objects_per_fetch: base
                .max_objects_per_fetch
                .min(platform_ceilings.max_objects_per_fetch),
            max_bytes_per_fetch: base
                .max_bytes_per_fetch
                .min(platform_ceilings.max_bytes_per_fetch),
            max_requests_per_minute: base
                .max_requests_per_minute
                .min(platform_ceilings.max_requests_per_minute),
        }
    }
}

/// Per-owner aggregate limits: the ceiling across **all** of an
/// owner's public shares. The decision record leaves the aggregate
/// numbers to the serving deployment ("owner aggregate defaults set
/// by the serving deployment and reported to the owner"), so this
/// type has no compiled-in default — a deployment constructs it
/// explicitly from its own configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerAggregateLimits {
    /// Maximum objects served across all of the owner's shares per
    /// window.
    pub max_objects_per_window: u64,
    /// Maximum bytes served across all of the owner's shares per
    /// window.
    pub max_bytes_per_window: u64,
    /// Window length in milliseconds for the object/byte budgets.
    pub window_ms: u64,
    /// Maximum fetches in flight at once across all of the owner's
    /// shares.
    pub max_concurrent_fetches: u32,
}

/// Which per-share snapshot cap stopped a fetch. The serde names are
/// the stable wire vocabulary of the serving-limit outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchCap {
    /// The fetch stopped at the per-fetch object cap.
    Objects,
    /// The fetch stopped at the per-fetch byte cap.
    Bytes,
}

impl FetchCap {
    /// The stable name of this cap, as it appears on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Objects => "objects",
            Self::Bytes => "bytes",
        }
    }
}

/// One planned reader fetch: a contiguous page of a snapshot cut.
///
/// `capped_by` and `resume_from` are set together or not at all:
/// when the snapshot has more objects beyond this page, `capped_by`
/// names the cap that stopped the fetch and `resume_from` is the
/// object index the reader resumes from under #5800's chunk-resume
/// rule; when this page completes the snapshot both are `None` —
/// including when the page lands exactly on a cap with nothing left
/// over, which is a completion, not a cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchPage {
    /// Index of the page's first object within the snapshot cut.
    pub start: u64,
    /// Objects served by this page.
    pub objects: u64,
    /// Bytes served by this page.
    pub bytes: u64,
    /// The cap that stopped this fetch, if it stopped early.
    pub capped_by: Option<FetchCap>,
    /// Resume position (next object index) when `capped_by` is set.
    pub resume_from: Option<u64>,
}

/// Plan one reader fetch of a snapshot cut.
///
/// `object_sizes` is the cut's object byte sizes in snapshot order
/// (the serving path holds the manifest; this core needs only the
/// sizes). `from` is the object index to start at — 0 for a fresh
/// fetch, or a previous page's `resume_from` for a resumed one.
///
/// The page serves objects in order from `from`, stopping at the
/// first object that would cross the object or byte cap. A fetch
/// always serves at least one object when any remain: caps are
/// enforced at object boundaries, so a single object larger than the
/// byte cap is served alone rather than split or dropped. When both
/// caps would trip at the same boundary the object cap is reported —
/// the choice is arbitrary but deterministic, and either way the
/// page names a cap and a resume position instead of truncating
/// silently.
#[must_use]
pub fn plan_fetch(object_sizes: &[u64], from: u64, limits: &ShareServingLimits) -> FetchPage {
    let total = u64::try_from(object_sizes.len()).unwrap_or(u64::MAX);
    let mut objects = 0u64;
    let mut bytes = 0u64;
    let mut idx = from;
    while idx < total {
        // idx < total == object_sizes.len(), so the cast cannot lose
        // the index on any platform where the slice exists.
        let Ok(i) = usize::try_from(idx) else {
            break;
        };
        let size = object_sizes[i];
        let next_objects = objects + 1;
        let next_bytes = bytes.saturating_add(size);
        if objects > 0
            && (next_objects > limits.max_objects_per_fetch
                || next_bytes > limits.max_bytes_per_fetch)
        {
            let capped_by = if next_objects > limits.max_objects_per_fetch {
                FetchCap::Objects
            } else {
                FetchCap::Bytes
            };
            return FetchPage {
                start: from,
                objects,
                bytes,
                capped_by: Some(capped_by),
                resume_from: Some(idx),
            };
        }
        objects = next_objects;
        bytes = next_bytes;
        idx += 1;
    }
    FetchPage {
        start: from,
        objects,
        bytes,
        capped_by: None,
        resume_from: None,
    }
}

/// A serving-limit refusal: the stable, named outcome a refused
/// fetch returns. The serde names are the stable wire vocabulary —
/// a reader (and the owner's accounting surface) can match on them
/// across versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServingRefusal {
    /// The reader is over the share's request-rate cap; retry after
    /// the named delay (milliseconds until the window opens).
    RateLimited {
        /// Milliseconds until the rate window opens.
        retry_after_ms: u64,
    },
    /// The owner's aggregate object/byte budget for the window is
    /// spent; serving any of the owner's shares further this window
    /// would cross the ceiling.
    OwnerAggregateExceeded,
    /// Too many of the owner's fetches are already in flight.
    OwnerConcurrencyExceeded,
}

impl ServingRefusal {
    /// The stable name of this refusal, as it appears on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RateLimited { .. } => "rate_limited",
            Self::OwnerAggregateExceeded => "owner_aggregate_exceeded",
            Self::OwnerConcurrencyExceeded => "owner_concurrency_exceeded",
        }
    }
}

/// One reader's fixed rate window against one share.
#[derive(Debug)]
struct RateWindow {
    start_ms: u64,
    count: u32,
}

/// Per-(share, reader) request-rate limiter: fixed one-minute
/// windows, keyed by reader identity-or-address exactly as the
/// record frames the cap ("per reader identity-or-address").
#[derive(Debug, Default)]
pub struct RateLimiter {
    windows: HashMap<(String, String), RateWindow>,
}

impl RateLimiter {
    /// A limiter with no windows recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit one fetch request, or refuse it with
    /// [`ServingRefusal::RateLimited`] and the delay until the
    /// reader's window opens. Admitted requests are counted; refused
    /// ones are not (a refused reader cannot extend its own window
    /// by retrying).
    pub fn check(
        &mut self,
        share: &str,
        reader: &str,
        limits: &ShareServingLimits,
        now_ms: u64,
    ) -> Result<(), ServingRefusal> {
        let window = self
            .windows
            .entry((share.to_owned(), reader.to_owned()))
            .or_insert(RateWindow {
                start_ms: now_ms,
                count: 0,
            });
        if now_ms >= window.start_ms.saturating_add(RATE_WINDOW_MS) {
            window.start_ms = now_ms;
            window.count = 0;
        }
        if window.count >= limits.max_requests_per_minute {
            return Err(ServingRefusal::RateLimited {
                retry_after_ms: window
                    .start_ms
                    .saturating_add(RATE_WINDOW_MS)
                    .saturating_sub(now_ms),
            });
        }
        window.count += 1;
        Ok(())
    }

    /// Drop windows that have fully expired. An expired window is
    /// indistinguishable from a never-seen key, so this changes no
    /// admission decision; it only bounds retained state.
    pub fn prune(&mut self, now_ms: u64) {
        self.windows
            .retain(|_, w| now_ms < w.start_ms.saturating_add(RATE_WINDOW_MS));
    }
}

/// One owner's aggregate budget window across all their shares.
#[derive(Debug)]
struct OwnerWindow {
    start_ms: u64,
    objects: u64,
    bytes: u64,
}

/// Per-owner aggregate tracker: windowed object/byte budgets shared
/// across every public share the owner has, plus the concurrent
/// fetch ceiling. Charging is all-or-nothing — a charge that would
/// cross the ceiling is refused whole and nothing is applied, so
/// the accounting never silently absorbs an over-ceiling fetch.
#[derive(Debug)]
pub struct OwnerAggregateTracker {
    limits: OwnerAggregateLimits,
    windows: HashMap<String, OwnerWindow>,
    in_flight: HashMap<String, u32>,
}

impl OwnerAggregateTracker {
    /// A tracker with the deployment's aggregate limits and no
    /// usage recorded.
    #[must_use]
    pub fn new(limits: OwnerAggregateLimits) -> Self {
        Self {
            limits,
            windows: HashMap::new(),
            in_flight: HashMap::new(),
        }
    }

    /// The limits this tracker enforces.
    #[must_use]
    pub const fn limits(&self) -> &OwnerAggregateLimits {
        &self.limits
    }

    fn window_mut(&mut self, owner: &str, now_ms: u64) -> &mut OwnerWindow {
        let window = self.windows.entry(owner.to_owned()).or_insert(OwnerWindow {
            start_ms: now_ms,
            objects: 0,
            bytes: 0,
        });
        if now_ms >= window.start_ms.saturating_add(self.limits.window_ms) {
            window.start_ms = now_ms;
            window.objects = 0;
            window.bytes = 0;
        }
        window
    }

    /// Charge one served fetch against the owner's aggregate budget.
    /// Refuses with [`ServingRefusal::OwnerAggregateExceeded`] —
    /// applying nothing — when the charge would cross the window's
    /// object or byte ceiling.
    pub fn charge(
        &mut self,
        owner: &str,
        objects: u64,
        bytes: u64,
        now_ms: u64,
    ) -> Result<(), ServingRefusal> {
        let limits = self.limits;
        let window = self.window_mut(owner, now_ms);
        if window.objects.saturating_add(objects) > limits.max_objects_per_window
            || window.bytes.saturating_add(bytes) > limits.max_bytes_per_window
        {
            return Err(ServingRefusal::OwnerAggregateExceeded);
        }
        window.objects += objects;
        window.bytes += bytes;
        Ok(())
    }

    /// Mark one of the owner's fetches as in flight, or refuse with
    /// [`ServingRefusal::OwnerConcurrencyExceeded`] when the owner's
    /// concurrent-fetch ceiling is reached. Pair every admission
    /// with [`OwnerAggregateTracker::end_fetch`].
    pub fn begin_fetch(&mut self, owner: &str) -> Result<(), ServingRefusal> {
        let count = self.in_flight.entry(owner.to_owned()).or_insert(0);
        if *count >= self.limits.max_concurrent_fetches {
            return Err(ServingRefusal::OwnerConcurrencyExceeded);
        }
        *count += 1;
        Ok(())
    }

    /// Mark one of the owner's in-flight fetches as finished.
    /// Ending a fetch that was never begun is a no-op.
    pub fn end_fetch(&mut self, owner: &str) {
        let drained = match self.in_flight.get_mut(owner) {
            Some(count) => {
                *count = count.saturating_sub(1);
                *count == 0
            }
            None => false,
        };
        if drained {
            self.in_flight.remove(owner);
        }
    }

    /// Drop budget windows that have fully expired and hold no
    /// in-flight fetches. Like [`RateLimiter::prune`], this changes
    /// no admission decision; it only bounds retained state.
    pub fn prune(&mut self, now_ms: u64) {
        let window_ms = self.limits.window_ms;
        let in_flight = &self.in_flight;
        self.windows.retain(|owner, w| {
            now_ms < w.start_ms.saturating_add(window_ms) || in_flight.contains_key(owner)
        });
    }
}

/// The serving gate: the composition the serving path calls per
/// reader fetch. [`ServingGate::serve_fetch`] runs the layers in
/// order — rate check, page planning under the per-share snapshot
/// caps, aggregate charge — and returns either the page to serve or
/// the one named refusal that stopped it. A refused fetch serves
/// nothing and charges nothing.
#[derive(Debug)]
pub struct ServingGate {
    rate_limiter: RateLimiter,
    owner_aggregate: OwnerAggregateTracker,
}

impl ServingGate {
    /// A gate enforcing the deployment's per-owner aggregate limits;
    /// per-share limits are supplied per fetch, since each share
    /// carries its own tuned values.
    #[must_use]
    pub fn new(owner_limits: OwnerAggregateLimits) -> Self {
        Self {
            rate_limiter: RateLimiter::new(),
            owner_aggregate: OwnerAggregateTracker::new(owner_limits),
        }
    }

    /// Serve one reader fetch of one share's snapshot cut.
    ///
    /// - `owner` / `share` / `reader` are the opaque ids of the
    ///   share's owner, the share, and the reader
    ///   (identity-or-address);
    /// - `object_sizes` and `from` are as in [`plan_fetch`];
    /// - `limits` is the share's effective limits
    ///   ([`ShareServingLimits::effective`]).
    pub fn serve_fetch(
        &mut self,
        owner: &str,
        share: &str,
        reader: &str,
        object_sizes: &[u64],
        from: u64,
        limits: &ShareServingLimits,
        now_ms: u64,
    ) -> Result<FetchPage, ServingRefusal> {
        self.rate_limiter.check(share, reader, limits, now_ms)?;
        let page = plan_fetch(object_sizes, from, limits);
        self.owner_aggregate
            .charge(owner, page.objects, page.bytes, now_ms)?;
        Ok(page)
    }

    /// Mark one of the owner's fetches as in flight (the concurrency
    /// layer, held around byte emission by the serving path).
    pub fn begin_owner_fetch(&mut self, owner: &str) -> Result<(), ServingRefusal> {
        self.owner_aggregate.begin_fetch(owner)
    }

    /// Mark one of the owner's in-flight fetches as finished.
    pub fn end_owner_fetch(&mut self, owner: &str) {
        self.owner_aggregate.end_fetch(owner);
    }

    /// Prune expired rate and aggregate windows.
    pub fn prune(&mut self, now_ms: u64) {
        self.rate_limiter.prune(now_ms);
        self.owner_aggregate.prune(now_ms);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn limits(objects: u64, bytes: u64, rate: u32) -> ShareServingLimits {
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

    #[test]
    fn v1_defaults_are_the_records_numbers() {
        let d = ShareServingLimits::v1_defaults();
        assert_eq!(d.max_objects_per_fetch, 10_000);
        assert_eq!(d.max_bytes_per_fetch, 100 * 1024 * 1024);
        assert_eq!(d.max_requests_per_minute, 60);
    }

    #[test]
    fn effective_clamps_owner_tuning_to_platform_ceilings() {
        let ceilings = limits(1_000, 1_000_000, 30);
        // Owner tunes downward: the owner's values stand.
        let tuned = limits(10, 500, 5);
        let eff = ShareServingLimits::effective(Some(&tuned), &ceilings);
        assert_eq!(eff, tuned);
        // Owner value above a ceiling: clamped to the ceiling.
        let greedy = limits(999_999, 999_999_999, 9_999);
        let eff = ShareServingLimits::effective(Some(&greedy), &ceilings);
        assert_eq!(eff, ceilings);
        // No tuning: the v1 defaults, still clamped to the ceilings.
        let eff = ShareServingLimits::effective(None, &ceilings);
        assert_eq!(eff, ceilings);
        let roomy = limits(u64::MAX, u64::MAX, u32::MAX);
        let eff = ShareServingLimits::effective(None, &roomy);
        assert_eq!(eff, ShareServingLimits::v1_defaults());
    }

    #[test]
    fn small_snapshot_is_one_complete_page() {
        let sizes = [10, 20, 30];
        let page = plan_fetch(&sizes, 0, &limits(100, 1_000, 60));
        assert_eq!(page.objects, 3);
        assert_eq!(page.bytes, 60);
        assert_eq!(page.capped_by, None);
        assert_eq!(page.resume_from, None);
    }

    #[test]
    fn object_cap_caps_and_resumes() {
        let sizes = [1, 1, 1, 1, 1];
        let lim = limits(2, 1_000, 60);
        let p1 = plan_fetch(&sizes, 0, &lim);
        assert_eq!(p1.objects, 2);
        assert_eq!(p1.capped_by, Some(FetchCap::Objects));
        assert_eq!(p1.resume_from, Some(2));
        let p2 = plan_fetch(&sizes, p1.resume_from.unwrap(), &lim);
        assert_eq!(p2.objects, 2);
        assert_eq!(p2.resume_from, Some(4));
        let p3 = plan_fetch(&sizes, p2.resume_from.unwrap(), &lim);
        assert_eq!(p3.objects, 1);
        assert_eq!(p3.capped_by, None);
    }

    #[test]
    fn byte_cap_caps_and_resumes() {
        let sizes = [100, 100, 100];
        let page = plan_fetch(&sizes, 0, &limits(100, 250, 60));
        assert_eq!(page.objects, 2);
        assert_eq!(page.bytes, 200);
        assert_eq!(page.capped_by, Some(FetchCap::Bytes));
        assert_eq!(page.resume_from, Some(2));
    }

    #[test]
    fn landing_exactly_on_a_cap_with_nothing_left_is_complete() {
        let sizes = [100, 100];
        let page = plan_fetch(&sizes, 0, &limits(2, 200, 60));
        assert_eq!(page.objects, 2);
        assert_eq!(page.capped_by, None);
        assert_eq!(page.resume_from, None);
    }

    #[test]
    fn single_oversize_object_is_served_alone_not_dropped() {
        let sizes = [5_000, 10];
        let lim = limits(100, 1_000, 60);
        let p1 = plan_fetch(&sizes, 0, &lim);
        assert_eq!(p1.objects, 1);
        assert_eq!(p1.bytes, 5_000);
        assert_eq!(p1.capped_by, Some(FetchCap::Bytes));
        assert_eq!(p1.resume_from, Some(1));
        let p2 = plan_fetch(&sizes, 1, &lim);
        assert_eq!(p2.objects, 1);
        assert_eq!(p2.capped_by, None);
    }

    #[test]
    fn empty_and_exhausted_snapshots_are_complete_empty_pages() {
        let lim = limits(10, 10, 60);
        let page = plan_fetch(&[], 0, &lim);
        assert_eq!(page.objects, 0);
        assert_eq!(page.capped_by, None);
        let page = plan_fetch(&[1, 2], 5, &lim);
        assert_eq!(page.objects, 0);
        assert_eq!(page.capped_by, None);
        assert_eq!(page.resume_from, None);
    }

    proptest! {
        #[test]
        fn pages_partition_the_snapshot_and_respect_caps(
            sizes in prop::collection::vec(0u64..500, 0..64),
            max_objects in 1u64..20,
            max_bytes in 1u64..2_000,
        ) {
            let lim = limits(max_objects, max_bytes, 60);
            let total = u64::try_from(sizes.len()).unwrap();
            let mut from = 0u64;
            let mut seen = 0u64;
            loop {
                let page = plan_fetch(&sizes, from, &lim);
                prop_assert_eq!(page.start, from);
                if page.objects == 0 {
                    prop_assert!(page.capped_by.is_none());
                    prop_assert!(from >= total);
                    break;
                }
                prop_assert!(page.objects <= max_objects);
                if page.objects > 1 {
                    prop_assert!(page.bytes <= max_bytes);
                }
                seen += page.objects;
                match page.resume_from {
                    Some(next) => {
                        prop_assert!(page.capped_by.is_some());
                        prop_assert_eq!(next, from + page.objects);
                        from = next;
                    }
                    None => {
                        prop_assert!(page.capped_by.is_none());
                        break;
                    }
                }
            }
            prop_assert_eq!(seen, total);
        }
    }

    #[test]
    fn rate_limiter_admits_the_cap_then_refuses_with_retry_after() {
        let mut rl = RateLimiter::new();
        let lim = limits(10, 10, 3);
        for _ in 0..3 {
            assert!(rl.check("share", "reader", &lim, 1_000).is_ok());
        }
        let refusal = rl.check("share", "reader", &lim, 1_000).unwrap_err();
        // Window opened at t=1_000 and closes at t=61_000.
        assert_eq!(
            refusal,
            ServingRefusal::RateLimited {
                retry_after_ms: 60_000,
            }
        );
        // Still refused just before the window opens.
        assert!(rl.check("share", "reader", &lim, 60_999).is_err());
        // A refused request does not extend the window: at the
        // window's end the reader is admitted again.
        assert!(rl.check("share", "reader", &lim, 61_000).is_ok());
    }

    #[test]
    fn rate_limiter_keys_are_per_share_and_per_reader() {
        let mut rl = RateLimiter::new();
        let lim = limits(10, 10, 1);
        assert!(rl.check("share-a", "reader", &lim, 0).is_ok());
        assert!(rl.check("share-a", "reader", &lim, 0).is_err());
        assert!(rl.check("share-b", "reader", &lim, 0).is_ok());
        assert!(rl.check("share-a", "other-reader", &lim, 0).is_ok());
    }

    #[test]
    fn aggregate_budget_is_shared_across_an_owners_shares() {
        let mut agg = OwnerAggregateTracker::new(owner_limits(10, 1_000, 2));
        // Charges from two different shares draw on the one budget.
        assert!(agg.charge("owner", 6, 100, 0).is_ok());
        assert!(agg.charge("owner", 4, 100, 0).is_ok());
        assert_eq!(
            agg.charge("owner", 1, 1, 0),
            Err(ServingRefusal::OwnerAggregateExceeded)
        );
        // Another owner has its own budget.
        assert!(agg.charge("other", 10, 1_000, 0).is_ok());
        // The window resets.
        assert!(agg.charge("owner", 10, 1_000, 60_000).is_ok());
    }

    #[test]
    fn aggregate_refusal_applies_nothing() {
        let mut agg = OwnerAggregateTracker::new(owner_limits(10, 1_000, 2));
        assert!(agg.charge("owner", 6, 0, 0).is_ok());
        // Would cross the ceiling: refused whole.
        assert!(agg.charge("owner", 6, 0, 0).is_err());
        // The refused 6 was not applied: exactly the remaining 4 fit.
        assert!(agg.charge("owner", 4, 0, 0).is_ok());
        assert!(agg.charge("owner", 1, 0, 0).is_err());
    }

    #[test]
    fn concurrency_ceiling_refuses_and_end_frees_a_slot() {
        let mut agg = OwnerAggregateTracker::new(owner_limits(10, 1_000, 2));
        assert!(agg.begin_fetch("owner").is_ok());
        assert!(agg.begin_fetch("owner").is_ok());
        assert_eq!(
            agg.begin_fetch("owner"),
            Err(ServingRefusal::OwnerConcurrencyExceeded)
        );
        agg.end_fetch("owner");
        assert!(agg.begin_fetch("owner").is_ok());
        // Ending a fetch that was never begun is a no-op.
        agg.end_fetch("nobody");
    }

    #[test]
    fn gate_serves_pages_and_charges_the_owner() {
        let mut gate = ServingGate::new(owner_limits(5, 1_000, 2));
        let lim = limits(2, 1_000, 60);
        let sizes = [10, 10, 10, 10, 10, 10];
        let p1 = gate
            .serve_fetch("owner", "share", "reader", &sizes, 0, &lim, 0)
            .unwrap();
        assert_eq!(p1.objects, 2);
        assert_eq!(p1.capped_by, Some(FetchCap::Objects));
        let p2 = gate
            .serve_fetch("owner", "share", "reader", &sizes, 2, &lim, 0)
            .unwrap();
        assert_eq!(p2.objects, 2);
        // 4 of the owner's 5-object window budget is spent, so the
        // next 2-object page no longer fits the aggregate ceiling.
        let refusal = gate
            .serve_fetch("owner", "share", "reader", &sizes, 4, &lim, 0)
            .unwrap_err();
        assert_eq!(refusal, ServingRefusal::OwnerAggregateExceeded);
    }

    #[test]
    fn gate_rate_refusal_short_circuits() {
        let mut gate = ServingGate::new(owner_limits(1_000, 1_000_000, 2));
        let lim = limits(10, 1_000, 1);
        let sizes = [10];
        assert!(
            gate.serve_fetch("owner", "share", "reader", &sizes, 0, &lim, 0)
                .is_ok()
        );
        let refusal = gate
            .serve_fetch("owner", "share", "reader", &sizes, 0, &lim, 0)
            .unwrap_err();
        assert_eq!(refusal.name(), "rate_limited");
    }

    #[test]
    fn refusal_and_cap_wire_names_are_stable() {
        assert_eq!(
            ServingRefusal::RateLimited { retry_after_ms: 5 }.name(),
            "rate_limited"
        );
        assert_eq!(
            ServingRefusal::OwnerAggregateExceeded.name(),
            "owner_aggregate_exceeded"
        );
        assert_eq!(
            ServingRefusal::OwnerConcurrencyExceeded.name(),
            "owner_concurrency_exceeded"
        );
        assert_eq!(FetchCap::Objects.name(), "objects");
        assert_eq!(FetchCap::Bytes.name(), "bytes");
        assert_eq!(
            serde_json::to_string(&ServingRefusal::OwnerAggregateExceeded).unwrap(),
            "\"owner_aggregate_exceeded\""
        );
        assert_eq!(
            serde_json::to_string(&ServingRefusal::RateLimited { retry_after_ms: 5 }).unwrap(),
            "{\"rate_limited\":{\"retry_after_ms\":5}}"
        );
        assert_eq!(
            serde_json::to_string(&FetchCap::Bytes).unwrap(),
            "\"bytes\""
        );
    }
}
