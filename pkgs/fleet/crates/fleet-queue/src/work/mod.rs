//! Work-item domain models for the PM orchestration design
//! (developing-today/code#9196): pure, persistence-free models that
//! the queue/work surfaces consume. Each submodule owns one design
//! slice's contract and its tests; slices land independently as
//! their design gates clear.

pub mod authority;
