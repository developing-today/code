//! Admission for synchronous guest burns (#1067, F12.10 slice 2;
//! design: `doc/2026-11-22_design_id1067-admission.md`).
//!
//! A guest call may burn its whole fuel budget in one synchronous
//! execution. Unbounded, as many worlds burn at once as there are
//! callers, and on a shared runtime that saturates every core: the
//! slice-1 profile showed the victim world's spinning-phase p99 was
//! wake starvation, not service. This module is the bound: a
//! semaphore-class budget over concurrent guest calls, shared
//! process-wide (the hub is the process).
//!
//! A call past the bound *queues*; it is never refused. Refusal is
//! the vocabulary for requests a world is not allowed to make — a
//! guest call is an execution the world is already entitled to, and
//! rejecting it because *other* worlds are burning would invert the
//! fairness criterion. Queueing cannot deadlock: every admitted
//! call is fuel-bounded, so every permit returns within one fuel
//! budget of work.
//!
//! Permits are granted fairly, not FIFO: the waiting call whose
//! charge key has the least attained service (settled fuel, decayed
//! with a one-second half-life) goes first. A world that just burned
//! a full fuel budget queues behind a world that has burned almost
//! nothing, so a light world's wait is about one permit turnover
//! rather than a rotation through every heavy world.
//!
//! The per-key counters behind [`GuestBurnBudget::snapshot`] are
//! the charge record the per-account quota criterion hangs off:
//! when world→account ownership is decided (see the issue), keys
//! aggregate by account without touching this admission path.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Half-life of attained service for grant ordering: a key's settled
/// fuel counts half as much after this long. One second forgives a
/// burn burst quickly enough that a world which behaved for a few
/// seconds competes as fresh, while within a burst the heavy keys
/// stay ordered behind the light ones.
const ATTAINED_HALF_LIFE: Duration = Duration::from_secs(1);

/// Default concurrency bound for the shared budget: every core but
/// two — one reserved for the async runtime and the actors, one for
/// drivers, journal, and the OS — so a full burn pool still leaves
/// a core for a victim's driver and actor to wake on.
pub fn default_max_concurrent() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .saturating_sub(2)
        .max(1)
}

/// Per-key charge counters, as read by [`GuestBurnBudget::snapshot`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestBurnKeySnapshot {
    /// Calls of this key executing right now.
    pub in_flight: usize,
    /// Calls admitted since the budget was created.
    pub admitted: u64,
    /// Calls that had to wait for a permit.
    pub waits: u64,
    /// Total fuel settled by this key's calls (actual spend; the
    /// full allowance for a call that never settled).
    pub settled_fuel: u64,
}

/// A point-in-time read of one budget's counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestBurnSnapshot {
    /// The concurrency bound.
    pub max_concurrent: usize,
    /// Calls executing right now, across all keys.
    pub in_flight: usize,
    /// High-water mark of `in_flight`.
    pub in_flight_max: usize,
    /// Charge counters by key.
    pub keys: BTreeMap<String, GuestBurnKeySnapshot>,
}

#[derive(Default)]
struct KeyCharge {
    in_flight: usize,
    admitted: u64,
    waits: u64,
    settled_fuel: u64,
    /// Decayed attained service (fuel) as of `attained_at`.
    attained: f64,
    attained_at: Option<Instant>,
}

impl KeyCharge {
    /// Attained service at `now`, with decay applied.
    fn attained_at(&self, now: Instant) -> f64 {
        let Some(at) = self.attained_at else {
            return self.attained;
        };
        let elapsed = now.saturating_duration_since(at).as_secs_f64();
        self.attained
            * 0.5_f64.powf(elapsed / ATTAINED_HALF_LIFE.as_secs_f64())
    }

    /// Fold a settled spend into attained service.
    fn settle(&mut self, fuel: u64, now: Instant) {
        self.attained = self.attained_at(now) + fuel as f64;
        self.attained_at = Some(now);
        self.settled_fuel = self.settled_fuel.saturating_add(fuel);
    }
}

struct Waiter {
    ticket: u64,
    key: String,
}

#[derive(Default)]
struct BudgetState {
    in_flight: usize,
    in_flight_max: usize,
    next_ticket: u64,
    keys: BTreeMap<String, KeyCharge>,
    waiters: VecDeque<Waiter>,
}

impl BudgetState {
    fn key_mut(&mut self, key: &str) -> &mut KeyCharge {
        self.keys.entry(key.to_owned()).or_default()
    }

    /// The ticket entitled to the next permit: least attained
    /// service first, arrival order breaking ties.
    fn next_grant(&self, now: Instant) -> Option<u64> {
        self.waiters
            .iter()
            .min_by(|a, b| {
                let attained = |w: &Waiter| {
                    self.keys
                        .get(&w.key)
                        .map_or(0.0, |charge| charge.attained_at(now))
                };
                attained(a)
                    .total_cmp(&attained(b))
                    .then(a.ticket.cmp(&b.ticket))
            })
            .map(|waiter| waiter.ticket)
    }
}

/// A bounded-concurrency budget for synchronous guest calls,
/// charged per key. See the module docs for the fairness rule.
pub struct GuestBurnBudget {
    max_concurrent: usize,
    state: Mutex<BudgetState>,
    available: Condvar,
}

impl std::fmt::Debug for GuestBurnBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestBurnBudget")
            .field("max_concurrent", &self.max_concurrent)
            .finish_non_exhaustive()
    }
}

impl GuestBurnBudget {
    /// A budget admitting at most `max_concurrent` guest calls at
    /// once. The bound is a hard resource limit, so it is clamped
    /// to at least one: a zero bound would refuse-by-deadlock every
    /// call, which this mechanism exists to avoid.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent: max_concurrent.max(1),
            state: Mutex::new(BudgetState::default()),
            available: Condvar::new(),
        }
    }

    /// The process-wide default budget, sized by
    /// [`default_max_concurrent`]. One hub is one process, so one
    /// shared budget is the hub's burn pool.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<GuestBurnBudget>> = OnceLock::new();
        SHARED
            .get_or_init(|| Arc::new(Self::new(default_max_concurrent())))
            .clone()
    }

    /// The process-wide budget for an explicitly configured bound
    /// (the serve path's `RuntimeLimits::guest_burn_concurrency`).
    /// The first configured value wins — one hub, one budget — and
    /// `0` means the auto-sized [`Self::shared`] budget.
    pub fn shared_for(max_concurrent: usize) -> Arc<Self> {
        if max_concurrent == 0 {
            return Self::shared();
        }
        static CONFIGURED: OnceLock<Arc<GuestBurnBudget>> = OnceLock::new();
        CONFIGURED
            .get_or_init(|| Arc::new(Self::new(max_concurrent)))
            .clone()
    }

    /// The concurrency bound.
    pub const fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Lock the counters, recovering from poisoning: the state is
    /// plain counters updated in short non-panicking stretches, and
    /// a poisoned budget that stalled every guest call forever
    /// would be strictly worse than recovering the counts.
    fn lock_state(&self) -> MutexGuard<'_, BudgetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Admit one guest call charged to `key`, waiting for a permit
    /// if the bound is reached. `fuel_allowance` is the call's
    /// worst-case burn (its fuel limit), charged if the call never
    /// settles with a measured spend.
    pub fn acquire(self: &Arc<Self>, key: &str, fuel_allowance: u64) -> GuestBurnPermit {
        let mut state = self.lock_state();
        if state.waiters.is_empty() && state.in_flight < self.max_concurrent {
            Self::admit(&mut state, key);
        } else {
            let ticket = state.next_ticket;
            state.next_ticket += 1;
            state.key_mut(key).waits += 1;
            state.waiters.push_back(Waiter {
                ticket,
                key: key.to_owned(),
            });
            loop {
                if state.in_flight < self.max_concurrent
                    && state.next_grant(Instant::now()) == Some(ticket)
                {
                    state.waiters.retain(|waiter| waiter.ticket != ticket);
                    Self::admit(&mut state, key);
                    break;
                }
                state = self
                    .available
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        GuestBurnPermit {
            budget: Arc::clone(self),
            key: key.to_owned(),
            fuel_allowance,
            settled: false,
        }
    }

    fn admit(state: &mut BudgetState, key: &str) {
        state.in_flight += 1;
        state.in_flight_max = state.in_flight_max.max(state.in_flight);
        let charge = state.key_mut(key);
        charge.in_flight += 1;
        charge.admitted += 1;
    }

    fn release(&self, key: &str, fuel_charged: u64) {
        let mut state = self.lock_state();
        state.in_flight = state.in_flight.saturating_sub(1);
        let charge = state.key_mut(key);
        charge.in_flight = charge.in_flight.saturating_sub(1);
        charge.settle(fuel_charged, Instant::now());
        drop(state);
        self.available.notify_all();
    }

    /// The current counters, for tests, metrics, and the future
    /// quota layer.
    pub fn snapshot(&self) -> GuestBurnSnapshot {
        let state = self.lock_state();
        GuestBurnSnapshot {
            max_concurrent: self.max_concurrent,
            in_flight: state.in_flight,
            in_flight_max: state.in_flight_max,
            keys: state
                .keys
                .iter()
                .map(|(key, charge)| {
                    (
                        key.clone(),
                        GuestBurnKeySnapshot {
                            in_flight: charge.in_flight,
                            admitted: charge.admitted,
                            waits: charge.waits,
                            settled_fuel: charge.settled_fuel,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Calls currently waiting for a permit. Observability for the
    /// same readers as [`Self::snapshot`]; tests use it to order
    /// concurrent waiters deterministically.
    pub fn waiting(&self) -> usize {
        self.lock_state().waiters.len()
    }
}

/// One admitted guest call. Settle it with the call's measured fuel
/// spend via [`Self::settle`]; dropping it unsettled charges the
/// full allowance and still releases the permit.
pub struct GuestBurnPermit {
    budget: Arc<GuestBurnBudget>,
    key: String,
    fuel_allowance: u64,
    settled: bool,
}

impl std::fmt::Debug for GuestBurnPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestBurnPermit")
            .field("key", &self.key)
            .field("fuel_allowance", &self.fuel_allowance)
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}

impl GuestBurnPermit {
    /// Record the call's actual spent fuel and release the permit.
    pub fn settle(mut self, fuel_spent: u64) {
        self.budget.release(&self.key, fuel_spent);
        self.settled = true;
    }
}

impl Drop for GuestBurnPermit {
    fn drop(&mut self) {
        if !self.settled {
            self.budget.release(&self.key, self.fuel_allowance);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    /// Spin until `condition` holds, bounded so a broken budget
    /// fails the test instead of hanging the suite.
    fn wait_until(condition: impl Fn() -> bool) {
        for _ in 0..10_000 {
            if condition() {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("condition not met within the polling bound");
    }

    #[test]
    fn the_bound_is_never_exceeded_and_every_call_completes() {
        let budget = Arc::new(GuestBurnBudget::new(2));
        let running = Arc::new(AtomicUsize::new(0));
        let running_max = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for n in 0..8 {
            let budget = Arc::clone(&budget);
            let running = Arc::clone(&running);
            let running_max = Arc::clone(&running_max);
            threads.push(thread::spawn(move || {
                let permit = budget.acquire(&format!("world-{n}"), 100);
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                running_max.fetch_max(now, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(10));
                running.fetch_sub(1, Ordering::SeqCst);
                permit.settle(100);
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.in_flight_max, 2, "bound held at every instant");
        assert_eq!(snapshot.in_flight, 0, "every permit released");
        assert_eq!(snapshot.keys.len(), 8);
        assert!(
            snapshot
                .keys
                .values()
                .all(|key| key.admitted == 1 && key.settled_fuel == 100),
            "each key charged its settled spend: {snapshot:?}"
        );
    }

    #[test]
    fn charges_are_attributed_per_key_and_an_unsettled_permit_charges_its_allowance() {
        let budget = Arc::new(GuestBurnBudget::new(4));
        budget.acquire("victim", 10_000_000).settle(120);
        budget.acquire("victim", 10_000_000).settle(80);
        budget.acquire("spinner", 20_000_000).settle(20_000_000);
        // Dropped without settling: the allowance is the charge.
        drop(budget.acquire("spinner", 20_000_000));

        let snapshot = budget.snapshot();
        let victim = &snapshot.keys["victim"];
        assert_eq!(victim.admitted, 2);
        assert_eq!(victim.settled_fuel, 200);
        let spinner = &snapshot.keys["spinner"];
        assert_eq!(spinner.admitted, 2);
        assert_eq!(spinner.settled_fuel, 40_000_000);
        assert_eq!(snapshot.in_flight, 0);
    }

    #[test]
    fn a_call_past_the_bound_waits_and_then_runs() {
        let budget = Arc::new(GuestBurnBudget::new(1));
        let held = budget.acquire("holder", 10);
        let waiter_budget = Arc::clone(&budget);
        let waiter = thread::spawn(move || {
            let permit = waiter_budget.acquire("waiter", 10);
            permit.settle(10);
        });
        wait_until(|| budget.waiting() == 1);
        assert_eq!(budget.snapshot().keys["waiter"].admitted, 0);
        held.settle(10);
        waiter.join().unwrap();
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.keys["waiter"].admitted, 1);
        assert_eq!(snapshot.keys["waiter"].waits, 1);
        assert_eq!(snapshot.in_flight, 0);
    }

    #[test]
    fn grant_order_prefers_the_least_attained_key_over_an_earlier_heavy_waiter() {
        let budget = Arc::new(GuestBurnBudget::new(1));
        // The "heavy" key first completes a full-budget burn, so it
        // carries 20M of attained service into the contention below.
        budget.acquire("heavy", 20_000_000).settle(20_000_000);
        // A blocker then holds the only permit while both waiters queue.
        let held = budget.acquire("holder", 20_000_000);

        let grants = Arc::new(Mutex::new(Vec::new()));
        let spawn_waiter = |key: &'static str| {
            let budget = Arc::clone(&budget);
            let grants = Arc::clone(&grants);
            thread::spawn(move || {
                let permit = budget.acquire(key, 20_000_000);
                grants.lock().unwrap().push(key);
                permit.settle(20_000_000);
            })
        };
        // The heavy waiter queues first; the light waiter second.
        // FIFO would grant heavy first — the fairness rule must not.
        let heavy = spawn_waiter("heavy");
        wait_until(|| budget.waiting() == 1);
        let light = spawn_waiter("light");
        wait_until(|| budget.waiting() == 2);

        held.settle(20_000_000);
        heavy.join().unwrap();
        light.join().unwrap();
        assert_eq!(
            *grants.lock().unwrap(),
            ["light", "heavy"],
            "the least-attained key is granted before the earlier heavy waiter"
        );
    }
}
