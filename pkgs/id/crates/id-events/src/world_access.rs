//! The two questions the blob gate and the pull policy ask of the
//! world graph (moved down in step 2b of the crate split, #4405).
//!
//! [`WorldAccess`] is sync-gating vocabulary: a leaf trait over an
//! endpoint ID and a blob hash, with no group-layer logic of its own.
//! The implementations stay where the folded worlds live — the
//! facade's `blob_gate::FoldedWorlds` until step 4 — and the facade
//! re-exports this trait at its historical `id::blob_gate::WorldAccess`
//! path.

use iroh_base::EndpointId;
use iroh_blobs::Hash;

/// The two questions the gate asks of the world graph.
pub trait WorldAccess: Send + Sync + 'static {
    /// Whether `requester` is a current member of `world`.
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool;

    /// The worlds whose content names `hash`. Empty for a hash that no world names, which
    /// includes every unknown hash.
    fn worlds_referencing(&self, hash: &Hash) -> Vec<String>;
}
