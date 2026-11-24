//! Ordered application of static journal deltas to a headless Bevy world.

use bevy_ecs::prelude::{Entity, World};
use thiserror::Error;

use crate::fixture::JournalDelta;
use crate::map::{IdEntityMap, WorldName, WorldPosition};
use crate::schema::{BridgeSchema, ComponentRule, DataClass};

/// Failures in the fixture projection; malformed classified values reject
/// the delta rather than silently creating partial world state.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectionError {
    /// Fixture positions were not strictly increasing.
    #[error("journal positions must be strictly increasing")]
    PositionOrder,
    /// A classified predicate's value did not match its schema rule.
    #[error("invalid value for classified predicate `{0}`")]
    InvalidValue(String),
}

/// Apply an ordered fixture to `world` using `schema`.
///
/// Only explicitly classified `World` predicates project. `Presentation`
/// and `Local` classes are ignored, as are unclassified predicates. Entity
/// allocation and mapping are bridge-owned; retraction despawns the mapped
/// entity and removes the mapping at that journal position (design §1.1).
///
/// Returns the final applied journal position, or `None` for an empty
/// fixture. A malformed classified value or unordered position aborts.
pub fn project_fixture(
    world: &mut World,
    schema: &BridgeSchema,
    deltas: &[JournalDelta],
) -> Result<Option<u64>, ProjectionError> {
    if !world.contains_resource::<IdEntityMap>() {
        world.insert_resource(IdEntityMap::new());
    }

    let mut last_position = None;
    for delta in deltas {
        let position = delta.position();
        if last_position.is_some_and(|last| position <= last) {
            return Err(ProjectionError::PositionOrder);
        }
        match delta {
            JournalDelta::Upsert { id, values, .. } => {
                let entity = match world.resource::<IdEntityMap>().entity(id) {
                    Some(entity) => entity,
                    None => {
                        let entity = world.spawn_empty().id();
                        world
                            .resource_mut::<IdEntityMap>()
                            .insert(id, entity, position);
                        entity
                    }
                };
                for (predicate, value) in values {
                    let Some(rule) = schema.rule(predicate) else {
                        continue;
                    };
                    if rule.class != DataClass::World {
                        continue;
                    }
                    match rule.component {
                        ComponentRule::Name => {
                            let Some(value) = value.as_str() else {
                                return Err(ProjectionError::InvalidValue(predicate.clone()));
                            };
                            world
                                .entity_mut(entity)
                                .insert(WorldName(value.to_string()));
                        }
                        ComponentRule::Position2 => {
                            let Some(values) = value.as_array() else {
                                return Err(ProjectionError::InvalidValue(predicate.clone()));
                            };
                            if values.len() != 2 {
                                return Err(ProjectionError::InvalidValue(predicate.clone()));
                            }
                            let (Some(x), Some(y)) = (values[0].as_f64(), values[1].as_f64())
                            else {
                                return Err(ProjectionError::InvalidValue(predicate.clone()));
                            };
                            world.entity_mut(entity).insert(WorldPosition {
                                x: x as f32,
                                y: y as f32,
                            });
                        }
                    }
                }
            }
            JournalDelta::Retract { id, .. } => {
                let entity = world.resource_mut::<IdEntityMap>().remove(id);
                if let Some(entity) = entity {
                    world.despawn(entity);
                }
            }
        }
        last_position = Some(position);
    }
    Ok(last_position)
}

/// Read the Bevy entity mapped to an ID entity. Kept as a helper for tests
/// and downstream bridge consumers that need to inspect a projected world.
#[must_use]
pub fn mapped_entity(world: &World, id: &str) -> Option<Entity> {
    world.get_resource::<IdEntityMap>()?.entity(id)
}
