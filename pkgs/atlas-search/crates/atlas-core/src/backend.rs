//! The `SearchBackend` seam (design section 4): everything the
//! search orchestration needs from a data source, with the ceiling
//! enforced inside branch construction. Two implementations ship in
//! this crate: [`crate::turso::TursoBackend`] (libsql HTTPS pipeline,
//! read-only token) and [`crate::local::LocalBackend`] (SQLite
//! publish files; side stores joined by content hash).

use crate::error::Result;
use crate::filters::Filters;
use crate::filters::Sensitivity;
use crate::occurrences::ChunkOccurrence;
use crate::shards::TierCeiling;
use crate::types::{BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow};

/// The window (in chunk_idx units) the default
/// [`SearchBackend::message_chunks`] composes from
/// [`SearchBackend::neighbors`]: wide enough to cover any
/// message in the corpus — chunk_idx is sparse in the publish
/// corpus (observed span 0..=1730, measured 2026-11-22 for
/// #12624) — with orders of magnitude of headroom.
pub const MESSAGE_CHUNKS_WINDOW: i64 = 1_000_000;

pub trait SearchBackend {
    /// `"turso"` or `"local"` (the envelope's `backend` field).
    fn name(&self) -> &'static str;

    fn vec_table(&self) -> &str;

    /// The corpus snapshot this backend currently serves, when it
    /// knows one: `ShardSet::snapshot_id()` for a sharded backend
    /// (#5668 slice 3, issue #9943). The service's snapshot-bound
    /// result cache keys entries by it, so a snapshot bump (a
    /// re-shard, a promotion batch) invalidates every cached
    /// result at once. Backends with no shard set (the Turso
    /// monolith) return `None`: their corpus state has no snapshot
    /// id to bind, and the cache treats them as one stable
    /// snapshot.
    fn snapshot_id(&self) -> Option<String> {
        None
    }

    /// The effective filter-surface manifest this backend serves
    /// under (#6228 Phase 3/4): the manifest loaded from the corpus
    /// by [`crate::manifest::load_manifest`], or the built-in
    /// surface. Serve mode binds its cache keys to this manifest's
    /// `(version, content_hash)`, so a surface change can never be
    /// answered from entries computed under another surface. The
    /// default is the built-in surface, matching the loader's
    /// fallback for backends that carry no manifest.
    fn manifest(&self) -> crate::manifest::Manifest {
        crate::manifest::builtin_manifest()
    }

    /// Notes the backend has accumulated (setup notes, load-time
    /// coverage notes), in the order the Python appends them to
    /// `be.notes`. The orchestration snapshots this before and after
    /// the branch run, mirroring the Python's late-note fold.
    fn notes(&self) -> Vec<String>;

    /// Whether a vector store backs this backend (`None` = the
    /// question does not apply, i.e. Turso). Drives the Python's
    /// "no vector store found" note for the local backend.
    fn has_vector_store(&self) -> Option<bool> {
        None
    }

    /// Run the branches and return the pre-filtered, ranked pools.
    /// Classification filters and the sensitivity ceiling are
    /// applied inside, BEFORE ranking — a restricted chunk is never
    /// ranked, scored, or snippeted for a caller whose ceiling
    /// excludes it.
    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools>;

    /// Detail rows for explicit chunk ids (the Python's
    /// `fetch_details`). Like the Python, this is a by-id lookup
    /// with no ceiling filtering: the search path only ever passes
    /// ids its pre-filtered branches produced, and `get-chunk` is an
    /// explicit-id tool. Absent ids are simply missing from the
    /// result.
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>>;

    /// Chunk ids whose stored canonical content hash matches exactly.
    /// Backends without a hash index expose no candidates.
    fn chunk_ids_by_hash(&self, _canonical_hash: &str) -> Result<Vec<i64>> {
        Ok(Vec::new())
    }

    /// Same-message neighbor rows around (`message_id`, any
    /// `chunk_idx` in the window), ordered by chunk_idx.
    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>>;

    /// Every chunk row of one message, ordered by chunk_idx:
    /// the enumeration the full-content span filter (#12624)
    /// classifies before a `--with-message` / hydration payload
    /// may cross the output boundary. The default composes
    /// [`Self::neighbors`] with a window wide enough to cover
    /// any message ([`MESSAGE_CHUNKS_WINDOW`]); backends with a
    /// native unbounded query override it. Wrappers that route
    /// `neighbors` (the shard fan-out) inherit correct routing
    /// through the default.
    fn message_chunks(&self, message_id: &str) -> Result<Vec<NeighborRow>> {
        self.neighbors(message_id, 0, MESSAGE_CHUNKS_WINDOW)
    }

    /// Every recorded occurrence of the chunk content whose
    /// canonical hash is `canonical_hash` (#3125): the shard
    /// fan-out answers from its `chunk_occurrences` registries
    /// under `ceiling`; every other backend answers empty.
    /// `ceiling` admits a shard only under the SAME tier ladder
    /// the vector fan-out enforces, so a restricted caller never
    /// learns higher-tier source locators from a hash it holds.
    fn occurrences(
        &self,
        canonical_hash: &str,
        tier_ceiling: TierCeiling,
        caller_ceiling: Sensitivity,
    ) -> Result<Vec<ChunkOccurrence>> {
        let _ = (canonical_hash, tier_ceiling, caller_ceiling);
        Ok(Vec::new())
    }

    /// One message row by id (for `--with-message` payloads).
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>>;
}

/// Boxed backends (the CLI's `pick_backend`) compose like any other.
impl<T: SearchBackend + ?Sized> SearchBackend for Box<T> {
    fn name(&self) -> &'static str {
        (**self).name()
    }
    fn vec_table(&self) -> &str {
        (**self).vec_table()
    }
    fn snapshot_id(&self) -> Option<String> {
        (**self).snapshot_id()
    }
    fn manifest(&self) -> crate::manifest::Manifest {
        (**self).manifest()
    }
    fn notes(&self) -> Vec<String> {
        (**self).notes()
    }
    fn has_vector_store(&self) -> Option<bool> {
        (**self).has_vector_store()
    }
    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools> {
        (**self).branch_pools(q, filters)
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        (**self).chunk_details(ids)
    }
    fn chunk_ids_by_hash(&self, canonical_hash: &str) -> Result<Vec<i64>> {
        (**self).chunk_ids_by_hash(canonical_hash)
    }
    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>> {
        (**self).neighbors(message_id, chunk_idx, n)
    }
    fn message_chunks(&self, message_id: &str) -> Result<Vec<NeighborRow>> {
        (**self).message_chunks(message_id)
    }
    fn occurrences(
        &self,
        canonical_hash: &str,
        tier_ceiling: TierCeiling,
        caller_ceiling: Sensitivity,
    ) -> Result<Vec<ChunkOccurrence>> {
        (**self).occurrences(canonical_hash, tier_ceiling, caller_ceiling)
    }
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        (**self).message(message_id)
    }
}
