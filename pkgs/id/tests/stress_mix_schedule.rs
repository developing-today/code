//! Thin target for the stress event-shape schedule oracles (#16044).
//!
//! The schedule and its invariant tests live in
//! [`common::stress_mix`](crate::common::stress_mix) — the same module
//! the `world_bench` stress generator runs — so this target's only job
//! is to pull `common` in and give the oracles a narrow shard and a
//! narrow `cargo test --test` filter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
