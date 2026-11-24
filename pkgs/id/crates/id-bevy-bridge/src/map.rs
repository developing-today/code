//! [`IdEntityMap`] and the world-state components it guards.
//!
//! Design §1.1: every projected ID entity corresponds to exactly one
//! Bevy [`Entity`](bevy_ecs::prelude::Entity), allocated by the bridge on
//! first projection. The bridge owns this mapping table, including the
//! journal position at which each mapping was created. Bevy entities with
//! no ID counterpart carry [`LocalOnly`] and never enter the map.

use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Marker for Bevy entities with no ID counterpart (particles, UI nodes,
/// purely local actors). The bridge never journals them and the mapping
/// table never contains them (design §1.1).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalOnly;

/// World-state name component: carries the journaled value verbatim
/// (design §1.2). Bevy systems read; mutation only via intents (§2.2).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct WorldName(pub String);

/// World-state position component: carries the journaled value verbatim
/// (design §1.2).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct WorldPosition {
    /// Journaled x coordinate.
    pub x: f64,
    /// Journaled y coordinate.
    pub y: f64,
}

/// Bridge-owned mapping table: ID entity id ↔ Bevy entity, plus the
/// journal position at which each mapping was created (design §1.1).
#[derive(Resource, Debug, Default)]
pub struct IdEntityMap {
    entries: HashMap<String, (Entity, u64)>,
}

impl IdEntityMap {
    /// Create an empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a mapping created at `position`. Overwrites any existing
    /// entry for the same ID (re-projection after a despawn reallocates).
    pub fn insert(&mut self, id: &str, entity: Entity, position: u64) {
        self.entries.insert(id.to_string(), (entity, position));
    }

    /// Look up the Bevy entity for an ID entity.
    #[must_use]
    pub fn entity(&self, id: &str) -> Option<Entity> {
        self.entries.get(id).map(|(entity, _)| *entity)
    }

    /// Look up the journal position at which a mapping was created.
    #[must_use]
    pub fn position(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|(_, position)| *position)
    }

    /// Whether the map contains an entry for `id`.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// Remove the entry for `id`. Returns the removed entity, if any.
    /// Called when the projection applies a retraction (design §1.1).
    pub fn remove(&mut self, id: &str) -> Option<Entity> {
        self.entries.remove(id).map(|(entity, _)| entity)
    }

    /// Number of mapped entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over `(id, entity, position)` triples.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Entity, u64)> + '_ {
        self.entries
            .iter()
            .map(|(id, (entity, position))| (id.as_str(), *entity, *position))
    }
}
