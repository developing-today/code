//! Host-event delivery for the world actor: the delivery outcome,
//! the participant/host delivery paths, fuel-refusal retries, and
//! restored-form invalidation.
//! Extracted from `execution.rs` for the production-line guard
//! (#16255 slice 8): moves only, no behaviour change.

use super::*;

/// What one host-event delivery did (#1067). A bare `bool` conflates the
/// recoverable case with every other drop, which is exactly how a
/// fuel-refused capability result used to be lost: only `FuelRefused`
/// guarantees the program never ran, so only it may be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::world::actor) enum DeliveryOutcome {
    /// The program ran the event and it was journaled.
    Delivered,
    /// The account's fuel-rate window is exhausted: the program never
    /// ran and stays healthy, so the event may be retried once the
    /// window rolls.
    FuelRefused,
    /// Any other drop: the program is unhealthy, storage failed, the
    /// event was too large, or the program rejected it.
    Failed,
}

impl DeliveryOutcome {
    /// Whether the event reached the program and was journaled.
    pub(in crate::world::actor) const fn delivered(self) -> bool {
        matches!(self, Self::Delivered)
    }
}

impl WorldActor {
    /// Run a participant-input update and its record projection with the
    /// authenticated caller account installed only for these guest calls.
    pub(in crate::world::actor) fn update_for_caller(
        &mut self,
        event: &WorldEvent,
    ) -> Result<Option<Records>> {
        let account = self.acting_account.clone();
        let previous_account = self.program.replace_caller_account(account.as_deref());
        let result = self
            .program
            .update(event)
            .and_then(|()| program_records(self.program.as_mut()));
        self.program
            .replace_caller_account(previous_account.as_deref());
        result
    }

    /// Deliver a host-originated event to the program: journal it first is
    /// impossible (the program must accept it for the journal to be true), so
    /// the order is update, then journal, exactly like participant input.
    /// A read-only replica never delivers: host events are authoritative
    /// program effects and must not originate from a replica.
    pub(in crate::world::actor) async fn deliver(&mut self, text: String) -> DeliveryOutcome {
        self.deliver_system(text, None).await
    }

    /// Deliver a form response with the submitter's account scoped to guest
    /// update/records calls, restoring the previous context on every outcome.
    /// Durably spend their use budget only after acceptance and journaling.
    pub(in crate::world::actor) async fn deliver_form_result(
        &mut self,
        text: String,
        participant_id: Option<u64>,
    ) -> DeliveryOutcome {
        let account = self.acting_account.clone();
        let previous_account = self.program.replace_caller_account(account.as_deref());
        let outcome = self.deliver_system(text, participant_id).await;
        self.program
            .replace_caller_account(previous_account.as_deref());
        outcome
    }

    pub(in crate::world::actor) async fn deliver_system(
        &mut self,
        text: String,
        participant_id: Option<u64>,
    ) -> DeliveryOutcome {
        if self.read_only {
            return DeliveryOutcome::Failed;
        }
        if !self.program_healthy || self.storage_failed {
            return DeliveryOutcome::Failed;
        }
        if text.len() > crate::world_caps::MAX_EVENT_BYTES {
            tracing::error!("world event exceeds the delivery bound; dropped");
            return DeliveryOutcome::Failed;
        }
        let event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: text },
        };
        if let Err(error) = self.program.update(&event) {
            if super::super::is_fuel_rate_refusal(&error) {
                // Recoverable admission refusal (#1067): the program
                // never ran and stays healthy; this host event is
                // dropped and the next tick or capability result is
                // delivered as usual once the window rolls.
                tracing::warn!("world program refused a host event at its fuel rate: {error:#}");
                return DeliveryOutcome::FuelRefused;
            }
            self.program_healthy = false;
            tracing::error!("world program rejected a host event: {error:#}");
            return DeliveryOutcome::Failed;
        }
        match program_records(self.program.as_mut()) {
            Ok(records) => {
                let records = records.unwrap_or_default();
                publish_records(&self.records_sender, self.core.sequence, records);
            }
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed: {error:#}");
                return DeliveryOutcome::Failed;
            }
        }
        let journaled = self
            .persist(JournalEntry::System {
                event: event.clone(),
            })
            .is_ok();
        if journaled {
            if let Some(participant_id) = participant_id {
                self.core.spend(participant_id);
            }
            self.core.record_admin_history(event);
            self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        }
        let _ = self.view_changed.send(self.core.sequence);
        if journaled {
            DeliveryOutcome::Delivered
        } else {
            DeliveryOutcome::Failed
        }
    }

    /// Record that a delivery was refused at the account's fuel rate
    /// (#1067): no refused delivery is retried before one full window
    /// past now, which is guaranteed to land in a fresh budget window
    /// — the refused window started no later than the refusal. The
    /// actor clock's instant face shares the wall clock's domain
    /// (`SystemClock::now_instant` is `Instant::now()`; a manual clock
    /// is anchored to it at construction), so this bound holds on
    /// either clock.
    pub(super) fn note_fuel_refusal(&mut self) {
        self.fuel_retry_not_before =
            Some(self.core.clock().now_instant() + id_caps::account_quota::FUEL_RATE_WINDOW);
    }

    /// Deliver a capability result, or queue it when the account's fuel
    /// rate refuses it (#1067, Codex review on #12557): the side effect
    /// already ran, so the result event must not be re-executed — it
    /// waits in `pending_results` until the window rolls, when
    /// [`Self::retry_pending_results`] delivers it. Any other failure
    /// drops the event, as a host event is dropped today: only a fuel
    /// refusal guarantees the program never ran.
    pub(super) async fn deliver_result(&mut self, event: String) {
        if let DeliveryOutcome::FuelRefused = self.deliver(event.clone()).await {
            self.note_fuel_refusal();
            self.pending_results.push_back(event);
        }
    }

    /// Retry queued capability results once the fuel-rate window has
    /// rolled (#1067). Stops at the first refusal — the rest stay
    /// queued behind it, in order — and drops a result any other
    /// failure discards. A result is delivered at most once: the
    /// program never ran on a refused attempt, so a retry cannot
    /// duplicate it.
    pub(super) async fn retry_pending_results(&mut self) {
        if self.pending_results.is_empty()
            || !self.program_healthy
            || self.storage_failed
            || self
                .fuel_retry_not_before
                .is_some_and(|deadline| self.core.clock().now_instant() < deadline)
        {
            return;
        }
        while let Some(event) = self.pending_results.pop_front() {
            match self.deliver(event.clone()).await {
                DeliveryOutcome::Delivered | DeliveryOutcome::Failed => {}
                DeliveryOutcome::FuelRefused => {
                    self.note_fuel_refusal();
                    self.pending_results.push_front(event);
                    break;
                }
            }
        }
        if self.pending_results.is_empty() {
            self.fuel_retry_not_before = None;
        }
    }

    /// Pending forms intentionally are not durable. On reopen, tell a program
    /// that had form authority to clear stale views and re-present them with
    /// fresh tokens; otherwise it could leave an unanswerable form on screen.
    pub(in crate::world::actor) async fn invalidate_restored_form_presentations(&mut self) {
        if !self.core.ledger.is_granted("form.result") {
            return;
        }
        let event = serde_json::json!({
            "cap": "form.result",
            "event": "invalidated",
            "reason": "restarted",
        })
        .to_string();
        let _ = self.deliver(event).await;
    }
}
