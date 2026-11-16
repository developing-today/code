//! B03 replay guard — integration test (slice B03, #10206).
//!
//! Oracle (invariant, from the issue): the same signed event ID
//! re-presented is rejected as replay; a new event ID carrying an
//! identical payload is accepted as new, never conflated with replay.
//! Freshness bounds admission: an event older than its window is
//! `Expired`, never replay-permissively admitted.
//!
//! Independence: expected outcomes come from `RefGuard`, a reference
//! state machine written from the settled design text (versioned
//! design, *Errors / state outcomes*; #10206) rather than from the
//! module under test. It stores bindings in a plain map and decides
//! with its own predicates; the named cases assert both the reference
//! decision and the implementation result, and a generated-sequence
//! check (model-based testing) compares the two over random
//! presentation traces. No signature or digest value here is produced
//! by the implementation under test.
//!
//! The federation roots deliberately do not declare slice modules
//! (slice 9110-02), so this test includes the slice file by path;
//! #9110-09 wires versioned public exports later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b03_replay_guard.rs"]
mod b03_replay_guard;

use std::collections::HashMap;

use b03_replay_guard::{
    Accepted, EventId, FreshnessWindow, PresentedEvent, ReplayError, ReplayGuard,
};
use proptest::prelude::*;

const MAX_AGE: u64 = 600;
const SKEW: u64 = 30;

fn event_id(tag: u8) -> EventId {
    let mut id = [0u8; 32];
    id[0] = tag;
    id[31] = 0x5a;
    id
}

fn payload(tag: u8) -> [u8; 32] {
    let mut p = [0u8; 32];
    p[0] = tag;
    p[31] = 0xa5;
    p
}

fn event(tag: u8, payload_tag: u8, issued_at: u64) -> PresentedEvent {
    PresentedEvent {
        event_id: event_id(tag),
        payload_digest: payload(payload_tag),
        issued_at,
    }
}

// ---------------------------------------------------------------------
// Independent reference state machine (the oracle).
// ---------------------------------------------------------------------

/// What the design says should happen to one presentation. Written by
/// hand from the spec, not derived from `ReplayError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefOutcome {
    Accepted,
    Malformed,
    NotYetValid,
    Expired,
    Replay,
    BindingConflict,
    AtCapacity,
}

/// Reference admission state: bindings keyed by event ID, with the
/// freshness retained only while the entry can still be presented
/// inside its own window.
struct RefGuard {
    window: FreshnessWindow,
    capacity: usize,
    admitted: HashMap<EventId, (u8, u64)>,
}

impl RefGuard {
    fn new(window: FreshnessWindow, capacity: usize) -> Self {
        Self {
            window,
            capacity,
            admitted: HashMap::new(),
        }
    }

    fn decide(&mut self, ev: &PresentedEvent, now: u64, payload_tag: u8) -> RefOutcome {
        // Null IDs are structurally malformed in the new profile.
        if ev.event_id == [0u8; 32] {
            return RefOutcome::Malformed;
        }
        // Drop entries that can no longer be presented as fresh; they
        // would be Expired anyway, and expiry, not replay, answers a
        // late presentation.
        let window = self.window;
        self.admitted
            .retain(|_, (_, issued_at)| now <= issued_at.saturating_add(window.max_age));
        if ev.issued_at > now.saturating_add(self.window.max_future_skew) {
            return RefOutcome::NotYetValid;
        }
        if now > ev.issued_at.saturating_add(self.window.max_age) {
            return RefOutcome::Expired;
        }
        if let Some(&(seen_payload, seen_issued_at)) = self.admitted.get(&ev.event_id) {
            if seen_payload != payload_tag || seen_issued_at != ev.issued_at {
                return RefOutcome::BindingConflict;
            }
            return RefOutcome::Replay;
        }
        if self.admitted.len() >= self.capacity {
            return RefOutcome::AtCapacity;
        }
        self.admitted
            .insert(ev.event_id, (payload_tag, ev.issued_at));
        RefOutcome::Accepted
    }
}

/// Classify the implementation's result into the same outcome space.
/// This only maps variants; every expected value comes from `RefGuard`.
fn classify(result: Result<Accepted, ReplayError>) -> RefOutcome {
    match result {
        Ok(_) => RefOutcome::Accepted,
        Err(ReplayError::MalformedEnvelope { .. }) => RefOutcome::Malformed,
        Err(ReplayError::NotYetValid { .. }) => RefOutcome::NotYetValid,
        Err(ReplayError::Expired { .. }) => RefOutcome::Expired,
        Err(ReplayError::Replay { .. }) => RefOutcome::Replay,
        Err(ReplayError::BindingConflict { .. }) => RefOutcome::BindingConflict,
        Err(ReplayError::LedgerSaturated { .. }) => RefOutcome::AtCapacity,
    }
}

fn window() -> FreshnessWindow {
    FreshnessWindow {
        max_age: MAX_AGE,
        max_future_skew: SKEW,
    }
}

// ---------------------------------------------------------------------
// Named invariant cases.
// ---------------------------------------------------------------------

#[test]
fn same_event_id_replayed_is_rejected_as_replay() {
    let mut guard = ReplayGuard::new(window(), 16);
    let mut reference = RefGuard::new(window(), 16);
    let ev = event(7, 1, 1_000);

    assert_eq!(guard.window(), window());
    assert_eq!(reference.decide(&ev, 1_000, 1), RefOutcome::Accepted);
    assert_eq!(classify(guard.admit(ev, 1_000)), RefOutcome::Accepted);

    // The very same signed event ID, identical bytes, presented again:
    // replay, with the original admission time as evidence.
    assert_eq!(reference.decide(&ev, 1_000, 1), RefOutcome::Replay);
    assert_eq!(
        guard.admit(ev, 1_000),
        Err(ReplayError::Replay {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );

    // Still replay later inside the freshness window.
    assert_eq!(reference.decide(&ev, 1_500, 1), RefOutcome::Replay);
    assert_eq!(
        guard.admit(ev, 1_500),
        Err(ReplayError::Replay {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );

    // Rejection never perturbs the ledger: one ID, one entry.
    assert_eq!(guard.ledger_len(), 1);
}

#[test]
fn new_event_id_with_identical_payload_is_accepted_as_new() {
    let mut guard = ReplayGuard::new(window(), 16);
    let mut reference = RefGuard::new(window(), 16);

    // Same payload bytes and same issued_at, different event ID: two
    // distinct signed events. Dedup keys on event ID only, never on
    // the payload digest.
    let first = event(7, 1, 1_000);
    let second = event(8, 1, 1_000);

    assert_eq!(reference.decide(&first, 1_000, 1), RefOutcome::Accepted);
    assert_eq!(classify(guard.admit(first, 1_000)), RefOutcome::Accepted);
    assert_eq!(reference.decide(&second, 1_000, 1), RefOutcome::Accepted);
    assert_eq!(classify(guard.admit(second, 1_000)), RefOutcome::Accepted);

    assert_eq!(guard.ledger_len(), 2);

    // Each ID now replays independently.
    assert_eq!(
        guard.admit(first, 1_000),
        Err(ReplayError::Replay {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );
    assert_eq!(
        guard.admit(second, 1_000),
        Err(ReplayError::Replay {
            event_id: event_id(8),
            first_seen_at: 1_000,
        })
    );
}

#[test]
fn stale_event_is_expired_never_admitted_or_replayed() {
    let mut guard = ReplayGuard::new(window(), 16);
    let mut reference = RefGuard::new(window(), 16);
    let ev = event(7, 1, 1_000);

    assert_eq!(classify(guard.admit(ev, 1_000)), RefOutcome::Accepted);
    assert_eq!(reference.decide(&ev, 1_000, 1), RefOutcome::Accepted);

    // Last instant inside the window: replay, not expiry.
    assert_eq!(
        reference.decide(&ev, 1_000 + MAX_AGE, 1),
        RefOutcome::Replay
    );
    assert_eq!(
        guard.admit(ev, 1_000 + MAX_AGE),
        Err(ReplayError::Replay {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );

    // One second later the freshness rule answers, and the retained
    // replay entry is gone: the same ID is Expired — never "new".
    let late = 1_000 + MAX_AGE + 1;
    assert_eq!(reference.decide(&ev, late, 1), RefOutcome::Expired);
    assert_eq!(
        guard.admit(ev, late),
        Err(ReplayError::Expired {
            issued_at: 1_000,
            now: late,
            max_age: MAX_AGE,
        })
    );
    assert_eq!(guard.ledger_len(), 0);

    // An unseen ID carrying the same stale time is just as Expired.
    let unseen = event(9, 1, 1_000);
    assert_eq!(reference.decide(&unseen, late, 1), RefOutcome::Expired);
    assert_eq!(
        guard.admit(unseen, late),
        Err(ReplayError::Expired {
            issued_at: 1_000,
            now: late,
            max_age: MAX_AGE,
        })
    );
    assert_eq!(guard.ledger_len(), 0);
}

#[test]
fn issued_at_beyond_future_skew_is_not_yet_valid() {
    let mut guard = ReplayGuard::new(window(), 16);
    let mut reference = RefGuard::new(window(), 16);
    let ev = event(7, 1, 1_000 + SKEW + 1);

    assert_eq!(reference.decide(&ev, 1_000, 1), RefOutcome::NotYetValid);
    assert_eq!(
        guard.admit(ev, 1_000),
        Err(ReplayError::NotYetValid {
            issued_at: 1_000 + SKEW + 1,
            now: 1_000,
            max_future_skew: SKEW,
        })
    );

    // Exactly at the skew bound it is admissible.
    let at_bound = event(7, 1, 1_000 + SKEW);
    assert_eq!(reference.decide(&at_bound, 1_000, 1), RefOutcome::Accepted);
    assert_eq!(classify(guard.admit(at_bound, 1_000)), RefOutcome::Accepted);
}

#[test]
fn same_event_id_with_changed_binding_is_conflict_not_new() {
    let mut guard = ReplayGuard::new(window(), 16);
    let mut reference = RefGuard::new(window(), 16);
    let ev = event(7, 1, 1_000);

    assert_eq!(classify(guard.admit(ev, 1_000)), RefOutcome::Accepted);
    assert_eq!(reference.decide(&ev, 1_000, 1), RefOutcome::Accepted);

    // The event ID binds the whole envelope: a different payload under
    // the same ID is a binding conflict, never a fresh event.
    let changed_payload = event(7, 2, 1_000);
    assert_eq!(
        reference.decide(&changed_payload, 1_000, 2),
        RefOutcome::BindingConflict
    );
    assert_eq!(
        guard.admit(changed_payload, 1_000),
        Err(ReplayError::BindingConflict {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );

    // A rewritten issued_at under the same ID is equally a conflict.
    let changed_time = event(7, 1, 1_001);
    assert_eq!(
        reference.decide(&changed_time, 1_000, 1),
        RefOutcome::BindingConflict
    );
    assert_eq!(
        guard.admit(changed_time, 1_000),
        Err(ReplayError::BindingConflict {
            event_id: event_id(7),
            first_seen_at: 1_000,
        })
    );
}

#[test]
fn saturated_ledger_fails_closed_and_recovers_as_entries_expire() {
    let mut guard = ReplayGuard::new(window(), 2);
    let mut reference = RefGuard::new(window(), 2);

    // Fill the retained ledger (both the implementation and the model).
    for tag in 1..=2 {
        let ev = event(tag, tag, 1_000);
        assert_eq!(reference.decide(&ev, 1_000, tag), RefOutcome::Accepted);
        assert_eq!(classify(guard.admit(ev, 1_000)), RefOutcome::Accepted);
    }

    // Rollback posture: at capacity a new admission is a typed
    // rejection, never an eviction of a live entry (which would be
    // replay-permissive).
    let overflow = event(3, 3, 1_000);
    assert_eq!(
        reference.decide(&overflow, 1_000, 3),
        RefOutcome::AtCapacity
    );
    assert_eq!(
        guard.admit(overflow, 1_000),
        Err(ReplayError::LedgerSaturated { capacity: 2 })
    );
    assert_eq!(guard.ledger_len(), 2);

    // The entries already admitted still answer Replay, not Accepted.
    let first = event(1, 1, 1_000);
    assert_eq!(
        guard.admit(first, 1_000),
        Err(ReplayError::Replay {
            event_id: event_id(1),
            first_seen_at: 1_000,
        })
    );

    // Once the window closes, pruning frees capacity without ever
    // having admitted a replay.
    let after_window = 1_000 + MAX_AGE + 1;
    assert_eq!(
        reference.decide(&overflow, after_window, 3),
        RefOutcome::Expired
    );
    assert_eq!(
        guard.admit(overflow, after_window),
        Err(ReplayError::Expired {
            issued_at: 1_000,
            now: after_window,
            max_age: MAX_AGE,
        })
    );
    assert_eq!(guard.ledger_len(), 0);

    let fresh = event(3, 3, after_window);
    assert_eq!(
        reference.decide(&fresh, after_window, 3),
        RefOutcome::Accepted
    );
    assert_eq!(
        classify(guard.admit(fresh, after_window)),
        RefOutcome::Accepted
    );
}

// ---------------------------------------------------------------------
// Model-based check: generated presentation traces.
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every generated trace is decided twice — by the reference state
    /// machine and by the implementation — and the outcomes must agree.
    #[test]
    fn implementation_matches_reference_state_machine(
        steps in prop::collection::vec(
            (0u8..=5u8, 0u8..=2u8, 0u16..=200u16, 0u16..=100u16),
            1..40,
        ),
    ) {
        let window = FreshnessWindow { max_age: 150, max_future_skew: 20 };
        let mut guard = ReplayGuard::new(window, 4);
        let mut reference = RefGuard::new(window, 4);
        let mut now = 1_000u64;
        let mut trace = String::new();

        for (id_tag, payload_tag, issued_off, now_step) in steps {
            now += u64::from(now_step);
            let issued_at = 1_000 + u64::from(issued_off);
            // Tag 0 is the null event ID: structurally malformed.
            let ev = PresentedEvent {
                event_id: if id_tag == 0 { [0u8; 32] } else { event_id(id_tag) },
                payload_digest: payload(payload_tag),
                issued_at,
            };
            let expected = reference.decide(&ev, now, payload_tag);
            let actual = classify(guard.admit(ev, now));
            trace.push_str(&format!(
                "now={now} id={id_tag} payload={payload_tag} issued={issued_at} -> {expected:?}/{actual:?}\n"
            ));
            prop_assert_eq!(actual, expected, "trace:\n{}", trace);
            prop_assert_eq!(guard.ledger_len(), reference.admitted.len(), "trace:\n{}", trace);
        }
    }
}
