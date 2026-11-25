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

/// The most tasks one batch claim may take (#8217). A batch is
/// only legitimate paired with immediate per-item dispatch, and
/// the dispatcher must launch + verify-bind every item inside one
/// touch before leases age: five dedicated launches fit that
/// window with margin, and a bigger batch would recreate the
/// claimed-and-idle pile the single-session design forbids.
///
/// The bound lives at the crate root because both surfaces share
/// it: the client validates `--count` locally so a bad value
/// fails instantly, and the broker enforces the same bound on
/// the wire request (`turso_queue` re-exports it from here).
/// Before #13078 the definition sat inside the broker-gated
/// `turso_queue` tree, so a client-only build could not compile.
pub const BATCH_CLAIM_MAX: i64 = 5;

/// The bundle cap (#11333): one launch — one (lane, session)
/// pair — may make at most this many ordinary claims. The
/// scheduled-lane runbooks promise "up to 10 tasks per launch",
/// but until #11333 the only enforcement was the claiming
/// model's own count, and a claim loop walked straight past it
/// (a scheduled session claimed 12 on 2026-11-17). The broker
/// now counts a launch's ordinary claims itself and refuses the
/// claim that would pass the cap; see
/// `turso_queue::lifecycle::bundle_claim_count`. Defined at the
/// crate root beside BATCH_CLAIM_MAX so both surfaces share the
/// value.
pub const BUNDLE_CLAIM_CAP: i64 = 10;

pub mod backoff;
pub mod bin_args;
pub mod creation_outbox;
#[cfg(any(feature = "client", feature = "queue-broker"))]
pub mod disposition;
mod home;
pub mod log;
pub mod merge_intent;
pub mod token_file;
pub mod work;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod fallback;
#[cfg(feature = "client")]
pub mod needs_drew;
pub mod person_wait;
#[cfg(feature = "client")]
pub mod placement;
#[cfg(feature = "queue-broker")]
pub mod queue_broker;
#[cfg(feature = "secrets-broker")]
pub mod secrets_broker;
#[cfg(any(feature = "queue-broker", feature = "secrets-broker"))]
mod tcp_http;
// BT-3 (#14401): broker TLS Phase 1 — the server half serves
// the queue broker's TCP leg; the client half validates it
// against the BT-2 trust store.
#[cfg(any(feature = "client", feature = "queue-broker"))]
pub mod broker_tls;
#[cfg(feature = "queue-broker")]
pub mod turso_queue;
#[cfg(any(feature = "queue-broker", feature = "secrets-broker"))]
pub mod uds_http;
// The validation library is shared by both surfaces: the broker
// validates whole packets, and the client validates individual
// flag values (e.g. `--not-before`) against the same rules, so
// the module compiles for either surface (#13078). It depends
// only on serde_json, never on a broker-only dependency.
#[cfg(any(feature = "client", feature = "queue-broker"))]
pub mod validate;
