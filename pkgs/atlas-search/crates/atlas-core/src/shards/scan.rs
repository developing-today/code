//! The fan-out query path: exact cosine scan on every selected
//! shard in parallel (top-k each), merged by distance.
//!
//! Exactness: each shard returns its own exact top-k, so the global
//! top-k is contained in the union and the merge is exact — the
//! property the restore parity run checked live (7/7 held-out
//! queries identical to the monolith). Tests re-prove it offline
//! against a single-store oracle.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{ShardInfo, ShardSet, Tier, TierCeiling};
use crate::backend::SearchBackend;
use crate::error::{Error, Result};
use crate::filters::{Filters, turso_vector_scan_stmt};
use crate::turso::qvec_json;
use crate::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow, Stmt, VectorBackend,
};

/// One merged hit with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ShardHit {
    pub chunk_id: i64,
    pub dist: f64,
    pub tier: Tier,
    pub shard: String,
}

pub struct FanoutQuery<'a> {
    pub qvec: &'a [f32],
    pub shard_top_k: usize,
    pub global_top_k: usize,
    pub filters: &'a Filters,
    /// Manifest topics to scope to (`None` = every topic).
    pub topics: Option<&'a [String]>,
    pub ceiling: TierCeiling,
}

#[derive(Debug, Clone)]
pub struct FanoutResult {
    pub hits: Vec<ShardHit>,
    /// Names of the shards that were scanned (selection order).
    pub scanned: Vec<String>,
}

/// Merge per-shard hits: ascending distance; ties by tier merge rank
/// (internal first), then shard name, then chunk id. A chunk id seen
/// twice (a re-shard in flight) is kept once, at its best position.
/// Hits without a finite distance are dropped.
pub fn merge_top_k(hits: Vec<ShardHit>, k: usize) -> Vec<ShardHit> {
    let mut hits: Vec<ShardHit> = hits.into_iter().filter(|h| h.dist.is_finite()).collect();
    hits.sort_by(|a, b| {
        a.dist
            .total_cmp(&b.dist)
            .then(a.tier.merge_rank().cmp(&b.tier.merge_rank()))
            .then_with(|| a.shard.cmp(&b.shard))
            .then(a.chunk_id.cmp(&b.chunk_id))
    });
    let mut seen = BTreeSet::new();
    hits.retain(|h| seen.insert(h.chunk_id));
    hits.truncate(k);
    hits
}

/// Scan one shard. The last line of defence for the tier ceiling:
/// even a caller that hands in a shard it should not have refuses
/// here, before any statement is sent.
fn scan_one(
    set: &ShardSet,
    shard: &ShardInfo,
    ceiling: TierCeiling,
    stmt: &Stmt,
) -> Result<Vec<ShardHit>> {
    if !ceiling.admits(shard.tier) {
        return Err(Error::fatal(format!(
            "shard fan-out: refusing to scan {} (tier {} above ceiling {})",
            shard.name, shard.tier, ceiling.0
        )));
    }
    let t = set.transport(&shard.name).ok_or_else(|| {
        Error::backend(format!(
            "shard fan-out: {} has no open transport",
            shard.name
        ))
    })?;
    let rows = t
        .pipeline(std::slice::from_ref(stmt))
        .map_err(|e| Error::backend(format!("shard {}: {e}", shard.name)))?
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(ShardHit {
                chunk_id: r.get_i64("cid")?,
                dist: r.get_f64("dist")?,
                tier: shard.tier,
                shard: shard.name.clone(),
            })
        })
        .collect())
}

/// Fan the exact scan out to the caller's shards in parallel and
/// merge. A failing shard fails the query: a partial answer that
/// silently omits a shard is worse than an error.
pub fn fanout_scan(set: &ShardSet, q: &FanoutQuery<'_>) -> Result<FanoutResult> {
    if q.qvec.is_empty() {
        return Err(Error::fatal("shard fan-out: empty query vector"));
    }
    let (session_clauses, _) = q.filters.session_clauses("s2", "l2");
    if !session_clauses.is_empty() {
        return Err(Error::fatal(
            "shard fan-out does not support classification/date filters \
             (--kind/--topic/--project/--client/--since/--until): scan shards carry no \
             sessions or labels; those queries stay on the monolith",
        ));
    }
    let shards = set.select(q.ceiling, q.topics)?;
    let stmt = turso_vector_scan_stmt(
        set.vec_table(),
        q.filters,
        &qvec_json(q.qvec),
        q.shard_top_k,
    );
    let per_shard: Vec<Result<Vec<ShardHit>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = shards
            .iter()
            .map(|s| scope.spawn(|| scan_one(set, s, q.ceiling, &stmt)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::backend("shard scan worker panicked")))
            })
            .collect()
    });
    let mut all = Vec::new();
    for r in per_shard {
        all.extend(r?);
    }
    Ok(FanoutResult {
        hits: merge_top_k(all, q.global_top_k),
        scanned: shards.iter().map(|s| s.name.clone()).collect(),
    })
}

/// A [`SearchBackend`] that serves the **vector** branch from the
/// shard core and delegates everything else (FTS, details,
/// neighbors, messages, notes) to the wrapped backend — the libSQL
/// monolith until cutover. Chunk ids are the monolith's (the shards
/// were restored from it with ids preserved), so the monolith
/// hydrates what the shards ranked.
///
/// Keyword search deliberately stays on the monolith: TursoDB MVCC
/// shards have no `chunks_fts` (#5669 deviation, #5755 FTS gap).
pub struct ShardedBackend<B> {
    inner: B,
    set: Arc<ShardSet>,
    topics: Option<Vec<String>>,
}

impl<B: SearchBackend> ShardedBackend<B> {
    pub fn new(inner: B, set: Arc<ShardSet>) -> ShardedBackend<B> {
        ShardedBackend {
            inner,
            set,
            topics: None,
        }
    }

    /// Scope vector scans to these manifest topics.
    pub fn with_topics(mut self, topics: Vec<String>) -> ShardedBackend<B> {
        self.topics = Some(topics);
        self
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn shard_set(&self) -> &Arc<ShardSet> {
        &self.set
    }
}

fn vector_pool(
    set: &ShardSet,
    topics: Option<&[String]>,
    q: &BranchQuery,
    filters: &Filters,
) -> Result<Vec<BranchHit>> {
    let Some(qvec) = q.query_vector.as_deref() else {
        return Ok(Vec::new());
    };
    if q.vector_backend != VectorBackend::Scan {
        return Err(Error::fatal(
            "--vector-backend index modes are not available on the shard fan-out \
             (exact-cosine scan only)",
        ));
    }
    let res = fanout_scan(
        set,
        &FanoutQuery {
            qvec,
            shard_top_k: q.shard_top_k,
            global_top_k: q.global_top_k,
            filters,
            topics,
            ceiling: TierCeiling::from_sensitivity(filters.max_sensitivity),
        },
    )?;
    Ok(res
        .hits
        .into_iter()
        .map(|h| BranchHit {
            chunk_id: h.chunk_id,
            dist: Some(h.dist),
        })
        .collect())
}

impl<B: SearchBackend> SearchBackend for ShardedBackend<B> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn vec_table(&self) -> &str {
        self.inner.vec_table()
    }

    fn notes(&self) -> Vec<String> {
        let mut notes = self.inner.notes();
        notes.push(format!(
            "vector branch served by shard fan-out over {} shards (snapshot {})",
            self.set.shards().len(),
            &self.set.snapshot_id()[..12]
        ));
        notes
    }

    fn has_vector_store(&self) -> Option<bool> {
        self.inner.has_vector_store()
    }

    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools> {
        // FTS (if asked) on the wrapped backend, vector on the
        // shards, concurrently; fusion happens upstream over both.
        let fts_query = BranchQuery {
            match_expr: q.match_expr.clone(),
            query_vector: None,
            pool: q.pool,
            shard_top_k: q.shard_top_k,
            global_top_k: q.global_top_k,
            vector_backend: q.vector_backend,
        };
        let (set, topics) = (&*self.set, self.topics.as_deref());
        std::thread::scope(|scope| {
            let vec_handle = q
                .query_vector
                .is_some()
                .then(|| scope.spawn(|| vector_pool(set, topics, q, filters)));
            let mut pools = BranchPools::default();
            if q.match_expr.is_some() {
                pools.fts = self.inner.branch_pools(&fts_query, filters)?.fts;
            }
            if let Some(h) = vec_handle {
                pools.vector = h
                    .join()
                    .map_err(|_| Error::backend("shard vector branch panicked"))??;
            }
            Ok(pools)
        })
    }

    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        self.inner.chunk_details(ids)
    }

    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>> {
        self.inner.neighbors(message_id, chunk_idx, n)
    }

    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        self.inner.message(message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shards::{ShardEndpoint, ShardLocation, ShardTransport};
    use crate::types::Row;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>);
    impl crate::turso::PipelineTransport for Counting {
        fn pipeline(&self, _s: &[Stmt]) -> Result<Vec<Vec<Row>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![vec![]])
        }
    }

    #[test]
    fn scan_one_refuses_a_shard_above_the_ceiling_before_sending_anything() {
        let calls = Arc::new(AtomicUsize::new(0));
        let info = ShardInfo {
            name: "atlas-core-secret-main-01".into(),
            tier: Tier::Secret,
            topic: "main".into(),
            part: 1,
            expected_chunks: None,
            chunks: 0,
            endpoint: ShardEndpoint {
                name: "atlas-core-secret-main-01".into(),
                location: ShardLocation::File("/nonexistent".into()),
            },
        };
        let t: Arc<dyn ShardTransport> = Arc::new(Counting(calls.clone()));
        let set = ShardSet {
            shards: vec![info.clone()],
            transports: BTreeMap::from([(info.name.clone(), t)]),
            vec_table: "v".into(),
            discovery_ceiling: TierCeiling(Tier::Secret),
            rejections: vec![],
        };
        let stmt = Stmt::new("SELECT 1", vec![]);
        let err = scan_one(&set, &info, TierCeiling(Tier::Internal), &stmt).unwrap_err();
        assert!(err.to_string().contains("above ceiling"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was sent");
        // The same shard under a secret ceiling is scanned.
        scan_one(&set, &info, TierCeiling(Tier::Secret), &stmt).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn hit(id: i64, dist: f64, tier: Tier, shard: &str) -> ShardHit {
        ShardHit {
            chunk_id: id,
            dist,
            tier,
            shard: shard.to_string(),
        }
    }

    #[test]
    fn merge_orders_by_distance_then_internal_first() {
        let merged = merge_top_k(
            vec![
                hit(1, 0.30, Tier::Secret, "s"),
                hit(2, 0.10, Tier::Internal, "b"),
                hit(3, 0.10, Tier::Secret, "a"),
                hit(4, 0.10, Tier::Internal, "a"),
                hit(5, 0.05, Tier::Public, "p"),
            ],
            4,
        );
        let ids: Vec<i64> = merged.iter().map(|h| h.chunk_id).collect();
        // 0.05 first; the 0.10 tie: internal (a, b) before secret.
        assert_eq!(ids, [5, 4, 2, 3]);
    }

    #[test]
    fn merge_dedupes_ids_drops_nan_and_truncates() {
        let merged = merge_top_k(
            vec![
                hit(1, 0.2, Tier::Internal, "b"),
                hit(1, 0.1, Tier::Internal, "a"),
                hit(2, f64::NAN, Tier::Internal, "a"),
                hit(3, 0.3, Tier::Internal, "a"),
            ],
            2,
        );
        assert_eq!(
            merged
                .iter()
                .map(|h| (h.chunk_id, h.shard.as_str()))
                .collect::<Vec<_>>(),
            [(1, "a"), (3, "a")]
        );
        assert!(merge_top_k(vec![hit(1, 0.1, Tier::Internal, "a")], 0).is_empty());
    }
}
