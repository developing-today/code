//! Spike S7: range reconciliation over (author, seq) keys, on real redb stores, over iroh.
//!
//! - [`chain`]: real signed events (p2panda-core headers, 256-byte bodies) and their verification.
//! - [`store`]: the redb event store, with S6's key layout.
//! - [`range`]: S1's range scheme over a store's sorted keys, with a wire encoding.
//! - [`session`]: one sync session over an iroh stream, and the two-endpoint runner.

pub mod chain;
pub mod range;
pub mod session;
pub mod store;
