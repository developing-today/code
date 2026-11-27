//! Startup bind guard (#15533): survive Tailscale not being ready.
//!
//! Verified firsthand on the broker host (issue journal): at boot the
//! brokers start before Tailscale assigns `100.99.113.103`, so
//! `TcpListener::bind` fails with `Cannot assign requested address`
//! (EADDRNOTAVAIL, errno 99), the services burn through their systemd
//! restart budget (`start-limit-hit`), and stay down after the address
//! appears — a ~20.5h outage until a manual reset+restart.
//!
//! Fix (issue option a, recorded): bind-retry with backoff inside the
//! brokers (and the laptop-agent daemon), host-agnostic and testable
//! in-repo, composing with supervisors. Retry ONLY the
//! Tailscale-not-ready signature (EADDRNOTAVAIL / AddrNotAvailable);
//! every other bind error (EADDRINUSE, permission, bad address) fails
//! fast exactly as before, so a real misconfiguration never spins.
//!
//! Policy: 10-minute budget, 1s base delay doubling to a 15s cap.
//! Nominal delays: 1, 2, 4, 8, 15, 15, … (≈40 tries over 10 min).

use std::time::Duration;

/// Total time budget for bind retries (10 minutes).
pub const BUDGET: Duration = Duration::from_secs(10 * 60);
/// First retry delay.
pub const BASE_DELAY: Duration = Duration::from_secs(1);
/// Cap for the doubled delay.
pub const CAP_DELAY: Duration = Duration::from_secs(15);

/// Nominal delay before the retry after `failed_attempt` failures
/// (0-based: `delay_for_attempt(0)` is the delay before the 2nd try).
/// 1s doubling to a 15s cap: 1, 2, 4, 8, 15, 15, …
pub fn delay_for_attempt(failed_attempt: u32) -> Duration {
    let shift = failed_attempt.min(4);
    let secs = BASE_DELAY.as_secs().saturating_mul(1 << shift);
    Duration::from_secs(secs.min(CAP_DELAY.as_secs()))
}

/// True when a bind error is the Tailscale-not-ready signature: the
/// address does not exist (yet) on this host. Matches by
/// `ErrorKind::AddrNotAvailable` and by raw errno 99
/// (`libc::EADDRNOTAVAIL`), so the classification holds however the
/// OS error surfaces through `TcpListener::bind`.
pub fn is_retryable_bind(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::AddrNotAvailable
        || err.raw_os_error() == Some(libc::EADDRNOTAVAIL)
}

/// Generic retry driver, factored pure for tests (the oracle names
/// the shape, not the clock): call `bind` until it succeeds, a
/// non-retryable error arrives, or `budget` elapses. `is_retryable`
/// classifies the error; `sleep` performs the delay (real sleep in
/// production, a record-and-advance-fake-clock in tests). Returns the
/// successful value, or the last error. At least one attempt always
/// runs, even with a zero budget.
pub fn retry_with_budget<T, E>(
    budget: Duration,
    mut bind: impl FnMut() -> Result<T, E>,
    mut is_retryable: impl FnMut(&E) -> bool,
    mut sleep: impl FnMut(Duration),
    now: impl Fn() -> Duration,
) -> Result<T, E> {
    let start = now();
    let mut attempt: u32 = 0;
    loop {
        match bind() {
            Ok(v) => return Ok(v),
            Err(e) => {
                let elapsed = now().checked_sub(start).unwrap_or(Duration::ZERO);
                if !is_retryable(&e) || elapsed >= budget {
                    return Err(e);
                }
                sleep(delay_for_attempt(attempt).min(budget - elapsed));
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

/// Bind a TCP listener to `bind`, retrying the Tailscale-not-ready
/// signature within [`BUDGET`]. All other errors return immediately.
/// Logs each retry through `crate::log` so the journal shows the
/// wait instead of a restart spiral.
pub fn tcp_bind_with_retry(domain: &str, bind: &str) -> std::io::Result<std::net::TcpListener> {
    let start = std::time::Instant::now();
    retry_with_budget(
        BUDGET,
        || std::net::TcpListener::bind(bind),
        is_retryable_bind,
        |delay| {
            crate::log::info(
                domain,
                &format!("address unavailable: cannot bind {bind} yet; retrying in {delay:?}"),
            );
            std::thread::sleep(delay);
        },
        || start.elapsed(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    proptest::proptest! {
        /// Oracle: saturating exponential policy, positive, monotonic,
        /// and capped for every failure count, including u32::MAX.
        #[test]
        fn delay_is_positive_monotonic_and_capped(attempt in proptest::num::u32::ANY) {
            let delay = delay_for_attempt(attempt);
            proptest::prop_assert!(delay >= BASE_DELAY);
            proptest::prop_assert!(delay <= CAP_DELAY);
            proptest::prop_assert!(delay <= delay_for_attempt(attempt.saturating_add(1)));
        }
    }

    /// Oracle: real loopback bind succeeds immediately without retries.
    #[test]
    fn ready_address_binds_immediately() {
        let listener = tcp_bind_with_retry("bind-guard-test", "127.0.0.1:0").unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }

    /// Oracle: virtual sleeps sum exactly to the budget, including
    /// a clipped final delay; the original error survives exhaustion.
    #[test]
    fn sleep_is_clipped_to_remaining_budget() {
        let (sleeps, sleep, now) = fake_clock();
        let mut calls = 0;
        let err = retry_with_budget(
            Duration::from_secs(5),
            || {
                calls += 1;
                Err::<(), _>(std::io::Error::from_raw_os_error(libc::EADDRNOTAVAIL))
            },
            is_retryable_bind,
            sleep,
            now,
        )
        .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EADDRNOTAVAIL));
        assert_eq!(calls, 4);
        assert_eq!(*sleeps.borrow(), [1, 2, 2].map(Duration::from_secs));
    }

    /// A fake clock shared between the sleep recorder and the `now`
    /// probe: both closures hold a clone, so no double-mutable-borrow.
    /// Returns the recorded sleeps, the sleep fn, and the now fn.
    fn fake_clock() -> (
        Rc<std::cell::RefCell<Vec<Duration>>>,
        impl FnMut(Duration),
        impl Fn() -> Duration,
    ) {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let sleep_clock = Rc::clone(&clock);
        let now_clock = Rc::clone(&clock);
        let sleeps = Rc::new(std::cell::RefCell::new(Vec::new()));
        let sleep_sleeps = Rc::clone(&sleeps);
        (
            sleeps,
            move |d: Duration| {
                sleep_clock.set(sleep_clock.get() + d);
                sleep_sleeps.borrow_mut().push(d);
            },
            move || now_clock.get(),
        )
    }

    /// Oracle: the Tailscale-not-ready signature retries; everything
    /// else fails fast. EADDRNOTAVAIL by kind and by raw errno 99
    /// both retry; EADDRINUSE, permission, and refused never do.
    #[test]
    fn retryable_class_is_only_eaddrnotavail() {
        use std::io::ErrorKind;
        let retryable = [
            std::io::Error::new(
                ErrorKind::AddrNotAvailable,
                "Cannot assign requested address",
            ),
            std::io::Error::from_raw_os_error(libc::EADDRNOTAVAIL),
        ];
        for e in &retryable {
            assert!(is_retryable_bind(e), "must retry {e:?}");
        }
        let fatal = [
            std::io::Error::new(ErrorKind::AddrInUse, "address in use"),
            std::io::Error::from_raw_os_error(libc::EADDRINUSE),
            std::io::Error::new(ErrorKind::PermissionDenied, "permission denied"),
            std::io::Error::new(ErrorKind::ConnectionRefused, "refused"),
        ];
        for e in &fatal {
            assert!(!is_retryable_bind(e), "must fail fast {e:?}");
        }
    }

    /// Oracle: 1s doubling to a 15s cap — 1, 2, 4, 8, 15, 15, …
    #[test]
    fn backoff_doubles_from_1s_to_a_15s_cap() {
        let expected = [1, 2, 4, 8, 15, 15, 15, 15];
        for (attempt, secs) in expected.iter().enumerate() {
            assert_eq!(
                delay_for_attempt(attempt as u32),
                Duration::from_secs(*secs),
                "attempt {attempt}"
            );
        }
    }

    /// Oracle: a flapping bind (EADDRNOTAVAIL twice, then success)
    /// succeeds after exactly two sleeps with the 1s/2s schedule —
    /// the reboot shape from the issue journal, where the address
    /// appears after the brokers start.
    #[test]
    fn flapping_bind_succeeds_after_two_retries() {
        let mut calls = 0;
        let (sleeps, sleep, now) = fake_clock();
        let res = retry_with_budget(
            Duration::from_secs(600),
            || {
                calls += 1;
                if calls < 3 {
                    Err(std::io::Error::from_raw_os_error(libc::EADDRNOTAVAIL))
                } else {
                    Ok("listening")
                }
            },
            is_retryable_bind,
            sleep,
            now,
        );
        assert_eq!(res.unwrap(), "listening");
        assert_eq!(calls, 3);
        assert_eq!(
            *sleeps.borrow(),
            vec![Duration::from_secs(1), Duration::from_secs(2)]
        );
    }

    /// Oracle: a permanently-broken bind (EADDRINUSE — a real
    /// misconfiguration) fails fast with exactly one attempt and no
    /// sleep; the guard must never spin on config errors.
    #[test]
    fn fatal_bind_error_fails_fast_with_one_attempt() {
        let mut calls = 0;
        let (_, mut sleep, now) = fake_clock();
        let mut slept = false;
        let mut sleep_or_fail = |d: Duration| {
            slept = true;
            sleep(d);
        };
        let res = retry_with_budget(
            Duration::from_secs(600),
            || {
                calls += 1;
                Err::<(), _>(std::io::Error::from_raw_os_error(libc::EADDRINUSE))
            },
            is_retryable_bind,
            &mut sleep_or_fail,
            now,
        );
        assert!(res.is_err());
        assert_eq!(calls, 1);
        assert!(!slept, "must not sleep on fatal errors");
    }

    /// Oracle: an address that never appears exhausts the budget and
    /// returns the last error — bounded, never infinite. A 3s budget
    /// with the 1s/2s schedule allows exactly 3 attempts (t=0, 1, 3;
    /// the t=6 retry would exceed the budget).
    #[test]
    fn retryable_error_past_budget_returns_the_last_error() {
        let mut calls = 0;
        let (sleeps, sleep, now) = fake_clock();
        let res = retry_with_budget(
            Duration::from_secs(3),
            || {
                calls += 1;
                Err::<(), _>(std::io::Error::from_raw_os_error(libc::EADDRNOTAVAIL))
            },
            is_retryable_bind,
            sleep,
            now,
        );
        assert!(res.is_err());
        assert_eq!(calls, 3, "attempts at t=0,1,3 within a 3s budget");
        assert_eq!(
            *sleeps.borrow(),
            vec![Duration::from_secs(1), Duration::from_secs(2)]
        );
    }
}
