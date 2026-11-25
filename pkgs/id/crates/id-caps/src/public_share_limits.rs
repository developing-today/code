//! Serving limits for public shares (#14783; decision record #5802,
//! D3.4 and invariant 8).
//!
//! A public share is the widest grant ID can express, and its read load
//! lands on the owner's serving path. So every public share carries
//! mandatory serving limits at two layers on the serving side:
//!
//! - **Per share:** a cap on what one reader fetch may return (objects
//!   and bytes, whichever is hit first), and a request-rate cap per
//!   reader identity or address.
//! - **Per owner:** an aggregate ceiling across all of the owner's public
//!   shares, so one busy share cannot starve the owner's other serving.
//!
//! The third layer, the receiving side's own caps (#5800 §5), is the
//! reader's and is untouched here.
//!
//! The platform sets the ceilings ([`PlatformCeilings`]); an owner can
//! only tune a share's limits downward ([`ShareLimits::tuned`]). The v1
//! defaults are policy values from the decision record, not invariants.
//!
//! **Caps fail loudly (invariant 8).** Every refusal is a
//! [`ServingLimit`] with a stable name. A snapshot fetch that hits a cap
//! is never silently truncated: [`plan_snapshot_fetch`] returns a
//! [`FetchPlan`] whose `capped` field names the cap and the resume point,
//! so the reader can continue under #5800's chunk-resume rule. A capped
//! fetch never reports how much was left behind, because a public surface
//! shows no totals and no "N of M" (invariant 3).
//!
//! Nothing here performs I/O or reads a clock: callers pass the time of
//! every request, so the accounting is fully determined by its inputs, in
//! the shape of [`crate::account_quota`].

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// V1 default byte cap on one reader fetch: 100 MB (decimal), from the
/// #5802 decision record.
pub const DEFAULT_FETCH_MAX_BYTES: u64 = 100 * 1000 * 1000;

/// V1 default object cap on one reader fetch: 10,000 objects.
pub const DEFAULT_FETCH_MAX_OBJECTS: u64 = 10_000;

/// V1 default request rate per reader per share: 60 requests per minute.
pub const DEFAULT_REQUESTS_PER_MINUTE: u32 = 60;

/// The window the request-rate and aggregate caps are counted over.
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// The limits one public share serves under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareLimits {
    /// The most bytes one reader fetch may return.
    pub fetch_max_bytes: u64,
    /// The most objects one reader fetch may return.
    pub fetch_max_objects: u64,
    /// The most requests one reader may make to the share per minute.
    pub requests_per_minute: u32,
}

impl Default for ShareLimits {
    fn default() -> Self {
        Self {
            fetch_max_bytes: DEFAULT_FETCH_MAX_BYTES,
            fetch_max_objects: DEFAULT_FETCH_MAX_OBJECTS,
            requests_per_minute: DEFAULT_REQUESTS_PER_MINUTE,
        }
    }
}

/// The aggregate ceiling across all of one owner's public shares.
///
/// The serving deployment sets these values and reports them to the
/// owner (D3.4); there is no protocol-wide default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerAggregateLimits {
    /// The most requests all of the owner's public shares may serve per
    /// minute, across every reader.
    pub requests_per_minute: u32,
    /// The most bytes all of the owner's public shares may serve per
    /// minute, across every reader.
    pub bytes_per_minute: u64,
}

/// The platform's ceilings: the most any share, and any owner, may be
/// configured to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCeilings {
    /// The ceiling on every share's limits, and the default a new share
    /// starts at.
    pub share: ShareLimits,
    /// The aggregate ceiling for each owner.
    pub owner: OwnerAggregateLimits,
}

/// Why a limits configuration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitsError {
    /// A limit is zero, which would disable serving rather than limit it.
    Zero(&'static str),
    /// The owner asked for more than the platform ceiling. Owners may
    /// only tune downward.
    AboveCeiling(&'static str),
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero(field) => write!(f, "{field} must be nonzero"),
            Self::AboveCeiling(field) => {
                write!(
                    f,
                    "{field} is above the platform ceiling; owners may only lower it"
                )
            }
        }
    }
}

impl std::error::Error for LimitsError {}

impl ShareLimits {
    /// Check that every limit is nonzero.
    ///
    /// # Errors
    ///
    /// [`LimitsError::Zero`] naming the first zero limit.
    pub const fn validate(&self) -> Result<(), LimitsError> {
        if self.fetch_max_bytes == 0 {
            return Err(LimitsError::Zero("fetch_max_bytes"));
        }
        if self.fetch_max_objects == 0 {
            return Err(LimitsError::Zero("fetch_max_objects"));
        }
        if self.requests_per_minute == 0 {
            return Err(LimitsError::Zero("requests_per_minute"));
        }
        Ok(())
    }

    /// The owner's tuning of a share, checked against the platform
    /// ceiling: every limit must be nonzero and at or below the ceiling.
    ///
    /// # Errors
    ///
    /// [`LimitsError`] naming the first offending limit. Nothing is
    /// clamped: an owner who asks for more than the ceiling is told so.
    pub const fn tuned(requested: Self, ceiling: Self) -> Result<Self, LimitsError> {
        if let Err(err) = requested.validate() {
            return Err(err);
        }
        if requested.fetch_max_bytes > ceiling.fetch_max_bytes {
            return Err(LimitsError::AboveCeiling("fetch_max_bytes"));
        }
        if requested.fetch_max_objects > ceiling.fetch_max_objects {
            return Err(LimitsError::AboveCeiling("fetch_max_objects"));
        }
        if requested.requests_per_minute > ceiling.requests_per_minute {
            return Err(LimitsError::AboveCeiling("requests_per_minute"));
        }
        Ok(requested)
    }
}

impl OwnerAggregateLimits {
    /// Check that both limits are nonzero.
    ///
    /// # Errors
    ///
    /// [`LimitsError::Zero`] naming the zero limit.
    pub const fn validate(&self) -> Result<(), LimitsError> {
        if self.requests_per_minute == 0 {
            return Err(LimitsError::Zero("owner requests_per_minute"));
        }
        if self.bytes_per_minute == 0 {
            return Err(LimitsError::Zero("owner bytes_per_minute"));
        }
        Ok(())
    }
}

impl PlatformCeilings {
    /// Check every ceiling.
    ///
    /// # Errors
    ///
    /// [`LimitsError::Zero`] naming the first zero ceiling.
    pub const fn validate(&self) -> Result<(), LimitsError> {
        if let Err(err) = self.share.validate() {
            return Err(err);
        }
        self.owner.validate()
    }
}

/// Which cap a snapshot fetch hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchCap {
    /// The per-fetch object cap.
    Objects,
    /// The per-fetch byte cap.
    Bytes,
}

impl FetchCap {
    /// The cap's stable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Objects => "fetch_objects",
            Self::Bytes => "fetch_bytes",
        }
    }
}

/// A stable, named serving-limit outcome (invariant 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServingLimit<K> {
    /// A snapshot fetch stopped at a per-fetch cap. The objects before
    /// `resume_after` were served; the reader resumes after that key
    /// under #5800's chunk-resume rule. `resume_after` is `None` when
    /// the cap was hit before any object was served, so the reader
    /// resumes from where its request started.
    SnapshotCapped {
        /// The cap that was hit.
        cap: FetchCap,
        /// The last object key served, if any.
        resume_after: Option<K>,
    },
    /// The reader hit the share's request-rate cap.
    ShareRateLimited {
        /// When the reader may try again.
        retry_after: Duration,
    },
    /// The owner's aggregate ceiling across all public shares was hit.
    OwnerAggregateLimited {
        /// When any reader may try again.
        retry_after: Duration,
    },
}

impl<K> ServingLimit<K> {
    /// The outcome's stable name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SnapshotCapped { .. } => "snapshot_capped",
            Self::ShareRateLimited { .. } => "share_rate_limited",
            Self::OwnerAggregateLimited { .. } => "owner_aggregate_limited",
        }
    }
}

impl<K> fmt::Display for ServingLimit<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SnapshotCapped { cap, .. } => {
                write!(f, "serving limit: snapshot capped at {}", cap.name())
            }
            Self::ShareRateLimited { retry_after } => write!(
                f,
                "serving limit: share request rate; retry after {}s",
                retry_after.as_secs()
            ),
            Self::OwnerAggregateLimited { retry_after } => write!(
                f,
                "serving limit: owner aggregate; retry after {}s",
                retry_after.as_secs()
            ),
        }
    }
}

impl<K: fmt::Debug> std::error::Error for ServingLimit<K> {}

/// What one snapshot fetch will serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan<K> {
    /// The keys of the objects to serve, in order.
    pub served: Vec<K>,
    /// The bytes those objects add up to.
    pub bytes: u64,
    /// `None` when the fetch reached the end of the share. Otherwise the
    /// named cap and resume point: the fetch is incomplete and says so.
    pub capped: Option<ServingLimit<K>>,
}

impl<K> FetchPlan<K> {
    /// Whether the fetch served everything it was asked for.
    pub const fn is_complete(&self) -> bool {
        self.capped.is_none()
    }
}

/// Plan one reader fetch over the share's objects, in the order they will
/// be served, as `(key, size_in_bytes)` pairs.
///
/// Objects are served whole and in order until the next one would pass
/// the object or byte cap. The plan then stops and records a
/// [`ServingLimit::SnapshotCapped`] naming the cap and the last key
/// served. The objects past the cut are not counted or reported, so a
/// capped plan says nothing about how much was left (invariant 3).
pub fn plan_snapshot_fetch<K: Clone, I>(limits: &ShareLimits, objects: I) -> FetchPlan<K>
where
    I: IntoIterator<Item = (K, u64)>,
{
    let mut served: Vec<K> = Vec::new();
    let mut bytes = 0_u64;
    for (key, size) in objects {
        let count = u64::try_from(served.len()).unwrap_or(u64::MAX);
        let cap = if count >= limits.fetch_max_objects {
            Some(FetchCap::Objects)
        } else if bytes
            .checked_add(size)
            .is_none_or(|total| total > limits.fetch_max_bytes)
        {
            Some(FetchCap::Bytes)
        } else {
            None
        };
        if let Some(cap) = cap {
            let resume_after = served.last().cloned();
            return FetchPlan {
                served,
                bytes,
                capped: Some(ServingLimit::SnapshotCapped { cap, resume_after }),
            };
        }
        bytes += size;
        served.push(key);
    }
    FetchPlan {
        served,
        bytes,
        capped: None,
    }
}

/// A fixed one-minute window: when it opened and what it has counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    started: Instant,
    requests: u32,
    bytes: u64,
}

impl Window {
    const fn new(now: Instant) -> Self {
        Self {
            started: now,
            requests: 0,
            bytes: 0,
        }
    }

    /// Roll the window if a full [`RATE_WINDOW`] has passed. A timestamp
    /// before the window start (a clock stepping back) stays in the
    /// current window rather than opening a free one.
    fn roll(&mut self, now: Instant) {
        if now
            .checked_duration_since(self.started)
            .is_some_and(|elapsed| elapsed >= RATE_WINDOW)
        {
            *self = Self::new(now);
        }
    }

    /// How long until the window rolls.
    fn retry_after(&self, now: Instant) -> Duration {
        let elapsed = now.checked_duration_since(self.started).unwrap_or_default();
        RATE_WINDOW.saturating_sub(elapsed)
    }
}

/// Per-reader request-rate accounting for one public share.
///
/// `R` is the reader's identity or address. Each reader gets its own
/// fixed one-minute window; a refused request counts nothing.
#[derive(Debug, Clone)]
pub struct ShareRateLimiter<R> {
    requests_per_minute: u32,
    readers: HashMap<R, Window>,
}

impl<R: Eq + Hash + Clone> ShareRateLimiter<R> {
    /// A limiter enforcing `limits.requests_per_minute` per reader.
    pub fn new(limits: &ShareLimits) -> Self {
        Self {
            requests_per_minute: limits.requests_per_minute,
            readers: HashMap::new(),
        }
    }

    /// Whether `reader` may make a request at `now`, without counting it.
    ///
    /// # Errors
    ///
    /// [`ServingLimit::ShareRateLimited`] when the reader's window is full.
    pub fn check<K>(&self, reader: &R, now: Instant) -> Result<(), ServingLimit<K>> {
        let Some(window) = self.readers.get(reader) else {
            return Ok(());
        };
        let mut window = *window;
        window.roll(now);
        if window.requests >= self.requests_per_minute {
            return Err(ServingLimit::ShareRateLimited {
                retry_after: window.retry_after(now),
            });
        }
        Ok(())
    }

    /// Count one request by `reader` at `now`, or refuse it whole.
    ///
    /// # Errors
    ///
    /// [`ServingLimit::ShareRateLimited`] when the reader's window is full.
    pub fn admit<K>(&mut self, reader: &R, now: Instant) -> Result<(), ServingLimit<K>> {
        self.check(reader, now)?;
        let window = self
            .readers
            .entry(reader.clone())
            .or_insert_with(|| Window::new(now));
        window.roll(now);
        window.requests = window.requests.saturating_add(1);
        Ok(())
    }

    /// Forget readers whose window has expired, to bound memory.
    pub fn prune(&mut self, now: Instant) {
        self.readers.retain(|_, window| {
            now.checked_duration_since(window.started)
                .is_none_or(|elapsed| elapsed < RATE_WINDOW)
        });
    }
}

/// The owner's aggregate budget across all of their public shares.
#[derive(Debug, Clone)]
pub struct OwnerAggregateBudget {
    limits: OwnerAggregateLimits,
    window: Option<Window>,
}

impl OwnerAggregateBudget {
    /// A fresh budget with nothing counted.
    pub const fn new(limits: OwnerAggregateLimits) -> Self {
        Self {
            limits,
            window: None,
        }
    }

    /// The limits this budget enforces, as reported to the owner.
    pub const fn limits(&self) -> OwnerAggregateLimits {
        self.limits
    }

    fn current(&self, now: Instant) -> Window {
        let mut window = self.window.unwrap_or_else(|| Window::new(now));
        window.roll(now);
        window
    }

    /// Whether one more request serving `bytes` fits, without counting it.
    ///
    /// # Errors
    ///
    /// [`ServingLimit::OwnerAggregateLimited`] when it does not fit.
    pub fn check<K>(&self, now: Instant, bytes: u64) -> Result<(), ServingLimit<K>> {
        let window = self.current(now);
        let over_requests = window.requests >= self.limits.requests_per_minute;
        let over_bytes = window
            .bytes
            .checked_add(bytes)
            .is_none_or(|total| total > self.limits.bytes_per_minute);
        if over_requests || over_bytes {
            return Err(ServingLimit::OwnerAggregateLimited {
                retry_after: window.retry_after(now),
            });
        }
        Ok(())
    }

    /// Count one request serving `bytes` at `now`, or refuse it whole.
    ///
    /// # Errors
    ///
    /// [`ServingLimit::OwnerAggregateLimited`] when it does not fit.
    pub fn admit<K>(&mut self, now: Instant, bytes: u64) -> Result<(), ServingLimit<K>> {
        self.check(now, bytes)?;
        let mut window = self.current(now);
        window.requests = window.requests.saturating_add(1);
        window.bytes = window.bytes.saturating_add(bytes);
        self.window = Some(window);
        Ok(())
    }
}

/// Admit one snapshot fetch by `reader` against the share's rate cap and
/// the owner's aggregate budget, and plan what it serves.
///
/// The share's rate cap is checked first, then the fetch is planned under
/// the share's per-fetch caps, then the owner's budget is checked for the
/// planned bytes. Nothing is counted unless every check passes, so a
/// refused fetch leaves both ledgers as they were. A fetch that hits a
/// per-fetch cap is still served, up to the cap, and its plan says it was
/// capped.
///
/// # Errors
///
/// [`ServingLimit::ShareRateLimited`] or
/// [`ServingLimit::OwnerAggregateLimited`]; nothing is served.
pub fn admit_fetch<K: Clone, R: Eq + Hash + Clone, I>(
    limits: &ShareLimits,
    share: &mut ShareRateLimiter<R>,
    owner: &mut OwnerAggregateBudget,
    reader: &R,
    now: Instant,
    objects: I,
) -> Result<FetchPlan<K>, ServingLimit<K>>
where
    I: IntoIterator<Item = (K, u64)>,
{
    share.check(reader, now)?;
    let plan = plan_snapshot_fetch(limits, objects);
    owner.check(now, plan.bytes)?;
    share.admit(reader, now)?;
    owner.admit(now, plan.bytes)?;
    Ok(plan)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn small() -> ShareLimits {
        ShareLimits {
            fetch_max_bytes: 100,
            fetch_max_objects: 3,
            requests_per_minute: 2,
        }
    }

    fn objects(sizes: &[u64]) -> Vec<(u32, u64)> {
        (1_u32..).zip(sizes.iter().copied()).collect()
    }

    #[test]
    fn v1_defaults_match_the_decision_record() {
        let limits = ShareLimits::default();
        assert_eq!(limits.fetch_max_bytes, 100_000_000);
        assert_eq!(limits.fetch_max_objects, 10_000);
        assert_eq!(limits.requests_per_minute, 60);
        assert_eq!(limits.validate(), Ok(()));
    }

    #[test]
    fn owners_may_only_tune_downward() {
        let ceiling = ShareLimits::default();
        let lower = ShareLimits {
            fetch_max_bytes: 1_000,
            fetch_max_objects: 10,
            requests_per_minute: 5,
        };
        assert_eq!(ShareLimits::tuned(lower, ceiling), Ok(lower));
        let higher = ShareLimits {
            requests_per_minute: 61,
            ..ceiling
        };
        assert_eq!(
            ShareLimits::tuned(higher, ceiling),
            Err(LimitsError::AboveCeiling("requests_per_minute"))
        );
        let zero = ShareLimits {
            fetch_max_objects: 0,
            ..ceiling
        };
        assert_eq!(
            ShareLimits::tuned(zero, ceiling),
            Err(LimitsError::Zero("fetch_max_objects"))
        );
        let ceilings = PlatformCeilings {
            share: ceiling,
            owner: OwnerAggregateLimits {
                requests_per_minute: 0,
                bytes_per_minute: 1,
            },
        };
        assert_eq!(
            ceilings.validate(),
            Err(LimitsError::Zero("owner requests_per_minute"))
        );
    }

    #[test]
    fn a_fetch_under_both_caps_is_complete() {
        let plan = plan_snapshot_fetch(&small(), objects(&[10, 20, 30]));
        assert_eq!(plan.served, vec![1, 2, 3]);
        assert_eq!(plan.bytes, 60);
        assert!(plan.is_complete());
    }

    #[test]
    fn the_object_cap_stops_the_fetch_and_names_the_resume_point() {
        let plan = plan_snapshot_fetch(&small(), objects(&[1, 1, 1, 1, 1]));
        assert_eq!(plan.served, vec![1, 2, 3]);
        assert!(!plan.is_complete());
        let capped = plan.capped.unwrap();
        assert_eq!(capped.name(), "snapshot_capped");
        assert_eq!(
            capped,
            ServingLimit::SnapshotCapped {
                cap: FetchCap::Objects,
                resume_after: Some(3)
            }
        );
    }

    #[test]
    fn the_byte_cap_stops_before_the_object_that_would_pass_it() {
        let plan = plan_snapshot_fetch(&small(), objects(&[60, 40, 1]));
        assert_eq!(plan.served, vec![1, 2]);
        assert_eq!(plan.bytes, 100);
        assert_eq!(
            plan.capped,
            Some(ServingLimit::SnapshotCapped {
                cap: FetchCap::Bytes,
                resume_after: Some(2)
            })
        );
    }

    #[test]
    fn an_oversized_first_object_is_capped_not_dropped() {
        let plan = plan_snapshot_fetch(&small(), objects(&[101]));
        assert!(plan.served.is_empty());
        assert_eq!(
            plan.capped,
            Some(ServingLimit::SnapshotCapped {
                cap: FetchCap::Bytes,
                resume_after: None
            })
        );
    }

    #[test]
    fn exactly_at_the_cap_is_complete_when_nothing_follows() {
        let plan = plan_snapshot_fetch(&small(), objects(&[50, 50]));
        assert!(plan.is_complete());
        let plan = plan_snapshot_fetch(&small(), objects(&[1, 1, 1]));
        assert!(plan.is_complete());
    }

    #[test]
    fn share_rate_is_per_reader_and_rolls_each_minute() {
        let start = Instant::now();
        let mut share = ShareRateLimiter::new(&small());
        share.admit::<u32>(&"alice", start).unwrap();
        share.admit::<u32>(&"alice", start).unwrap();
        let refused = share.admit::<u32>(&"alice", start + Duration::from_secs(10));
        assert_eq!(
            refused,
            Err(ServingLimit::ShareRateLimited {
                retry_after: Duration::from_secs(50)
            })
        );
        // Another reader has its own window.
        share.admit::<u32>(&"bob", start).unwrap();
        // A clock stepping back does not open a free window.
        assert!(share.admit::<u32>(&"alice", start).is_err());
        // A minute later the window rolls.
        share.admit::<u32>(&"alice", start + RATE_WINDOW).unwrap();
        share.prune(start + RATE_WINDOW + RATE_WINDOW);
        assert!(share.readers.is_empty());
    }

    #[test]
    fn owner_aggregate_counts_across_shares() {
        let start = Instant::now();
        let mut owner = OwnerAggregateBudget::new(OwnerAggregateLimits {
            requests_per_minute: 10,
            bytes_per_minute: 100,
        });
        owner.admit::<u32>(start, 60).unwrap();
        assert_eq!(
            owner.admit::<u32>(start, 41),
            Err(ServingLimit::OwnerAggregateLimited {
                retry_after: RATE_WINDOW
            })
        );
        owner.admit::<u32>(start, 40).unwrap();
        owner.admit::<u32>(start + RATE_WINDOW, 100).unwrap();
    }

    #[test]
    fn a_refused_fetch_counts_nothing() {
        let start = Instant::now();
        let limits = small();
        let mut share = ShareRateLimiter::new(&limits);
        let mut owner = OwnerAggregateBudget::new(OwnerAggregateLimits {
            requests_per_minute: 10,
            bytes_per_minute: 50,
        });
        let refused = admit_fetch(
            &limits,
            &mut share,
            &mut owner,
            &"alice",
            start,
            objects(&[30, 30]),
        );
        assert_eq!(refused.unwrap_err().name(), "owner_aggregate_limited");
        // Neither ledger moved: the reader still has both requests.
        let plan = admit_fetch(
            &limits,
            &mut share,
            &mut owner,
            &"alice",
            start,
            objects(&[30]),
        )
        .unwrap();
        assert!(plan.is_complete());
        admit_fetch::<u32, _, _>(&limits, &mut share, &mut owner, &"alice", start, vec![]).unwrap();
        let refused =
            admit_fetch::<u32, _, _>(&limits, &mut share, &mut owner, &"alice", start, vec![]);
        assert_eq!(refused.unwrap_err().name(), "share_rate_limited");
    }

    #[test]
    fn a_capped_fetch_is_served_and_says_so() {
        let start = Instant::now();
        let limits = small();
        let mut share = ShareRateLimiter::new(&limits);
        let mut owner = OwnerAggregateBudget::new(OwnerAggregateLimits {
            requests_per_minute: 10,
            bytes_per_minute: 1_000,
        });
        let plan = admit_fetch(
            &limits,
            &mut share,
            &mut owner,
            &"alice",
            start,
            objects(&[10, 10, 10, 10]),
        )
        .unwrap();
        assert_eq!(plan.served, vec![1, 2, 3]);
        assert_eq!(
            plan.capped.unwrap().to_string(),
            "serving limit: snapshot capped at fetch_objects"
        );
    }
}
