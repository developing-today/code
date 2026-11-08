//! Conversion-executor proof of concept (#2289, from the #2063 design).
//!
//! **This is a PoC, not the permanent feature.** Per the program rule
//! on #2056, nothing production-shaped may be built on top of it
//! without a review round.
//!
//! Contents:
//! - `ledger` — TigerBeetle-shaped types, the [`ledger::LedgerBackend`]
//!   seam, and an in-memory backend implementing the TigerBeetle
//!   semantics the executor relies on (atomic linked chains,
//!   idempotent transfer IDs, balance-invariant flags).
//! - `intent` — write-ahead intent records, persisted before any
//!   submission, with in-memory and file-backed stores.
//! - `executor` — the conversion executor: LP pre-flight, write-ahead
//!   intent, one linked batch (T1 user→LP, optional fee leg, T3
//!   LP→user), every leg stamped `user_data_128 = conversion_id`.
//! - `recovery` — the recovery scanner reconciling unresolved intents
//!   by pre-generated transfer-ID lookup.
//! - `harness` — step 0: a throwaway single-replica TigerBeetle test
//!   cluster (pinned binary, start/stop) for running the same
//!   scenarios against a real cluster once a client binding lands.
//!
//! Scenario coverage (the #2063 §4 cases) is in `tests`: a posted
//! conversion findable by `user_data_128`, atomic failure moving
//! nothing, crash-recovery without double-posting (both crash
//! windows), and LP pre-flight rejection.
//!
//! Results are reported on #2289 and linked from #2063.

pub mod executor;
pub mod harness;
pub mod intent;
pub mod ledger;
pub mod recovery;

#[cfg(test)]
mod tests;
