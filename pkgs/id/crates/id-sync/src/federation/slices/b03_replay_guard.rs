//! B03 replay guard — contract module (slice B03, #10206).
//!
//! Replay/freshness rules for re-presented signed data. The contract,
//! in one paragraph: a signed event is admitted at most once. Its
//! `event_id` identifies the complete canonical signed envelope
//! (including the signature), so the same ID presented again is a
//! `Replay` — never a second admission — while a different ID carrying
//! an identical payload digest is a different assertion and is
//! accepted as new. Admission is also bounded in time: an event whose
//! `issued_at` lies further in the past than the freshness window's
//! `max_age`, or further in the future than `max_future_skew`, is
//! rejected (`Expired` / `NotYetValid`); the past check runs before the
//! replay check, so a late re-presentation is answered by freshness
//! and can never be mistaken for new data. The retained replay ledger
//! is finite: entries are pruned once their event can no longer be
//! presented inside its own window, and when the configured capacity is
//! reached new admissions are a typed `LedgerSaturated` rejection
//! rather than an eviction of a live entry — the guard never falls back
//! to replay-permissive ingest (the slice's rollback posture).
//!
//! Times are logical seconds supplied by the caller (the module never
//! reads a clock), which keeps the state machine deterministic and
//! independently testable. This module is deliberately self-contained:
//! the federation roots record the slice path without declaring it
//! (slice 9110-02), so the slice's integration test includes this file
//! directly and no shared root names it yet; #9110-09 wires versioned
//! public exports after module outputs land.

use std::collections::BTreeMap;
use std::fmt;

/// Logical seconds, supplied by the caller.
pub type LogicalTime = u64;

/// The stable identity of a signed event: the digest of the complete
/// canonical signed envelope, signature included. Two presentations
/// bearing the same ID are the same assertion.
pub type EventId = [u8; 32];

/// The digest of the event's payload bytes. It identifies content, not
/// an assertion: two independently signed events may carry the same
/// payload digest, and they are not replays of one another.
pub type PayloadId = [u8; 32];

/// The freshness bounds for admission, in logical seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessWindow {
    /// How old an event may be and still be admitted: it stays fresh
    /// while `now <= issued_at + max_age` (inclusive).
    pub max_age: LogicalTime,
    /// How far ahead of `now` an `issued_at` may be, tolerating
    /// bounded clock disagreement (inclusive).
    pub max_future_skew: LogicalTime,
}

impl FreshnessWindow {
    /// The last logical second at which an event issued at
    /// `issued_at` is still fresh (inclusive).
    pub fn fresh_until(&self, issued_at: LogicalTime) -> LogicalTime {
        issued_at.saturating_add(self.max_age)
    }
}

/// One presentation of a signed event to the admission gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedEvent {
    pub event_id: EventId,
    pub payload_digest: PayloadId,
    pub issued_at: LogicalTime,
}

/// The immutable fields an `event_id` binds. Comparing them lets the
/// guard distinguish a replay of the same envelope from a changed
/// binding under a reused ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub payload_digest: PayloadId,
    pub issued_at: LogicalTime,
}

impl PresentedEvent {
    /// The binding this event claims; the event ID must be the digest
    /// of these fields together with all other envelope fields.
    pub fn binding(&self) -> Binding {
        Binding {
            payload_digest: self.payload_digest,
            issued_at: self.issued_at,
        }
    }
}

/// Why an admission was rejected. Each variant is a typed, auditable
/// outcome; rejection is never a silent drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// The new profile reserves the all-zero event ID; a presentation
    /// carrying it is structurally malformed.
    MalformedEnvelope { event_id: EventId },
    /// `issued_at` is further ahead of logical now than the window's
    /// `max_future_skew` allows.
    NotYetValid {
        issued_at: LogicalTime,
        now: LogicalTime,
        max_future_skew: LogicalTime,
    },
    /// The event is older than the window's `max_age`; it is not
    /// fresh, regardless of whether its ID was seen before.
    Expired {
        issued_at: LogicalTime,
        now: LogicalTime,
        max_age: LogicalTime,
    },
    /// The same signed event ID was already admitted. `first_seen_at`
    /// is the `issued_at` recorded at that admission.
    Replay {
        event_id: EventId,
        first_seen_at: LogicalTime,
    },
    /// The event ID was already admitted, but the presented binding
    /// differs: the ID no longer names the same signed envelope.
    BindingConflict {
        event_id: EventId,
        first_seen_at: LogicalTime,
    },
    /// The retained replay ledger is full. New admissions are refused
    /// rather than evicting a live entry, which would re-open replay.
    LedgerSaturated { capacity: usize },
    /// The caller's logical clock moved backwards: `now` is below the
    /// highest time previously presented to `admit`. Fail-closed: with
    /// a regressed clock, an event ID pruned as expired could otherwise
    /// be admitted a second time, breaking the at-most-once contract.
    ClockRegressed {
        highest_now: LogicalTime,
        now: LogicalTime,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedEnvelope { .. } => write!(f, "malformed envelope: null event ID"),
            Self::NotYetValid {
                issued_at,
                now,
                max_future_skew,
            } => write!(
                f,
                "event issued at {issued_at} is too far ahead of now {now} (skew {max_future_skew})"
            ),
            Self::Expired {
                issued_at,
                now,
                max_age,
            } => write!(
                f,
                "event issued at {issued_at} is expired at {now} (max age {max_age})"
            ),
            Self::Replay { first_seen_at, .. } => {
                write!(f, "replayed signed event, first seen at {first_seen_at}")
            }
            Self::BindingConflict { first_seen_at, .. } => write!(
                f,
                "event ID reused with a changed binding, first seen at {first_seen_at}"
            ),
            Self::LedgerSaturated { capacity } => {
                write!(f, "replay ledger saturated at {capacity} entries")
            }
            Self::ClockRegressed { highest_now, now } => write!(
                f,
                "logical clock regressed: now {now} is below the highest seen {highest_now}"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// A successful, first admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub event_id: EventId,
    pub payload_digest: PayloadId,
    pub issued_at: LogicalTime,
    /// Last logical second at which this event is still fresh.
    pub fresh_until: LogicalTime,
}

/// The replay guard: freshness bounds plus a finite ledger of admitted
/// event IDs and their bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayGuard {
    window: FreshnessWindow,
    capacity: usize,
    ledger: BTreeMap<EventId, Binding>,
    /// Highest logical time ever presented to `admit`. Pruning is only
    /// sound while the caller's clock is non-decreasing, so `admit`
    /// rejects a regressed `now` (see `ReplayError::ClockRegressed`).
    highest_now: Option<LogicalTime>,
}

impl ReplayGuard {
    /// A guard admitting events under `window`, retaining at most
    /// `capacity` live event IDs. Entries leave the ledger only when
    /// their event can no longer be fresh, so capacity must exceed the
    /// number of events admitted within one freshness window to keep
    /// accepting; reaching it is a fail-closed rejection.
    pub fn new(window: FreshnessWindow, capacity: usize) -> Self {
        Self {
            window,
            capacity,
            ledger: BTreeMap::new(),
            highest_now: None,
        }
    }

    /// The freshness bounds this guard enforces.
    pub fn window(&self) -> FreshnessWindow {
        self.window
    }

    /// Number of event IDs currently retained (live within their
    /// freshness window).
    pub fn ledger_len(&self) -> usize {
        self.ledger.len()
    }

    /// Present one signed event for admission at logical time `now`.
    ///
    /// Ordering is part of the contract: structural validity, then
    /// freshness, then replay/binding, then capacity. Checking
    /// freshness first means a late re-presentation is `Expired`, never
    /// admitted merely because its ledger entry had aged out. The
    /// caller's clock must be non-decreasing across calls: a `now`
    /// below the highest previously seen is `ClockRegressed`.
    pub fn admit(
        &mut self,
        event: PresentedEvent,
        now: LogicalTime,
    ) -> Result<Accepted, ReplayError> {
        if event.event_id == [0u8; 32] {
            return Err(ReplayError::MalformedEnvelope {
                event_id: event.event_id,
            });
        }

        // Fail closed on a regressed clock before any state changes:
        // pruning at a backwards `now` could re-open admission for an
        // ID that already aged out at a later time.
        if let Some(highest_now) = self.highest_now {
            if now < highest_now {
                return Err(ReplayError::ClockRegressed { highest_now, now });
            }
        }
        self.highest_now = Some(now);

        // Keep only IDs that could still be presented as fresh.
        self.prune(now);

        if event.issued_at > now.saturating_add(self.window.max_future_skew) {
            return Err(ReplayError::NotYetValid {
                issued_at: event.issued_at,
                now,
                max_future_skew: self.window.max_future_skew,
            });
        }
        if now > self.window.fresh_until(event.issued_at) {
            return Err(ReplayError::Expired {
                issued_at: event.issued_at,
                now,
                max_age: self.window.max_age,
            });
        }

        if let Some(prior) = self.ledger.get(&event.event_id) {
            let first_seen_at = prior.issued_at;
            if *prior != event.binding() {
                return Err(ReplayError::BindingConflict {
                    event_id: event.event_id,
                    first_seen_at,
                });
            }
            return Err(ReplayError::Replay {
                event_id: event.event_id,
                first_seen_at,
            });
        }

        if self.ledger.len() >= self.capacity {
            return Err(ReplayError::LedgerSaturated {
                capacity: self.capacity,
            });
        }

        self.ledger.insert(event.event_id, event.binding());
        Ok(Accepted {
            event_id: event.event_id,
            payload_digest: event.payload_digest,
            issued_at: event.issued_at,
            fresh_until: self.window.fresh_until(event.issued_at),
        })
    }

    /// Drop entries whose event can no longer be fresh at `now`. A
    /// pruned ID cannot be replayed into acceptance, because the
    /// freshness check above answers any later presentation — and
    /// `admit` rejects a regressed clock, so `now` never moves
    /// backwards across calls.
    fn prune(&mut self, now: LogicalTime) {
        let window = self.window;
        self.ledger
            .retain(|_, binding| now <= window.fresh_until(binding.issued_at));
    }
}
