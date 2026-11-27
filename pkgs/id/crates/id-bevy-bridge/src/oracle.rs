//! Replay oracle: the bridge schema's canonical comparison (design §4).
//!
//! The journal is the only authority, so a recorded journal replayed
//! through the bridge from an empty [`World`] must produce world-state
//! components identical to the live run's at the same journal position.
//! [`canonical_state`] reads a world into the comparison form, keyed by ID
//! entity rather than by Bevy [`Entity`], because a live world allocates
//! entities for presentation and local state that a replay never sees.
//!
//! The comparison covers world-state components only. Presentation and
//! local state ([`PredictedPosition`](crate::intent::PredictedPosition),
//! [`LocalOnly`](crate::map::LocalOnly) entities) are exempt, but a Bevy
//! entity outside the mapping table that carries a world-state component
//! is counted, because that is local state feeding back into world state.
//!
//! [`check_replay`] is the oracle. It is also the acceptance gate for a
//! bump of the pinned `bevy_ecs` version (design Q2): a bump that changes
//! a projection outcome fails it, and is then fixed as a mapping bug or
//! recorded as a named migration, never accepted as a silent diff.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::map::{IdEntityMap, WorldName, WorldPosition};
use crate::projection::{ProjectionError, applied_position, apply_position};
use crate::schema::BridgeSchema;
use crate::tail::PositionBatch;

/// A [`WorldPosition`] in comparison form: the bit patterns of both
/// coordinates, so equality is exact, with no float tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalPosition {
    /// Bit pattern of the x coordinate.
    pub x_bits: u64,
    /// Bit pattern of the y coordinate.
    pub y_bits: u64,
}

impl CanonicalPosition {
    /// The x coordinate.
    #[must_use]
    pub const fn x(self) -> f64 {
        f64::from_bits(self.x_bits)
    }

    /// The y coordinate.
    #[must_use]
    pub const fn y(self) -> f64 {
        f64::from_bits(self.y_bits)
    }
}

impl From<WorldPosition> for CanonicalPosition {
    fn from(at: WorldPosition) -> Self {
        Self {
            x_bits: at.x.to_bits(),
            y_bits: at.y.to_bits(),
        }
    }
}

/// The world state of one mapped ID entity, in comparison form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEntity {
    /// Journal position at which the mapping was created.
    pub mapped_at: u64,
    /// The entity's [`WorldName`], if it has one.
    pub name: Option<String>,
    /// The entity's [`WorldPosition`], if it has one.
    pub position: Option<CanonicalPosition>,
}

/// A world's state in comparison form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalState {
    /// The applied journal position.
    pub applied: Option<u64>,
    /// Every mapped ID entity, by ID.
    pub entities: BTreeMap<String, CanonicalEntity>,
    /// Bevy entities outside the mapping table that carry a world-state
    /// component. Always zero for a world only the bridge has written.
    pub unmapped_world_state: usize,
}

/// Read `world` into comparison form.
pub fn canonical_state(world: &mut World) -> CanonicalState {
    let mut entities = BTreeMap::new();
    let mut mapped = HashSet::new();
    if let Some(map) = world.get_resource::<IdEntityMap>() {
        for (id, entity, mapped_at) in map.iter() {
            mapped.insert(entity);
            entities.insert(
                id.to_owned(),
                CanonicalEntity {
                    mapped_at,
                    name: world.get::<WorldName>(entity).map(|name| name.0.clone()),
                    position: world
                        .get::<WorldPosition>(entity)
                        .copied()
                        .map(CanonicalPosition::from),
                },
            );
        }
    }
    let mut carriers = world.query_filtered::<Entity, Or<(With<WorldName>, With<WorldPosition>)>>();
    let unmapped_world_state = carriers
        .iter(world)
        .filter(|entity| !mapped.contains(entity))
        .count();
    CanonicalState {
        applied: applied_position(world),
        entities,
        unmapped_world_state,
    }
}

/// Replay the positions of `journal` at or below `upto` into an empty
/// [`World`] and read the result into comparison form. `None` replays
/// nothing: the state before the first position.
pub fn replay(
    schema: &BridgeSchema,
    journal: &[PositionBatch],
    upto: Option<u64>,
) -> Result<CanonicalState, ProjectionError> {
    let mut world = World::new();
    world.insert_resource(IdEntityMap::new());
    if let Some(upto) = upto {
        for batch in journal.iter().filter(|batch| batch.position <= upto) {
            apply_position(&mut world, schema, batch.position, &batch.deltas)?;
        }
    }
    Ok(canonical_state(&mut world))
}

/// How a live world differs from the replay of its journal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Divergence {
    /// The recorded journal itself does not replay.
    #[error("the recorded journal does not replay: {0}")]
    Replay(ProjectionError),
    /// The applied positions differ.
    #[error("applied position differs: live {live:?}, replay {replay:?}")]
    Applied {
        /// The live world's applied position.
        live: Option<u64>,
        /// The replay's applied position.
        replay: Option<u64>,
    },
    /// One ID entity's world state differs, or exists on one side only.
    #[error("entity `{id}` differs: live {live:?}, replay {replay:?}")]
    Entity {
        /// The ID entity.
        id: String,
        /// Its live world state.
        live: Option<CanonicalEntity>,
        /// Its replayed world state.
        replay: Option<CanonicalEntity>,
    },
    /// Entities outside the mapping table carry world state.
    #[error("{live} live entities carry world state without a mapping (replay: {replay})")]
    Unmapped {
        /// Count in the live world.
        live: usize,
        /// Count in the replay.
        replay: usize,
    },
}

/// Compare two canonical states; the first difference found is returned,
/// checking the applied position, then entities in ID order, then
/// unmapped world state.
// A divergence is the oracle's failure report: it is built once, on the
// failing path, and carries the two entity states so the report shows
// both. Boxing it would only move that cost and complicate every match.
#[allow(clippy::result_large_err)]
pub fn compare(live: &CanonicalState, replay: &CanonicalState) -> Result<(), Divergence> {
    if live.applied != replay.applied {
        return Err(Divergence::Applied {
            live: live.applied,
            replay: replay.applied,
        });
    }
    let ids: BTreeSet<&String> = live.entities.keys().chain(replay.entities.keys()).collect();
    for id in ids {
        let (left, right) = (live.entities.get(id), replay.entities.get(id));
        if left != right {
            return Err(Divergence::Entity {
                id: id.clone(),
                live: left.cloned(),
                replay: right.cloned(),
            });
        }
    }
    if live.unmapped_world_state != replay.unmapped_world_state {
        return Err(Divergence::Unmapped {
            live: live.unmapped_world_state,
            replay: replay.unmapped_world_state,
        });
    }
    Ok(())
}

/// The replay oracle: replay `journal` up to `live`'s applied position
/// from an empty world and require the same canonical state.
#[allow(clippy::result_large_err)] // See `compare`.
pub fn check_replay(
    live: &mut World,
    schema: &BridgeSchema,
    journal: &[PositionBatch],
) -> Result<(), Divergence> {
    let live_state = canonical_state(live);
    let replayed = replay(schema, journal, live_state.applied).map_err(Divergence::Replay)?;
    compare(&live_state, &replayed)
}
