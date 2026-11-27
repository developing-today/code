//! Work-item domain models for the PM orchestration design
//! (developing-today/code#9196): pure, persistence-free models that
//! the queue/work surfaces consume. Each submodule owns one design
//! slice's contract and its tests; slices land independently as
//! their design gates clear.

pub mod authority;
pub mod completion;
pub mod context;
pub mod contracts;
pub mod degraded;
pub mod journal;
pub mod lifecycle;
pub mod operations;
pub mod priority;
pub mod relations;
pub mod routing;
pub mod selection;
#[cfg(feature = "queue-broker")]
pub mod sqlite_replica;
