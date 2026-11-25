//! Command execution for the world actor: journal persistence,
//! entity transfer, delivery, and the program-request executors
//! (provider, forms, location, records, chat, schedules, portals),
//! plus moderation and caps reporting.
//! Extracted from `actor.rs` for the production-line guard (#10456;
//! split tracked in #8592): moves only, no behaviour change.

use super::*;

use crate::world_caps::{CapError, CapsReport, Decision, Request, Subscription};
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeSet, HashMap};
use tokio::sync::oneshot;

mod portal;

impl WorldActor {
    /// Durably record `entry`, or turn the world read-only.
    pub(super) fn persist(&mut self, entry: JournalEntry) -> Result<()> {
        ensure!(
            !self.read_only,
            "world is read-only (replica); local mutations are refused"
        );
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        let world_id = self.core.world_id().to_owned();
        let replication_sender = self.replication_sender.clone();
        if let Some(journal) = self.journal.as_mut() {
            // The append includes its fsync, so this is the full durable
            // cost of the entry (F11.7 journal append + fsync latency).
            let started = std::time::Instant::now();
            let version_before = journal.replication_header_version();
            if let Err(error) = journal.append(&entry) {
                self.storage_failed = true;
                tracing::error!("world journal append failed: {error:#}");
                if version_before != journal.replication_header_version() {
                    let _ = replication_sender.send(ReplicationUpdate::ResetFailed);
                }
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
            // Write locality (#2275): an entity-addressed input is a
            // region-scoped write, so it lands in the locality table
            // under its entity reference.
            if let JournalEntry::Committed { event } | JournalEntry::System { event } = &entry {
                if let WorldEventKind::EntityInput { entity_ref, .. } = &event.kind {
                    self.metrics.record_write_locality(entity_ref);
                }
            }
            if let Some(header_version) = journal.replication_header_version() {
                if version_before != Some(header_version) {
                    let update = journal
                        .replication_snapshot()
                        .map(ReplicationUpdate::Reset)
                        .unwrap_or_else(|error| {
                            tracing::error!(
                                "world {world_id}: replication reset snapshot failed: {error:#}"
                            );
                            ReplicationUpdate::ResetFailed
                        });
                    let _ = replication_sender.send(update);
                } else {
                    let _ = replication_sender.send(ReplicationUpdate::Append {
                        header_version,
                        entry: journal_entry_line(&entry),
                    });
                }
            }
        }
        Ok(())
    }

    /// Durably record a batch of entries with one journal sync (group
    /// commit, #8239), or turn the world read-only. Replication feed
    /// semantics mirror `persist`: a header version change resets the
    /// feed from a snapshot; otherwise each entry is appended to the
    /// feed in order.
    pub(super) fn persist_batch(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.read_only,
            "world is read-only (replica); local mutations are refused"
        );
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        let world_id = self.core.world_id().to_owned();
        let replication_sender = self.replication_sender.clone();
        if let Some(journal) = self.journal.as_mut() {
            let started = std::time::Instant::now();
            let version_before = journal.replication_header_version();
            if let Err(error) = journal.append_batch(entries) {
                self.storage_failed = true;
                tracing::error!("world journal batch append failed: {error:#}");
                if version_before != journal.replication_header_version() {
                    let _ = replication_sender.send(ReplicationUpdate::ResetFailed);
                }
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
            if let Some(header_version) = journal.replication_header_version() {
                if version_before != Some(header_version) {
                    let update = journal
                        .replication_snapshot()
                        .map(ReplicationUpdate::Reset)
                        .unwrap_or_else(|error| {
                            tracing::error!(
                                "world {world_id}: replication reset snapshot failed: {error:#}"
                            );
                            ReplicationUpdate::ResetFailed
                        });
                    let _ = replication_sender.send(update);
                } else {
                    for entry in entries {
                        let _ = replication_sender.send(ReplicationUpdate::Append {
                            header_version,
                            entry: journal_entry_line(entry),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle a drained batch of unsigned Inputs as one group commit
    /// (#8239). Staging runs in queue order - prepare at an explicit
    /// sequence with staged-spend accounting, then the program's update,
    /// exactly as the single-Input path orders them - and stops at the
    /// first failure: the staged prefix is persisted with one sync and
    /// committed in order, the failing input is answered with the error
    /// the single path would have produced for it, and every input after
    /// it goes through the ordinary single-command path untouched, so
    /// later inputs see exactly today's semantics (a program the batch
    /// marked unhealthy refuses them as unavailable, and so on).
    pub(crate) async fn handle_input_batch(&mut self, batch: Vec<WorldCommand>) {
        type InputParts = (
            JoinCapability,
            Vec<u8>,
            Option<SessionId>,
            Option<serde_json::Value>,
            oneshot::Sender<Result<WorldEvent>>,
        );
        let mut items: std::collections::VecDeque<InputParts> = batch
            .into_iter()
            .filter_map(|command| match command {
                WorldCommand::Input {
                    token,
                    input,
                    session_id,
                    device_input,
                    signed_move: None,
                    reply,
                } => Some((token, input, session_id, device_input, reply)),
                // The run loop only drains unsigned Inputs into a batch.
                _ => None,
            })
            .collect();
        if let Err(error) = self.check_writable() {
            let message = format!("{error:#}");
            for (_, _, _, _, reply) in items {
                let _ = reply.send(Err(anyhow::anyhow!(message.clone())));
            }
            return;
        }
        struct Staged {
            candidate: WorldEvent,
            records: Option<Records>,
            reply: oneshot::Sender<Result<WorldEvent>>,
        }
        let mut staged: Vec<Staged> = Vec::new();
        let mut staged_spends: HashMap<u64, u32> = HashMap::new();
        let mut failed: Option<(oneshot::Sender<Result<WorldEvent>>, anyhow::Error)> = None;
        while let Some((token, input, session_id, device_input, reply)) = items.pop_front() {
            let sequence = self.core.sequence + staged.len() as u64 + 1;
            let candidate =
                match self
                    .core
                    .prepare_input_staged(&token, &input, sequence, &staged_spends)
                {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        failed = Some((reply, error));
                        break;
                    }
                };
            if self.storage_failed {
                failed = Some((
                    reply,
                    anyhow::anyhow!("world storage failed; the world is read-only"),
                ));
                break;
            }
            if !self.program_healthy {
                failed = Some((
                    reply,
                    anyhow::anyhow!("world program is unavailable after a previous failure"),
                ));
                break;
            }
            self.acting = Some(candidate.participant_id);
            self.acting_session = session_id;
            self.acting_device_input = device_input;
            match self
                .program
                .update(&candidate)
                .and_then(|()| program_records(self.program.as_mut()))
            {
                Ok(records) => {
                    self.core
                        .note_staged_spend(candidate.participant_id, &mut staged_spends);
                    staged.push(Staged {
                        candidate,
                        records,
                        reply,
                    });
                }
                Err(error) => {
                    if super::is_fuel_rate_refusal(&error) {
                        // Recoverable admission refusal (#1067): the
                        // program never ran and stays healthy, so the
                        // failing input is answered with the refusal and
                        // the inputs after it go through the single
                        // path, where the same refusal — not an
                        // unavailable program — answers them too.
                        failed = Some((
                            reply,
                            error.context("world program refused input at its fuel rate"),
                        ));
                    } else {
                        self.program_healthy = false;
                        failed = Some((reply, error.context("world program rejected input")));
                    }
                    break;
                }
            }
        }
        if !staged.is_empty() {
            let entries: Vec<JournalEntry> = staged
                .iter()
                .map(|staged| JournalEntry::Committed {
                    event: staged.candidate.clone(),
                })
                .collect();
            match self.persist_batch(&entries) {
                Ok(()) => {
                    for staged in staged {
                        match self.core.commit_prepared(staged.candidate) {
                            Ok(event) => {
                                let records = staged.records.unwrap_or_default();
                                self.republished(records, event.sequence);
                                let _ = self.event_sender.send(event.clone());
                                let _ = staged.reply.send(Ok(event));
                            }
                            Err(error) => {
                                let _ = staged.reply.send(Err(error));
                            }
                        }
                    }
                }
                Err(error) => {
                    // persist_batch has marked storage failed; no staged
                    // input may be acknowledged. The first reply carries
                    // the append error itself, the rest the storage state
                    // every later command will also see.
                    let mut replies = staged.into_iter();
                    if let Some(first) = replies.next() {
                        let _ = first.reply.send(Err(error));
                    }
                    for staged in replies {
                        let _ = staged.reply.send(Err(anyhow::anyhow!(
                            "world storage failed; the world is read-only"
                        )));
                    }
                }
            }
        }
        if let Some((reply, error)) = failed {
            let _ = reply.send(Err(error));
        }
        // Whatever the batch did not stage goes through the single-command
        // path, in order, with its semantics untouched.
        for (token, input, session_id, device_input, reply) in items {
            self.handle(WorldCommand::Input {
                token,
                input,
                session_id,
                device_input,
                signed_move: None,
                reply,
            })
            .await;
        }
    }

    /// Durably record related entries as one transaction, or turn the world
    /// read-only. Builder install events and their audit attribution must never
    /// be split across a crash.
    pub(super) fn persist_transaction(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        if let Some(journal) = self.journal.as_mut() {
            let started = std::time::Instant::now();
            if let Err(error) = journal.append_transaction(entries) {
                self.storage_failed = true;
                tracing::error!("world journal transaction failed: {error:#}");
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
        }
        Ok(())
    }

    pub(super) fn export_entity(
        &mut self,
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
    ) -> Result<HandoffTransferState> {
        self.check_storage()?;
        ensure!(self.program_healthy, "world program is unavailable");
        ensure!(
            self.journal.is_some(),
            "entity export requires a durable world journal"
        );
        ensure!(
            self.core.limits.region_sharded,
            "entity export requires a region-sharded world"
        );
        ensure!(
            handoff_id.len() == 32 && handoff_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "handoff ID must be 128-bit hexadecimal"
        );
        core::validate_entity_ref(&entity_ref)?;
        for (name, value) in [
            ("source region", source_region.as_str()),
            ("target region", target_region.as_str()),
        ] {
            ensure!(!value.trim().is_empty(), "{name} must not be empty");
            ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
            ensure!(
                !value.chars().any(char::is_control),
                "{name} contains a control character"
            );
        }
        ensure!(
            source_region != target_region,
            "source and target regions match"
        );
        let program_module_hash = self
            .core
            .active_module_hash
            .clone()
            .context("world has no installed program for entity export")?;
        // Exact-retry recovery: if this export already committed (its reply
        // was lost, or the actor restarted and replayed it), return the
        // stored receipt instead of preparing a duplicate the core would
        // refuse. A stored export whose fields differ from the request is
        // not a retry — it falls through and validate_entity_export
        // rejects the conflicting handoff ID / entity.
        if let Some(existing) = self.core.handoff_transfers.iter().find(|transfer| {
            transfer.direction == HandoffDirection::Export && transfer.handoff_id == handoff_id
        }) && existing.entity_ref == entity_ref
            && existing.source_region == source_region
            && existing.target_region == target_region
            && existing.prior_owner_epoch == prior_owner_epoch
            && existing.program_module_hash == program_module_hash
        {
            ensure!(
                !self.transfer_replay_pending(HandoffDirection::Export, &handoff_id),
                "entity export {handoff_id:?} is durably recorded but has not been applied; replay required"
            );
            return Ok(existing.clone());
        }
        let request = EntityTransferRequest::Export {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            prior_owner_epoch,
            program_module_hash: program_module_hash.clone(),
        };
        let EntityTransferPlan::Export {
            payload_schema,
            payload,
            app_event,
        } = self.program.prepare_entity_transfer(&request)?
        else {
            bail!("world program returned an import plan for an export request");
        };
        ensure!(
            !payload_schema.trim().is_empty() && payload_schema.len() <= 128,
            "entity transfer payload schema must be 1..=128 bytes"
        );
        ensure!(
            !payload_schema.chars().any(char::is_control),
            "entity transfer payload schema contains a control character"
        );
        ensure!(
            payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        let payload_digest = hex_encode(blake3::hash(&payload).as_bytes());
        let event_text = serde_json::json!({
            "protocol": "world.entity-transfer.v1",
            "direction": "export",
            "handoff_id": handoff_id,
            "entity_ref": entity_ref,
            "source_region": source_region,
            "target_region": target_region,
            "prior_owner_epoch": prior_owner_epoch,
            "module_hash": program_module_hash,
            "payload_schema": payload_schema,
            "payload_digest": payload_digest,
            "app_event": app_event,
        })
        .to_string();
        ensure!(
            event_text.len() <= crate::world_caps::MAX_EVENT_BYTES,
            "entity transfer event exceeds {} bytes",
            crate::world_caps::MAX_EVENT_BYTES
        );
        let source_event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: event_text },
        };
        let transfer = HandoffTransferState {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            direction: HandoffDirection::Export,
            prior_owner_epoch,
            new_owner_epoch: None,
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload_digest: payload_digest.clone(),
            payload: Some(payload.clone()),
            event: source_event.clone(),
        };
        self.core.validate_entity_export(&transfer)?;

        // The durable entry is the source-side fence. Install it before
        // applying the prepared event so no subsequent command can mutate
        // this entity, even when the program rejects the post-append update.
        self.persist(JournalEntry::EntityExported {
            handoff_id,
            entity_ref,
            source_region,
            target_region,
            prior_owner_epoch,
            source_event: source_event.clone(),
            payload_schema,
            program_module_hash,
            payload,
            payload_digest,
        })?;
        self.core.record_entity_export(transfer.clone());
        if let Err(error) = self.program.update(&source_event) {
            self.pending_transfer_replays.push(transfer);
            self.program_healthy = false;
            tracing::error!("world program rejected a durable entity export: {error:#}");
            return Err(
                error.context("world program rejected a durable entity export; replay required")
            );
        }
        let records = match program_records(self.program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed after entity export: {error:#}");
                return Err(error.context(
                    "world program failed after a durable entity export; replay required",
                ));
            }
        };
        self.republished(records, self.core.sequence);
        let _ = self.view_changed.send(self.core.sequence);
        Ok(transfer)
    }

    /// Whether a durably recorded transfer is still waiting for a program
    /// to apply it (#3902): while it is, its stored receipt is withheld
    /// from exact retries instead of being acknowledged.
    pub(super) fn transfer_replay_pending(
        &self,
        direction: HandoffDirection,
        handoff_id: &str,
    ) -> bool {
        self.pending_transfer_replays
            .iter()
            .any(|pending| pending.direction == direction && pending.handoff_id == handoff_id)
    }

    pub(super) fn import_entity(
        &mut self,
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        program_module_hash: String,
        payload_schema: String,
        payload: Vec<u8>,
        payload_digest: String,
    ) -> Result<HandoffTransferState> {
        self.check_storage()?;
        ensure!(self.program_healthy, "world program is unavailable");
        ensure!(
            self.journal.is_some(),
            "entity import requires a durable world journal"
        );
        ensure!(
            self.core.limits.region_sharded,
            "entity import requires a region-sharded world"
        );
        ensure!(
            handoff_id.len() == 32 && handoff_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "handoff ID must be 128-bit hexadecimal"
        );
        core::validate_entity_ref(&entity_ref)?;
        for (name, value) in [
            ("source region", source_region.as_str()),
            ("target region", target_region.as_str()),
        ] {
            ensure!(!value.trim().is_empty(), "{name} must not be empty");
            ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
            ensure!(
                !value.chars().any(char::is_control),
                "{name} contains a control character"
            );
        }
        ensure!(
            source_region != target_region,
            "source and target regions match"
        );
        // Validate the envelope before any mutation (design #2818 \§3):
        // the module hash and payload schema must be ones this destination
        // runs, and the payload must hash to the digest the source recorded.
        let active_module_hash = self
            .core
            .active_module_hash
            .clone()
            .context("world has no installed program for entity import")?;
        ensure!(
            program_module_hash == active_module_hash,
            "entity import program module hash does not match the destination program"
        );
        ensure!(
            !payload_schema.trim().is_empty() && payload_schema.len() <= 128,
            "entity transfer payload schema must be 1..=128 bytes"
        );
        ensure!(
            !payload_schema.chars().any(char::is_control),
            "entity transfer payload schema contains a control character"
        );
        ensure!(
            payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        ensure!(
            payload_digest == hex_encode(blake3::hash(&payload).as_bytes()),
            "entity transfer payload digest does not match"
        );
        let new_owner_epoch = prior_owner_epoch
            .checked_add(1)
            .context("entity import owner epoch overflow")?;
        // Exact-retry recovery: if this import already committed (its reply
        // was lost, or the actor restarted and replayed it), return the
        // stored receipt instead of applying the entity twice. A stored
        // import whose fields differ from the request is not a retry \— it
        // falls through and validate_entity_import rejects the conflicting
        // handoff ID / entity, failing closed, never overwriting.
        if let Some(existing) = self.core.handoff_transfers.iter().find(|transfer| {
            transfer.direction == HandoffDirection::Import && transfer.handoff_id == handoff_id
        }) && existing.entity_ref == entity_ref
            && existing.source_region == source_region
            && existing.target_region == target_region
            && existing.prior_owner_epoch == prior_owner_epoch
            && existing.new_owner_epoch == Some(new_owner_epoch)
            && existing.program_module_hash == program_module_hash
            && existing.payload_schema == payload_schema
            && existing.payload_digest == payload_digest
        {
            ensure!(
                !self.transfer_replay_pending(HandoffDirection::Import, &handoff_id),
                "entity import {handoff_id:?} is durably recorded but has not been applied; replay required"
            );
            return Ok(existing.clone());
        }
        let request = EntityTransferRequest::Import {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            prior_owner_epoch,
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload: payload.clone(),
            payload_digest: payload_digest.clone(),
        };
        let EntityTransferPlan::Import { app_event } =
            self.program.prepare_entity_transfer(&request)?
        else {
            bail!("world program returned an export plan for an import request");
        };
        let event_text = serde_json::json!({
            "protocol": "world.entity-transfer.v1",
            "direction": "import",
            "handoff_id": handoff_id,
            "entity_ref": entity_ref,
            "source_region": source_region,
            "target_region": target_region,
            "prior_owner_epoch": prior_owner_epoch,
            "new_owner_epoch": new_owner_epoch,
            "module_hash": program_module_hash,
            "payload_schema": payload_schema,
            "payload_digest": payload_digest,
            "payload_hex": hex_encode(&payload),
            "app_event": app_event,
        })
        .to_string();
        ensure!(
            event_text.len() <= crate::world_caps::MAX_EVENT_BYTES,
            "entity transfer event exceeds {} bytes",
            crate::world_caps::MAX_EVENT_BYTES
        );
        let destination_event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: event_text },
        };
        let transfer = HandoffTransferState {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            direction: HandoffDirection::Import,
            prior_owner_epoch,
            new_owner_epoch: Some(new_owner_epoch),
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload_digest: payload_digest.clone(),
            payload: None,
            event: destination_event.clone(),
        };
        self.core.validate_entity_import(&transfer)?;

        // Persist-before-apply: the durable entry is the import receipt.
        // Record it in the dedupe ledger before applying the prepared
        // event, so a retry after a crash observes the receipt instead of
        // applying the entity twice. The receipt is returned only after
        // the program applies the event; a post-append failure withholds
        // it and recovery replays the durable entry.
        self.persist(JournalEntry::EntityImported {
            handoff_id,
            entity_ref,
            source_region,
            target_region,
            prior_owner_epoch,
            new_owner_epoch,
            program_module_hash,
            payload_schema,
            payload_digest,
            destination_event: destination_event.clone(),
        })?;
        self.core.record_entity_import(transfer.clone());
        if let Err(error) = self.program.update(&destination_event) {
            self.pending_transfer_replays.push(transfer);
            self.program_healthy = false;
            tracing::error!("world program rejected a durable entity import: {error:#}");
            return Err(
                error.context("world program rejected a durable entity import; replay required")
            );
        }
        let records = match program_records(self.program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed after entity import: {error:#}");
                return Err(error.context(
                    "world program failed after a durable entity import; replay required",
                ));
            }
        };
        self.republished(records, self.core.sequence);
        let _ = self.view_changed.send(self.core.sequence);
        Ok(transfer)
    }

    /// Publish freshly recomputed records. Participant events push their own
    /// view frame right after the event; host events signal the views watch.
    pub(super) fn republished(&mut self, records: Records, sequence: u64) {
        publish_records(&self.records_sender, sequence, records);
    }
}

/// What one host-event delivery did (#1067). A bare `bool` conflates the
/// recoverable case with every other drop, which is exactly how a
/// fuel-refused capability result used to be lost: only `FuelRefused`
/// guarantees the program never ran, so only it may be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeliveryOutcome {
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
    pub(super) const fn delivered(self) -> bool {
        matches!(self, Self::Delivered)
    }
}

impl WorldActor {
    /// Deliver a host-originated event to the program: journal it first is
    /// impossible (the program must accept it for the journal to be true), so
    /// the order is update, then journal, exactly like participant input.
    /// A read-only replica never delivers: host events are authoritative
    /// program effects and must not originate from a replica.
    pub(super) async fn deliver(&mut self, text: String) -> DeliveryOutcome {
        self.deliver_system(text, None).await
    }

    /// Deliver a form response and durably spend the authenticated submitter's
    /// use budget only after the system event was accepted and journaled.
    pub(super) async fn deliver_form_result(
        &mut self,
        text: String,
        participant_id: Option<u64>,
    ) -> DeliveryOutcome {
        self.deliver_system(text, participant_id).await
    }

    pub(super) async fn deliver_system(
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
            if super::is_fuel_rate_refusal(&error) {
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
    fn note_fuel_refusal(&mut self) {
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
    async fn deliver_result(&mut self, event: String) {
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
    async fn retry_pending_results(&mut self) {
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
    pub(super) async fn invalidate_restored_form_presentations(&mut self) {
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

    /// Recompute the program's `wants` and serve the delta: new subscriptions
    /// (armed or refused), new requests (executed, result delivered). Passes
    /// repeat while the program reacts to delivered results, so a chain of
    /// asks settles inside one change. A read-only replica never runs this:
    /// program requests are side effects (chat.say, schedule.at) that must
    /// not originate from a replica. Requests the program already listed
    /// at a restored open are refused, not executed (see
    /// [`stale_wants_at_open`](Self::stale_wants_at_open)).
    pub(crate) async fn after_change(&mut self) {
        if self.read_only || !self.holds_live_do_lease() {
            return;
        }
        // Capability results a fuel-rate refusal queued (see
        // `deliver_result`) go first, so the program reacts to them
        // in this same pass.
        self.retry_pending_results().await;
        for _ in 0..64 {
            if !self.program_healthy || self.storage_failed {
                break;
            }
            let Some(text) = (match self.program.wants() {
                Ok(wants) => wants,
                Err(error) => {
                    self.program_healthy = false;
                    tracing::error!("world program wants failed: {error:#}");
                    break;
                }
            }) else {
                break;
            };
            let wants = match crate::world_caps::parse_wants(&text) {
                Ok(wants) => wants,
                Err(error) => {
                    self.program_healthy = false;
                    tracing::error!("world program wants invalid: {error:#}");
                    break;
                }
            };
            self.apply_subscriptions(&wants).await;
            let mut fresh = Vec::new();
            for request in &wants.requests {
                if self.handled_requests.insert(request.id.clone()) {
                    fresh.push(request.clone());
                }
            }
            if fresh.is_empty() {
                break;
            }
            // A program inventing endless request ids cannot grow this set
            // without bound; old ids may then re-execute, which is the
            // documented at-least-once behavior.
            if self.handled_requests.len() > 4096 {
                let current: BTreeSet<String> = wants
                    .requests
                    .iter()
                    .map(|request| request.id.clone())
                    .collect();
                self.handled_requests.retain(|id| current.contains(id));
            }
            for request in fresh {
                // Pre-restart intent is never re-executed with world
                // authority: the side effect may already have happened, and
                // there is no acting participant to attribute a repeat to.
                // Refusing (rather than executing) keeps non-idempotent
                // providers (chat.say today; kv / http.fetch / notify as
                // they land) to at-most-once per id across a reopen; the
                // program re-issues the id afterwards if it still wants the
                // effect, and that new intent executes normally.
                if self.stale_wants_at_open.remove(&request.id) {
                    // F13.7 step 6: a restarted refusal of a
                    // client-provider request is a host provider
                    // decision like any other, so the broker records
                    // it in the F12.6 audit log (denied / restarted)
                    // — the audit trail has no gap at reopen. Host
                    // capabilities keep their existing accounting.
                    if let Some(manifest) = crate::world_caps::PROVIDER_REGISTRY
                        .get(&request.cap)
                        .filter(|manifest| manifest.side == crate::world_caps::ProviderSide::Client)
                    {
                        let actor = self
                            .acting
                            .map_or_else(|| "program".to_owned(), |id| format!("participant:{id}"));
                        self.provider_broker.note_restarted_refusal(
                            manifest,
                            &actor,
                            &request.args,
                        );
                    }
                    let refusal = crate::world_caps::result_event(
                        &request.id,
                        Err(CapError::actor_denied_with("restarted")),
                    );
                    self.deliver_result(refusal).await;
                    continue;
                }
                if crate::world_caps::PROVIDER_REGISTRY
                    .get(&request.cap)
                    .is_some_and(|manifest| {
                        manifest.side == crate::world_caps::ProviderSide::Client
                    })
                {
                    // F13.7 step 3: a client provider is brokered to
                    // the originating session, never executed on the
                    // host. A refusal is delivered now; an allowed
                    // request stays pending and its completion is
                    // delivered by drain_provider_completions.
                    match self.begin_client_provider(&request) {
                        Ok(()) => {}
                        Err(error) => {
                            let refusal = crate::world_caps::result_event(&request.id, Err(error));
                            self.deliver_result(refusal).await;
                        }
                    }
                    continue;
                }
                let outcome = self.execute(&request);
                self.deliver_result(crate::world_caps::result_event(&request.id, outcome))
                    .await;
            }
        }
        self.compact_if_due().await;
    }

    /// Subscribe new subscriptions, drop removed ones, refuse ungranted ones
    /// (once per spec), and keep ticks armed only while someone is present.
    /// Route one client-side provider request (F13.7 step 3) to the
    /// broker. `Ok(())` means the request is pending: its result is
    /// delivered when the bound session replies, by
    /// [`drain_provider_completions`](Self::drain_provider_completions).
    /// An `Err` is a policy refusal the caller delivers immediately.
    pub(super) fn begin_client_provider(&mut self, request: &Request) -> Result<(), CapError> {
        let Some(manifest) = crate::world_caps::PROVIDER_REGISTRY.get(&request.cap) else {
            return Err(CapError::unknown());
        };
        let actor_permits = match self.acting {
            None => true,
            Some(participant) => self
                .core
                .effective_scopes(participant)
                .is_some_and(|scopes| scopes.allows_effect(&request.cap)),
        };
        let actor = self
            .acting
            .map_or_else(|| "program".to_owned(), |id| format!("participant:{id}"));
        self.provider_broker.begin(BeginRequest {
            manifest,
            ceiling_allows: self.core.ledger.allowed().contains(&request.cap),
            actor_permits,
            granted: self.core.ledger.is_granted(&request.cap),
            actor,
            participant: self.acting,
            session: self.acting_session,
            guest_request_id: request.id.clone(),
            args: &request.args,
        })?;
        Ok(())
    }

    /// Deliver every broker completion (F13.7 step 3) to the guest as
    /// the ordinary capability result event, then let the program
    /// react. Expired requests are completed first, so a reply that
    /// raced its deadline cannot win after it.
    pub(super) async fn drain_provider_completions(&mut self) {
        if self.read_only {
            return;
        }
        for _ in 0..8 {
            self.provider_broker.collect_expired();
            let completions = self.provider_broker.drain_completions();
            if completions.is_empty() {
                break;
            }
            for completion in completions {
                let event = crate::world_caps::result_event(
                    &completion.guest_request_id,
                    completion.outcome,
                );
                self.deliver_result(event).await;
            }
            self.after_change().await;
        }
    }

    pub(super) async fn apply_subscriptions(&mut self, wants: &crate::world_caps::Wants) {
        for sub in &wants.subscribe {
            if !self.active_subs.insert(sub.clone()) {
                continue;
            }
            // The tick floor is a hard limit, not a grant question: a
            // sub-floor tick is refused like a denied subscription (once
            // per spec) and never armed, however it was granted.
            if let Subscription::Tick(ms) = sub
                && *ms < self.core.limits.min_tick_ms
            {
                let spec = sub.spec();
                if self.denied_subs.insert(spec.clone()) {
                    let error = crate::world_caps::CapError::bad_args_with(format!(
                        "time.tick interval {ms}ms is below the world minimum {}ms",
                        self.core.limits.min_tick_ms
                    ));
                    self.deliver(crate::world_caps::subscription_refused_event(&spec, &error))
                        .await;
                }
                continue;
            }
            // A loc.watch spec that fails the core parse (a foreign
            // shape, a non-numeric interval, or an interval below the
            // floor) is refused like a sub-floor tick: once per spec,
            // and never armed (#2341 slice 2).
            if let Subscription::Unknown(spec) = sub
                && spec.starts_with("loc.watch")
                && let Err(error) = crate::world_caps::parse_loc_watch_interval_ms(spec)
            {
                if self.denied_subs.insert(spec.clone()) {
                    self.deliver(crate::world_caps::subscription_refused_event(spec, &error))
                        .await;
                }
                continue;
            }
            let decision = self.core.ledger.decide(sub.cap());
            self.audit_provider_decision(sub.cap(), &decision, "program".to_owned());
            match decision {
                Decision::Allow => self.arm(sub),
                Decision::AllowAndGrant => {
                    let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                    if self.persist(JournalEntry::CapsGranted { granted }).is_ok() {
                        self.arm(sub);
                    }
                }
                Decision::Deny(error) => {
                    let spec = sub.spec();
                    if self.denied_subs.insert(spec.clone()) {
                        self.deliver(crate::world_caps::subscription_refused_event(&spec, &error))
                            .await;
                    }
                }
            }
        }
        let current = wants.subscribe.clone();
        self.active_subs.retain(|sub| current.contains(sub));
        // Timers for subscriptions that just left the wants are
        // disarmed now, not at their distant deadlines (PR #8643
        // Codex round-2 P2): otherwise every wants update can leave
        // up to MAX_LOC_WATCHES stale timers behind, growing the map
        // — and every next_deadline scan — without bound. This is
        // the same prune reconcile_subscriptions applies on an
        // authority change.
        let active = &self.active_subs;
        self.loc_watches
            .retain(|ms, _| active.contains(&Subscription::LocWatch(*ms)));
        self.denied_subs
            .retain(|spec| current.iter().any(|sub| &sub.spec() == spec));
        if self.presence.values().all(|count| *count == 0) {
            self.ticks.clear();
            self.loc_watches.clear();
        }
    }

    /// Arm a granted subscription's timer, if any and if anyone is present.
    /// A tick below the world's `min_tick_ms` floor is never armed.
    pub(super) fn arm(&mut self, sub: &Subscription) {
        // A loc.watch arms only while consent stands (#2341 slice 2):
        // an unconsented watch is known but never delivers, and if
        // consent later stands, the presence path arms it.
        if let Subscription::LocWatch(ms) = sub
            && self.location_consent == crate::world_caps::LocationConsent::Granted
            && !self.presence.is_empty()
            && !self.loc_watches.contains_key(ms)
        {
            self.loc_watches.insert(
                *ms,
                self.core.clock().now_instant() + std::time::Duration::from_millis(*ms),
            );
        }
        if let Subscription::Tick(ms) = sub
            && *ms >= self.core.limits.min_tick_ms
            && !self.presence.is_empty()
            && !self.ticks.contains_key(ms)
        {
            self.ticks.insert(
                *ms,
                self.core.clock().now_instant() + std::time::Duration::from_millis(*ms),
            );
        }
    }

    /// Deliver one tick event per due interval and re-arm.
    pub(crate) async fn fire_ticks(&mut self) {
        if self.read_only || !self.holds_live_do_lease() {
            return;
        }
        let now = self.core.clock().now_instant();
        let due: Vec<u64> = self
            .ticks
            .iter()
            .filter(|(_, next)| **next <= now)
            .map(|(ms, _)| *ms)
            .collect();
        for ms in due {
            self.ticks
                .insert(ms, now + std::time::Duration::from_millis(ms));
            let event = serde_json::json!({
                "cap": "time.tick",
                "interval": ms,
                // The tick's "now" reads the injected clock too, so a manual
                // clock drives the payload as well as the deadline.
                "now": self.core.clock().now_ms(),
            });
            self.deliver(event.to_string()).await;
        }
    }

    /// Deliver one reading per due `loc.watch` interval and re-arm
    /// (#2341 slice 2), mirroring [`fire_ticks`](Self::fire_ticks).
    /// A watch whose consent has been withdrawn since it armed is
    /// closed with exactly the revocation event and removed from the
    /// active subscriptions. With consent standing but no host fix,
    /// nothing is delivered — a reading is never fabricated.
    pub(crate) async fn fire_loc_watches(&mut self) {
        if self.read_only || !self.holds_live_do_lease() {
            return;
        }
        let now = self.core.clock().now_instant();
        let due: Vec<u64> = self
            .loc_watches
            .iter()
            .filter(|(_, next)| **next <= now)
            .map(|(ms, _)| *ms)
            .collect();
        for ms in due {
            // A watch that is no longer an active, granted
            // subscription is stale (its authority was removed after
            // it armed): disarm it without delivering a reading.
            if !self.active_subs.contains(&Subscription::LocWatch(ms))
                || !self.core.ledger.is_granted("loc.watch")
            {
                self.loc_watches.remove(&ms);
                self.active_subs.remove(&Subscription::LocWatch(ms));
                continue;
            }
            if self.location_consent != crate::world_caps::LocationConsent::Granted {
                self.loc_watches.remove(&ms);
                self.active_subs.remove(&Subscription::LocWatch(ms));
                self.deliver(crate::world_caps::loc_watch_revocation_event())
                    .await;
                continue;
            }
            self.loc_watches
                .insert(ms, now + std::time::Duration::from_millis(ms));
            if let Some(reading) = self.current_location_reading() {
                self.deliver(crate::world_caps::loc_watch_reading_event(&reading))
                    .await;
            }
        }
    }

    /// Fire the schedules that are due, earliest first, at most
    /// [`crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP`] per wakeup. Each
    /// fire is delivered as a tick is, and its schedule is consumed only after
    /// that delivery is journaled. A refused fire, or one that could not be
    /// journaled, leaves its schedule pending, so the next run delivers it.
    /// A fuel-rate refusal backs the next attempt off until the window
    /// rolls (see `fuel_retry_not_before`): without the backoff the run
    /// loop would retry the same refused fire on every pass until
    /// rollover. A read-only replica never fires: schedules are writer
    /// authority.
    pub(crate) async fn fire_schedules(&mut self) {
        if self.read_only || !self.holds_live_do_lease() {
            return;
        }
        if self
            .fuel_retry_not_before
            .is_some_and(|deadline| self.core.clock().now_instant() < deadline)
        {
            return;
        }
        let now = self.core.clock().now_ms();
        let mut due: Vec<(u64, String)> = self
            .core
            .schedules
            .iter()
            .filter(|(_, schedule)| schedule.at_ms <= now)
            .map(|(id, schedule)| (schedule.at_ms, id.clone()))
            .collect();
        due.sort_unstable();
        due.truncate(crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP);
        for (_, id) in due {
            let late = self.overdue_at_open.contains(&id);
            let Some(schedule) = self.core.schedules.get(&id).cloned() else {
                continue;
            };
            let delivered = match &schedule.target {
                None => match self.deliver(schedule_fired_event(&id, late)).await {
                    DeliveryOutcome::Delivered => true,
                    DeliveryOutcome::FuelRefused => {
                        self.note_fuel_refusal();
                        false
                    }
                    DeliveryOutcome::Failed => false,
                },
                Some(target) => {
                    // Cross-world (#1355 part 2): consent is re-checked at
                    // fire time — a grant revoked or an isolation change
                    // since the set leaves the occurrence pending, exactly
                    // like an unavailable target.
                    let consented = self.core.isolation == Isolation::Unisolated
                        && self
                            .core
                            .schedule_grants
                            .get(target)
                            .is_some_and(|grant| grant.send);
                    if !consented {
                        break;
                    }
                    let Some(sink) = self.schedule_delivery.clone() else {
                        break;
                    };
                    sink.deliver_schedule(
                        &self.core.world_id.clone(),
                        target,
                        &id,
                        schedule.at_ms,
                        late,
                    )
                    .await
                }
            };
            if !delivered {
                break;
            }
            if self
                .persist(JournalEntry::ScheduleFired { id: id.clone() })
                .is_err()
            {
                break;
            }
            // One journaled transition, shared with replay: a recurring
            // schedule re-arms in place (fixed-rate, one slot), a one-shot
            // is consumed.
            self.core.apply_schedule_fired(&id);
            self.overdue_at_open.remove(&id);
        }
    }

    /// Execute one capability request. Anything the world has not granted is
    /// an error the program sees.
    pub(super) fn execute(&mut self, request: &Request) -> Result<serde_json::Value, CapError> {
        if request.cap == "form.result" {
            return Err(CapError::bad_args_with("event_only"));
        }
        if crate::world_caps::PROVIDER_REGISTRY
            .get(&request.cap)
            .is_some_and(|manifest| manifest.side == crate::world_caps::ProviderSide::Client)
        {
            // Client providers are brokered asynchronously (F13.7
            // step 3); after_change routes them to the broker before
            // execute is ever called. Reaching here means a caller
            // bypassed that route — fail closed, never execute a
            // client resource on the host.
            return Err(CapError::failed_with("client_provider_async"));
        }
        let actor_permits = match self.acting {
            None => true,
            Some(participant) => self
                .core
                .effective_scopes(participant)
                .is_some_and(|scopes| scopes.allows_effect(&request.cap)),
        };
        let decision = self.core.ledger.decide_for(&request.cap, actor_permits);
        let actor = self
            .acting
            .map_or_else(|| "program".to_owned(), |id| format!("participant:{id}"));
        self.audit_provider_decision(&request.cap, &decision, actor.clone());
        match decision {
            Decision::Allow => {}
            Decision::AllowAndGrant => {
                let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                self.persist(JournalEntry::CapsGranted { granted })
                    .map_err(|_| CapError::failed_with("capability_grant_persist_failed"))?;
            }
            Decision::Deny(error) => return Err(error),
        }
        match request.cap.as_str() {
            "time.now" => Ok(serde_json::json!({"now": unix_ms()})),
            "random.u64" => Ok(serde_json::json!({"value": rand::rng().random::<u64>()})),
            "players.list" => Ok(serde_json::json!({"players": self.players_list()})),
            "world.info" => Ok(serde_json::json!({
                "world": self.core.world_id,
                "sequence": self.core.sequence,
            })),
            "device.info" => self.execute_device_info(&request.args),
            "form.present" => self.execute_form_present(&request.args),
            "form.result" => Err(CapError::bad_args_with("event_only")),
            "form.cancel" => self.execute_form_cancel(&request.args),
            "loc.position" => self.execute_loc_position(&request.args),
            "chat.say" => self.execute_chat(&request.args),
            "chat.history" => {
                // A supplied `count` must be an unsigned integer; only an
                // omitted count falls back to the default. Treating a
                // malformed count as omitted silently turns a client error
                // into a successful read (#7892).
                let count = match request.args.get("count") {
                    None => 16,
                    Some(value) => value
                        .as_u64()
                        .ok_or_else(CapError::bad_args)?
                        .min(crate::world_caps::MAX_CHAT_HISTORY_LINES as u64)
                        as usize,
                };
                Ok(serde_json::json!({
                    "lines": chat_history(
                        &self.core.events,
                        count.min(crate::world_caps::MAX_CHAT_HISTORY_LINES),
                    ),
                }))
            }
            "records.read" => self.execute_records_read(&request.args),
            "kv.get" => self.execute_kv_get(&request.args),
            "kv.set" => self.execute_kv_set(&request.args),
            "kv.delete" => self.execute_kv_delete(&request.args),
            "blob.put" => self.execute_blob_put(&request.args),
            "blob.get" => self.execute_blob_get(&request.args),
            "blob.delete" => self.execute_blob_delete(&request.args),
            "http.fetch" => self.execute_http_fetch(&request.args),
            "notify" => self.execute_notify(&request.args),
            "schedule.at" => self.execute_schedule_at(&request.args),
            "schedule.cancel" => self.execute_schedule_cancel(&request.args),
            _ => Err(CapError::unknown()),
        }
    }

    /// Opening a form opens the deferred result channel. Apply normal grant,
    /// ceiling and actor-scope policy here, after the presentation is valid;
    /// the submission boundary rechecks the grant in case it is later revoked.
    pub(super) fn authorize_form_result(&mut self) -> Result<(), CapError> {
        let actor_permits = match self.acting {
            None => true,
            Some(participant) => self
                .core
                .effective_scopes(participant)
                .is_some_and(|scopes| scopes.allows_effect("form.result")),
        };
        let decision = self.core.ledger.decide_for("form.result", actor_permits);
        let actor = self
            .acting
            .map_or_else(|| "program".to_owned(), |id| format!("participant:{id}"));
        self.audit_provider_decision("form.result", &decision, actor);
        match decision {
            Decision::Allow => Ok(()),
            Decision::AllowAndGrant => {
                let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                self.persist(JournalEntry::CapsGranted { granted })
                    .map_err(|_| CapError::failed_with("capability_grant_persist_failed"))
            }
            Decision::Deny(error) => Err(error),
        }
    }

    pub(super) fn fresh_form_presentation_token(&self) -> String {
        loop {
            let candidate = format!("{:032x}", rand::rng().random::<u128>());
            if !self
                .pending_forms
                .values()
                .any(|pending| pending.presentation_token == candidate)
            {
                return candidate;
            }
        }
    }

    pub(super) fn validate_form_presentation_size(
        &self,
        form: &id_core::world_presentation::FormSurface,
        presentation_token: &str,
    ) -> Result<(), CapError> {
        use id_core::world_presentation::MAX_FORM_BYTES;

        let mut presented = form.clone();
        presented.presentation_token = Some(presentation_token.to_owned());
        presented
            .validate()
            .map_err(|_| CapError::bad_args_with("field_invalid"))?;
        let mut envelope = serde_json::to_value(presented)
            .map_err(|_| CapError::bad_args_with("field_invalid"))?;
        envelope["kind"] = serde_json::json!("form");
        let view_bytes = serde_json::to_vec(&envelope)
            .map_err(|_| CapError::bad_args_with("field_invalid"))?
            .len();
        if view_bytes > MAX_FORM_BYTES {
            return Err(CapError::bad_args_with("form_view_too_large"));
        }
        if view_bytes > self.core.limits.presentation_bytes {
            return Err(CapError::bad_args_with("presentation_too_large"));
        }
        Ok(())
    }

    /// `device.info` (#2338): the hosting device's capability
    /// manifest, in the v1 shape the taxonomy specifies. The manifest
    /// comes from the adapter boundary's cached copy — this provider
    /// never enumerates the device itself. A host that cannot
    /// describe its device refuses with `unknown_cap`, and a request
    /// naming a manifest version this host does not speak is refused
    /// rather than silently interpreted.
    pub(super) fn execute_device_info(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        crate::world_caps::DeviceManifest::requested_version(args)?;
        let Some(manifest) = &self.device_manifest else {
            return Err(CapError::unknown());
        };
        Ok(manifest.to_json())
    }

    /// Register a bounded form for the adapter's trusted widget renderer.
    /// The form itself is also the current `WorldSurface::Form` view; this
    /// request only opens its deferred result correlation window.
    pub(super) fn execute_form_present(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        use id_core::world_presentation::FormSurface;

        let Some(args) = args.as_object() else {
            return Err(CapError::bad_args());
        };
        if args.len() != 2 || !args.contains_key("form_id") || !args.contains_key("form") {
            return Err(CapError::bad_args());
        }
        let form_id = args
            .get("form_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(CapError::bad_args)?;
        let mut raw_form = args.get("form").cloned().ok_or_else(CapError::bad_args)?;
        if let Some(object) = raw_form.as_object_mut()
            && object.get("kind").and_then(serde_json::Value::as_str) == Some("form")
        {
            object.remove("kind");
        }
        if let Some(object) = raw_form.as_object_mut() {
            // A form view may carry a token copied from an earlier
            // presentation; only this host boundary may mint a new one.
            object.remove("presentation_token");
        }
        let form: FormSurface = serde_json::from_value(raw_form)
            .map_err(|_| CapError::bad_args_with("field_invalid"))?;
        form.validate()
            .map_err(|_| CapError::bad_args_with("field_invalid"))?;
        if form.form_id != form_id {
            return Err(CapError::bad_args_with("field_invalid"));
        }
        let args_size = serde_json::to_vec(args).map_or(usize::MAX, |encoded| encoded.len());
        if args_size > crate::world_caps::MAX_ARGS_BYTES {
            return Err(CapError::bad_args_with("args_too_large"));
        }
        let presenter_input = self.acting_device_input.as_ref().or_else(|| {
            self.device_manifest.as_ref().and_then(|manifest| {
                (manifest
                    .display
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    != Some("headless"))
                .then_some(&manifest.input)
            })
        });
        if let Some(input) = presenter_input
            && let Err(error) = form.validate_device_input(input)
        {
            let reason = if format!("{error:#}").contains("text entry unavailable") {
                "text_entry_unavailable"
            } else {
                "input_unavailable"
            };
            return Err(CapError::bad_args_with(reason));
        }
        if let Some(existing) = self.pending_forms.get(form_id) {
            if existing.surface != form {
                return Err(CapError::bad_args_with("field_invalid"));
            }
            let presentation_token = self.fresh_form_presentation_token();
            self.validate_form_presentation_size(&form, &presentation_token)?;
            self.authorize_form_result()?;
            let Some(existing) = self.pending_forms.get_mut(form_id) else {
                return Err(CapError::failed_with("pending_form_disappeared"));
            };
            existing.presentation_token = presentation_token.clone();
            return Ok(serde_json::json!({
                "form_id": form_id,
                "presentation_token": presentation_token,
                "presented": true
            }));
        }
        if self.pending_forms.len() >= crate::world_caps::MAX_PENDING_FORMS {
            return Err(CapError::failed_with("too_many_pending_forms"));
        }
        let presentation_token = self.fresh_form_presentation_token();
        self.validate_form_presentation_size(&form, &presentation_token)?;
        self.authorize_form_result()?;
        self.pending_forms.insert(
            form_id.to_owned(),
            PendingForm {
                surface: form,
                presentation_token: presentation_token.clone(),
            },
        );
        Ok(serde_json::json!({
            "form_id": form_id,
            "presentation_token": presentation_token,
            "presented": true
        }))
    }

    pub(super) fn execute_form_cancel(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let Some(args) = args.as_object() else {
            return Err(CapError::bad_args());
        };
        if args.len() != 1 || !args.contains_key("form_id") {
            return Err(CapError::bad_args());
        }
        let form_id = args
            .get("form_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(CapError::bad_args)?;
        let cancelled = self.pending_forms.remove(form_id).is_some();
        Ok(serde_json::json!({"form_id": form_id, "cancelled": cancelled}))
    }

    /// Consume one host response. Invalid field values are reported once to the
    /// program and leave the form pending for explicit adapter re-presentation;
    /// this path never retries or loops on the server.
    pub(super) async fn submit_form_result(
        &mut self,
        token: Option<JoinCapability>,
        session_id: Option<SessionId>,
        result: id_core::world_presentation::FormResult,
    ) -> Result<FormSubmissionStatus> {
        let participant_id = if let Some(token) = token {
            let Ok(participant) = self.core.authorize(&token, crate::world::WorldScopes::JOIN)
            else {
                return Ok(FormSubmissionStatus::Unavailable {
                    error: "join_denied",
                    reason: "invalid_capability",
                });
            };
            if !self
                .core
                .effective_scopes(participant)
                .is_some_and(|scopes| scopes.allows_effect("form.result"))
            {
                return Ok(FormSubmissionStatus::Unavailable {
                    error: "actor_denied",
                    reason: "effect_scope_denied",
                });
            }
            Some(participant)
        } else {
            None
        };
        if !self.core.ledger.is_granted("form.result") {
            return Ok(FormSubmissionStatus::Unavailable {
                error: "cap_denied",
                reason: "capability_revoked",
            });
        }
        let Some(pending) = self.pending_forms.get(&result.form_id) else {
            return Ok(FormSubmissionStatus::Unavailable {
                error: "bad_args",
                reason: "form_cancelled",
            });
        };
        if result.presentation_token != pending.presentation_token {
            return Ok(FormSubmissionStatus::Unavailable {
                error: "bad_args",
                reason: "stale_presentation",
            });
        }
        if let Some(participant_id) = participant_id
            && self.core.ensure_uses_left(participant_id).is_err()
        {
            return Ok(FormSubmissionStatus::Unavailable {
                error: "actor_denied",
                reason: "use_limit_exhausted",
            });
        }
        let valid = pending.surface.validate_result(&result).is_ok();
        let (event, retry_token) = if valid {
            (
                serde_json::json!({
                    "cap": "form.result",
                    "form_id": result.form_id,
                    "action": result.action,
                    "values": result.values,
                }),
                None,
            )
        } else {
            let token = self.fresh_form_presentation_token();
            (
                serde_json::json!({
                    "cap": "form.result",
                    "form_id": result.form_id,
                    "ok": false,
                    "error": "bad_args",
                    "reason": "field_invalid",
                    "presentation_token": token,
                }),
                Some(token),
            )
        };
        let mut event = event;
        if let Some(participant_id) = participant_id {
            event["participant_id"] = serde_json::json!(participant_id);
        }
        let event = event.to_string();
        ensure!(
            event.len() <= crate::world_caps::MAX_EVENT_BYTES,
            "form.result event exceeds MAX_EVENT_BYTES"
        );
        self.acting = participant_id;
        self.acting_session = participant_id.and(session_id);
        ensure!(
            self.deliver_form_result(event, participant_id)
                .await
                .delivered(),
            "form.result event was not delivered"
        );
        if valid {
            self.pending_forms.remove(&result.form_id);
            Ok(FormSubmissionStatus::Accepted)
        } else {
            let presentation_token = retry_token
                .ok_or_else(|| anyhow::anyhow!("invalid result omitted its retry token"))?;
            if let Some(pending) = self.pending_forms.get_mut(&result.form_id) {
                pending.presentation_token = presentation_token.clone();
            }
            Ok(FormSubmissionStatus::Rejected { presentation_token })
        }
    }
    /// Record the host location source's latest fix (#2341 slice 2),
    /// stamped with the world clock so serving can report the fix's
    /// true age at delivery time.
    pub(crate) fn set_location_fix(&mut self, reading: crate::world_caps::LocationReading) {
        self.location_fix_received_at = Some(self.core.clock().now_instant());
        self.location_fix = Some(reading);
    }

    /// The cached fix with its age updated to now: `fix_age_ms` is
    /// defined as the age when delivered, so the elapsed time since
    /// receipt is added at serve time (PR #8643 Codex round-2 P2).
    /// A fix recorded without a receipt stamp (a direct field write)
    /// is served as recorded.
    pub(super) fn current_location_reading(&self) -> Option<crate::world_caps::LocationReading> {
        let mut reading = self.location_fix?;
        if let Some(received_at) = self.location_fix_received_at {
            let elapsed = self
                .core
                .clock()
                .now_instant()
                .saturating_duration_since(received_at);
            reading.fix_age_ms = reading
                .fix_age_ms
                .saturating_add(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        }
        Some(reading)
    }

    /// `loc.position` (#2341 slice 2): one reading from the host
    /// location source, gated in order by the cap ledger (in
    /// [`execute`](Self::execute), before dispatch), the consent seam,
    /// and the source itself. A consent refusal is the consent seam's
    /// own error; no source or no fix is `no_fix`; a fix less precise
    /// than the request's `max_accuracy_m` ceiling is refused with
    /// `precision_ceiling`, never silently degraded.
    pub(super) fn execute_loc_position(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        if let Some(refusal) = self.location_consent.refusal() {
            return Err(refusal);
        }
        let Some(reading) = self.current_location_reading() else {
            return Err(CapError::no_fix());
        };
        // A supplied ceiling that is not an unsigned integer is
        // malformed, not absent: refuse it instead of silently
        // dropping the precision check (#2341, PR #8643 Codex P2).
        if let Some(raw_ceiling) = args.get("max_accuracy_m") {
            let ceiling = raw_ceiling.as_u64().ok_or_else(CapError::bad_args)?;
            reading.refuse_beyond_ceiling(ceiling as f64)?;
        }
        Ok(reading.to_json())
    }

    pub(super) fn players_list(&self) -> Vec<serde_json::Value> {
        self.core
            .participants()
            .into_iter()
            .map(|participant| {
                serde_json::json!({
                    "id": participant.id,
                    "name": participant.display_name,
                    "present": self.presence.get(&participant.id).is_some_and(|c| *c > 0),
                })
            })
            .collect()
    }

    /// `records.read`: a byte-bounded, prefix-scoped page of the world's
    /// published records, key-ordered with the same exclusive `after` cursor
    /// the session protocol uses. Follow `next_after` while it is non-empty.
    /// The page is bounded by the cap-result budget
    /// (world_caps::MAX_RECORDS_RESULT_PAGE_BYTES), not the session
    /// protocol's larger page budget, so an admitted page always fits
    /// the result event that delivers it (#4888).
    pub(super) fn execute_records_read(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let records = self.records_sender.borrow().1.clone();
        // #7899: a present-but-non-string prefix/after is bad_args,
        // not "omitted" — treating it as omitted silently widens a
        // scoped read to every record, or restarts pagination.
        let prefix = match args.get("prefix") {
            None => None,
            Some(value) => Some(value.as_str().ok_or(CapError::bad_args())?),
        };
        let after = match args.get("after") {
            None => None,
            Some(value) => Some(value.as_str().ok_or(CapError::bad_args())?),
        };
        let mut page: Vec<serde_json::Value> = Vec::new();
        let mut used = 0usize;
        let mut last_included: Option<String> = None;
        let mut more = false;
        for (key, value) in records.iter() {
            if let Some(prefix) = prefix
                && !key.starts_with(prefix)
            {
                continue;
            }
            if let Some(after) = after
                && key.as_str() <= after
            {
                continue;
            }
            // Count the entry as it will actually be serialized in
            // the result value, plus one byte for its array comma.
            let size = serde_json::to_vec(&serde_json::json!({"key": key, "value": value}))
                .map_or(usize::MAX, |entry| entry.len())
                + 1;
            if used + size > crate::world_caps::MAX_RECORDS_RESULT_PAGE_BYTES {
                if page.is_empty() {
                    return Err(CapError::bad_args());
                }
                more = true;
                break;
            }
            used += size;
            last_included = Some(key.clone());
            page.push(serde_json::json!({"key": key, "value": value}));
        }
        Ok(serde_json::json!({
            "records": page,
            "next_after": more.then_some(last_included).flatten(),
        }))
    }

    /// `kv.get` (#1062): the value stored under the key, or `null`
    /// when the key is absent. The store keeps each value's JSON
    /// encoding; a stored encoding that no longer parses means the
    /// journal is corrupt, so it fails rather than answering `null`.
    pub(super) fn execute_kv_get(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let key = args
            .get("key")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_kv_key(key).is_err() {
            return Err(CapError::bad_args());
        }
        let value = match self.core.kv.get(key) {
            None => serde_json::Value::Null,
            Some(stored) => {
                serde_json::from_str(stored).map_err(|_| CapError::failed_with("kv_corrupt"))?
            }
        };
        Ok(serde_json::json!({"key": key, "value": value}))
    }

    /// `kv.set` (#1062): store a JSON value under a key, replacing any
    /// value already there. The write is journaled before the result
    /// is delivered, and bounds are checked against the store as it
    /// will be after the write.
    pub(super) fn execute_kv_set(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let key = args
            .get("key")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        let value = args.get("value").ok_or(CapError::bad_args())?;
        let encoded = serde_json::to_string(value).map_err(|_| CapError::failed())?;
        if let Err(error) = self.core.check_kv_set(key, encoded.len()) {
            tracing::debug!("world {}: kv set refused: {error:#}", self.core.world_id);
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        let value_bytes = encoded.len();
        if self
            .persist(JournalEntry::KvSet {
                key: key.to_owned(),
                value: encoded.clone(),
            })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.kv.insert(key.to_owned(), encoded);
        Ok(serde_json::json!({"key": key, "value_bytes": value_bytes}))
    }

    /// `kv.delete` (#1062): remove the entry under a key. `deleted` is
    /// false when nothing was stored under it, and nothing is
    /// journaled in that case.
    pub(super) fn execute_kv_delete(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let key = args
            .get("key")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_kv_key(key).is_err() {
            return Err(CapError::bad_args());
        }
        if !self.core.kv.contains_key(key) {
            return Ok(serde_json::json!({"key": key, "deleted": false}));
        }
        if self
            .persist(JournalEntry::KvDeleted {
                key: key.to_owned(),
            })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.kv.remove(key);
        Ok(serde_json::json!({"key": key, "deleted": true}))
    }

    /// `blob.put` (#1062): store content under its iroh-blobs
    /// address. The write is journaled before the result is
    /// delivered; putting content the world already holds is a
    /// no-op that journals nothing, because the address is the same.
    pub(super) fn execute_blob_put(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let encoded = args
            .get("data")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        let bytes = super::super::blob_decode(encoded).ok_or(CapError::bad_args())?;
        let hash = super::super::blob_id(&bytes);
        if let Err(error) = self.core.check_blob_put(&hash, bytes.len()) {
            tracing::debug!("world {}: blob put refused: {error:#}", self.core.world_id);
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        let size = bytes.len();
        if !self.core.blobs.contains_key(&hash) {
            if self
                .persist(JournalEntry::BlobStored {
                    hash: hash.clone(),
                    data: super::super::blob_encode(&bytes),
                })
                .is_err()
            {
                return Err(CapError::failed());
            }
            self.core.blobs.insert(hash.clone(), bytes);
        }
        Ok(serde_json::json!({"hash": hash, "size": size}))
    }

    /// `blob.get` (#1062): the content stored under the address, or
    /// `null` data when the world holds no such blob.
    pub(super) fn execute_blob_get(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let hash = args
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_blob_hash(hash).is_err() {
            return Err(CapError::bad_args());
        }
        let data = match self.core.blobs.get(hash) {
            None => serde_json::Value::Null,
            Some(bytes) => serde_json::json!(super::super::blob_encode(bytes)),
        };
        Ok(serde_json::json!({"hash": hash, "data": data}))
    }

    /// `blob.delete` (#1062): remove the blob under an address.
    /// `deleted` is false when the world held no such blob, and
    /// nothing is journaled in that case.
    pub(super) fn execute_blob_delete(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let hash = args
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_blob_hash(hash).is_err() {
            return Err(CapError::bad_args());
        }
        if !self.core.blobs.contains_key(hash) {
            return Ok(serde_json::json!({"hash": hash, "deleted": false}));
        }
        if self
            .persist(JournalEntry::BlobRemoved {
                hash: hash.to_owned(),
            })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.blobs.remove(hash);
        Ok(serde_json::json!({"hash": hash, "deleted": true}))
    }

    /// `http.fetch` (#1062): a GET of an allowlisted URL. The core
    /// check refuses a malformed URL, a host outside the admin
    /// allowlist, and an exhausted rate window before any
    /// connection is made; the attempt is then recorded (failed
    /// fetches spend the budget too) and the fetch runs on its own
    /// thread. No fetch state is journaled: a fetch is an effect
    /// against the live network, not world state, so replay
    /// applies nothing for one.
    pub(super) fn execute_http_fetch(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if let Err(error) = self.core.check_http_fetch(url) {
            tracing::debug!(
                "world {}: http fetch refused: {error:#}",
                self.core.world_id
            );
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        self.core.note_http_fetch_attempt();
        let allowed_hosts = self.core.limits.http_fetch_hosts.clone();
        let outcome = super::super::http_fetch::fetch(url, &allowed_hosts)?;
        Ok(serde_json::json!({
            "url": outcome.url,
            "status": outcome.status,
            "body": super::super::blob_encode(&outcome.body),
            "body_bytes": outcome.body.len(),
        }))
    }

    /// `notify` (#1062): a user-visible outbound notification,
    /// committed to the world's event stream as a `Notification`
    /// event. The core check refuses an oversized or
    /// empty-after-sanitizing text and an exhausted rate window
    /// before anything is committed; the attempt is then recorded
    /// (a notification whose commit fails spends the budget too)
    /// and the event is journaled before the result is delivered,
    /// in the `chat.say` discipline — so a sent notification is a
    /// durable fact of the world's history, and the stale-wants
    /// rule keeps it at-most-once across a reopen.
    pub(super) fn execute_notify(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let Some(text) = args.get("text").and_then(serde_json::Value::as_str) else {
            return Err(CapError::bad_args());
        };
        if let Err(error) = self.core.check_notify(text) {
            tracing::debug!("world {}: notify refused: {error:#}", self.core.world_id);
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        self.core.note_notify_attempt();
        match self.core.prepare_notification(text) {
            Ok(event) => {
                if self
                    .persist(JournalEntry::Committed {
                        event: event.clone(),
                    })
                    .is_err()
                {
                    return Err(CapError::failed());
                }
                if self.core.commit_prepared(event.clone()).is_err() {
                    return Err(CapError::failed());
                }
                let _ = self.event_sender.send(event.clone());
                Ok(serde_json::json!({"sequence": event.sequence}))
            }
            Err(_) => Err(CapError::bad_args()),
        }
    }

    /// `chat.say` commits a host chat line; its result is the new sequence.
    pub(super) fn execute_chat(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let Some(text) = args.get("text").and_then(serde_json::Value::as_str) else {
            return Err(CapError::bad_args());
        };
        match self.core.prepare_system_chat(text) {
            Ok(event) => {
                if self
                    .persist(JournalEntry::Committed {
                        event: event.clone(),
                    })
                    .is_err()
                {
                    return Err(CapError::failed());
                }
                if self.core.commit_prepared(event.clone()).is_err() {
                    return Err(CapError::failed());
                }
                let _ = self.event_sender.send(event.clone());
                Ok(serde_json::json!({"sequence": event.sequence}))
            }
            Err(_) => Err(CapError::bad_args()),
        }
    }

    /// Accept (or refuse) one cross-world schedule fire arriving here
    /// (#1355 part 2). Acceptance requires this world unisolated, a
    /// receive grant for the source, and no receipt for the same
    /// (source, id, planned time) — a retry after the receipt is answered
    /// true without delivering again. The fire's `System` entry is
    /// journaled by `deliver`; the receipt follows it, so the source
    /// consumes its occurrence only once both are durable.
    pub(super) async fn accept_schedule_delivery(
        &mut self,
        source: &str,
        id: &str,
        at_ms: u64,
        late: bool,
    ) -> bool {
        if self.read_only {
            return false;
        }
        if self.core.isolation != Isolation::Unisolated {
            return false;
        }
        if !self
            .core
            .schedule_grants
            .get(source)
            .is_some_and(|grant| grant.receive)
        {
            return false;
        }
        let key = (source.to_owned(), id.to_owned(), at_ms);
        if self.core.schedule_receipts.contains(&key) {
            return true;
        }
        if !self
            .deliver(schedule_fired_event_from(id, late, Some(source)))
            .await
            .delivered()
        {
            return false;
        }
        if self
            .persist(JournalEntry::ScheduleReceived {
                source: source.to_owned(),
                id: id.to_owned(),
                at_ms,
            })
            .is_err()
        {
            return false;
        }
        self.core.schedule_receipts.insert(key);
        true
    }

    /// `schedule.at`: pend a fire of `id` at `at` (unix ms). With
    /// `interval_ms` and `count` the schedule is recurring (#1355): it
    /// re-arms fixed-rate after each fire until `count` total fires have
    /// happened. The schedule is journaled before the result is delivered.
    /// A refused request changes nothing but the usage count.
    pub(super) fn execute_schedule_at(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let id = args
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        let at_ms = args
            .get("at")
            .and_then(serde_json::Value::as_u64)
            .ok_or(CapError::bad_args())?;
        let interval_ms = args.get("interval_ms").and_then(serde_json::Value::as_u64);
        let count = match args.get("count") {
            None => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|count| u32::try_from(count).ok())
                    .ok_or(CapError::bad_args())?,
            ),
        };
        if args.get("interval_ms").is_some() && interval_ms.is_none() {
            return Err(CapError::bad_args());
        }
        let target = args.get("world").and_then(serde_json::Value::as_str);
        if args.get("world").is_some() && target.is_none() {
            return Err(CapError::bad_args());
        }
        if let Err(error) = self.core.check_schedule_targeted(
            id,
            at_ms,
            interval_ms,
            count,
            target,
            self.core.clock().now_ms(),
        ) {
            tracing::debug!("world {}: schedule refused: {error:#}", self.core.world_id);
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        if self
            .persist(JournalEntry::ScheduleSet {
                id: id.to_owned(),
                at_ms,
                interval_ms,
                remaining: count.filter(|_| interval_ms.is_some()),
                target: target.map(str::to_owned),
            })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.schedules.insert(
            id.to_owned(),
            PendingSchedule {
                at_ms,
                interval_ms,
                remaining: count.unwrap_or(1),
                target: target.map(str::to_owned),
            },
        );
        let mut result = serde_json::json!({"id": id, "fires_at": at_ms});
        if let (Some(interval), Some(count)) = (interval_ms, count) {
            result["interval_ms"] = serde_json::json!(interval);
            result["count"] = serde_json::json!(count);
        }
        if let Some(target) = target {
            result["world"] = serde_json::json!(target);
        }
        Ok(result)
    }

    /// `schedule.cancel`: drop a pending schedule before it fires. `cancelled`
    /// is false when nothing by that id is pending.
    pub(super) fn execute_schedule_cancel(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let id = args
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_schedule_id(id).is_err() {
            return Err(CapError::bad_args());
        }
        if !self.core.schedules.contains_key(id) {
            return Ok(serde_json::json!({"cancelled": false}));
        }
        if self
            .persist(JournalEntry::ScheduleCancelled { id: id.to_owned() })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.schedules.remove(id);
        self.overdue_at_open.remove(id);
        Ok(serde_json::json!({"cancelled": true}))
    }

    /// A participant opened or closed a session: presence transitions deliver
    /// `players` events and arm or disarm ticks.
    pub(super) async fn handle_presence(&mut self, participant: u64, joined: bool) {
        let was_present = self
            .presence
            .get(&participant)
            .is_some_and(|count| *count > 0);
        let count = self.presence.entry(participant).or_default();
        if joined {
            *count += 1;
        } else if *count > 0 {
            *count -= 1;
        }
        let now_present = *count > 0;
        if *count == 0 {
            self.presence.remove(&participant);
        }
        if now_present && !was_present && self.ticks.is_empty() {
            let intervals: Vec<u64> = self
                .active_subs
                .iter()
                .filter_map(|sub| match sub {
                    Subscription::Tick(ms)
                        if *ms >= self.core.limits.min_tick_ms
                            && self.core.ledger.is_granted(sub.cap()) =>
                    {
                        Some(*ms)
                    }
                    _ => None,
                })
                .collect();
            for ms in intervals {
                self.ticks.entry(ms).or_insert_with(|| {
                    self.core.clock().now_instant() + std::time::Duration::from_millis(ms)
                });
            }
        }
        if now_present && !was_present && self.loc_watches.is_empty() {
            let intervals: Vec<u64> = self
                .active_subs
                .iter()
                .filter_map(|sub| match sub {
                    Subscription::LocWatch(ms)
                        if self.location_consent == crate::world_caps::LocationConsent::Granted
                            && self.core.ledger.is_granted(sub.cap()) =>
                    {
                        Some(*ms)
                    }
                    _ => None,
                })
                .collect();
            for ms in intervals {
                self.loc_watches.entry(ms).or_insert_with(|| {
                    self.core.clock().now_instant() + std::time::Duration::from_millis(ms)
                });
            }
        }
        if self.presence.is_empty() {
            self.ticks.clear();
            self.loc_watches.clear();
        }
        if now_present != was_present
            && self.active_subs.contains(&Subscription::Players)
            && self.core.ledger.is_granted("players")
        {
            let name = self
                .core
                .participant(participant)
                .map_or_else(|| participant.to_string(), |p| p.display_name);
            self.deliver(presence_event(participant, &name, now_present))
                .await;
        }
    }

    pub(super) fn require_moderation(&self, authority: &Authority, what: &str) -> Result<()> {
        let directory = self.core.directory();
        let allowed = match authority {
            Authority::Admin => true,
            Authority::Signed(key) => directory
                .account_for_key(key)
                .is_some_and(|account| directory.is_moderator(&account)),
        };
        if allowed {
            Ok(())
        } else {
            Err(crate::directory::Refusal::Forbidden(format!(
                "only an admin or a moderator {what}"
            ))
            .into())
        }
    }

    pub(super) fn set_isolation(
        &mut self,
        authority: &Authority,
        isolation: Isolation,
    ) -> Result<Isolation> {
        self.check_writable()?;
        self.require_moderation(authority, "changes isolation")?;
        self.persist(JournalEntry::Isolation { isolation })?;
        self.core.isolation = isolation;
        Ok(isolation)
    }

    pub(super) fn set_cross_world_write(
        &mut self,
        authority: &Authority,
        enabled: bool,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes cross-world writes")?;
        if !enabled && self.core.cross_world_forced && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "the admin forced cross-world writes on; only the admin can turn them off"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWrite { enabled })?;
        self.core.cross_world_write = enabled;
        if !enabled {
            self.core.cross_world_forced = false;
        }
        Ok(self.core.policy())
    }

    pub(super) fn force_cross_world_write(&mut self, authority: &Authority) -> Result<WorldPolicy> {
        self.check_writable()?;
        if *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin forces cross-world writes".to_owned(),
            )
            .into());
        }
        if self.core.isolation == Isolation::Isolated {
            return Err(crate::directory::Refusal::Forbidden(
                "an isolated world cannot be forced to accept cross-world writes; unisolate it first"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWriteForced)?;
        self.core.cross_world_write = true;
        self.core.cross_world_forced = true;
        Ok(self.core.policy())
    }

    pub(super) fn set_file_scope(
        &mut self,
        authority: &Authority,
        scope: FileScope,
        write_override: Option<bool>,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes the file scope")?;
        if write_override.is_some() && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin sets the file write override".to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::FileScope { scope })?;
        self.core.file_scope = scope;
        if let Some(enabled) = write_override {
            self.persist(JournalEntry::FileWriteOverride { enabled })?;
            self.core.file_write_override = enabled;
        }
        Ok(self.core.policy())
    }

    /// Replace the world's capability ceiling. It takes effect now: uses and
    /// subscriptions the ceiling excludes are refused from here on. Grants are
    /// kept, so lifting the ceiling again restores them.
    pub(super) fn set_caps_ceiling(&mut self, ceiling: Vec<String>) -> Result<CapsReport> {
        self.check_writable()?;
        crate::world_caps::validate_ceiling_names(&ceiling)?;
        let ceiling: BTreeSet<String> = ceiling.into_iter().collect();
        self.persist(JournalEntry::CapsCeiling {
            ceiling: ceiling.iter().cloned().collect(),
        })?;
        self.core.caps_ceiling = ceiling;
        self.core.sync_ledger_ceiling();
        self.reconcile_subscriptions();
        self.caps_report()
    }

    /// Re-check subscriptions after an authority change. An armed one that is
    /// no longer granted is dropped and its tick disarmed. Every refused one is
    /// dropped too, so the next pass decides it again: a refusal that still
    /// holds is announced again, and one the change now grants is armed.
    pub(super) fn reconcile_subscriptions(&mut self) {
        let ledger = &self.core.ledger;
        let refused = &self.denied_subs;
        self.active_subs
            .retain(|sub| !refused.contains(&sub.spec()) && ledger.is_granted(sub.cap()));
        self.denied_subs.clear();
        let active = &self.active_subs;
        self.ticks
            .retain(|ms, _| active.contains(&Subscription::Tick(*ms)));
        // Location watches disarm alongside ticks (#2341, PR #8643
        // Codex P1): a watch whose capability authority was removed
        // must stop delivering readings, not merely lose its
        // subscription record.
        self.loc_watches
            .retain(|ms, _| active.contains(&Subscription::LocWatch(*ms)));
    }

    /// The capability report an admin sees.
    pub(super) fn caps_report(&mut self) -> Result<CapsReport> {
        let requested = match self.program.wants() {
            Ok(Some(text)) => crate::world_caps::parse_wants(&text)
                .map(|wants| wants.caps().into_iter().collect::<Vec<_>>())
                .unwrap_or_default(),
            Ok(None) => Vec::new(),
            Err(_) => Vec::new(),
        };
        let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
        let effective: Vec<String> = self.core.ledger.effective().into_iter().collect();
        let missing: Vec<String> = requested
            .iter()
            .filter(|cap| !effective.contains(cap))
            .cloned()
            .collect();
        Ok(CapsReport {
            granted,
            requested,
            missing,
            usage: self.core.ledger.usage().clone(),
            policy: if self.core.ledger.grant_on_use() {
                "grant-on-use"
            } else {
                "deny"
            }
            .to_owned(),
            catalog: crate::world_caps::PROVIDER_REGISTRY
                .ordered_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ceiling: self.core.caps_ceiling.iter().cloned().collect(),
            effective,
            view_failures: self.view_failures,
        })
    }
}
