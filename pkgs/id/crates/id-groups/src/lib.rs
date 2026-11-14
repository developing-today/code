//! Groups layer of the `id` crate.
//!
//! Step 4 of the pkgs/id crate split (#3447 umbrella, #4407): the
//! policy/enforcement remainder of the group layer lives here, over
//! the group model and fold in `id-events` (step 2b ruling on #4405:
//! the model, `fold`, `policy`, `friendship` and the writer-set
//! vocabulary are group-projection vocabulary owned by the events
//! layer, and do not move again):
//!
//! - [`groups`]: the actions a user takes on groups, the group log
//!   one node keeps for its groups, and the local size/churn warnings.
//!   The submodule re-exports the `id_events::groups` model at the
//!   same paths the facade has always presented.
//! - [`blob_gate`]: who may fetch or observe a blob — the gate's
//!   policy, verdicts and provider event wiring over the
//!   [`blob_gate::WorldAccess`] trait from `id-events`. The
//!   `FoldedWorlds` implementation of that trait stays in the facade
//!   with `groups::worlds` until step 5 extracts `id-worlds`.
//! - [`encryption`]: group encryption on the `DataScheme` of
//!   `p2panda-encryption` — the session adapter, causal ordering,
//!   history modes and welcome chunking. Its one upward reference was
//!   the frame cap: `MAX_FRAME_BYTES` already lives in
//!   `id_core::frames` since step 3a (frames moved to id-core; the
//!   facade's `world_session` only re-exports it), so the welcome
//!   chunking names it there.
//!
//! The facade re-exports each module at its historical `id::` path,
//! so no call site outside the moved modules changed.

pub mod blob_gate;
pub mod encryption;
pub mod groups;
