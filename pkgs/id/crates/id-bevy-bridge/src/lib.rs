//! Bevy projection bridge for ID worlds, slice 1 (#13702).
//!
//! Design of record: `doc/2026-11-08_design_id5781.md` (adopted on #5781),
//! §1 state mapping and answers Q1 (separate bridge crate, `bevy_ecs`
//! first), Q2 (pin per release — this crate pins the workspace's 0.19
//! line), Q4 (headless-logic-first proving target).
//!
//! The journal is the only authority; Bevy holds a materialized
//! projection. This slice implements the projection of a static journal
//! fixture:
//!
//! - [`map::IdEntityMap`]: ID entity id ↔ Bevy [`Entity`](bevy_ecs::prelude::Entity),
//!   plus the journal position at which each mapping was created.
//!   [`map::LocalOnly`] entities never enter the map.
//! - [`schema::BridgeSchema`]: predicate → component type, data class,
//!   merge/project rule, as data. Classification is total and
//!   fail-closed: an unclassified predicate does not project.
//! - [`projection::project_fixture`]: position-ordered application of
//!   fixture deltas into a headless [`World`](bevy_ecs::prelude::World),
//!   allocating entities on first projection and despawning on
//!   retraction.
//! - [`fixture`]: the static journal fixture the acceptance tests project.

pub mod fixture;
pub mod map;
pub mod projection;
pub mod schema;

pub use fixture::JournalDelta;
pub use map::{IdEntityMap, LocalOnly, WorldName, WorldPosition};
pub use projection::project_fixture;
pub use schema::{BridgeSchema, ComponentRule, DataClass, PredicateRule};
