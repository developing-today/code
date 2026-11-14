//! The shared retry policy for the PostgREST statement-timeout
//! error class (#6450 posture, enacted on Drew's 2026-11-06 ruling:
//! "formalized client backoff on 57014", pass-2 proposal item 3).
//!
//! One policy, used by both ends of the queue path so lanes stop
//! improvising their own retry shapes:
//! - the queue-broker's PostgREST caller (queue_broker::State), and
//! - the repo-queue client's broker caller (client::call).
//!
//! Retryable class: the PostgreSQL statement-timeout code `57014`
//! ("canceling statement due to statement timeout") and PostgREST's
//! schema-cache code `PGRST002`. Both mean the statement never
//! committed — a 57014 is Postgres *cancelling* the statement, so a
//! timed-out write has rolled back and retrying it cannot
//! double-apply (the posture's verify-before-retry rule is satisfied
//! by that guarantee; enqueue dedupe keys are a second belt).
//! Everything else (validation, auth, no-row) fails immediately.
//!
//! Shape: exponential backoff with full jitter.
//!   base 500 ms, multiplier 2, cap 20 s, 6 attempts total
//!   nominal delays: 0.5s, 1s, 2s, 4s, 8s (the 6th attempt is final)
//! Full jitter draws each actual delay uniformly from [0, nominal],
//! which desynchronizes the 20+ lanes that hit a window together.

use std::time::Duration;

pub const BASE_MS: u64 = 500;
pub const MULTIPLIER: f64 = 2.0;
pub const CAP_MS: u64 = 20_000;
/// Total tries, including the first one.
pub const MAX_ATTEMPTS: u32 = 6;

/// True when an error text belongs to the statement-timeout class:
/// its PostgREST `code` field is exactly the Postgres `57014` code or
/// PostgREST's `PGRST002` (#6616).
///
/// The decision reads the code field ONLY — never a substring of the
/// whole text. Permanent RPC errors interpolate the task ID into their
/// message (`fail_repo_task: task % is not claimed by %`), so a bare
/// `contains("57014")` misclassifies any permanent failure on a task
/// whose ID contains those digits (57014, 157014, 570149, …) as a
/// retryable statement timeout, stepping outside the no-double-apply
/// argument above, which holds only when the error *actually is* a
/// statement cancellation.
pub fn is_retryable(err_text: &str) -> bool {
    matches!(postgrest_code(err_text), Some("57014" | "PGRST002"))
}

/// Extract the value of the first well-formed `"code"` field in a
/// PostgREST error body embedded in `err_text`: `"code":"<value>"` or
/// `"code": "<value>"` (whitespace around the colon tolerated).
/// Occurrences of the digits in message/detail/hint prose are not a
/// code field and never match; a malformed candidate (no colon, no
/// quoted value) is skipped in favour of a later well-formed one.
fn postgrest_code(err_text: &str) -> Option<&str> {
    let mut rest = err_text;
    while let Some(pos) = rest.find("\"code\"") {
        let after_key = &rest[pos + "\"code\"".len()..];
        let value = after_key
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('"'))
            .and_then(|v| v.split('"').next());
        if let Some(value) = value {
            return Some(value);
        }
        rest = after_key;
    }
    None
}

/// The nominal (pre-jitter) delay after `failed_attempt` failures
/// (0-based: the delay before the second try is delay(0)).
pub fn nominal_delay(failed_attempt: u32) -> Duration {
    let ms = (BASE_MS as f64 * MULTIPLIER.powi(failed_attempt as i32)).min(CAP_MS as f64);
    Duration::from_millis(ms as u64)
}

/// The actual delay: full jitter, a uniform draw from [0, nominal].
/// `unit` is the draw in [0, 1] — a parameter so tests are exact.
pub fn delay(failed_attempt: u32, unit: f64) -> Duration {
    let nominal = nominal_delay(failed_attempt).as_millis() as f64;
    Duration::from_millis((nominal * unit.clamp(0.0, 1.0)) as u64)
}

/// A cheap entropy draw in [0, 1) for jitter: xorshift over the
/// current time and pid. Jitter needs decorrelation, not crypto.
fn jitter_unit() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    let mut x = nanos ^ (u64::from(std::process::id()) << 32) ^ 0xa076_1d64_78bd_642f;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let bits = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) & ((1u64 << 53) - 1);
    bits as f64 / (1u64 << 53) as f64
}

/// Sleep for the policy delay after `failed_attempt` failures and
/// return the duration slept (callers log it).
pub fn sleep_after_failure(failed_attempt: u32) -> Duration {
    let d = delay(failed_attempt, jitter_unit());
    std::thread::sleep(d);
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn retryable_class_matches_only_the_timeout_codes() {
        assert!(is_retryable(
            "HTTP 500 from PostgREST: {\"code\":\"57014\",\"message\":\"canceling statement due to statement timeout\"}"
        ));
        assert!(is_retryable(
            "HTTP 503 from PostgREST: {\"code\":\"PGRST002\",\"message\":\"Could not query the database for the schema cache\"}"
        ));
        assert!(!is_retryable(
            "HTTP 400 from PostgREST: {\"code\":\"PGRST102\"}"
        ));
        assert!(!is_retryable("packet validation failed (1 error(s))"));
        assert!(!is_retryable("claim_repo_task: no matching claimed row"));
    }

    #[test]
    fn permanent_errors_naming_task_ids_with_timeout_digits_are_not_retryable() {
        // #6616 regression: the RPCs' permanent errors interpolate the
        // task ID into the message; IDs containing 57014 must not be
        // mistaken for the statement-timeout code.
        for task_id in ["57014", "157014", "570149", "570140", "257014"] {
            let err = format!(
                "HTTP 400 from PostgREST: {{\"code\":\"P0001\",\"message\":\"fail_repo_task: task {task_id} is not claimed by repo-worker-code\"}}"
            );
            assert!(!is_retryable(&err), "task {task_id} misclassified");
        }
    }

    #[test]
    fn timeout_digits_in_detail_or_hint_prose_are_not_retryable() {
        assert!(!is_retryable(
            "HTTP 400 from PostgREST: {\"code\":\"P0001\",\"message\":\"boom\",\"detail\":\"statement 57014 seen earlier\",\"hint\":\"PGRST002\"}"
        ));
        assert!(!is_retryable("plain prose mentioning 57014 and PGRST002"));
    }

    #[test]
    fn code_field_with_space_after_colon_is_retryable() {
        assert!(is_retryable(
            "HTTP 500 from PostgREST: {\"code\": \"57014\", \"message\": \"canceling statement due to statement timeout\"}"
        ));
        assert!(is_retryable(
            "HTTP 503 from PostgREST: {\"code\": \"PGRST002\"}"
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// PBT (#6616, testing doctrine): retryability depends ONLY on
        /// exact equality of the generated top-level `code` value with
        /// `57014`/`PGRST002` — the generated code is the oracle.
        /// Message/detail/hint prose (which may embed the timeout
        /// digits, e.g. an interpolated task ID), field ordering, and
        /// whitespace around the colon must never change the outcome.
        #[test]
        fn retryable_depends_only_on_exact_code_field(
            code in prop_oneof![
                Just("57014".to_string()), Just("PGRST002".to_string()),
                Just("P0001".to_string()), Just("PGRST102".to_string()),
                Just("5701".to_string()), Just("570140".to_string()),
                Just("057014".to_string()), Just("pgrst002".to_string()),
                "[A-Z0-9]{1,8}",
            ],
            prose in prop::collection::vec(prop_oneof![
                Just("task".to_string()), Just("57014".to_string()),
                Just("PGRST002".to_string()), Just("157014".to_string()),
                Just("570149".to_string()), Just("failed".to_string()),
                "[a-z]{1,6}",
            ], 0..6),
            sp_before in "[ ]{0,2}",
            sp_after in "[ ]{0,2}",
            code_pos in 0usize..4,
        ) {
            let prose = prose.join(" ");
            let mut fields = vec![
                format!("\"message\":\"{prose}\""),
                format!("\"detail\":\"{prose}\""),
                format!("\"hint\":\"{prose}\""),
            ];
            fields.insert(
                code_pos.min(fields.len()),
                format!("\"code\"{sp_before}:{sp_after}\"{code}\""),
            );
            let err = format!(
                "HTTP 400 from PostgREST: {{{}}}",
                fields.join(",")
            );
            let oracle = code == "57014" || code == "PGRST002";
            prop_assert_eq!(is_retryable(&err), oracle, "err={}", err);
        }
    }

    #[test]
    fn nominal_delay_doubles_to_the_cap() {
        assert_eq!(nominal_delay(0), Duration::from_millis(500));
        assert_eq!(nominal_delay(1), Duration::from_millis(1_000));
        assert_eq!(nominal_delay(2), Duration::from_millis(2_000));
        assert_eq!(nominal_delay(3), Duration::from_millis(4_000));
        assert_eq!(nominal_delay(4), Duration::from_millis(8_000));
        assert_eq!(nominal_delay(5), Duration::from_millis(16_000));
        assert_eq!(nominal_delay(6), Duration::from_millis(20_000));
        assert_eq!(nominal_delay(30), Duration::from_millis(20_000));
    }

    #[test]
    fn jittered_delay_never_exceeds_nominal() {
        for attempt in 0..8 {
            assert!(delay(attempt, 0.0) <= nominal_delay(attempt));
            assert!(delay(attempt, 0.5) <= nominal_delay(attempt));
            assert_eq!(delay(attempt, 1.0), nominal_delay(attempt));
            assert_eq!(delay(attempt, 0.0), Duration::ZERO);
        }
    }
}
