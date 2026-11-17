//! Admin-history byte accounting (#4633): encoded event sizes and
//! the incremental ledger over the retained history. Split out of
//! `core.rs` for the F11.10 production-line guard (#10607);
//! behaviour is unchanged.

use super::super::WorldEvent;
use std::collections::VecDeque;

/// Encoded byte size of one admin-history event, or `None` when the event
/// cannot be serialized at all. Serialization failure is deliberate in the
/// accounting below (#4633): an unserializable event counts as
/// `usize::MAX` bytes, exactly as the previous rescan-every-time helper
/// treated it, so it forces the retained history to trim rather than
/// silently occupying unaccounted space.
pub(super) fn admin_history_event_bytes(event: &WorldEvent) -> Option<usize> {
    serde_json::to_vec(event).ok().map(|bytes| bytes.len())
}

pub(super) fn admin_history_bytes<'a>(events: impl IntoIterator<Item = &'a WorldEvent>) -> usize {
    events
        .into_iter()
        .map(|event| admin_history_event_bytes(event).unwrap_or(usize::MAX))
        .fold(0usize, usize::saturating_add)
}

/// Incrementally maintained byte accounting for the retained admin history
/// (#4633). `record_admin_history` used to re-serialize every retained
/// event on every accepted event; this ledger caches each event's encoded
/// size once, when the event is recorded (or once during restore, where a
/// one-shot scan is acceptable), and maintains the running total as events
/// are pushed and trimmed, so recording is amortized O(1) in the retained
/// history size.
///
/// Invariant: `sizes` holds one entry per retained admin-history event, in
/// the same order as `WorldCore::admin_history`. A `None` entry is an event
/// that failed to serialize; while any `None` is retained,
/// [`AdminHistoryByteLedger::effective_bytes`] is `usize::MAX`, matching
/// the legacy rescan semantics.
#[derive(Debug, Default)]
pub(crate) struct AdminHistoryByteLedger {
    sizes: VecDeque<Option<usize>>,
    serializable_bytes: usize,
    unserializable: usize,
}

impl AdminHistoryByteLedger {
    /// Start tracking one newly retained event of the given encoded size.
    pub(crate) fn track(&mut self, size: Option<usize>) {
        self.sizes.push_back(size);
        match size {
            Some(bytes) => {
                self.serializable_bytes = self.serializable_bytes.saturating_add(bytes);
            }
            None => {
                self.unserializable = self.unserializable.saturating_add(1);
            }
        }
    }

    /// Stop tracking the oldest retained event (it was trimmed).
    pub(crate) fn untrack_front(&mut self) {
        if let Some(size) = self.sizes.pop_front() {
            match size {
                Some(bytes) => {
                    self.serializable_bytes = self.serializable_bytes.saturating_sub(bytes);
                }
                None => {
                    self.unserializable = self.unserializable.saturating_sub(1);
                }
            }
        }
    }

    /// The byte total the trim loop compares against the cap: the running
    /// total, or `usize::MAX` while an unserializable event is retained.
    pub(crate) fn effective_bytes(&self) -> usize {
        if self.unserializable > 0 {
            usize::MAX
        } else {
            self.serializable_bytes
        }
    }

    /// Number of events currently tracked; equals the retained history len.
    pub(crate) fn tracked(&self) -> usize {
        self.sizes.len()
    }

    /// Rebuild the ledger for a restored history with a one-shot scan.
    pub(crate) fn rebuild<'a>(events: impl IntoIterator<Item = &'a WorldEvent>) -> Self {
        let mut ledger = Self::default();
        for event in events {
            ledger.track(admin_history_event_bytes(event));
        }
        ledger
    }
}
