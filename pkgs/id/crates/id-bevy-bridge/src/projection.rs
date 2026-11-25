//! Ordered, atomic application of journal deltas to a headless Bevy world.
//!
//! Every journal position is applied in two steps: all of its deltas are
//! decoded against the schema first, and only if every one of them is
//! valid does any of them touch the world (design §2.1: a position applies
//! wholly or not at all). [`AppliedPosition`] records the last position
//! that applied; nothing at or below it can apply again.

use bevy_ecs::prelude::{Entity, Resource, World};
use thiserror::Error;

use crate::fixture::JournalDelta;
use crate::map::{IdEntityMap, WorldName, WorldPosition};
use crate::schema::{BridgeSchema, ComponentRule, DataClass};

/// Failures in projection. A failure leaves the world exactly as it was
/// before the position was attempted.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ProjectionError {
    /// A position at or below the applied position, or a fixture whose
    /// positions were not strictly increasing.
    #[error("journal positions must be strictly increasing")]
    PositionOrder,
    /// A delta carried a different position from the batch it arrived in.
    #[error("delta at position {found} arrived in the batch for position {expected}")]
    PositionMismatch {
        /// The batch's position.
        expected: u64,
        /// The delta's own position.
        found: u64,
    },
    /// A classified predicate's value did not match its schema rule.
    #[error("invalid value for classified predicate `{0}`")]
    InvalidValue(String),
}

/// The last journal position applied to this world (design §2.1). Any
/// system may read it; only the bridge advances it.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppliedPosition(Option<u64>);

impl AppliedPosition {
    /// The applied position, or `None` before the first position applies.
    #[must_use]
    pub const fn get(self) -> Option<u64> {
        self.0
    }
}

/// Read the applied position of `world`.
#[must_use]
pub fn applied_position(world: &World) -> Option<u64> {
    world
        .get_resource::<AppliedPosition>()
        .and_then(|p| p.get())
}

/// One decoded world-state component value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Name(String),
    Position(WorldPosition),
}

/// A delta decoded against the schema; applying it cannot fail.
#[derive(Debug, Clone, PartialEq)]
enum Planned {
    Upsert { id: String, values: Vec<Value> },
    Retract { id: String },
}

fn plan(schema: &BridgeSchema, delta: &JournalDelta) -> Result<Planned, ProjectionError> {
    match delta {
        JournalDelta::Upsert { id, values, .. } => {
            let mut decoded = Vec::with_capacity(values.len());
            for (predicate, value) in values {
                let Some(rule) = schema.rule(predicate) else {
                    continue;
                };
                if rule.class != DataClass::World {
                    continue;
                }
                let invalid = || ProjectionError::InvalidValue(predicate.clone());
                match rule.component {
                    ComponentRule::Name => {
                        let text = value.as_str().ok_or_else(invalid)?;
                        decoded.push(Value::Name(text.to_owned()));
                    }
                    ComponentRule::Position2 => {
                        let pair = value.as_array().ok_or_else(invalid)?;
                        let [x, y] = pair.as_slice() else {
                            return Err(invalid());
                        };
                        let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else {
                            return Err(invalid());
                        };
                        decoded.push(Value::Position(WorldPosition { x, y }));
                    }
                }
            }
            Ok(Planned::Upsert {
                id: id.clone(),
                values: decoded,
            })
        }
        JournalDelta::Retract { id, .. } => Ok(Planned::Retract { id: id.clone() }),
    }
}

fn apply_planned(world: &mut World, position: u64, planned: Planned) {
    match planned {
        Planned::Upsert { id, values } => {
            let entity = if let Some(entity) = world.resource::<IdEntityMap>().entity(&id) {
                entity
            } else {
                let entity = world.spawn_empty().id();
                world
                    .resource_mut::<IdEntityMap>()
                    .insert(&id, entity, position);
                entity
            };
            let mut entity = world.entity_mut(entity);
            for value in values {
                match value {
                    Value::Name(text) => {
                        entity.insert(WorldName(text));
                    }
                    Value::Position(at) => {
                        entity.insert(at);
                    }
                }
            }
        }
        Planned::Retract { id } => {
            let entity = world.resource_mut::<IdEntityMap>().remove(&id);
            if let Some(entity) = entity {
                world.despawn(entity);
            }
        }
    }
}

/// Apply every delta at one journal position, or none of them.
///
/// The position must be above [`AppliedPosition`] and every delta must
/// carry it. All deltas are decoded before any is applied, so an invalid
/// value anywhere in the batch leaves the world, the mapping table and the
/// applied position unchanged. Unclassified predicates and the
/// `Presentation` and `Local` classes never project (design §1.2).
pub fn apply_position(
    world: &mut World,
    schema: &BridgeSchema,
    position: u64,
    deltas: &[JournalDelta],
) -> Result<(), ProjectionError> {
    if applied_position(world).is_some_and(|applied| position <= applied) {
        return Err(ProjectionError::PositionOrder);
    }
    let mut planned = Vec::with_capacity(deltas.len());
    for delta in deltas {
        if delta.position() != position {
            return Err(ProjectionError::PositionMismatch {
                expected: position,
                found: delta.position(),
            });
        }
        planned.push(plan(schema, delta)?);
    }
    if !world.contains_resource::<IdEntityMap>() {
        world.insert_resource(IdEntityMap::new());
    }
    for step in planned {
        apply_planned(world, position, step);
    }
    world.insert_resource(AppliedPosition(Some(position)));
    Ok(())
}

/// Apply an ordered fixture to `world` using `schema`, one delta per
/// position.
///
/// Positions must be strictly increasing within the fixture and above the
/// world's [`AppliedPosition`]; an out-of-order fixture is rejected before
/// anything applies. Each position then applies atomically through
/// [`apply_position`]; a malformed value stops the fixture at the last
/// position that applied.
///
/// Returns the final applied journal position, or `None` for an empty
/// fixture.
pub fn project_fixture(
    world: &mut World,
    schema: &BridgeSchema,
    deltas: &[JournalDelta],
) -> Result<Option<u64>, ProjectionError> {
    if !world.contains_resource::<IdEntityMap>() {
        world.insert_resource(IdEntityMap::new());
    }
    let mut last = applied_position(world);
    for delta in deltas {
        if last.is_some_and(|last| delta.position() <= last) {
            return Err(ProjectionError::PositionOrder);
        }
        last = Some(delta.position());
    }
    let mut last_position = None;
    for delta in deltas {
        apply_position(world, schema, delta.position(), std::slice::from_ref(delta))?;
        last_position = Some(delta.position());
    }
    Ok(last_position)
}

/// Read the Bevy entity mapped to an ID entity. Kept as a helper for tests
/// and downstream bridge consumers that need to inspect a projected world.
#[must_use]
pub fn mapped_entity(world: &World, id: &str) -> Option<Entity> {
    world.get_resource::<IdEntityMap>()?.entity(id)
}
