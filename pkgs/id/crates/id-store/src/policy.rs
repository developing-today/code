//! Group policy vocabulary that sits below the groups layer.
//!
//! For now this is only [`History`]: the chat module in this crate narrows
//! a timeline by a channel group's history policy, so the shape lives here
//! (step 2a of the crate split, #4405). The policy record itself stays in
//! the facade's `groups::policy` until step 4 — it names the groups
//! layer's own `Key`/`Role` and its wire helpers — and re-exports this
//! enum at its historical path.

use serde::{Deserialize, Serialize};

/// What a joiner may see of earlier history (design §8.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum History {
    /// The whole history.
    #[default]
    Full,
    /// The last N epochs (P4.5). A joiner reads older messages only by asking a member (P4.6).
    /// N is at least 1.
    Recent(u32),
    /// Only what came after the join.
    FromJoin,
    /// Causal history (deferred).
    Causal,
}
