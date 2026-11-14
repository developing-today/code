//! Client/operations surface for region-seam handoffs (#3870 phase 5,
//! design #2818 §4/§6/§8 packet 5).
//!
//! Phases 1–4 built the journal format, the source fence, the
//! destination import primitive, and the [`Coordinator`] state
//! machine, but nothing a client or an operator could call: the
//! coordinator was not wired to any routing decision, there was no
//! feature gate, and there was no observability beyond the raw
//! journal. This module is that surface. It owns no entity state and
//! journals nothing of its own; every decision is derived from the
//! coordinator's owner map and active handoffs, so replaying the
//! coordinator journal reproduces the exact same routing (design §7
//! invariant 4).
//!
//! Three packet-5 obligations live here:
//!
//! - **Gated enablement.** [`HandoffOperations::new`] starts with
//!   handoffs **disabled** — the design's "handoffs disabled by
//!   default until the suite passes" (§8 packet 5). Starting a
//!   handoff while disabled is refused with
//!   [`OpsError::HandoffsDisabled`] and journals nothing. Enabling is
//!   an explicit operator action ([`HandoffOperations::set_enabled`]);
//!   routing and status for handoffs already in flight stay available
//!   while disabled, so turning the gate off never strands an
//!   in-transit entity without a status answer.
//! - **Fail-closed client routing.** [`HandoffOperations::route_operation`]
//!   is the one decision a client operation goes through. While an
//!   entity is in transit, *both* reads and writes are rejected with
//!   the typed, retryable [`EntityInTransit`] status (design §4:
//!   entity data reads are rejected because the host has no entity
//!   snapshot projection; commands are never queued or replayed). A
//!   client presenting a route the coordinator's map no longer agrees
//!   with — the classic case is the old source route after a
//!   completion — gets a typed [`StaleRoute`] response carrying the
//!   **new owner epoch**, so it can re-route through the coordinator
//!   instead of writing to the fenced source (design §4, §7
//!   invariants 1 and 5).
//! - **Age/reason observability.** [`HandoffOperations::ops_snapshot`]
//!   exposes every active handoff with its age, time since last
//!   journaled progress, stage, and a machine-readable reason, plus
//!   the stuck count (progress older than
//!   [`super::STUCK_THRESHOLD_MS`]) that alerting keys off. A stuck
//!   handoff is an alert classification only: the snapshot never
//!   completes, rolls back, or re-routes anything (design §4/§5).

use super::{
    ActiveHandoff, Coordinator, CoordinatorError, EntityInTransit, HandoffStage, STUCK_THRESHOLD_MS,
};

/// Whether a client operation reads or writes entity data. In transit
/// both fail closed (design #2818 §4); the distinction exists so the
/// routing decision — and its tests — name the legal-operations table
/// explicitly instead of implying reads might be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// An entity data read.
    Read,
    /// An entity data write/mutation.
    Write,
}

/// The route a client believes an entity lives at, as presented with
/// an operation (learned from an earlier response or a cached
/// region map).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRoute {
    /// Region the client addressed.
    pub region: String,
    /// Owner epoch the client addressed.
    pub owner_epoch: u64,
}

/// Typed stale-route response (design #2818 §4): the presented route
/// is not the current owner — most importantly a client still holding
/// the source route after a handoff completed. Carries the new owner
/// epoch so the client can detect the epoch bump and re-route through
/// the coordinator; the old owner never resumes writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleRoute {
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region the client presented.
    pub presented_region: String,
    /// Owner epoch the client presented.
    pub presented_owner_epoch: u64,
    /// Region that currently owns the entity.
    pub current_region: String,
    /// The new (current) owner epoch at `current_region`.
    pub new_owner_epoch: u64,
    /// Region-map epoch the current map was read at.
    pub region_map_epoch: u64,
    /// Always true: re-route through the coordinator and retry.
    pub retryable: bool,
}

/// The routing decision for one client operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteDecision {
    /// Serve the operation at this region/owner epoch (the entity is
    /// at its source — including a started handoff with no export
    /// yet — or at its destination after completion).
    Serve {
        /// Region that owns the entity.
        region: String,
        /// Current owner epoch there.
        owner_epoch: u64,
    },
    /// Fail closed: the entity is in transit. Neither reads nor
    /// writes are served and nothing is queued; the payload is the
    /// status-only transit response (no entity data, no
    /// success-shaped view).
    InTransit(EntityInTransit),
    /// The presented route is stale; re-route via the coordinator.
    StaleRoute(StaleRoute),
}

/// Why an operations-surface call was refused. Every refusal leaves
/// the coordinator journal and owner map untouched.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpsError {
    /// Handoffs are feature-gated off (the default) on this surface.
    #[error("region-seam handoffs are disabled by the feature gate")]
    HandoffsDisabled,
    /// The coordinator refused the underlying operation.
    #[error(transparent)]
    Coordinator(#[from] CoordinatorError),
}

/// One active handoff in the operations snapshot: the age/reason row
/// an operator (or an alert rule) reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffOpsEntry {
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Coordinator-generated handoff ID (hex).
    pub handoff_id: String,
    /// Region the entity leaves.
    pub source_region: String,
    /// Region the entity enters.
    pub target_region: String,
    /// How far the handoff has progressed.
    pub stage: HandoffStage,
    /// Milliseconds since the handoff was requested.
    pub age_ms: u64,
    /// Milliseconds since the latest coordinator-journaled progress.
    pub since_last_progress_ms: u64,
    /// Whether the handoff is classified `stuck` (export committed,
    /// no progress inside [`STUCK_THRESHOLD_MS`]).
    pub stuck: bool,
    /// Machine-readable reason for the current state: one of
    /// `awaiting_source_export`, `awaiting_destination_import`,
    /// `awaiting_coordinator_completion`, or
    /// `stuck_no_progress_over_threshold` (which overrides the
    /// awaiting-* reason for the stage once the threshold passes).
    pub reason: &'static str,
}

/// A point-in-time operations view over the coordinator: gate state,
/// totals alerting keys off, and one [`HandoffOpsEntry`] per active
/// handoff (ordered by entity reference, as the coordinator stores
/// them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffOpsSnapshot {
    /// Whether the feature gate currently allows new handoffs.
    pub handoffs_enabled: bool,
    /// The coordinator's current region-map epoch.
    pub region_map_epoch: u64,
    /// Handoffs past the export fence (the entities failing closed).
    pub in_transit_count: usize,
    /// Active handoffs classified stuck — the alerting signal.
    pub stuck_count: usize,
    /// One row per active handoff, stuck or not.
    pub entries: Vec<HandoffOpsEntry>,
}

/// The phase-5 client/operations surface over a [`Coordinator`].
///
/// The surface is deliberately thin: the coordinator stays the only
/// state machine and the only journal writer. Driving steps a caller
/// performs on behalf of a handoff (observing receipts, completing,
/// reconciling after a restart) go through
/// [`HandoffOperations::coordinator_mut`] and keep the coordinator's
/// own refusal semantics; this type adds the gate, the routing
/// decision, and the observability the packet requires.
#[derive(Clone, Debug)]
pub struct HandoffOperations {
    coordinator: Coordinator,
    handoffs_enabled: bool,
}

impl HandoffOperations {
    /// A surface over `coordinator` with the feature gate **off**:
    /// handoffs are disabled by default (design §8 packet 5) until an
    /// operator explicitly enables them after the acceptance suite
    /// passes.
    #[must_use]
    pub fn new(coordinator: Coordinator) -> Self {
        Self {
            coordinator,
            handoffs_enabled: false,
        }
    }

    /// A surface with the gate in the given state. Prefer
    /// [`HandoffOperations::new`] plus an explicit
    /// [`HandoffOperations::set_enabled`] at the point an operator
    /// turns the flow on, so enablement is a visible act.
    #[must_use]
    pub fn with_gate(coordinator: Coordinator, handoffs_enabled: bool) -> Self {
        Self {
            coordinator,
            handoffs_enabled,
        }
    }

    /// Whether new handoffs may be started through this surface.
    #[must_use]
    pub fn handoffs_enabled(&self) -> bool {
        self.handoffs_enabled
    }

    /// Flip the feature gate. Disabling refuses *new* handoffs only;
    /// in-flight handoffs keep their routing/status answers and can
    /// still be driven to completion through the coordinator.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.handoffs_enabled = enabled;
    }

    /// Read access to the wrapped coordinator (owner map, journal,
    /// in-transit status).
    #[must_use]
    pub fn coordinator(&self) -> &Coordinator {
        &self.coordinator
    }

    /// Mutable access to the wrapped coordinator for the driving
    /// steps (receipt observation, completion, reconciliation,
    /// cancellation) and owner-map seeding.
    pub fn coordinator_mut(&mut self) -> &mut Coordinator {
        &mut self.coordinator
    }

    /// Begin a handoff through the feature gate. Refused with
    /// [`OpsError::HandoffsDisabled`] — journaling nothing — while
    /// the gate is off; otherwise exactly
    /// [`Coordinator::start_handoff`].
    #[allow(clippy::too_many_arguments)]
    pub fn begin_handoff(
        &mut self,
        handoff_id: &str,
        entity_ref: &str,
        target_region: &str,
        program_module_hash: &str,
        payload_schema: &str,
        requested_at: u64,
    ) -> Result<(), OpsError> {
        if !self.handoffs_enabled {
            return Err(OpsError::HandoffsDisabled);
        }
        self.coordinator.start_handoff(
            handoff_id,
            entity_ref,
            target_region,
            program_module_hash,
            payload_schema,
            requested_at,
        )?;
        Ok(())
    }

    /// The status-only in-transit response for an entity, or `None`
    /// when it is not in transit (delegates to
    /// [`Coordinator::in_transit_status`]; available regardless of
    /// the gate so an in-flight handoff is never stranded without a
    /// status answer).
    #[must_use]
    pub fn in_transit_status(&self, entity_ref: &str, now_ms: u64) -> Option<EntityInTransit> {
        self.coordinator.in_transit_status(entity_ref, now_ms)
    }

    /// Decide one client operation (design §4's legal-operations
    /// table):
    ///
    /// - in transit (export committed, completion not journaled):
    ///   reads *and* writes fail closed with
    ///   [`RouteDecision::InTransit`]; nothing is queued or replayed;
    /// - otherwise the current owner serves — unless the client
    ///   presented a route that disagrees with the owner map on
    ///   region or owner epoch, which is answered
    ///   [`RouteDecision::StaleRoute`] carrying the new owner epoch;
    /// - an entity the coordinator does not track is
    ///   [`CoordinatorError::UnknownEntity`], never a guess.
    pub fn route_operation(
        &self,
        entity_ref: &str,
        presented: Option<&ClientRoute>,
        _kind: OperationKind,
        now_ms: u64,
    ) -> Result<RouteDecision, OpsError> {
        if let Some(transit) = self.coordinator.in_transit_status(entity_ref, now_ms) {
            return Ok(RouteDecision::InTransit(transit));
        }
        let owner = self
            .coordinator
            .owner_of(entity_ref)
            .ok_or_else(|| CoordinatorError::UnknownEntity(entity_ref.to_string()))?;
        if let Some(presented) = presented {
            if presented.region != owner.region || presented.owner_epoch != owner.owner_epoch {
                return Ok(RouteDecision::StaleRoute(StaleRoute {
                    entity_ref: entity_ref.to_string(),
                    presented_region: presented.region.clone(),
                    presented_owner_epoch: presented.owner_epoch,
                    current_region: owner.region.clone(),
                    new_owner_epoch: owner.owner_epoch,
                    region_map_epoch: self.coordinator.region_map_epoch(),
                    retryable: true,
                }));
            }
        }
        Ok(RouteDecision::Serve {
            region: owner.region,
            owner_epoch: owner.owner_epoch,
        })
    }

    /// The age/reason observability snapshot (see the module docs).
    /// Purely derived from coordinator state at `now_ms`; taking a
    /// snapshot mutates nothing and classifies nothing into a state
    /// change — `stuck` here is the same alert classification as
    /// [`ActiveHandoff::is_stuck`].
    #[must_use]
    pub fn ops_snapshot(&self, now_ms: u64) -> HandoffOpsSnapshot {
        // `Coordinator` exposes active handoffs per entity; the
        // snapshot enumerates the entities this surface can be asked
        // about via the owner map keys the coordinator reports.
        let mut entries = Vec::new();
        for entity_ref in self.coordinator.tracked_entities() {
            if let Some(active) = self.coordinator.active_handoff(&entity_ref) {
                entries.push(ops_entry(active, now_ms));
            }
        }
        let in_transit_count = entries
            .iter()
            .filter(|e| e.stage != HandoffStage::Started)
            .count();
        let stuck_count = entries.iter().filter(|e| e.stuck).count();
        HandoffOpsSnapshot {
            handoffs_enabled: self.handoffs_enabled,
            region_map_epoch: self.coordinator.region_map_epoch(),
            in_transit_count,
            stuck_count,
            entries,
        }
    }
}

/// Build one snapshot row for an active handoff at `now_ms`.
fn ops_entry(active: &ActiveHandoff, now_ms: u64) -> HandoffOpsEntry {
    let stuck = active.is_stuck(now_ms);
    let reason = if stuck {
        "stuck_no_progress_over_threshold"
    } else {
        match active.stage {
            HandoffStage::Started => "awaiting_source_export",
            HandoffStage::ExportObserved => "awaiting_destination_import",
            HandoffStage::ImportObserved => "awaiting_coordinator_completion",
        }
    };
    HandoffOpsEntry {
        entity_ref: active.entity_ref.clone(),
        handoff_id: active.handoff_id.clone(),
        source_region: active.source_region.clone(),
        target_region: active.target_region.clone(),
        stage: active.stage,
        age_ms: now_ms.saturating_sub(active.requested_at),
        since_last_progress_ms: now_ms.saturating_sub(active.last_progress_at),
        stuck,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{ExportReceipt, ImportReceipt, ObserveOutcome, TransitStage};

    const MODULE: &str = "module-hash-1";
    const SCHEMA: &str = "entity-payload/v1";
    const DIGEST: &str = "digest-1";

    fn export_receipt(handoff_id: &str, entity_ref: &str) -> ExportReceipt {
        ExportReceipt {
            handoff_id: handoff_id.to_string(),
            entity_ref: entity_ref.to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 7,
            program_module_hash: MODULE.to_string(),
            payload_schema: SCHEMA.to_string(),
            payload_digest: DIGEST.to_string(),
        }
    }

    fn import_receipt(handoff_id: &str, entity_ref: &str) -> ImportReceipt {
        ImportReceipt {
            handoff_id: handoff_id.to_string(),
            entity_ref: entity_ref.to_string(),
            source_region: "region-a".to_string(),
            target_region: "region-b".to_string(),
            prior_owner_epoch: 7,
            new_owner_epoch: 8,
            program_module_hash: MODULE.to_string(),
            payload_schema: SCHEMA.to_string(),
            payload_digest: DIGEST.to_string(),
        }
    }

    /// A surface with two entities owned by region-a at epoch 7 and
    /// the gate enabled (enablement is explicit in every test that
    /// starts a handoff).
    fn surface() -> HandoffOperations {
        let mut coordinator = Coordinator::new(3);
        coordinator
            .register_owner("entity-1", "region-a", 7)
            .unwrap();
        coordinator
            .register_owner("entity-2", "region-a", 7)
            .unwrap();
        HandoffOperations::with_gate(coordinator, true)
    }

    fn route(ops: &HandoffOperations, entity: &str, kind: OperationKind) -> RouteDecision {
        ops.route_operation(entity, None, kind, 1_000).unwrap()
    }

    #[test]
    fn gate_is_off_by_default_and_refuses_without_journaling() {
        let mut ops = HandoffOperations::new(Coordinator::new(3));
        ops.coordinator_mut()
            .register_owner("entity-1", "region-a", 7)
            .unwrap();
        assert!(!ops.handoffs_enabled());
        let err = ops
            .begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap_err();
        assert_eq!(err, OpsError::HandoffsDisabled);
        // Nothing journaled, no handoff started, routing unaffected.
        assert!(ops.coordinator().journal().is_empty());
        assert!(ops.coordinator().active_handoff("entity-1").is_none());
        assert_eq!(
            route(&ops, "entity-1", OperationKind::Write),
            RouteDecision::Serve {
                region: "region-a".to_string(),
                owner_epoch: 7
            }
        );
        // Explicit enablement is the only way in.
        ops.set_enabled(true);
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();
        assert!(ops.coordinator().active_handoff("entity-1").is_some());
    }

    #[test]
    fn started_handoff_without_export_still_serves_at_source() {
        let mut ops = surface();
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();
        // Design §4: at_source includes a journaled HandoffStarted;
        // the source remains the writable owner until the fence.
        for kind in [OperationKind::Read, OperationKind::Write] {
            assert_eq!(
                route(&ops, "entity-1", kind),
                RouteDecision::Serve {
                    region: "region-a".to_string(),
                    owner_epoch: 7
                }
            );
        }
        assert!(ops.in_transit_status("entity-1", 1_000).is_none());
        let snap = ops.ops_snapshot(1_500);
        assert_eq!(snap.in_transit_count, 0);
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.entries[0].reason, "awaiting_source_export");
        assert_eq!(snap.entries[0].age_ms, 500);
    }

    #[test]
    fn in_transit_fails_closed_for_reads_and_writes() {
        let mut ops = surface();
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();
        ops.coordinator_mut()
            .observe_export(&export_receipt("h1", "entity-1"), 2_000)
            .unwrap();
        for kind in [OperationKind::Read, OperationKind::Write] {
            let decision = ops.route_operation("entity-1", None, kind, 3_000).unwrap();
            let RouteDecision::InTransit(transit) = decision else {
                panic!("expected fail-closed in-transit, got {decision:?}");
            };
            assert_eq!(transit.stage, TransitStage::Exported);
            assert_eq!(transit.handoff_id, "h1");
            assert!(transit.retryable);
        }
        // The untouched entity keeps serving throughout.
        assert_eq!(
            route(&ops, "entity-2", OperationKind::Write),
            RouteDecision::Serve {
                region: "region-a".to_string(),
                owner_epoch: 7
            }
        );
    }

    #[test]
    fn stuck_classification_and_ops_snapshot_age_reason() {
        let mut ops = surface();
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();
        ops.coordinator_mut()
            .observe_export(&export_receipt("h1", "entity-1"), 2_000)
            .unwrap();
        // 60s threshold: at exactly the boundary not stuck, past it stuck.
        let at_boundary = ops.ops_snapshot(2_000 + STUCK_THRESHOLD_MS);
        assert_eq!(at_boundary.stuck_count, 0);
        let now = 2_000 + STUCK_THRESHOLD_MS + 1;
        let snap = ops.ops_snapshot(now);
        assert_eq!(snap.stuck_count, 1);
        assert_eq!(snap.in_transit_count, 1);
        let entry = &snap.entries[0];
        assert!(entry.stuck);
        assert_eq!(entry.reason, "stuck_no_progress_over_threshold");
        assert_eq!(entry.age_ms, now - 1_000);
        assert_eq!(entry.since_last_progress_ms, now - 2_000);
        // The typed client response carries the Stuck stage too, and
        // stuck changes no routing: still fail-closed, never served.
        let decision = ops
            .route_operation("entity-1", None, OperationKind::Read, now)
            .unwrap();
        let RouteDecision::InTransit(transit) = decision else {
            panic!("expected in-transit, got {decision:?}");
        };
        assert_eq!(transit.stage, TransitStage::Stuck);
    }

    #[test]
    fn acceptance_end_to_end_no_loss_no_duplicate_and_stale_route() {
        // The packet-5 acceptance suite at the operations surface:
        // one crossing driven export -> import -> completion, with a
        // duplicate delivery in the middle, proves no duplicate
        // entity/transition, no accepted write at the source after
        // export, and no completion without a matching durable
        // import; the stale source route afterwards is answered with
        // the new owner epoch.
        let mut ops = surface();
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();

        // No completion without a matching durable import.
        assert!(ops.coordinator_mut().complete("h1").is_err());

        ops.coordinator_mut()
            .observe_export(&export_receipt("h1", "entity-1"), 2_000)
            .unwrap();
        // Still no completion from the export alone.
        assert!(ops.coordinator_mut().complete("h1").is_err());
        // No accepted write at A after export (fail closed), for the
        // whole transit window.
        assert!(matches!(
            ops.route_operation("entity-1", None, OperationKind::Write, 2_500)
                .unwrap(),
            RouteDecision::InTransit(_)
        ));

        // At-least-once delivery, exactly-once effect: the same
        // export receipt delivered twice records once.
        let again = ops
            .coordinator_mut()
            .observe_export(&export_receipt("h1", "entity-1"), 2_600)
            .unwrap();
        assert_eq!(again, ObserveOutcome::AlreadyRecorded);

        ops.coordinator_mut()
            .observe_import(&import_receipt("h1", "entity-1"), 3_000)
            .unwrap();
        // Import observed but completion not journaled: still in
        // transit, at the ImportedAwaitingCoordinator stage.
        let decision = ops
            .route_operation("entity-1", None, OperationKind::Read, 3_500)
            .unwrap();
        let RouteDecision::InTransit(transit) = decision else {
            panic!("expected in-transit, got {decision:?}");
        };
        assert_eq!(transit.stage, TransitStage::ImportedAwaitingCoordinator);

        let completion = ops.coordinator_mut().complete("h1").unwrap();
        assert_eq!(completion.new_owner_epoch, 8);
        // Exactly one completion and one owner transition in the journal.
        let journal = ops.coordinator().journal();
        assert_eq!(
            journal
                .iter()
                .filter(|e| matches!(
                    e,
                    crate::world::CoordinatorJournalEntry::HandoffCompleted { .. }
                ))
                .count(),
            1
        );

        // A client still holding the source route gets the typed
        // stale-route response with the NEW owner epoch.
        let stale = ops
            .route_operation(
                "entity-1",
                Some(&ClientRoute {
                    region: "region-a".to_string(),
                    owner_epoch: 7,
                }),
                OperationKind::Write,
                4_000,
            )
            .unwrap();
        let RouteDecision::StaleRoute(stale) = stale else {
            panic!("expected stale-route, got {stale:?}");
        };
        assert_eq!(stale.current_region, "region-b");
        assert_eq!(stale.new_owner_epoch, 8);
        assert!(stale.retryable);

        // Re-routed operations serve at the destination, epoch 8 —
        // and only there: owner epochs are monotonic, the old owner
        // never serves again.
        assert_eq!(
            ops.route_operation(
                "entity-1",
                Some(&ClientRoute {
                    region: "region-b".to_string(),
                    owner_epoch: 8,
                }),
                OperationKind::Write,
                4_000,
            )
            .unwrap(),
            RouteDecision::Serve {
                region: "region-b".to_string(),
                owner_epoch: 8
            }
        );

        // Replay reproduces the same routing (no loss, no duplicate):
        // a coordinator recovered from the journal serves region-b/8
        // and has no active handoff.
        let recovered = Coordinator::recover(
            ops.coordinator().journal().to_vec(),
            3,
            [
                (
                    "entity-1".to_string(),
                    crate::world::EntityOwner {
                        region: "region-a".to_string(),
                        owner_epoch: 7,
                    },
                ),
                (
                    "entity-2".to_string(),
                    crate::world::EntityOwner {
                        region: "region-a".to_string(),
                        owner_epoch: 7,
                    },
                ),
            ],
        )
        .unwrap();
        let replayed = HandoffOperations::with_gate(recovered, true);
        assert_eq!(
            replayed
                .route_operation("entity-1", None, OperationKind::Write, 4_000)
                .unwrap(),
            RouteDecision::Serve {
                region: "region-b".to_string(),
                owner_epoch: 8
            }
        );
        assert!(replayed.ops_snapshot(4_000).entries.is_empty());
    }

    #[test]
    fn unknown_entity_is_an_error_not_a_route() {
        let ops = surface();
        let err = ops
            .route_operation("nope", None, OperationKind::Read, 1_000)
            .unwrap_err();
        assert_eq!(
            err,
            OpsError::Coordinator(CoordinatorError::UnknownEntity("nope".to_string()))
        );
    }

    #[test]
    fn disabling_the_gate_strands_nothing() {
        let mut ops = surface();
        ops.begin_handoff("h1", "entity-1", "region-b", MODULE, SCHEMA, 1_000)
            .unwrap();
        ops.coordinator_mut()
            .observe_export(&export_receipt("h1", "entity-1"), 2_000)
            .unwrap();
        ops.set_enabled(false);
        // New starts refused; the in-flight handoff still answers
        // status, still fails closed, and can still complete.
        assert_eq!(
            ops.begin_handoff("h2", "entity-2", "region-b", MODULE, SCHEMA, 2_000)
                .unwrap_err(),
            OpsError::HandoffsDisabled
        );
        assert!(ops.in_transit_status("entity-1", 3_000).is_some());
        ops.coordinator_mut()
            .observe_import(&import_receipt("h1", "entity-1"), 3_000)
            .unwrap();
        ops.coordinator_mut().complete("h1").unwrap();
        assert_eq!(
            route(&ops, "entity-1", OperationKind::Read),
            RouteDecision::Serve {
                region: "region-b".to_string(),
                owner_epoch: 8
            }
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod model_tests {
    //! Model-based property tests for the phase-5 operations surface
    //! (issue #9793; Codex P1 on PR #9175). The coordinator model
    //! tests exercise only `Coordinator`; this module extends the
    //! generated model to `HandoffOperations` itself, per the root
    //! AGENTS.md testing doctrine: generated handoff/retry/recovery
    //! sequences are driven through the surface (gate toggles,
    //! `begin_handoff`, receipt observation, completion,
    //! cancellation, crash recovery) and after every transition the
    //! routing decision for every entity — for both operation kinds
    //! and for absent, correct, and stale presented routes — plus
    //! the full `ops_snapshot`, are checked against the explicit
    //! model oracle below. A deterministic companion test checks
    //! journal-prefix recovery against the same oracle.

    use super::*;
    use crate::world::{EntityOwner, ExportReceipt, ImportReceipt, TransitStage};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    const SEED_MAP_EPOCH: u64 = 3;
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

    fn live_ops() -> HandoffOperations {
        let mut coordinator = Coordinator::new(SEED_MAP_EPOCH);
        for (entity, owner) in seed_owners() {
            coordinator
                .register_owner(&entity, &owner.region, owner.owner_epoch)
                .unwrap();
        }
        HandoffOperations::with_gate(coordinator, true)
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
        requested_at: u64,
        last_progress_at: u64,
    }

    #[derive(Clone, Debug)]
    struct Model {
        owners: BTreeMap<String, (String, u64)>,
        active: BTreeMap<String, MActive>,
        used: std::collections::BTreeSet<String>,
        map_epoch: u64,
        gate: bool,
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
                gate: true,
            }
        }

        fn active_by_id(&self, id: &str) -> Option<(&String, &MActive)> {
            self.active.iter().find(|(_, a)| a.id == id)
        }

        fn stuck(&self, active: &MActive, now_ms: u64) -> bool {
            active.stage != HandoffStage::Started
                && now_ms.saturating_sub(active.last_progress_at) > STUCK_THRESHOLD_MS
        }

        /// The oracle routing decision for one client operation.
        fn route(
            &self,
            entity: &str,
            presented: Option<&ClientRoute>,
            now_ms: u64,
        ) -> RouteDecision {
            if let Some(active) = self.active.get(entity) {
                if active.stage != HandoffStage::Started {
                    let stage = if self.stuck(active, now_ms) {
                        TransitStage::Stuck
                    } else if active.stage == HandoffStage::ImportObserved {
                        TransitStage::ImportedAwaitingCoordinator
                    } else {
                        TransitStage::Exported
                    };
                    return RouteDecision::InTransit(EntityInTransit {
                        entity_ref: entity.to_string(),
                        handoff_id: active.id.clone(),
                        source_region: active.source.clone(),
                        target_region: active.target.clone(),
                        stage,
                        retryable: true,
                    });
                }
            }
            let (region, epoch) = &self.owners[entity];
            if let Some(presented) = presented {
                if presented.region != *region || presented.owner_epoch != *epoch {
                    return RouteDecision::StaleRoute(StaleRoute {
                        entity_ref: entity.to_string(),
                        presented_region: presented.region.clone(),
                        presented_owner_epoch: presented.owner_epoch,
                        current_region: region.clone(),
                        new_owner_epoch: *epoch,
                        region_map_epoch: self.map_epoch,
                        retryable: true,
                    });
                }
            }
            RouteDecision::Serve {
                region: region.clone(),
                owner_epoch: *epoch,
            }
        }

        /// The oracle operations snapshot at `now_ms`.
        fn snapshot(&self, now_ms: u64) -> HandoffOpsSnapshot {
            let mut entries = Vec::new();
            for (entity, active) in &self.active {
                let stuck = self.stuck(active, now_ms);
                let reason = if stuck {
                    "stuck_no_progress_over_threshold"
                } else {
                    match active.stage {
                        HandoffStage::Started => "awaiting_source_export",
                        HandoffStage::ExportObserved => "awaiting_destination_import",
                        HandoffStage::ImportObserved => "awaiting_coordinator_completion",
                    }
                };
                entries.push(HandoffOpsEntry {
                    entity_ref: entity.clone(),
                    handoff_id: active.id.clone(),
                    source_region: active.source.clone(),
                    target_region: active.target.clone(),
                    stage: active.stage,
                    age_ms: now_ms.saturating_sub(active.requested_at),
                    since_last_progress_ms: now_ms.saturating_sub(active.last_progress_at),
                    stuck,
                    reason,
                });
            }
            HandoffOpsSnapshot {
                handoffs_enabled: self.gate,
                region_map_epoch: self.map_epoch,
                in_transit_count: entries
                    .iter()
                    .filter(|e| e.stage != HandoffStage::Started)
                    .count(),
                stuck_count: entries.iter().filter(|e| e.stuck).count(),
                entries,
            }
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

    /// Assert every observable of the operations surface agrees with
    /// the model oracle at `now_ms`: routing for both kinds and for
    /// absent / correct / stale presented routes, the unknown-entity
    /// refusal, and the full snapshot.
    fn assert_ops_in_sync(ops: &HandoffOperations, m: &Model, now_ms: u64, ctx: &str) {
        assert_eq!(ops.handoffs_enabled(), m.gate, "gate diverged: {ctx}");
        for entity in ENTITIES {
            let (region, epoch) = &m.owners[entity];
            let presented_variants = [
                None,
                Some(ClientRoute {
                    region: region.clone(),
                    owner_epoch: *epoch,
                }),
                Some(ClientRoute {
                    region: "region-a".to_string(),
                    owner_epoch: 0,
                }),
                Some(ClientRoute {
                    region: "region-z".to_string(),
                    owner_epoch: 999,
                }),
            ];
            for presented in &presented_variants {
                for kind in [OperationKind::Read, OperationKind::Write] {
                    let expected = m.route(entity, presented.as_ref(), now_ms);
                    let actual = ops
                        .route_operation(entity, presented.as_ref(), kind, now_ms)
                        .unwrap_or_else(|e| panic!("route refused for {entity}: {e}: {ctx}"));
                    assert_eq!(
                        actual, expected,
                        "route diverged for {entity} presented={presented:?} kind={kind:?}: {ctx}"
                    );
                }
            }
        }
        assert_eq!(
            ops.route_operation("nope", None, OperationKind::Read, now_ms),
            Err(OpsError::Coordinator(CoordinatorError::UnknownEntity(
                "nope".to_string()
            ))),
            "unknown entity must be refused, not routed: {ctx}"
        );
        assert_eq!(
            ops.ops_snapshot(now_ms),
            m.snapshot(now_ms),
            "snapshot diverged: {ctx}"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn handoff_operations_model_sequences(
            ops_seq in proptest::collection::vec((0u8..8, 0u8..8, 0u8..8, 0u8..8), 0..48)
        ) {
            let mut ops = live_ops();
            let mut m = Model::new();

            for (step, (kind, x, y, z)) in ops_seq.iter().enumerate() {
                // 7s per step: an in-transit handoff left alone for
                // nine steps crosses the 60s stuck threshold, so the
                // generated sequences exercise the Stuck stage too.
                let t = 1_000 + (step as u64) * 7_000;
                let ctx = format!("step {step} op ({kind},{x},{y},{z})");
                match kind {
                    0 => {
                        // Toggle the feature gate.
                        let enabled = z % 2 == 0;
                        ops.set_enabled(enabled);
                        m.gate = enabled;
                    }
                    1 => {
                        // Begin a handoff through the gate.
                        let entity = ENTITIES[(*x as usize) % ENTITIES.len()];
                        let id = IDS[(*y as usize) % IDS.len()];
                        let target = REGIONS[(*z as usize) % REGIONS.len()];
                        let (region, epoch) = m.owners[entity].clone();
                        let coord_ok = !m.active.contains_key(entity)
                            && !m.used.contains(id)
                            && target != region.as_str();
                        let before = ops.coordinator().journal().len();
                        let result = ops.begin_handoff(id, entity, target, "m", "s", t);
                        if !m.gate {
                            prop_assert_eq!(
                                result,
                                Err(OpsError::HandoffsDisabled),
                                "gate-off begin must be refused: {}",
                                ctx
                            );
                            prop_assert_eq!(
                                ops.coordinator().journal().len(),
                                before,
                                "gate-off begin journals nothing: {}",
                                ctx
                            );
                        } else {
                            prop_assert_eq!(result.is_ok(), coord_ok, "begin outcome: {}", ctx);
                            if coord_ok {
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
                                        requested_at: t,
                                        last_progress_at: t,
                                    },
                                );
                            }
                        }
                    }
                    2 => {
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
                                let ok = match active.stage {
                                    HandoffStage::Started => true,
                                    _ => active.digest.as_deref() == Some(digest.as_str()),
                                };
                                let result = ops.coordinator_mut().observe_export(
                                    &export_receipt(&active, &entity, digest.clone()),
                                    t,
                                );
                                prop_assert_eq!(result.is_ok(), ok, "export outcome: {}", ctx);
                                if ok && active.stage == HandoffStage::Started {
                                    let a = m.active.get_mut(&entity).unwrap();
                                    a.stage = HandoffStage::ExportObserved;
                                    a.digest = Some(digest);
                                    a.last_progress_at = t;
                                }
                            }
                            None => {
                                let result = ops.coordinator_mut().observe_export(&dummy_export(id), t);
                                prop_assert!(result.is_err(), "export for unknown id: {}", ctx);
                            }
                        }
                    }
                    3 => {
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
                                let result = ops.coordinator_mut().observe_import(
                                    &import_receipt(&active, &entity, digest),
                                    t,
                                );
                                prop_assert_eq!(result.is_ok(), ok, "import outcome: {}", ctx);
                                if ok && active.stage == HandoffStage::ExportObserved {
                                    let a = m.active.get_mut(&entity).unwrap();
                                    a.stage = HandoffStage::ImportObserved;
                                    a.last_progress_at = t;
                                }
                            }
                            None => {
                                let result = ops.coordinator_mut().observe_import(&dummy_import(id), t);
                                prop_assert!(result.is_err(), "import for unknown id: {}", ctx);
                            }
                        }
                    }
                    4 => {
                        // Complete.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let found = m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone()));
                        let ok = matches!(&found, Some((_, a)) if a.stage == HandoffStage::ImportObserved);
                        let result = ops.coordinator_mut().complete(id);
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
                    5 => {
                        // Cancel; the caller sometimes cannot prove
                        // the export is absent.
                        let id = IDS[(*x as usize) % IDS.len()];
                        let export_exists = z % 2 == 0;
                        let found = m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone()));
                        let ok = matches!(&found, Some((_, a)) if a.stage == HandoffStage::Started)
                            && !export_exists;
                        let result = ops.coordinator_mut().cancel(id, "prop", export_exists);
                        prop_assert_eq!(result.is_ok(), ok, "cancel outcome: {}", ctx);
                        if ok {
                            let (entity, _) = found.unwrap();
                            m.active.remove(&entity);
                        }
                    }
                    6 => {
                        // Crash + recover: replaying the coordinator
                        // journal must reproduce the exact same
                        // routing and snapshot on the surface.
                        let before: Vec<(String, RouteDecision)> = ENTITIES
                            .iter()
                            .map(|e| {
                                (
                                    e.to_string(),
                                    ops.route_operation(e, None, OperationKind::Write, t)
                                        .unwrap(),
                                )
                            })
                            .collect();
                        let before_snapshot = ops.ops_snapshot(t);
                        let recovered = Coordinator::recover(
                            ops.coordinator().journal().to_vec(),
                            SEED_MAP_EPOCH,
                            seed_owners(),
                        )
                        .unwrap();
                        ops = HandoffOperations::with_gate(recovered, m.gate);
                        for (entity, decision) in before {
                            prop_assert_eq!(
                                ops.route_operation(&entity, None, OperationKind::Write, t)
                                    .unwrap(),
                                decision,
                                "replay changed routing for {}: {}",
                                entity,
                                ctx
                            );
                        }
                        prop_assert_eq!(
                            ops.ops_snapshot(t),
                            before_snapshot,
                            "replay changed snapshot: {}",
                            ctx
                        );
                    }
                    _ => {
                        // Correlate garbage: a receipt whose schema
                        // disagrees with the journaled start is
                        // refused and changes nothing.
                        let id = IDS[(*x as usize) % IDS.len()];
                        if let Some((entity, active)) =
                            m.active_by_id(id).map(|(e, a)| (e.clone(), a.clone()))
                        {
                            let mut receipt = export_receipt(&active, &entity, digest_for(id));
                            receipt.payload_schema = "other-schema".to_string();
                            let before = ops.coordinator().journal().len();
                            let result = ops.coordinator_mut().observe_export(&receipt, t);
                            prop_assert!(result.is_err(), "schema mismatch refused: {}", ctx);
                            prop_assert_eq!(
                                ops.coordinator().journal().len(),
                                before,
                                "journal untouched: {}",
                                ctx
                            );
                        }
                    }
                }

                // Oracle check after every generated transition.
                assert_ops_in_sync(&ops, &m, t, &ctx);
            }
        }
    }

    /// Every crash prefix of a journal that drove two handoffs
    /// (one completed, one left in transit, one merely started)
    /// replays to a surface whose routing and snapshot match the
    /// model oracle state recorded at that exact journal length.
    #[test]
    fn journal_prefix_recovery_matches_model_oracle() {
        let mut ops = live_ops();
        let mut m = Model::new();
        // (journal length, model state at that length)
        let mut states: Vec<(usize, Model)> = vec![(0, m.clone())];
        let mut record = |ops: &HandoffOperations, m: &Model, states: &mut Vec<(usize, Model)>| {
            states.push((ops.coordinator().journal().len(), m.clone()));
        };

        // e0: a full crossing, begin -> export -> import -> complete.
        ops.begin_handoff("h0", "e0", "region-b", "m", "s", 1_000)
            .unwrap();
        {
            let (region, epoch) = m.owners["e0"].clone();
            m.used.insert("h0".to_string());
            m.active.insert(
                "e0".to_string(),
                MActive {
                    id: "h0".to_string(),
                    source: region,
                    target: "region-b".to_string(),
                    epoch,
                    stage: HandoffStage::Started,
                    digest: None,
                    requested_at: 1_000,
                    last_progress_at: 1_000,
                },
            );
        }
        record(&ops, &m, &mut states);
        let active = m.active["e0"].clone();
        ops.coordinator_mut()
            .observe_export(&export_receipt(&active, "e0", digest_for("h0")), 2_000)
            .unwrap();
        {
            let a = m.active.get_mut("e0").unwrap();
            a.stage = HandoffStage::ExportObserved;
            a.digest = Some(digest_for("h0"));
            a.last_progress_at = 2_000;
        }
        record(&ops, &m, &mut states);
        let active = m.active["e0"].clone();
        ops.coordinator_mut()
            .observe_import(&import_receipt(&active, "e0", digest_for("h0")), 3_000)
            .unwrap();
        {
            let a = m.active.get_mut("e0").unwrap();
            a.stage = HandoffStage::ImportObserved;
            a.last_progress_at = 3_000;
        }
        record(&ops, &m, &mut states);
        ops.coordinator_mut().complete("h0").unwrap();
        m.owners
            .insert("e0".to_string(), ("region-b".to_string(), 4));
        m.map_epoch += 1;
        m.active.remove("e0");
        record(&ops, &m, &mut states);

        // e1: begun and exported, left in transit (stuck by the
        // check time below).
        ops.begin_handoff("h1", "e1", "region-c", "m", "s", 5_000)
            .unwrap();
        {
            let (region, epoch) = m.owners["e1"].clone();
            m.used.insert("h1".to_string());
            m.active.insert(
                "e1".to_string(),
                MActive {
                    id: "h1".to_string(),
                    source: region,
                    target: "region-c".to_string(),
                    epoch,
                    stage: HandoffStage::Started,
                    digest: None,
                    requested_at: 5_000,
                    last_progress_at: 5_000,
                },
            );
        }
        record(&ops, &m, &mut states);
        let active = m.active["e1"].clone();
        ops.coordinator_mut()
            .observe_export(&export_receipt(&active, "e1", digest_for("h1")), 6_000)
            .unwrap();
        {
            let a = m.active.get_mut("e1").unwrap();
            a.stage = HandoffStage::ExportObserved;
            a.digest = Some(digest_for("h1"));
            a.last_progress_at = 6_000;
        }
        record(&ops, &m, &mut states);

        // e0 again: a second handoff merely started.
        ops.begin_handoff("h2", "e0", "region-c", "m", "s", 7_000)
            .unwrap();
        {
            let (region, epoch) = m.owners["e0"].clone();
            m.used.insert("h2".to_string());
            m.active.insert(
                "e0".to_string(),
                MActive {
                    id: "h2".to_string(),
                    source: region,
                    target: "region-c".to_string(),
                    epoch,
                    stage: HandoffStage::Started,
                    digest: None,
                    requested_at: 7_000,
                    last_progress_at: 7_000,
                },
            );
        }
        record(&ops, &m, &mut states);

        let journal = ops.coordinator().journal().to_vec();
        assert_eq!(journal.len(), 7);
        // 70s: e1's export is 64s old at its in-transit prefixes, so
        // the oracle expects the Stuck stage there.
        let now = 70_000;
        for prefix in 0..=journal.len() {
            let recovered =
                Coordinator::recover(journal[..prefix].to_vec(), SEED_MAP_EPOCH, seed_owners())
                    .unwrap();
            let replayed = HandoffOperations::with_gate(recovered, true);
            let (_, model_at_prefix) = states
                .iter()
                .find(|(len, _)| *len == prefix)
                .unwrap_or_else(|| panic!("no model state recorded for prefix {prefix}"));
            assert_ops_in_sync(
                &replayed,
                model_at_prefix,
                now,
                &format!("journal prefix {prefix}"),
            );
        }
    }
}
