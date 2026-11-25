//! Journal tail and the `JournalSet` schedule point (design §2.1).
//!
//! The bridge tails the journal from the world's [`AppliedPosition`] and
//! applies positions in an exclusive system inside [`JournalSet`]. The set
//! runs after [`InputSet`] and before [`GameplaySet`], so gameplay and
//! presentation systems always see a consistent journal prefix: an
//! exclusive system holds the whole world, and each position applies
//! wholly or not at all ([`apply_position`]).
//!
//! Catch-up is budgeted by [`LagBudget`]: at most that many positions apply
//! per tick, and [`JournalLag`] reports how many remain.

use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::fixture::JournalDelta;
use crate::intent::{confirm_intents, reconcile_predictions, submit_intents};
use crate::projection::{AppliedPosition, ProjectionError, applied_position, apply_position};
use crate::schema::BridgeSchema;

/// Every delta the journal recorded at one position.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionBatch {
    /// The journal position.
    pub position: u64,
    /// The deltas at that position; each carries the same position.
    pub deltas: Vec<JournalDelta>,
}

/// A journal the bridge can tail, such as a hosted world's session.
pub trait JournalSource: Send + Sync + 'static {
    /// Up to `max` positions strictly after `after` (`None` means from
    /// the start), in ascending order.
    fn read_after(&self, after: Option<u64>, max: usize) -> Vec<PositionBatch>;

    /// How many positions exist strictly after `after`.
    fn pending_after(&self, after: Option<u64>) -> u64;
}

/// How many positions the tail may apply in one tick (at least one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagBudget(usize);

impl LagBudget {
    /// A budget of `positions_per_tick`, raised to one if zero: a tail
    /// that can never apply anything would stall the world for good.
    #[must_use]
    pub fn new(positions_per_tick: usize) -> Self {
        Self(positions_per_tick.max(1))
    }

    /// Positions the tail may apply per tick.
    #[must_use]
    pub const fn per_tick(self) -> usize {
        self.0
    }
}

/// The bridge's tail: where it reads, how it projects and its budget.
#[derive(Resource)]
pub struct JournalTail {
    source: Arc<dyn JournalSource>,
    schema: BridgeSchema,
    budget: LagBudget,
}

impl fmt::Debug for JournalTail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JournalTail")
            .field("schema", &self.schema)
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

impl JournalTail {
    /// A tail over `source`, projecting through `schema`.
    #[must_use]
    pub fn new(source: Arc<dyn JournalSource>, schema: BridgeSchema, budget: LagBudget) -> Self {
        Self {
            source,
            schema,
            budget,
        }
    }

    /// The tail's catch-up budget.
    #[must_use]
    pub const fn budget(&self) -> LagBudget {
        self.budget
    }
}

/// How far behind the journal this world is after the last tick.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JournalLag {
    /// Positions recorded in the journal but not yet applied.
    pub behind: u64,
    /// Positions applied during the last tick.
    pub applied_last_tick: usize,
}

impl JournalLag {
    /// Whether the world has applied everything the journal holds.
    #[must_use]
    pub const fn caught_up(self) -> bool {
        self.behind == 0
    }
}

/// Set when a position could not apply. The tail stops at the last good
/// position rather than skip one (design §2.3), and stays stopped.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct JournalHalt {
    /// The position that failed.
    pub position: u64,
    /// Why it failed.
    pub error: ProjectionError,
}

/// Systems that collect input, before the journal applies.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputSet;

/// The bridge's fixed schedule point: submits intents, applies journal
/// positions, then confirms intents and reconciles predictions.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JournalSet;

/// Gameplay and presentation systems, which see only applied positions.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameplaySet;

/// Order [`InputSet`] → [`JournalSet`] → [`GameplaySet`] in `schedule`
/// and place the bridge's systems in [`JournalSet`].
pub fn configure_bridge_schedule(schedule: &mut Schedule) {
    schedule.configure_sets((InputSet, JournalSet, GameplaySet).chain());
    schedule.add_systems(
        (
            submit_intents,
            tail_journal,
            confirm_intents,
            reconcile_predictions,
        )
            .chain()
            .in_set(JournalSet),
    );
}

/// Apply up to the budget of journal positions after [`AppliedPosition`],
/// in order, each atomically, then record [`JournalLag`].
///
/// A position that fails to apply sets [`JournalHalt`] and leaves the
/// world at the previous position; a halted tail applies nothing more.
pub fn tail_journal(world: &mut World) {
    if !world.contains_resource::<JournalTail>() {
        return;
    }
    if !world.contains_resource::<AppliedPosition>() {
        world.insert_resource(AppliedPosition::default());
    }
    world.resource_scope(|world, tail: Mut<'_, JournalTail>| {
        let mut applied = 0;
        if !world.contains_resource::<JournalHalt>() {
            let batches = tail
                .source
                .read_after(applied_position(world), tail.budget.per_tick());
            for batch in batches.into_iter().take(tail.budget.per_tick()) {
                if let Err(error) =
                    apply_position(world, &tail.schema, batch.position, &batch.deltas)
                {
                    world.insert_resource(JournalHalt {
                        position: batch.position,
                        error,
                    });
                    break;
                }
                applied += 1;
            }
        }
        let behind = tail.source.pending_after(applied_position(world));
        world.insert_resource(JournalLag {
            behind,
            applied_last_tick: applied,
        });
    });
}
