//! Who may fetch or observe a blob (P3.4, decision 5) — facade shim
//! over `id-groups` and `id-worlds`.
//!
//! Step 4 of the pkgs/id crate split (#4407): the gate itself —
//! [`BlobPolicy`], [`Request`], [`decide`], [`decide_with_blocks`] and
//! the provider wiring in [`blob_events`] — lives in
//! `id_groups::blob_gate`, over the [`WorldAccess`] trait the events
//! layer owns (step 2b). Step 5 (#4408) moved [`FoldedWorlds`], the
//! `WorldAccess` implementation over the folded worlds, into
//! `id_worlds::blob_gate` together with seam 6's bridge file
//! (`groups::worlds`), which it folds through. It cannot live in
//! `id-groups`: that crate is below `id-worlds` and must not name
//! world types. This facade module now holds nothing but the
//! re-exports, so every historical `id::blob_gate` path is unchanged.
//!
//! The gate's own documentation, in `id_groups::blob_gate`: a fetch
//! is allowed only to a current member of a world that references
//! the hash; observe needs the same membership and a server policy
//! that serves observe for that world; refusals look the same
//! whether the hash is unreferenced or the requester may not reach
//! it; and an unknown requester is refused.

pub use id_groups::blob_gate::{
    BlobPolicy, Request, blob_events, blob_events_with_blocks, build_reference_index, decide,
    decide_with_blocks, decide_with_local, hashseq_children,
};

/// The two questions the gate asks of the world graph, re-exported at
/// its historical path (the trait itself is `id_events::world_access`).
pub use id_events::world_access::WorldAccess;

/// [`WorldAccess`] over the folded worlds; the implementation lives
/// in `id_worlds::blob_gate`.
pub use id_worlds::blob_gate::FoldedWorlds;

/// [`FoldedWorlds`] whose fold a producer can replace as it changes,
/// plus the group-log fold and the world key a group is indexed under;
/// the implementations live in `id_worlds::blob_gate`.
pub use id_worlds::blob_gate::{RefreshingWorlds, fold_group_events, world_key};
