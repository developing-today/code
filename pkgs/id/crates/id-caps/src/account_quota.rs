//! Per-account quota policy and fuel-rate accounting (#1067, F12.10).
//!
//! The bounds in [`crate::world_limits`] are per-world ceilings set by
//! the server admin: one invocation's fuel, one world's memory. They
//! say nothing about an *account* — the principal that owns worlds —
//! so one account can still monopolise a hub by spreading work across
//! many worlds or by invoking flat-out around the clock. F12.10 adds
//! the account level: quotas for how many worlds an account may hold,
//! how much storage it may use, and how much fuel it may burn per
//! second, so a hub scheduler can share fuel and CPU fairly across
//! accounts instead of first-come-first-served.
//!
//! This module is the contract half, in the shape #557 established for
//! the per-world memory budget: a validated policy type
//! ([`AccountQuotas`]) plus a deterministic accounting ledger
//! ([`FuelRateBudget`]) with typed, recoverable refusals. Enforcement
//! wiring charges the ledger at the invocation boundary: since slice
//! 2 the sandbox's `WorldInstance` (in `id-worlds`) admits each update
//! against an installed budget, priced at the invocation's fuel
//! allowance. Aggregating one budget across all of an account's
//! worlds at the hub is the remaining slice. Nothing here performs
//! I/O or reads a clock, so the accounting is fully determined by the
//! charges and timestamps its caller supplies.

use std::time::{Duration, Instant};

use anyhow::{Result, ensure};

/// Default per-account ceiling on worlds held at once.
///
/// A proposed policy value, not a measured result: an order of
/// magnitude above the one or two worlds an interactive account
/// typically keeps open, low enough that a runaway account cannot
/// crowd a hub's open-world bound on its own.
pub const DEFAULT_ACCOUNT_MAX_WORLDS: u32 = 16;

/// Default per-account storage ceiling (1 GiB of logical charges).
///
/// A proposed policy value, not a measured result.
pub const DEFAULT_ACCOUNT_MAX_STORAGE_BYTES: u64 = 1024 * 1024 * 1024;

/// Default per-account fuel rate: 100,000,000 fuel per second.
///
/// A proposed policy value, not a measured result: ten times the
/// default per-invocation fuel ceiling
/// ([`crate::world_limits::SandboxLimits`]), so an account may run
/// the equivalent of ten full-budget invocations every second,
/// sustained — and no more, however many worlds it spreads them over.
pub const DEFAULT_ACCOUNT_MAX_FUEL_PER_SECOND: u64 = 100_000_000;

/// Length of the fixed window a [`FuelRateBudget`] accounts over:
/// one second, matching the per-second statement of the fuel quota.
pub const FUEL_RATE_WINDOW: Duration = Duration::from_secs(1);

/// Per-account quotas, set by the server's admin.
///
/// The account-level complement to the per-world ceilings in
/// [`crate::world_limits`]: those bound any *one* world or call, these
/// bound the *sum* over everything one account owns. The values are
/// validated before the server starts, like the runtime limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountQuotas {
    /// Worlds one account may hold at once.
    pub max_worlds: u32,
    /// Logical storage bytes one account may charge in total.
    pub max_storage_bytes: u64,
    /// Fuel one account may burn per second, across all its worlds.
    pub max_fuel_per_second: u64,
}

impl AccountQuotas {
    /// Check the quotas are usable: every quota must be nonzero (a
    /// zero quota would silently disable the resource for every
    /// account, which is a configuration error, not a policy).
    ///
    /// Note the fuel rate should also be at least the server's
    /// per-invocation fuel ceiling: below it, a single full-budget
    /// invocation can never fit in any window and the account's
    /// worlds can never run. That cross-check belongs to the caller,
    /// which owns both values.
    ///
    /// # Errors
    ///
    /// Fails with a human-readable reason naming the offending quota.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.max_worlds > 0, "account world quota must be nonzero");
        ensure!(
            self.max_storage_bytes > 0,
            "account storage quota must be nonzero"
        );
        ensure!(
            self.max_fuel_per_second > 0,
            "account fuel-per-second quota must be nonzero"
        );
        Ok(())
    }

    /// A fresh fuel-rate ledger enforcing this policy's fuel quota.
    #[must_use]
    pub const fn fuel_budget(&self) -> FuelRateBudget {
        FuelRateBudget::new(self.max_fuel_per_second)
    }
}

impl Default for AccountQuotas {
    fn default() -> Self {
        Self {
            max_worlds: DEFAULT_ACCOUNT_MAX_WORLDS,
            max_storage_bytes: DEFAULT_ACCOUNT_MAX_STORAGE_BYTES,
            max_fuel_per_second: DEFAULT_ACCOUNT_MAX_FUEL_PER_SECOND,
        }
    }
}

/// A refused fuel charge against an account's per-second rate
/// (#1067).
///
/// Typed and recoverable, and deliberately distinct from a guest
/// trap and from a per-world ceiling refusal: the account is
/// unharmed, nothing was charged, and the same charge can succeed in
/// a later window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuelRateRefusal {
    /// The account's fuel-per-second limit.
    pub limit_per_second: u64,
    /// Fuel already charged in the current window when the charge
    /// was refused.
    pub charged_in_window: u64,
    /// Fuel the refused charge asked for.
    pub requested_fuel: u64,
}

impl std::fmt::Display for FuelRateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "account is at its fuel rate: {} charged + {} requested \
             exceeds the {}-fuel-per-second quota",
            self.charged_in_window, self.requested_fuel, self.limit_per_second
        )
    }
}

impl std::error::Error for FuelRateRefusal {}

/// Per-account fuel ledger over fixed one-second windows (#1067).
///
/// Deterministic accounting, not a gauge: the caller supplies the
/// timestamp of every charge, so a charge sequence always accounts
/// the same way. The invariant is `charged_in_window <=
/// limit_per_second` at every observable point; a charge that would
/// break it is refused whole — it charges nothing and bumps
/// `refusal_count` — so a refused invocation leaves the ledger
/// exactly as it was and can be retried in the next window. A window
/// rolls when a charge arrives at least [`FUEL_RATE_WINDOW`] after
/// the window started; an arrival timestamped before the window
/// start (a caller clock stepping back) is accounted in the current
/// window rather than opening a free one.
#[derive(Debug, Clone)]
pub struct FuelRateBudget {
    limit_per_second: u64,
    window_started: Option<Instant>,
    charged_in_window: u64,
    total_charged: u64,
    refusal_count: u64,
}

impl FuelRateBudget {
    /// A fresh ledger with nothing charged and no window open.
    #[must_use]
    pub const fn new(limit_per_second: u64) -> Self {
        Self {
            limit_per_second,
            window_started: None,
            charged_in_window: 0,
            total_charged: 0,
            refusal_count: 0,
        }
    }

    /// The per-second fuel limit this ledger enforces.
    #[must_use]
    pub const fn limit_per_second(&self) -> u64 {
        self.limit_per_second
    }

    /// Fuel charged in the current window, as of the last charge.
    #[must_use]
    pub const fn charged_in_window(&self) -> u64 {
        self.charged_in_window
    }

    /// Fuel charged over the ledger's whole life, across all
    /// windows. Monotone; for reporting, never for enforcement.
    #[must_use]
    pub const fn total_charged(&self) -> u64 {
        self.total_charged
    }

    /// How many charges this ledger has refused.
    #[must_use]
    pub const fn refusal_count(&self) -> u64 {
        self.refusal_count
    }

    /// Charge `fuel` at time `now`, or refuse the whole charge.
    ///
    /// # Errors
    ///
    /// Returns [`FuelRateRefusal`] — charging nothing — when the
    /// charge would take the current window's total past the limit
    /// (or overflow). A charge larger than the limit itself is
    /// refused in every window, including a fresh one.
    pub fn try_charge(&mut self, now: Instant, fuel: u64) -> Result<(), FuelRateRefusal> {
        if let Some(started) = self.window_started {
            // A timestamp before the window start (a caller clock
            // stepping back) stays in the current window: only a
            // forward step of a full window rolls it. `duration_since`
            // would panic on a backwards timestamp, so use the
            // checked form and treat `None` as "no rollover".
            if now
                .checked_duration_since(started)
                .is_some_and(|elapsed| elapsed >= FUEL_RATE_WINDOW)
            {
                self.window_started = Some(now);
                self.charged_in_window = 0;
            }
        } else {
            self.window_started = Some(now);
        }
        match self.charged_in_window.checked_add(fuel) {
            Some(total) if total <= self.limit_per_second => {
                self.charged_in_window = total;
                self.total_charged = self.total_charged.saturating_add(fuel);
                Ok(())
            }
            _ => {
                self.refusal_count += 1;
                Err(FuelRateRefusal {
                    limit_per_second: self.limit_per_second,
                    charged_in_window: self.charged_in_window,
                    requested_fuel: fuel,
                })
            }
        }
    }
}

impl Default for FuelRateBudget {
    fn default() -> Self {
        Self::new(DEFAULT_ACCOUNT_MAX_FUEL_PER_SECOND)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_default_quotas_are_valid() {
        AccountQuotas::default().validate().unwrap();
    }

    #[test]
    fn validation_rejects_a_zero_quota_in_any_field() {
        let valid = AccountQuotas::default();
        assert!(
            AccountQuotas {
                max_worlds: 0,
                ..valid
            }
            .validate()
            .is_err()
        );
        assert!(
            AccountQuotas {
                max_storage_bytes: 0,
                ..valid
            }
            .validate()
            .is_err()
        );
        assert!(
            AccountQuotas {
                max_fuel_per_second: 0,
                ..valid
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn charges_accumulate_within_a_window() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(100);
        budget.try_charge(start, 40).unwrap();
        budget
            .try_charge(start + Duration::from_millis(500), 60)
            .unwrap();
        assert_eq!(budget.charged_in_window(), 100);
        assert_eq!(budget.total_charged(), 100);
        assert_eq!(budget.refusal_count(), 0);
    }

    #[test]
    fn a_charge_past_the_rate_is_refused_whole() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(100);
        budget.try_charge(start, 90).unwrap();
        let refusal = budget
            .try_charge(start + Duration::from_millis(100), 11)
            .unwrap_err();
        assert_eq!(refusal.limit_per_second, 100);
        assert_eq!(refusal.charged_in_window, 90);
        assert_eq!(refusal.requested_fuel, 11);
        assert_eq!(budget.charged_in_window(), 90, "refusal must be atomic");
        assert_eq!(budget.total_charged(), 90);
        assert_eq!(budget.refusal_count(), 1);
    }

    #[test]
    fn the_window_rolls_only_after_a_full_second() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(100);
        budget.try_charge(start, 100).unwrap();
        // 999ms later the window is still the same one.
        assert!(
            budget
                .try_charge(start + Duration::from_millis(999), 1)
                .is_err()
        );
        // At a full second the allowance is fresh again: refusal is
        // recoverable, not poisoning.
        budget
            .try_charge(start + Duration::from_secs(1), 100)
            .unwrap();
        assert_eq!(budget.charged_in_window(), 100);
        assert_eq!(budget.total_charged(), 200);
    }

    #[test]
    fn an_idle_gap_opens_a_fresh_window_at_the_next_charge() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(50);
        budget.try_charge(start, 50).unwrap();
        budget
            .try_charge(start + Duration::from_secs(3600), 50)
            .unwrap();
        assert_eq!(budget.charged_in_window(), 50);
        assert_eq!(budget.total_charged(), 100);
    }

    #[test]
    fn a_charge_larger_than_the_limit_is_refused_in_a_fresh_window() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(100);
        assert!(budget.try_charge(start, 101).is_err());
        assert_eq!(budget.charged_in_window(), 0);
        assert_eq!(budget.refusal_count(), 1);
    }

    #[test]
    fn an_overflowing_charge_is_refused_not_wrapped() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(u64::MAX);
        budget.try_charge(start, u64::MAX).unwrap();
        assert!(budget.try_charge(start, 1).is_err());
        assert_eq!(budget.charged_in_window(), u64::MAX);
    }

    #[test]
    fn a_backwards_timestamp_stays_in_the_current_window() {
        let start = Instant::now();
        let mut budget = FuelRateBudget::new(100);
        budget
            .try_charge(start + Duration::from_millis(500), 90)
            .unwrap();
        // A timestamp before the window start is accounted in the
        // current window, not a free one.
        assert!(budget.try_charge(start, 11).is_err());
        assert_eq!(budget.charged_in_window(), 90);
        assert_eq!(budget.total_charged(), 90);
        assert_eq!(budget.refusal_count(), 1);
    }

    #[test]
    fn a_quota_policy_mints_a_budget_at_its_own_rate() {
        let quotas = AccountQuotas {
            max_fuel_per_second: 7,
            ..AccountQuotas::default()
        };
        let budget = quotas.fuel_budget();
        assert_eq!(budget.limit_per_second(), 7);
    }

    /// Property coverage for the ledger invariants
    /// (Codex review on #12539): bounded window charges,
    /// whole-charge refusals, and monotone lifetime accounting,
    /// over generated limits, fuel charges, and timestamp
    /// sequences — including window rollover and backwards
    /// timestamps. The oracle is a logical-time model of the
    /// ledger, checked after every charge.
    mod ledger_property {
        use super::*;
        use proptest::prelude::*;

        /// Logical-time model of [`FuelRateBudget`]: the oracle.
        /// Window state is tracked in milliseconds since the
        /// sequence start, so a backwards timestamp is just a
        /// smaller number and never opens a free window.
        #[derive(Debug, Default)]
        struct Model {
            limit: u64,
            window_started: Option<i64>,
            charged_in_window: u64,
            total_charged: u64,
            refusal_count: u64,
        }

        impl Model {
            fn charge(&mut self, now_ms: i64, fuel: u64) -> Result<(), FuelRateRefusal> {
                match self.window_started {
                    None => {
                        self.window_started = Some(now_ms);
                        self.charged_in_window = 0;
                    }
                    Some(started) if now_ms - started >= 1_000 => {
                        self.window_started = Some(now_ms);
                        self.charged_in_window = 0;
                    }
                    Some(_) => {}
                }
                match self.charged_in_window.checked_add(fuel) {
                    Some(total) if total <= self.limit => {
                        self.charged_in_window = total;
                        self.total_charged = self.total_charged.saturating_add(fuel);
                        Ok(())
                    }
                    _ => {
                        self.refusal_count += 1;
                        Err(FuelRateRefusal {
                            limit_per_second: self.limit,
                            charged_in_window: self.charged_in_window,
                            requested_fuel: fuel,
                        })
                    }
                }
            }
        }

        proptest! {
            #[test]
            fn ledger_matches_its_model_over_generated_sequences(
                limit in prop_oneof![
                    1 => Just(0u64),
                    8 => 1..=1_000_000u64,
                    1 => Just(u64::MAX),
                ],
                ops in prop::collection::vec(
                    (
                        prop_oneof![10 => 0..=1000u64, 1 => any::<u64>()],
                        // Millisecond steps between charges: forward
                        // steps roll windows, negative steps are
                        // backwards timestamps.
                        -1500i64..=3000i64,
                    ),
                    0..=64,
                ),
            ) {
                let start = Instant::now();
                let mut budget = FuelRateBudget::new(limit);
                let mut model = Model {
                    limit,
                    ..Model::default()
                };
                let mut now_ms: i64 = 0;
                let mut prev_total = 0u64;
                for (fuel, delta_ms) in ops {
                    now_ms += delta_ms;
                    let now = if now_ms >= 0 {
                        start + Duration::from_millis(now_ms as u64)
                    } else {
                        start
                            .checked_sub(Duration::from_millis(now_ms.unsigned_abs()))
                            .expect("generated timestamps stay within uptime")
                    };
                    let before = (
                        budget.charged_in_window(),
                        budget.total_charged(),
                        budget.refusal_count(),
                    );
                    let actual = budget.try_charge(now, fuel);
                    let expected = model.charge(now_ms, fuel);
                    prop_assert_eq!(actual.is_ok(), expected.is_ok());
                    if let Err(expected_refusal) = expected {
                        let actual_refusal = actual.unwrap_err();
                        prop_assert_eq!(actual_refusal, expected_refusal);
                        // A refusal charges nothing: the lifetime
                        // total does not move, and only the refusal
                        // count advances. The window counter is not
                        // pinned here — a charge arriving in a new
                        // window rolls it to zero first, which is
                        // time-driven, not charge-driven; it is
                        // checked against the model below.
                        prop_assert_eq!(budget.total_charged(), before.1);
                        prop_assert_eq!(budget.refusal_count(), before.2 + 1);
                    }
                    // Bounded window charges and monotone lifetime
                    // accounting, after every operation.
                    prop_assert!(budget.charged_in_window() <= limit);
                    prop_assert_eq!(budget.charged_in_window(), model.charged_in_window);
                    prop_assert_eq!(budget.total_charged(), model.total_charged);
                    prop_assert!(budget.total_charged() >= prev_total);
                    prev_total = budget.total_charged();
                    prop_assert_eq!(budget.refusal_count(), model.refusal_count);
                }
            }
        }
    }
}
