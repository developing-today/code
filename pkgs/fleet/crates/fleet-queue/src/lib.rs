//! fleet-queue — the queue domain of the fleet multicall binary
//! (design note §1/§6, epic developing-today/code#5412; step 2,
//! #5430: the pilot).
//!
//! This crate is the first real source home of the broker family
//! (code#5362): the `repo-queue` client surface and the
//! queue-broker + secrets-broker server roles, ported from the
//! sandbox `~/tools/fleet-brokers` workspace (archived in place the
//! day this landed; the laptop's `~/fleet-brokers/bin` copy is
//! superseded by the unit flips of the same step). Wire behavior is
//! preserved exactly — the brokers speak to existing clients, and
//! the laptop-agent session proxy keeps working as a client of the
//! brokers — while the internals sit on the platform crates:
//! secrets resolve by name through `fleet-secrets`, and logging is
//! one facility with the domain as a field ([`log`]).
//!
//! Surfaces are feature-gated so each shipped variant compiles
//! exactly what it carries: `client`, `queue-broker`,
//! `secrets-broker`. The crate also ships the three standalone
//! binaries under their historical names (src/bin/) — the
//! migration's standing rollback inventory (design note §7).

pub mod backoff;
pub mod bin_args;
pub mod log;
pub mod token_file;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod fallback;
#[cfg(feature = "queue-broker")]
pub mod queue_broker;
#[cfg(feature = "secrets-broker")]
pub mod secrets_broker;
#[cfg(any(feature = "queue-broker", feature = "secrets-broker"))]
pub mod uds_http;
#[cfg(feature = "queue-broker")]
pub mod validate;
