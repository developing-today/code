//! Coordinator state machine for region-seam handoffs (#3870 phase 4,
//! design #2818 §5/§6/§8 packet 4).
//!
//! The coordinator owns no entity state. It journals handoff progress in
//! its own journal ([`CoordinatorJournalEntry`], fixed by phase 1),
//! serializes handoffs per entity, correlates the source and destination
//! receipts by `handoff_id` + entity + epoch + module/schema + digest,
//! and switches its owner map only after a verified destination import
//! is durable in its journal as [`CoordinatorJournalEntry::HandoffCompleted`]
//! (appended *before* the in-memory map changes, per design §3 step 5).
//!
//! Recovery is replay plus reconciliation: [`Coordinator::recover`]
//! rebuilds the exact state from the journal, and
//! [`Coordinator::reconcile`] re-queries both sides' receipts for an
//! in-flight handoff, appends any missing observation, and completes
//! only when both receipts match the journaled start. A handoff ID is
//! never reused or replaced; a timeout is stuck/in-transit, never a
//! completion and never a rollback (design §5).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::CoordinatorJournalEntry;

/// Age after which an exported handoff with no coordinator-observed
/// progress is classified `stuck` (design #2818 §4). Operator visibility
/// only; it changes no ownership or retry behavior.
pub const STUCK_THRESHOLD_MS: u64 = 60_000;

/// The source's durable, applied export receipt, as the coordinator
/// verifies it (design #2818 §3 step 3). Mirrors the correlation fields
/// of the source journal's `EntityExported` entry; the payload itself
/// never passes through the coordinator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportReceipt {
    /// Coordinator-generated handoff ID (hex); the receipt key.
    pub handoff_id: String,
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region exporting the entity.
    pub source_region: String,
    /// Region importing the entity.
    pub target_region: String,
    /// Owner epoch at the source before this handoff.
    pub prior_owner_epoch: u64,
    /// Program module hash the transfer ran under.
    pub program_module_hash: String,
    /// Versioned payload schema name.
    pub payload_schema: String,
    /// Digest of the exported payload.
    pub payload_digest: String,
}

/// The destination's durable, applied import receipt, as the
/// coordinator verifies it (design #2818 §3 step 5). Mirrors the
/// correlation fields of the destination journal's `EntityImported`
/// entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportReceipt {
    /// Coordinator-generated handoff ID (hex); the dedupe key.
    pub handoff_id: String,
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region that exported the entity.
    pub source_region: String,
    /// Region that imported the entity.
    pub target_region: String,
    /// Owner epoch at the source before this handoff.
    pub prior_owner_epoch: u64,
    /// Owner epoch at the destination (`prior_owner_epoch + 1`).
    pub new_owner_epoch: u64,
    /// Program module hash the transfer ran under.
    pub program_module_hash: String,
    /// Versioned payload schema name.
    pub payload_schema: String,
    /// Digest of the imported payload.
    pub payload_digest: String,
}

/// Where a region-seam entity currently lives in the coordinator's
/// owner map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityOwner {
    /// Region (shard world) that owns the entity.
    pub region: String,
    /// Current owner epoch; increments once per completed handoff.
    pub owner_epoch: u64,
}

/// How far an active handoff has progressed through the coordinator
/// journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffStage {
    /// `HandoffStarted` journaled; no export observed yet. The source
    /// remains the writable owner (design #2818 §4 `at_source`).
    Started,
    /// The source's durable export receipt was verified and journaled.
    ExportObserved,
    /// The destination's durable import receipt was verified and
    /// journaled; completion may be appended.
    ImportObserved,
}

/// One in-flight handoff as the coordinator tracks it. Rebuilt exactly
/// by journal replay in [`Coordinator::recover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveHandoff {
    /// Coordinator-generated handoff ID (hex), reused for every retry.
    pub handoff_id: String,
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region the entity leaves.
    pub source_region: String,
    /// Region the entity enters.
    pub target_region: String,
    /// Owner epoch the coordinator expects at the source.
    pub expected_owner_epoch: u64,
    /// Region-map epoch observed when the transfer was requested.
    pub region_map_epoch: u64,
    /// Program module hash both regions must run.
    pub program_module_hash: String,
    /// Versioned payload schema name.
    pub payload_schema: String,
    /// Current stage.
    pub stage: HandoffStage,
    /// Payload digest, known once the export is observed.
    pub payload_digest: Option<String>,
    /// Unix milliseconds the handoff was requested.
    pub requested_at: u64,
    /// Unix milliseconds of the latest journaled progress (start,
    /// export observation, or import observation). Drives the §4
    /// `stuck` classification; ordering never depends on it.
    pub last_progress_at: u64,
}

impl ActiveHandoff {
    /// Whether this handoff is `stuck` at `now_ms` (design #2818 §4):
    /// the source export committed and the latest coordinator-journaled
    /// progress is older than [`STUCK_THRESHOLD_MS`]. A started handoff
    /// with no export is never stuck; it is still `at_source`.
    #[must_use]
    pub fn is_stuck(&self, now_ms: u64) -> bool {
        self.stage != HandoffStage::Started
            && now_ms.saturating_sub(self.last_progress_at) > STUCK_THRESHOLD_MS
    }
}

/// The transit stage named in the status-only client response
/// (design #2818 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitStage {
    /// Source export committed; destination not yet routable.
    Exported,
    /// Destination import durable; coordinator completion pending.
    ImportedAwaitingCoordinator,
    /// In transit with no progress inside the alert threshold.
    Stuck,
}

/// Status-only in-transit response for an entity (design #2818 §4):
/// no payload, no success-shaped entity view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityInTransit {
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Handoff ID the client can use for status lookup/retry.
    pub handoff_id: String,
    /// Region the entity leaves.
    pub source_region: String,
    /// Region the entity enters.
    pub target_region: String,
    /// Transit stage.
    pub stage: TransitStage,
    /// Always true: the client retries the original command once the
    /// coordinator reports the entity at its destination.
    pub retryable: bool,
}

/// What a completed handoff changed, returned by
/// [`Coordinator::complete`] and reconciliation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffCompletion {
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region the entity left.
    pub source_region: String,
    /// Region that now owns the entity.
    pub target_region: String,
    /// Owner epoch at the destination (`prior + 1`).
    pub new_owner_epoch: u64,
    /// Region-map epoch after this completion.
    pub new_region_map_epoch: u64,
    /// Digest of the transferred payload.
    pub payload_digest: String,
}

/// Whether an observation call appended a journal entry or found the
/// same fact already recorded (at-least-once receipt delivery is
/// idempotent, design #2818 §7 invariant 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserveOutcome {
    /// A new journal entry was appended.
    Appended,
    /// The identical observation was already journaled; nothing changed.
    AlreadyRecorded,
}

/// Where reconciliation of an in-flight handoff stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Both receipts verified; `HandoffCompleted` journaled and the
    /// owner map switched.
    Completed(HandoffCompletion),
    /// No source export receipt is available yet; the entity remains
    /// at its source (or in transit once an export appears).
    AwaitingExport,
    /// The export is verified; the destination import receipt is not
    /// available yet. The entity is in transit.
    AwaitingImport,
    /// A destination import receipt exists but the source export
    /// receipt cannot be queried, so the pair cannot be correlated.
    /// Completion is never inferred from one side alone (design §5).
    AwaitingSourceReceipt,
}

/// Why a coordinator operation was refused. Every refusal leaves the
/// journal and the owner map untouched.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CoordinatorError {
    /// The entity has no entry in the coordinator's owner map.
    #[error("unknown entity {0}")]
    UnknownEntity(String),
    /// The entity already has an in-flight handoff; handoffs for one
    /// entity are serialized (design #2818 §6).
    #[error("entity {0} already has an active handoff")]
    HandoffAlreadyActive(String),
    /// No active handoff carries this ID.
    #[error("unknown handoff {0}")]
    UnknownHandoff(String),
    /// The requested target already owns the entity.
    #[error("target region already owns the entity")]
    SameRegion,
    /// A receipt field disagrees with the journaled `HandoffStarted`.
    #[error("receipt field {0} does not match the journaled handoff start")]
    ReceiptMismatch(&'static str),
    /// The same handoff ID was observed with two different payload
    /// digests: an integrity error, never an overwrite (design §5).
    #[error("payload digest conflict for handoff {handoff_id}")]
    DigestConflict {
        /// The conflicting handoff.
        handoff_id: String,
    },
    /// The operation is not legal at the handoff's current stage.
    #[error("invalid coordinator transition: {0}")]
    InvalidTransition(&'static str),
    /// Cancellation was requested after the source export committed
    /// (or cannot be proven absent); post-export there is no rollback.
    #[error("cannot cancel: the source export already committed")]
    ExportAlreadyCommitted,
    /// Journal replay found entries that cannot come from this state
    /// machine; automatic recovery stops (design §5).
    #[error("coordinator journal integrity error: {0}")]
    Integrity(String),
}

impl fmt::Display for HandoffStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Started => "started",
            Self::ExportObserved => "export_observed",
            Self::ImportObserved => "import_observed",
        };
        f.write_str(name)
    }
}

/// The region coordinator's state machine (design #2818 §6): journals
/// starts, observed receipts, and completion; serializes handoffs per
/// entity; correlates receipts; retries idempotently; reconciles after
/// restart; switches the owner map only through a journaled, verified
/// completion. It never applies entity state and never rolls back a
/// post-export transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coordinator {
    journal: Vec<CoordinatorJournalEntry>,
    owners: BTreeMap<String, EntityOwner>,
    active: BTreeMap<String, ActiveHandoff>,
    used_handoff_ids: BTreeSet<String>,
    region_map_epoch: u64,
}

impl Coordinator {
    /// A coordinator with an empty journal and owner map at the given
    /// region-map epoch. Seed owners with
    /// [`Coordinator::register_owner`], or rebuild from a journal with
    /// [`Coordinator::recover`].
    #[must_use]
    pub fn new(region_map_epoch: u64) -> Self {
        Self {
            journal: Vec::new(),
            owners: BTreeMap::new(),
            active: BTreeMap::new(),
            used_handoff_ids: BTreeSet::new(),
            region_map_epoch,
        }
    }

    /// Rebuild a coordinator by replaying its journal (design #2818
    /// §5: replay is deterministic and yields the same routing as the
    /// live run). `region_map_epoch` and `owners` describe the map as
    /// it stood when the journal began; completions in the journal
    /// advance both. Any entry sequence this state machine could not
    /// have produced is an [`CoordinatorError::Integrity`] error and
    /// recovery stops.
    pub fn recover(
        journal: Vec<CoordinatorJournalEntry>,
        region_map_epoch: u64,
        owners: impl IntoIterator<Item = (String, EntityOwner)>,
    ) -> Result<Self, CoordinatorError> {
        let mut coordinator = Self {
            journal: Vec::new(),
            owners: owners.into_iter().collect(),
            active: BTreeMap::new(),
            used_handoff_ids: BTreeSet::new(),
            region_map_epoch,
        };
        for entry in journal {
            coordinator.replay(&entry)?;
            coordinator.journal.push(entry);
        }
        Ok(coordinator)
    }

    /// The coordinator's journal, in append order.
    #[must_use]
    pub fn journal(&self) -> &[CoordinatorJournalEntry] {
        &self.journal
    }

    /// The current region-map epoch.
    #[must_use]
    pub fn region_map_epoch(&self) -> u64 {
        self.region_map_epoch
    }

    /// The recorded owner of an entity, if the coordinator tracks it.
    #[must_use]
    pub fn owner_of(&self, entity_ref: &str) -> Option<EntityOwner> {
        self.owners.get(entity_ref).cloned()
    }

    /// The in-flight handoff for an entity, if any.
    #[must_use]
    pub fn active_handoff(&self, entity_ref: &str) -> Option<&ActiveHandoff> {
        self.active.get(entity_ref)
    }

    /// Record an entity's current owner in the map (bootstrap and
    /// region-map seeding; the map's own journaling is the host's
    /// concern, design §6). Refused while a handoff for the entity is
    /// in flight.
    pub fn register_owner(
        &mut self,
        entity_ref: &str,
        region: &str,
        owner_epoch: u64,
    ) -> Result<(), CoordinatorError> {
        if self.active.contains_key(entity_ref) {
            return Err(CoordinatorError::InvalidTransition(
                "cannot re-register an entity with an active handoff",
            ));
        }
        self.owners.insert(
            entity_ref.to_string(),
            EntityOwner {
                region: region.to_string(),
                owner_epoch,
            },
        );
        Ok(())
    }

    /// Begin a handoff: append `HandoffStarted` before any shard side
    /// effect (design §3 step 1). The caller supplies the
    /// coordinator-generated handoff ID; it is reused for every retry
    /// of this crossing. Serialized per entity: a second start for the
    /// same entity while one is in flight is refused.
    pub fn start_handoff(
        &mut self,
        handoff_id: &str,
        entity_ref: &str,
        target_region: &str,
        program_module_hash: &str,
        payload_schema: &str,
        requested_at: u64,
    ) -> Result<(), CoordinatorError> {
        let owner = self
            .owners
            .get(entity_ref)
            .ok_or_else(|| CoordinatorError::UnknownEntity(entity_ref.to_string()))?;
        if self.active.contains_key(entity_ref) {
            return Err(CoordinatorError::HandoffAlreadyActive(
                entity_ref.to_string(),
            ));
        }
        if self.used_handoff_ids.contains(handoff_id) {
            return Err(CoordinatorError::InvalidTransition(
                "handoff id was already used by an earlier transfer",
            ));
        }
        if owner.region == target_region {
            return Err(CoordinatorError::SameRegion);
        }
        let entry = CoordinatorJournalEntry::HandoffStarted {
            handoff_id: handoff_id.to_string(),
            entity_ref: entity_ref.to_string(),
            source_region: owner.region.clone(),
            target_region: target_region.to_string(),
            expected_owner_epoch: owner.owner_epoch,
            region_map_epoch: self.region_map_epoch,
            program_module_hash: program_module_hash.to_string(),
            payload_schema: payload_schema.to_string(),
            requested_at,
        };
        self.journal.push(entry.clone());
        self.apply_started(&entry);
        Ok(())
    }

    /// Verify the source's export receipt and append
    /// `HandoffExportObserved` (design §3 step 3). Retrying with the
    /// identical receipt is a no-op success; a receipt that disagrees
    /// with the journaled start on any correlation field is refused,
    /// and a different digest for the same handoff is an integrity
    /// conflict.
    pub fn observe_export(
        &mut self,
        receipt: &ExportReceipt,
        observed_at: u64,
    ) -> Result<ObserveOutcome, CoordinatorError> {
        let active = self
            .active_by_id(&receipt.handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(receipt.handoff_id.clone()))?;
        check_receipt_start(
            active,
            &receipt.entity_ref,
            &receipt.source_region,
            &receipt.target_region,
            receipt.prior_owner_epoch,
            &receipt.program_module_hash,
            &receipt.payload_schema,
        )?;
        match active.stage {
            HandoffStage::Started => {
                self.journal
                    .push(CoordinatorJournalEntry::HandoffExportObserved {
                        handoff_id: receipt.handoff_id.clone(),
                        payload_digest: receipt.payload_digest.clone(),
                        observed_at,
                    });
                let active = self
                    .active_by_id_mut(&receipt.handoff_id)
                    .ok_or_else(|| CoordinatorError::UnknownHandoff(receipt.handoff_id.clone()))?;
                active.stage = HandoffStage::ExportObserved;
                active.payload_digest = Some(receipt.payload_digest.clone());
                active.last_progress_at = observed_at;
                Ok(ObserveOutcome::Appended)
            }
            HandoffStage::ExportObserved | HandoffStage::ImportObserved => {
                if active.payload_digest.as_deref() == Some(receipt.payload_digest.as_str()) {
                    Ok(ObserveOutcome::AlreadyRecorded)
                } else {
                    Err(CoordinatorError::DigestConflict {
                        handoff_id: receipt.handoff_id.clone(),
                    })
                }
            }
        }
    }

    /// Verify the destination's import receipt and append
    /// `HandoffImportObserved` (design §3 step 5). The export must
    /// already be observed: completion is never inferred from the
    /// destination alone. The import digest must equal the export
    /// digest and `new_owner_epoch` must be `prior + 1`.
    pub fn observe_import(
        &mut self,
        receipt: &ImportReceipt,
        observed_at: u64,
    ) -> Result<ObserveOutcome, CoordinatorError> {
        let active = self
            .active_by_id(&receipt.handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(receipt.handoff_id.clone()))?;
        check_receipt_start(
            active,
            &receipt.entity_ref,
            &receipt.source_region,
            &receipt.target_region,
            receipt.prior_owner_epoch,
            &receipt.program_module_hash,
            &receipt.payload_schema,
        )?;
        if receipt.new_owner_epoch != active.expected_owner_epoch + 1 {
            return Err(CoordinatorError::ReceiptMismatch("new_owner_epoch"));
        }
        match active.stage {
            HandoffStage::Started => Err(CoordinatorError::InvalidTransition(
                "import observed before the export observation",
            )),
            HandoffStage::ExportObserved => {
                if active.payload_digest.as_deref() != Some(receipt.payload_digest.as_str()) {
                    return Err(CoordinatorError::DigestConflict {
                        handoff_id: receipt.handoff_id.clone(),
                    });
                }
                self.journal
                    .push(CoordinatorJournalEntry::HandoffImportObserved {
                        handoff_id: receipt.handoff_id.clone(),
                        payload_digest: receipt.payload_digest.clone(),
                        observed_at,
                    });
                let active = self
                    .active_by_id_mut(&receipt.handoff_id)
                    .ok_or_else(|| CoordinatorError::UnknownHandoff(receipt.handoff_id.clone()))?;
                active.stage = HandoffStage::ImportObserved;
                active.last_progress_at = observed_at;
                Ok(ObserveOutcome::Appended)
            }
            HandoffStage::ImportObserved => {
                if active.payload_digest.as_deref() == Some(receipt.payload_digest.as_str()) {
                    Ok(ObserveOutcome::AlreadyRecorded)
                } else {
                    Err(CoordinatorError::DigestConflict {
                        handoff_id: receipt.handoff_id.clone(),
                    })
                }
            }
        }
    }

    /// Complete a handoff whose import has been observed: append
    /// `HandoffCompleted` first, then switch the owner map and bump
    /// both epochs (design §3 step 5). A handoff is complete iff the
    /// destination's matching import is durable and this closure
    /// record is durable.
    pub fn complete(&mut self, handoff_id: &str) -> Result<HandoffCompletion, CoordinatorError> {
        let active = self
            .active_by_id(handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(handoff_id.to_string()))?;
        if active.stage != HandoffStage::ImportObserved {
            return Err(CoordinatorError::InvalidTransition(
                "completion requires a verified import observation",
            ));
        }
        let payload_digest = active.payload_digest.clone().ok_or_else(|| {
            CoordinatorError::Integrity("import observed without a digest".into())
        })?;
        let completion = HandoffCompletion {
            entity_ref: active.entity_ref.clone(),
            source_region: active.source_region.clone(),
            target_region: active.target_region.clone(),
            new_owner_epoch: active.expected_owner_epoch + 1,
            new_region_map_epoch: self.region_map_epoch + 1,
            payload_digest: payload_digest.clone(),
        };
        let entry = CoordinatorJournalEntry::HandoffCompleted {
            handoff_id: handoff_id.to_string(),
            entity_ref: completion.entity_ref.clone(),
            source_region: completion.source_region.clone(),
            target_region: completion.target_region.clone(),
            payload_digest,
            new_owner_epoch: completion.new_owner_epoch,
            new_region_map_epoch: completion.new_region_map_epoch,
            program_module_hash: active.program_module_hash.clone(),
            payload_schema: active.payload_schema.clone(),
        };
        self.journal.push(entry);
        self.apply_completion(&completion);
        Ok(completion)
    }

    /// Cancel a handoff before the source export commits (design §3
    /// step 6 / §5). The caller must prove no export exists
    /// (`source_export_exists == false`, from querying the source by
    /// `handoff_id`); once an export is committed or cannot be proven
    /// absent, cancellation is refused — there is no rollback.
    pub fn cancel(
        &mut self,
        handoff_id: &str,
        reason: &str,
        source_export_exists: bool,
    ) -> Result<(), CoordinatorError> {
        let active = self
            .active_by_id(handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(handoff_id.to_string()))?;
        if source_export_exists || active.stage != HandoffStage::Started {
            return Err(CoordinatorError::ExportAlreadyCommitted);
        }
        let entity_ref = active.entity_ref.clone();
        self.journal
            .push(CoordinatorJournalEntry::HandoffCancelled {
                handoff_id: handoff_id.to_string(),
                entity_ref: entity_ref.clone(),
                reason: reason.to_string(),
            });
        self.active.remove(&entity_ref);
        Ok(())
    }

    /// Reconcile one in-flight handoff against freshly queried
    /// receipts after a restart (design §5): append any missing
    /// observation, then complete if both receipts verify. Never
    /// creates a new handoff ID and never completes from one side's
    /// receipt alone.
    pub fn reconcile(
        &mut self,
        handoff_id: &str,
        source_receipt: Option<&ExportReceipt>,
        destination_receipt: Option<&ImportReceipt>,
        observed_at: u64,
    ) -> Result<ReconcileOutcome, CoordinatorError> {
        let stage = self
            .active_by_id(handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(handoff_id.to_string()))?
            .stage;
        if let Some(receipt) = source_receipt {
            if receipt.handoff_id != handoff_id {
                return Err(CoordinatorError::ReceiptMismatch("handoff_id"));
            }
        }
        if let Some(receipt) = destination_receipt {
            if receipt.handoff_id != handoff_id {
                return Err(CoordinatorError::ReceiptMismatch("handoff_id"));
            }
        }
        if stage == HandoffStage::Started
            && source_receipt.is_none()
            && destination_receipt.is_some()
        {
            // The destination claims an import but the source's export
            // receipt cannot be queried to correlate it against.
            return Ok(ReconcileOutcome::AwaitingSourceReceipt);
        }
        if let Some(receipt) = source_receipt {
            self.observe_export(receipt, observed_at)?;
        }
        if let Some(receipt) = destination_receipt {
            self.observe_import(receipt, observed_at)?;
        }
        let stage = self
            .active_by_id(handoff_id)
            .ok_or_else(|| CoordinatorError::UnknownHandoff(handoff_id.to_string()))?
            .stage;
        match stage {
            HandoffStage::Started => Ok(ReconcileOutcome::AwaitingExport),
            HandoffStage::ExportObserved => Ok(ReconcileOutcome::AwaitingImport),
            HandoffStage::ImportObserved => {
                Ok(ReconcileOutcome::Completed(self.complete(handoff_id)?))
            }
        }
    }

    /// The status-only in-transit response for an entity (design §4),
    /// or `None` when the entity is not in transit: it is unknown, at
    /// its source (including a started handoff with no export yet), or
    /// already at its destination.
    #[must_use]
    pub fn in_transit_status(&self, entity_ref: &str, now_ms: u64) -> Option<EntityInTransit> {
        let active = self.active.get(entity_ref)?;
        if active.stage == HandoffStage::Started {
            return None;
        }
        let stage = if active.is_stuck(now_ms) {
            TransitStage::Stuck
        } else if active.stage == HandoffStage::ImportObserved {
            TransitStage::ImportedAwaitingCoordinator
        } else {
            TransitStage::Exported
        };
        Some(EntityInTransit {
            entity_ref: active.entity_ref.clone(),
            handoff_id: active.handoff_id.clone(),
            source_region: active.source_region.clone(),
            target_region: active.target_region.clone(),
            stage,
            retryable: true,
        })
    }

    fn active_by_id(&self, handoff_id: &str) -> Option<&ActiveHandoff> {
        self.active.values().find(|a| a.handoff_id == handoff_id)
    }

    fn active_by_id_mut(&mut self, handoff_id: &str) -> Option<&mut ActiveHandoff> {
        self.active
            .values_mut()
            .find(|a| a.handoff_id == handoff_id)
    }

    fn apply_started(&mut self, entry: &CoordinatorJournalEntry) {
        if let CoordinatorJournalEntry::HandoffStarted {
            handoff_id,
            entity_ref,
            source_region,
            target_region,
            expected_owner_epoch,
            region_map_epoch,
            program_module_hash,
            payload_schema,
            requested_at,
        } = entry
        {
            self.used_handoff_ids.insert(handoff_id.clone());
            self.active.insert(
                entity_ref.clone(),
                ActiveHandoff {
                    handoff_id: handoff_id.clone(),
                    entity_ref: entity_ref.clone(),
                    source_region: source_region.clone(),
                    target_region: target_region.clone(),
                    expected_owner_epoch: *expected_owner_epoch,
                    region_map_epoch: *region_map_epoch,
                    program_module_hash: program_module_hash.clone(),
                    payload_schema: payload_schema.clone(),
                    stage: HandoffStage::Started,
                    payload_digest: None,
                    requested_at: *requested_at,
                    last_progress_at: *requested_at,
                },
            );
        }
    }

    fn apply_completion(&mut self, completion: &HandoffCompletion) {
        self.owners.insert(
            completion.entity_ref.clone(),
            EntityOwner {
                region: completion.target_region.clone(),
                owner_epoch: completion.new_owner_epoch,
            },
        );
        self.region_map_epoch = completion.new_region_map_epoch;
        self.active.remove(&completion.entity_ref);
    }

    /// Fold one journaled entry into the in-memory state during
    /// recovery, validating it against the state machine's rules.
    fn replay(&mut self, entry: &CoordinatorJournalEntry) -> Result<(), CoordinatorError> {
        match entry {
            CoordinatorJournalEntry::HandoffStarted {
                handoff_id,
                entity_ref,
                source_region,
                target_region,
                expected_owner_epoch,
                region_map_epoch,
                ..
            } => {
                if self.active.contains_key(entity_ref) {
                    return Err(CoordinatorError::Integrity(format!(
                        "second handoff started for entity {entity_ref}"
                    )));
                }
                if self.used_handoff_ids.contains(handoff_id) {
                    return Err(CoordinatorError::Integrity(format!(
                        "handoff id {handoff_id} reused in the journal"
                    )));
                }
                let owner = self.owners.get(entity_ref).ok_or_else(|| {
                    CoordinatorError::Integrity(format!(
                        "handoff started for unknown entity {entity_ref}"
                    ))
                })?;
                if owner.region != *source_region
                    || owner.owner_epoch != *expected_owner_epoch
                    || *region_map_epoch != self.region_map_epoch
                    || source_region == target_region
                {
                    return Err(CoordinatorError::Integrity(format!(
                        "handoff start does not match recovered ownership for entity {entity_ref}"
                    )));
                }
                self.apply_started(entry);
                Ok(())
            }
            CoordinatorJournalEntry::HandoffExportObserved {
                handoff_id,
                payload_digest,
                observed_at,
            } => {
                let active = self.replay_active_mut(handoff_id)?;
                if active.stage != HandoffStage::Started {
                    return Err(CoordinatorError::Integrity(format!(
                        "export observed out of order for handoff {handoff_id}"
                    )));
                }
                active.stage = HandoffStage::ExportObserved;
                active.payload_digest = Some(payload_digest.clone());
                active.last_progress_at = *observed_at;
                Ok(())
            }
            CoordinatorJournalEntry::HandoffImportObserved {
                handoff_id,
                payload_digest,
                observed_at,
            } => {
                let active = self.replay_active_mut(handoff_id)?;
                if active.stage != HandoffStage::ExportObserved
                    || active.payload_digest.as_deref() != Some(payload_digest.as_str())
                {
                    return Err(CoordinatorError::Integrity(format!(
                        "import observed without a matching export for handoff {handoff_id}"
                    )));
                }
                active.stage = HandoffStage::ImportObserved;
                active.last_progress_at = *observed_at;
                Ok(())
            }
            CoordinatorJournalEntry::HandoffCompleted {
                handoff_id,
                entity_ref,
                source_region,
                target_region,
                payload_digest,
                new_owner_epoch,
                new_region_map_epoch,
                ..
            } => {
                let active = self.replay_active(handoff_id)?;
                if active.stage != HandoffStage::ImportObserved
                    || active.entity_ref != *entity_ref
                    || active.source_region != *source_region
                    || active.target_region != *target_region
                    || active.payload_digest.as_deref() != Some(payload_digest.as_str())
                    || *new_owner_epoch != active.expected_owner_epoch + 1
                    || *new_region_map_epoch != self.region_map_epoch + 1
                {
                    return Err(CoordinatorError::Integrity(format!(
                        "completion does not match the in-flight handoff {handoff_id}"
                    )));
                }
                let completion = HandoffCompletion {
                    entity_ref: entity_ref.clone(),
                    source_region: active.source_region.clone(),
                    target_region: target_region.clone(),
                    new_owner_epoch: *new_owner_epoch,
                    new_region_map_epoch: *new_region_map_epoch,
                    payload_digest: payload_digest.clone(),
                };
                self.apply_completion(&completion);
                Ok(())
            }
            CoordinatorJournalEntry::HandoffCancelled {
                handoff_id,
                entity_ref,
                ..
            } => {
                let active = self.replay_active(handoff_id)?;
                if active.stage != HandoffStage::Started || active.entity_ref != *entity_ref {
                    return Err(CoordinatorError::Integrity(format!(
                        "cancellation after export for handoff {handoff_id}"
                    )));
                }
                self.active.remove(entity_ref);
                Ok(())
            }
        }
    }

    fn replay_active(&self, handoff_id: &str) -> Result<&ActiveHandoff, CoordinatorError> {
        self.active_by_id(handoff_id).ok_or_else(|| {
            CoordinatorError::Integrity(format!("entry for unknown handoff {handoff_id}"))
        })
    }

    fn replay_active_mut(
        &mut self,
        handoff_id: &str,
    ) -> Result<&mut ActiveHandoff, CoordinatorError> {
        self.active_by_id_mut(handoff_id).ok_or_else(|| {
            CoordinatorError::Integrity(format!("entry for unknown handoff {handoff_id}"))
        })
    }
}

/// Correlate a receipt against the journaled `HandoffStarted` fields
/// (design #2818 §3: handoff_id + entity + epoch + module/schema).
#[allow(clippy::too_many_arguments)]
fn check_receipt_start(
    active: &ActiveHandoff,
    entity_ref: &str,
    source_region: &str,
    target_region: &str,
    prior_owner_epoch: u64,
    program_module_hash: &str,
    payload_schema: &str,
) -> Result<(), CoordinatorError> {
    if entity_ref != active.entity_ref {
        return Err(CoordinatorError::ReceiptMismatch("entity_ref"));
    }
    if source_region != active.source_region {
        return Err(CoordinatorError::ReceiptMismatch("source_region"));
    }
    if target_region != active.target_region {
        return Err(CoordinatorError::ReceiptMismatch("target_region"));
    }
    if prior_owner_epoch != active.expected_owner_epoch {
        return Err(CoordinatorError::ReceiptMismatch("prior_owner_epoch"));
    }
    if program_module_hash != active.program_module_hash {
        return Err(CoordinatorError::ReceiptMismatch("program_module_hash"));
    }
    if payload_schema != active.payload_schema {
        return Err(CoordinatorError::ReceiptMismatch("payload_schema"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const ENTITY: &str = "entity-1";
    const HANDOFF: &str = "0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "blake3:payload";

    fn coordinator() -> Coordinator {
        let mut c = Coordinator::new(7);
        c.register_owner(ENTITY, "region-a", 3).unwrap();
        c
    }

    fn started() -> Coordinator {
        let mut c = coordinator();
        c.start_handoff(
            HANDOFF,
            ENTITY,
            "region-b",
            "module-hash",
            "schema-v1",
            1_000,
        )
        .unwrap();
        c
    }

    fn export_receipt() -> ExportReceipt {
        ExportReceipt {
            handoff_id: HANDOFF.to_string(),
            entity_ref: ENTITY.to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 3,
            program_module_hash: "module-hash".to_string(),
            payload_schema: "schema-v1".to_string(),
            payload_digest: DIGEST.to_string(),
        }
    }

    fn import_receipt() -> ImportReceipt {
        ImportReceipt {
            handoff_id: HANDOFF.to_string(),
            entity_ref: ENTITY.to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 3,
            new_owner_epoch: 4,
            program_module_hash: "module-hash".to_string(),
            payload_schema: "schema-v1".to_string(),
            payload_digest: DIGEST.to_string(),
        }
    }

    fn exported() -> Coordinator {
        let mut c = started();
        c.observe_export(&export_receipt(), 2_000).unwrap();
        c
    }

    fn imported() -> Coordinator {
        let mut c = exported();
        c.observe_import(&import_receipt(), 3_000).unwrap();
        c
    }

    #[test]
    fn happy_path_switches_owner_only_on_completion() {
        let mut c = imported();
        // Before completion the source still owns the entity.
        assert_eq!(c.owner_of(ENTITY).unwrap().region, "region-a");
        assert_eq!(c.owner_of(ENTITY).unwrap().owner_epoch, 3);
        let completion = c.complete(HANDOFF).unwrap();
        assert_eq!(completion.new_owner_epoch, 4);
        assert_eq!(completion.new_region_map_epoch, 8);
        assert_eq!(c.owner_of(ENTITY).unwrap().region, "region-b");
        assert_eq!(c.owner_of(ENTITY).unwrap().owner_epoch, 4);
        assert_eq!(c.region_map_epoch(), 8);
        assert!(c.active_handoff(ENTITY).is_none());
        assert_eq!(c.journal().len(), 4);
        // Completion is journaled before the map change is visible and
        // cannot be applied twice.
        assert!(matches!(
            c.journal().last(),
            Some(CoordinatorJournalEntry::HandoffCompleted { .. })
        ));
        assert_eq!(
            c.complete(HANDOFF),
            Err(CoordinatorError::UnknownHandoff(HANDOFF.to_string()))
        );
    }

    #[test]
    fn handoffs_are_serialized_per_entity_only() {
        let mut c = started();
        assert_eq!(
            c.start_handoff(
                "aaaa",
                ENTITY,
                "region-c",
                "module-hash",
                "schema-v1",
                1_100
            ),
            Err(CoordinatorError::HandoffAlreadyActive(ENTITY.to_string()))
        );
        // A different entity may cross concurrently.
        c.register_owner("entity-2", "region-a", 0).unwrap();
        c.start_handoff(
            "bbbb",
            "entity-2",
            "region-b",
            "module-hash",
            "schema-v1",
            1_100,
        )
        .unwrap();
    }

    #[test]
    fn start_refuses_unknown_entity_and_same_region() {
        let mut c = coordinator();
        assert_eq!(
            c.start_handoff(HANDOFF, "nope", "region-b", "m", "s", 1_000),
            Err(CoordinatorError::UnknownEntity("nope".to_string()))
        );
        assert_eq!(
            c.start_handoff(HANDOFF, ENTITY, "region-a", "m", "s", 1_000),
            Err(CoordinatorError::SameRegion)
        );
        assert!(c.journal().is_empty());
    }

    #[test]
    fn receipt_correlation_rejects_mismatched_fields() {
        let mut c = started();
        let mut wrong_module = export_receipt();
        wrong_module.program_module_hash = "other".to_string();
        assert_eq!(
            c.observe_export(&wrong_module, 2_000),
            Err(CoordinatorError::ReceiptMismatch("program_module_hash"))
        );
        let mut wrong_epoch = export_receipt();
        wrong_epoch.prior_owner_epoch = 99;
        assert_eq!(
            c.observe_export(&wrong_epoch, 2_000),
            Err(CoordinatorError::ReceiptMismatch("prior_owner_epoch"))
        );
        // Refusals leave the journal with only the start entry.
        assert_eq!(c.journal().len(), 1);
    }

    #[test]
    fn import_requires_export_and_matching_digest_and_epoch() {
        let mut c = started();
        // Import before any export observation: refused, no completion
        // is ever inferred from the destination alone.
        assert!(matches!(
            c.observe_import(&import_receipt(), 2_000),
            Err(CoordinatorError::InvalidTransition(_))
        ));
        let mut c = exported();
        let mut wrong_digest = import_receipt();
        wrong_digest.payload_digest = "blake3:other".to_string();
        assert_eq!(
            c.observe_import(&wrong_digest, 3_000),
            Err(CoordinatorError::DigestConflict {
                handoff_id: HANDOFF.to_string()
            })
        );
        let mut wrong_epoch = import_receipt();
        wrong_epoch.new_owner_epoch = 9;
        assert_eq!(
            c.observe_import(&wrong_epoch, 3_000),
            Err(CoordinatorError::ReceiptMismatch("new_owner_epoch"))
        );
    }

    #[test]
    fn duplicate_receipts_are_idempotent_but_conflicts_are_not() {
        let mut c = exported();
        assert_eq!(
            c.observe_export(&export_receipt(), 2_500).unwrap(),
            ObserveOutcome::AlreadyRecorded
        );
        assert_eq!(c.journal().len(), 2);
        let mut conflicting = export_receipt();
        conflicting.payload_digest = "blake3:other".to_string();
        assert_eq!(
            c.observe_export(&conflicting, 2_600),
            Err(CoordinatorError::DigestConflict {
                handoff_id: HANDOFF.to_string()
            })
        );
        let mut c = imported();
        assert_eq!(
            c.observe_import(&import_receipt(), 3_500).unwrap(),
            ObserveOutcome::AlreadyRecorded
        );
        assert_eq!(c.journal().len(), 3);
    }

    #[test]
    fn cancel_is_only_legal_before_export() {
        let mut c = started();
        // The caller cannot prove the export is absent.
        assert_eq!(
            c.cancel(HANDOFF, "operator", true),
            Err(CoordinatorError::ExportAlreadyCommitted)
        );
        c.cancel(HANDOFF, "operator", false).unwrap();
        assert!(c.active_handoff(ENTITY).is_none());
        assert_eq!(c.owner_of(ENTITY).unwrap().region, "region-a");

        let mut c = exported();
        assert_eq!(
            c.cancel(HANDOFF, "operator", false),
            Err(CoordinatorError::ExportAlreadyCommitted)
        );
    }

    #[test]
    fn restart_at_each_transition_recovers_the_same_state() {
        // Failure-injection matrix (design §5): kill the coordinator
        // after every protocol transition and rebuild from the journal.
        for prefix in 1..=4 {
            let live = match prefix {
                1 => started(),
                2 => exported(),
                3 => imported(),
                _ => {
                    let mut c = imported();
                    c.complete(HANDOFF).unwrap();
                    c
                }
            };
            let journal_prefix = live.journal()[..prefix].to_vec();
            let recovered = Coordinator::recover(
                journal_prefix,
                7,
                [(
                    ENTITY.to_string(),
                    EntityOwner {
                        region: "region-a".to_string(),
                        owner_epoch: 3,
                    },
                )],
            )
            .unwrap();
            assert_eq!(
                recovered.owner_of(ENTITY),
                live.owner_of(ENTITY),
                "prefix {prefix}"
            );
            assert_eq!(
                recovered.active_handoff(ENTITY).map(|a| a.stage),
                live.active_handoff(ENTITY).map(|a| a.stage),
                "prefix {prefix}"
            );
            assert_eq!(
                recovered.region_map_epoch(),
                live.region_map_epoch(),
                "prefix {prefix}"
            );
        }
    }

    #[test]
    fn reconcile_after_restart_completes_from_both_receipts() {
        // Coordinator died after B imported, before completion (§5).
        let live = exported();
        let mut recovered = Coordinator::recover(
            live.journal().to_vec(),
            7,
            [(
                ENTITY.to_string(),
                EntityOwner {
                    region: "region-a".to_string(),
                    owner_epoch: 3,
                },
            )],
        )
        .unwrap();
        let outcome = recovered
            .reconcile(
                HANDOFF,
                Some(&export_receipt()),
                Some(&import_receipt()),
                4_000,
            )
            .unwrap();
        assert!(matches!(outcome, ReconcileOutcome::Completed(_)));
        assert_eq!(recovered.owner_of(ENTITY).unwrap().region, "region-b");
        assert_eq!(recovered.owner_of(ENTITY).unwrap().owner_epoch, 4);
    }

    #[test]
    fn reconcile_never_completes_from_one_side_alone() {
        // Only the destination receipt is queryable after a restart at
        // the started stage: correlation is impossible, nothing moves.
        let live = started();
        let mut recovered = Coordinator::recover(
            live.journal().to_vec(),
            7,
            [(
                ENTITY.to_string(),
                EntityOwner {
                    region: "region-a".to_string(),
                    owner_epoch: 3,
                },
            )],
        )
        .unwrap();
        let outcome = recovered
            .reconcile(HANDOFF, None, Some(&import_receipt()), 4_000)
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::AwaitingSourceReceipt);
        assert_eq!(recovered.owner_of(ENTITY).unwrap().region, "region-a");
        assert_eq!(recovered.journal().len(), 1);

        // Only the export receipt: observed, still awaiting import.
        let outcome = recovered
            .reconcile(HANDOFF, Some(&export_receipt()), None, 4_100)
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::AwaitingImport);
        assert_eq!(recovered.owner_of(ENTITY).unwrap().region, "region-a");
    }

    #[test]
    fn stuck_classification_follows_design_section_4() {
        // Started with no export: at_source forever, never stuck.
        let c = started();
        assert!(c.in_transit_status(ENTITY, 10_000_000).is_none());
        // Exported recently: in transit, Exported stage.
        let c = exported();
        let status = c.in_transit_status(ENTITY, 2_500).unwrap();
        assert_eq!(status.stage, TransitStage::Exported);
        assert!(status.retryable);
        // Exported long ago with no progress: stuck.
        let status = c
            .in_transit_status(ENTITY, 2_000 + STUCK_THRESHOLD_MS + 1)
            .unwrap();
        assert_eq!(status.stage, TransitStage::Stuck);
        // Imported awaiting coordinator: its own stage until the
        // threshold passes.
        let c = imported();
        let status = c.in_transit_status(ENTITY, 3_500).unwrap();
        assert_eq!(status.stage, TransitStage::ImportedAwaitingCoordinator);
    }

    #[test]
    fn replay_rejects_impossible_journals() {
        // Completion without an import observation.
        let mut journal = started().journal().to_vec();
        journal.push(CoordinatorJournalEntry::HandoffCompleted {
            handoff_id: HANDOFF.to_string(),
            entity_ref: ENTITY.to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            payload_digest: DIGEST.to_string(),
            new_owner_epoch: 4,
            new_region_map_epoch: 8,
            program_module_hash: "module-hash".to_string(),
            payload_schema: "schema-v1".to_string(),
        });
        assert!(matches!(
            Coordinator::recover(
                journal,
                7,
                [(
                    ENTITY.to_string(),
                    EntityOwner {
                        region: "region-a".to_string(),
                        owner_epoch: 3,
                    }
                )]
            ),
            Err(CoordinatorError::Integrity(_))
        ));
        // Import observed with a digest that differs from the export.
        let mut journal = exported().journal().to_vec();
        journal.push(CoordinatorJournalEntry::HandoffImportObserved {
            handoff_id: HANDOFF.to_string(),
            payload_digest: "blake3:other".to_string(),
            observed_at: 3_000,
        });
        assert!(matches!(
            Coordinator::recover(
                journal,
                7,
                [(
                    ENTITY.to_string(),
                    EntityOwner {
                        region: "region-a".to_string(),
                        owner_epoch: 3,
                    }
                )]
            ),
            Err(CoordinatorError::Integrity(_))
        ));
    }

    #[test]
    fn owner_epoch_increments_once_per_completed_handoff() {
        let mut c = imported();
        c.complete(HANDOFF).unwrap();
        // The next crossing starts from the new epoch and lands at +1
        // again; the old owner can never become writable by replay.
        c.start_handoff(
            "fedcba9876543210fedcba9876543210",
            ENTITY,
            "region-a",
            "module-hash",
            "schema-v1",
            5_000,
        )
        .unwrap();
        let active = c.active_handoff(ENTITY).unwrap();
        assert_eq!(active.expected_owner_epoch, 4);
        assert_eq!(active.source_region, "region-b");
    }

    #[test]
    fn handoff_ids_are_single_use_across_terminal_transitions() {
        // Completed ID cannot name the return crossing: the shard
        // ledgers dedupe on historical IDs.
        let mut c = imported();
        c.complete(HANDOFF).unwrap();
        assert!(matches!(
            c.start_handoff(
                HANDOFF,
                ENTITY,
                "region-a",
                "module-hash",
                "schema-v1",
                5_000
            ),
            Err(CoordinatorError::InvalidTransition(_))
        ));
        // Cancelled IDs are single-use too.
        let mut c = started();
        c.cancel(HANDOFF, "operator", false).unwrap();
        assert!(matches!(
            c.start_handoff(
                HANDOFF,
                ENTITY,
                "region-b",
                "module-hash",
                "schema-v1",
                5_000
            ),
            Err(CoordinatorError::InvalidTransition(_))
        ));
        // And recovery preserves the used set: a journal whose second
        // start reuses an ID is an integrity error.
        let mut journal = imported().journal().to_vec();
        journal.push(CoordinatorJournalEntry::HandoffStarted {
            handoff_id: HANDOFF.to_string(),
            entity_ref: ENTITY.to_string(),
            source_region: "region-b".to_string(),
            target_region: "region-a".to_string(),
            expected_owner_epoch: 4,
            region_map_epoch: 8,
            program_module_hash: "module-hash".to_string(),
            payload_schema: "schema-v1".to_string(),
            requested_at: 5_000,
        });
        let mut with_completion = imported();
        with_completion.complete(HANDOFF).unwrap();
        let mut full = with_completion.journal().to_vec();
        full.push(journal.pop().unwrap());
        assert!(matches!(
            Coordinator::recover(full, 7, [seed_owner()]),
            Err(CoordinatorError::Integrity(_))
        ));
    }

    #[test]
    fn reconcile_rejects_receipts_for_another_handoff() {
        let mut c = coordinator();
        c.register_owner("entity-2", "region-a", 0).unwrap();
        c.start_handoff(
            HANDOFF,
            ENTITY,
            "region-b",
            "module-hash",
            "schema-v1",
            1_000,
        )
        .unwrap();
        c.start_handoff(
            "h2",
            "entity-2",
            "region-b",
            "module-hash",
            "schema-v1",
            1_000,
        )
        .unwrap();
        let other = ExportReceipt {
            handoff_id: "h2".to_string(),
            entity_ref: "entity-2".to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 0,
            program_module_hash: "module-hash".to_string(),
            payload_schema: "schema-v1".to_string(),
            payload_digest: DIGEST.to_string(),
        };
        assert_eq!(
            c.reconcile(HANDOFF, Some(&other), None, 2_000),
            Err(CoordinatorError::ReceiptMismatch("handoff_id"))
        );
        // Neither handoff moved.
        assert_eq!(
            c.active_handoff(ENTITY).unwrap().stage,
            HandoffStage::Started
        );
        assert_eq!(
            c.active_handoff("entity-2").unwrap().stage,
            HandoffStage::Started
        );
        assert_eq!(c.journal().len(), 2);
    }

    #[test]
    fn replay_rejects_a_start_that_disagrees_with_recovered_ownership() {
        // A stale/corrupted start claiming epoch 1 against an owner
        // seeded at epoch 3 must stop recovery, not roll ownership
        // backward when it completes.
        let mut journal = started().journal().to_vec();
        if let CoordinatorJournalEntry::HandoffStarted {
            expected_owner_epoch,
            ..
        } = &mut journal[0]
        {
            *expected_owner_epoch = 1;
        }
        assert!(matches!(
            Coordinator::recover(journal, 7, [seed_owner()]),
            Err(CoordinatorError::Integrity(_))
        ));
        // Wrong source region is rejected the same way.
        let mut journal = started().journal().to_vec();
        if let CoordinatorJournalEntry::HandoffStarted { source_region, .. } = &mut journal[0] {
            *source_region = "region-z".to_string();
        }
        assert!(matches!(
            Coordinator::recover(journal, 7, [seed_owner()]),
            Err(CoordinatorError::Integrity(_))
        ));
    }

    fn seed_owner() -> (String, EntityOwner) {
        (
            ENTITY.to_string(),
            EntityOwner {
                region: "region-a".to_string(),
                owner_epoch: 3,
            },
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod model_tests {
    //! Model-based property tests for the coordinator (design #2818
    //! §7 invariants; AGENTS.md testing doctrine: state machines with
    //! meaningful invariants get generated sequences checked against a
    //! reference model, and every crash prefix must replay to the same
    //! state). The oracle is the miniature model below plus three
    //! named invariants: replay equivalence, monotonic ownership
    //! epochs, and handoff-ID uniqueness.

    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    const SEED_MAP_EPOCH: u64 = 7;
    const ENTITIES: [&str; 2] = ["e0", "e1"];
    const IDS: [&str; 3] = ["h0", "h1", "h2"];
    const REGIONS: [&str; 3] = ["region-a", "region-b", "region-c"];

    fn seed_owners() -> Vec<(String, EntityOwner)> {
        vec![
            (
                "e0".to_string(),
                EntityOwner {
                    region: "region-a".to_string(),
                    owner_epoch: 3,
                },
            ),
            (
                "e1".to_string(),
                EntityOwner {
                    region: "region-a".to_string(),
                    owner_epoch: 0,
                },
            ),
        ]
    }

    fn live_coordinator() -> Coordinator {
        let mut c = Coordinator::new(SEED_MAP_EPOCH);
        for (entity, owner) in seed_owners() {
            c.register_owner(&entity, &owner.region, owner.owner_epoch)
                .unwrap();
        }
        c
    }

    fn digest_for(id: &str) -> String {
        format!("digest-{id}")
    }

    #[derive(Clone, Debug)]
    struct MActive {
        id: String,
        source: String,
        target: String,
        epoch: u64,
        stage: HandoffStage,
        digest: Option<String>,
    }

    #[derive(Clone, Debug)]
    struct Model {
        owners: BTreeMap<String, (String, u64)>,
        active: BTreeMap<String, MActive>,
        used: std::collections::BTreeSet<String>,
        map_epoch: u64,
    }

    impl Model {
        fn new() -> Self {
            Self {
                owners: seed_owners()
                    .into_iter()
                    .map(|(e, o)| (e, (o.region, o.owner_epoch)))
                    .collect(),
                active: BTreeMap::new(),
                used: std::collections::BTreeSet::new(),
                map_epoch: SEED_MAP_EPOCH,
            }
        }

        fn active_by_id(&self, id: &str) -> Option<(&String, &MActive)> {
            self.active.iter().find(|(_, a)| a.id == id)
        }
    }

    fn export_receipt(active: &MActive, entity: &str, digest: String) -> ExportReceipt {
        ExportReceipt {
            handoff_id: active.id.clone(),
            entity_ref: entity.to_string(),
            source_region: active.source.clone(),
            target_region: active.target.clone(),
            prior_owner_epoch: active.epoch,
            program_module_hash: "m".to_string(),
            payload_schema: "s".to_string(),
            payload_digest: digest,
        }
    }

    fn import_receipt(active: &MActive, entity: &str, digest: String) -> ImportReceipt {
        ImportReceipt {
            handoff_id: active.id.clone(),
            entity_ref: entity.to_string(),
            source_region: active.source.clone(),
            target_region: active.target.clone(),
            prior_owner_epoch: active.epoch,
            new_owner_epoch: active.epoch + 1,
            program_module_hash: "m".to_string(),
            payload_schema: "s".to_string(),
            payload_digest: digest,
        }
    }

    fn dummy_export(id: &str) -> ExportReceipt {
        ExportReceipt {
            handoff_id: id.to_string(),
            entity_ref: "e0".to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 0,
            program_module_hash: "m".to_string(),
            payload_schema: "s".to_string(),
            payload_digest: digest_for(id),
        }
    }

    fn dummy_import(id: &str) -> ImportReceipt {
        ImportReceipt {
            handoff_id: id.to_string(),
            entity_ref: "e0".to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 0,
            new_owner_epoch: 1,
            program_module_hash: "m".to_string(),
            payload_schema: "s".to_string(),
            payload_digest: digest_for(id),
        }
    }

    /// Assert the live coordinator and the model agree on everything
    /// observable: owners, active handoffs (id + stage), map epoch.
    fn assert_in_sync(c: &Coordinator, m: &Model, ctx: &str) {
        assert_eq!(
            c.region_map_epoch(),
            m.map_epoch,
            "map epoch diverged: {ctx}"
        );
        for entity in ENTITIES {
            let (region, epoch) = &m.owners[entity];
            let owner = c.owner_of(entity).expect("model owner missing live");
            assert_eq!(
                &owner.region, region,
                "owner region diverged for {entity}: {ctx}"
            );
            assert_eq!(
                owner.owner_epoch, *epoch,
                "owner epoch diverged for {entity}: {ctx}"
            );
            match (c.active_handoff(entity), m.active.get(entity)) {
                (None, None) => {}
                (Some(a), Some(ma)) => {
                    assert_eq!(
                        a.handoff_id, ma.id,
                        "active id diverged for {entity}: {ctx}"
                    );
                    assert_eq!(
                        a.stage, ma.stage,
                        "active stage diverged for {entity}: {ctx}"
                    );
                }
                (a, b) => assert!(
                    false,
                    "active presence diverged for {entity}: live={a:?} model={b:?}: {ctx}"
                ),
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn coordinator_model_sequences(
            ops in proptest::collection::vec((0u8..7, 0u8..8, 0u8..8, 0u8..8), 0..48)
        ) {
            let mut c = live_coordinator();
            let mut m = Model::new();
            let mut prev_map_epoch = m.map_epoch;
            let mut prev_epochs: BTreeMap<String, u64> =
                m.owners.iter().map(|(e, (_, ep))| (e.clone(), *ep)).collect();
            let mut prev_journal_len = 0usize;

            for (step, (kind, x, y, z)) in ops.iter().enumerate() {
                let t = 1_000 + (step as u64) * 1_000;
                let ctx = format!("step {step} op ({kind},{x},{y},{z})");
                match kind {
                    0 => {
                        // Start a handoff.
                        let entity = ENTITIES[(*x as usize) % ENTITIES.len()];
                        let id = IDS[(*y as usize) % IDS.len()];
                        let target = REGIONS[(*z as usize) % REGIONS.len()];
                        let (region, epoch) = m.owners[entity].clone();
                        let ok = !m.active.contains_key(entity)
                            && !m.used.contains(id)
                            && target != region.as_str();
                        let result = c.start_handoff(id, entity, target, "m", "s", t);
                        prop_assert_eq!(result.is_ok(), ok, "start outcome: {}", ctx);
                        if ok {
                            m.used.insert(id.to_string());
                            m.active.insert(
                                entity.to_string(),
                                MActive {
                                    id: id.to_string(),
                                    source: region,
                                    target: target.to_string(),
                                    epoch,
                                    stage: HandoffStage::Started,
                                    digest: None,
                                },
                            );
                        }
                    }
                    1 => {
                        // Observe an export receipt.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let bogus = z % 4 == 0;
                        match m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone())) {
                            Some((entity, active)) => {
                                let digest = if bogus {
                                    "bogus".to_string()
                                } else {
                                    digest_for(id)
                                };
                                // At Started any digest is accepted and
                                // becomes the recorded one; later stages
                                // must match it exactly.
                                let ok = match active.stage {
                                    HandoffStage::Started => true,
                                    _ => active.digest.as_deref() == Some(digest.as_str()),
                                };
                                let result = c.observe_export(
                                    &export_receipt(&active, &entity, digest.clone()),
                                    t,
                                );
                                prop_assert_eq!(result.is_ok(), ok, "export outcome: {}", ctx);
                                if ok && active.stage == HandoffStage::Started {
                                    let a = m.active.get_mut(&entity).unwrap();
                                    a.stage = HandoffStage::ExportObserved;
                                    a.digest = Some(digest);
                                }
                            }
                            None => {
                                let result = c.observe_export(&dummy_export(id), t);
                                prop_assert!(result.is_err(), "export for unknown id: {}", ctx);
                            }
                        }
                    }
                    2 => {
                        // Observe an import receipt.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let bogus = z % 4 == 0;
                        match m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone())) {
                            Some((entity, active)) => {
                                let digest = if bogus {
                                    "bogus".to_string()
                                } else {
                                    digest_for(id)
                                };
                                let ok = match active.stage {
                                    HandoffStage::Started => false,
                                    HandoffStage::ExportObserved => {
                                        active.digest.as_deref() == Some(digest.as_str())
                                    }
                                    HandoffStage::ImportObserved => {
                                        active.digest.as_deref() == Some(digest.as_str())
                                    }
                                };
                                let result = c.observe_import(
                                    &import_receipt(&active, &entity, digest),
                                    t,
                                );
                                prop_assert_eq!(result.is_ok(), ok, "import outcome: {}", ctx);
                                if ok && active.stage == HandoffStage::ExportObserved {
                                    m.active.get_mut(&entity).unwrap().stage =
                                        HandoffStage::ImportObserved;
                                }
                            }
                            None => {
                                let result = c.observe_import(&dummy_import(id), t);
                                prop_assert!(result.is_err(), "import for unknown id: {}", ctx);
                            }
                        }
                    }
                    3 => {
                        // Complete.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let found = m
                            .active_by_id(id)
                            .map(|(e, a)| (e.clone(), a.clone()));
                        let ok = matches!(&found, Some((_, a)) if a.stage == HandoffStage::ImportObserved);
                        let result = c.complete(id);
                        prop_assert_eq!(result.is_ok(), ok, "complete outcome: {}", ctx);
                        if ok {
                            let (entity, active) = found.unwrap();
                            m.owners.insert(
                                entity.clone(),
                                (active.target.clone(), active.epoch + 1),
                            );
                            m.map_epoch += 1;
                            m.active.remove(&entity);
                        }
                    }
                    4 => {
                        // Cancel; the caller sometimes cannot prove the
                        // export is absent.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let export_exists = z % 2 == 0;
                        let found = m
                            .active_by_id(id)
                            .map(|(e, a)| (e.clone(), a.clone()));
                        let ok = matches!(&found, Some((_, a)) if a.stage == HandoffStage::Started)
                            && !export_exists;
                        let result = c.cancel(id, "prop", export_exists);
                        prop_assert_eq!(result.is_ok(), ok, "cancel outcome: {}", ctx);
                        if ok {
                            let (entity, _) = found.unwrap();
                            m.active.remove(&entity);
                        }
                    }
                    5 => {
                        // Crash + recover: replay must reproduce the
                        // live coordinator exactly.
                        let recovered = Coordinator::recover(
                            c.journal().to_vec(),
                            SEED_MAP_EPOCH,
                            seed_owners(),
                        )
                        .unwrap();
                        prop_assert_eq!(&recovered, &c, "replay equivalence: {}", ctx);
                        c = recovered;
                    }
                    _ => {
                        // Correlate garbage: a receipt whose schema
                        // disagrees with the journaled start is refused
                        // and changes nothing.
                        let id = IDS[(*x as usize) % IDS.len()];
                        if let Some((entity, active)) =
                            m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone()))
                        {
                            let mut receipt = export_receipt(&active, &entity, digest_for(id));
                            receipt.payload_schema = "other-schema".to_string();
                            let before = c.journal().len();
                            let result = c.observe_export(&receipt, t);
                            prop_assert!(result.is_err(), "schema mismatch refused: {}", ctx);
                            prop_assert_eq!(c.journal().len(), before, "journal untouched: {}", ctx);
                        }
                    }
                }

                // Invariants after every step.
                assert_in_sync(&c, &m, &ctx);
                prop_assert!(
                    c.region_map_epoch() >= prev_map_epoch,
                    "map epoch went backward: {}",
                    ctx
                );
                prev_map_epoch = c.region_map_epoch();
                for entity in ENTITIES {
                    let epoch = c.owner_of(entity).unwrap().owner_epoch;
                    prop_assert!(epoch >= prev_epochs[entity], "owner epoch went backward: {}", ctx);
                    prev_epochs.insert(entity.to_string(), epoch);
                }
                prop_assert!(c.journal().len() >= prev_journal_len, "journal shrank: {}", ctx);
                prev_journal_len = c.journal().len();
                let started_ids: Vec<&str> = c
                    .journal()
                    .iter()
                    .filter_map(|e| match e {
                        CoordinatorJournalEntry::HandoffStarted { handoff_id, .. } => {
                            Some(handoff_id.as_str())
                        }
                        _ => None,
                    })
                    .collect();
                let unique: std::collections::BTreeSet<&str> = started_ids.iter().copied().collect();
                prop_assert_eq!(started_ids.len(), unique.len(), "handoff id reused: {}", ctx);
            }

            // Every crash prefix of the final journal replays cleanly.
            let journal = c.journal().to_vec();
            for prefix in 0..=journal.len() {
                let recovered =
                    Coordinator::recover(journal[..prefix].to_vec(), SEED_MAP_EPOCH, seed_owners())
                        .unwrap();
                if prefix == journal.len() {
                    prop_assert_eq!(&recovered, &c, "full-journal replay equivalence");
                }
            }
        }
    }
}
