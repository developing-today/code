//! Events layer of the `id` crate.
//!
//! Step 2b of the pkgs/id crate split (#3447 umbrella, #4405): the
//! event-store family lives here, together with the vocabulary the
//! design ruling on #4405 assigns to the events layer:
//!
//! - [`groups`]: the group model and [`groups::fold`] — a deterministic
//!   projection over the event log. Resolution/roster inside the event
//!   store consumes the fold, so the model and fold sit below the
//!   policy/enforcement remainder of the group layer, which follows in
//!   step 4 (`id-groups`) and will depend on this crate.
//! - [`idset`]: the writer-set surface. Its only group references were
//!   [`groups::Role`] and [`groups::fold`], both here now, so its deps
//!   resolve downward.
//! - [`pull_policy`], [`world_access`], [`local_claims`]: the
//!   sync-gating value types — small leaf-like policy vocabulary the
//!   event sync paths name. The facade's blob gate keeps its
//!   enforcement logic and re-exports [`world_access::WorldAccess`].
//!
//! The facade re-exports each module at its historical `id::` path,
//! so no call site outside the moved modules changed.

pub mod attestation;
pub mod event_store;
pub mod event_sync;
pub mod groups;
pub mod idset;
pub mod local_claims;
pub mod local_events;
pub mod pull_policy;
pub mod residency_sync;
pub mod world_access;
