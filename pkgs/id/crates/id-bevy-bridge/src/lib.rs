//! Bevy projection bridge for ID worlds, slices 1 and 2 (#13702, #13703).
//!
//! Design of record: `doc/2026-11-08_design_id5781.md` (adopted on #5781),
//! §1 state mapping, §2 event flow, and answers Q1 (separate bridge crate,
//! `bevy_ecs` first), Q2 (pin per release — this crate pins the
//! workspace's 0.19 line), Q4 (headless-logic-first proving target).
//!
//! The journal is the only authority; Bevy holds a materialized
//! projection.
//!
//! Slice 1, the projection:
//!
//! - [`map::IdEntityMap`]: ID entity id ↔ Bevy [`Entity`](bevy_ecs::prelude::Entity),
//!   plus the journal position at which each mapping was created.
//!   [`map::LocalOnly`] entities never enter the map.
//! - [`schema::BridgeSchema`]: predicate → component type, data class,
//!   merge/project rule, as data. Classification is total and
//!   fail-closed: an unclassified predicate does not project.
//! - [`projection::apply_position`]: one journal position applied wholly
//!   or not at all, above the world's [`projection::AppliedPosition`].
//! - [`projection::project_fixture`]: a static fixture, one position at a
//!   time.
//!
//! Slice 2, the event flow:
//!
//! - [`tail`]: the journal tail in `JournalSet`, between input and
//!   gameplay, with a per-tick lag budget and lag report.
//! - [`intent`]: intents sent to the append gate as shell-contract
//!   `input` frames, confirmed only when their journal position applies,
//!   with presentation-class prediction that snaps back on rejection.
//! - [`fixture::HostedWorldFixture`]: an in-memory hosted world (journal
//!   plus append gate) that the acceptance tests run against.

pub mod fixture;
pub mod intent;
pub mod map;
pub mod projection;
pub mod schema;
pub mod tail;

pub use fixture::{HostedWorldFixture, JournalDelta};
pub use intent::{
    AppendGate, GateVerdict, Intent, IntentGate, IntentLedger, IntentOutbox, IntentOutcome,
    PredictedPosition, Proposal, presented_position,
};
pub use map::{IdEntityMap, LocalOnly, WorldName, WorldPosition};
pub use projection::{AppliedPosition, applied_position, apply_position, project_fixture};
pub use schema::{BridgeSchema, ComponentRule, DataClass, PredicateRule};
pub use tail::{
    GameplaySet, InputSet, JournalHalt, JournalLag, JournalSet, JournalSource, JournalTail,
    LagBudget, PositionBatch, configure_bridge_schedule,
};
