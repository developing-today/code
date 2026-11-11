//! Injected clock for the world actor and scheduler.
//!
//! Every *time decision* the actor and scheduler make — when a schedule or
//! tick is due, whether a capability has expired, how long ago a chat line
//! was sent — reads through [`Clock`]. The default is the process's real
//! [`SystemClock`]; tests swap in a [`ManualClock`] they advance by hand, so
//! schedule and tick tests run without sleeping.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A source of wall-clock and monotonic time for the world runtime.
///
/// `Debug` is required so a [`crate::world::WorldCore`] holding an
/// `Arc<dyn Clock>` can keep its derived `Debug`.
pub trait Clock: std::fmt::Debug + Send + Sync + 'static {
    /// Milliseconds since the Unix epoch. This is the scheduler's absolute
    /// time: schedule due times and capability expiries are compared against
    /// it.
    fn now_ms(&self) -> u64;

    /// A monotonic instant, used for tick deadlines and loop wakeups.
    fn now_instant(&self) -> Instant;
}

/// The process's real clock.
#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        crate::time::unix_ms()
    }

    fn now_instant(&self) -> Instant {
        Instant::now()
    }
}

/// A clock a test advances by hand. [`ManualClock::advance`] moves `now_ms`
/// and `now_instant` forward together, so a schedule due at `now_ms() + N`
/// also crosses the monotonic deadline `now_instant() + N`.
///
/// The two faces share a single `AtomicU64` offset over a construction-time
/// base, so an `Arc<ManualClock>` can be advanced from the test while the
/// actor reads it through `Arc<dyn Clock>`.
#[derive(Debug)]
pub struct ManualClock {
    /// Milliseconds since the Unix epoch at construction.
    epoch_ms: u64,
    /// The monotonic instant that `epoch_ms` corresponds to.
    base: Instant,
    /// Milliseconds advanced past `epoch_ms`, for both faces.
    offset_ms: AtomicU64,
}

impl ManualClock {
    /// A clock that starts at `epoch_ms` (unix milliseconds) and the current
    /// monotonic instant.
    #[must_use]
    pub fn new(epoch_ms: u64) -> Self {
        Self {
            epoch_ms,
            base: Instant::now(),
            offset_ms: AtomicU64::new(0),
        }
    }

    /// Advance both faces of the clock by `ms` milliseconds.
    pub fn advance(&self, ms: u64) {
        self.offset_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.epoch_ms
            .saturating_add(self.offset_ms.load(Ordering::SeqCst))
    }

    fn now_instant(&self) -> Instant {
        self.base + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}
