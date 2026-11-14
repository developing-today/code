//! The `SearchBackend` seam (design section 4): everything the
//! search orchestration needs from a data source, with the ceiling
//! enforced inside branch construction. Two implementations ship in
//! this crate: [`crate::turso::TursoBackend`] (libsql HTTPS pipeline,
//! read-only token) and [`crate::local::LocalBackend`] (SQLite
//! publish files; side stores joined by content hash).

use crate::error::Result;
use crate::filters::Filters;
use crate::types::{BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow};

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

    /// Same-message neighbor rows around (`message_id`, any
    /// `chunk_idx` in the window), ordered by chunk_idx.
    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>>;

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
    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>> {
        (**self).neighbors(message_id, chunk_idx, n)
    }
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        (**self).message(message_id)
    }
}
