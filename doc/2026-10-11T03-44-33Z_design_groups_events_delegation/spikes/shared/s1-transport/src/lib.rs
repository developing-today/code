//! Shared code for spike S1 (event transport).
//!
//! - [`header`]: event header on p2panda-core's 0-based header, and our own verifier.
//! - [`log_sizes`]: wire sizes of p2panda-sync's LogSync messages.
//! - [`range_sim`]: range-based reconciliation, simulated in memory.
//! - [`framed`]: length-prefixed request/reply over iroh streams (transport spike).

pub mod framed;
pub mod header;
pub mod log_sizes;
pub mod range_sim;
