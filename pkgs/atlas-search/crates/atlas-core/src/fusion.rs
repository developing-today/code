//! The FTS match-expression builder and reciprocal rank fusion —
//! ports of the Python's `fts_match_expr` and the fusion block of
//! `run_search`.

use std::collections::HashMap;

use serde_json::{Map, Value as Json};

use crate::types::BranchHit;

/// Production RRF constant (decided 2026-10-31; the Python's `RRF_K`).
pub const RRF_K: usize = 10;

/// The Python's `fts_match_expr` (the eval3.py hybrid-branch
/// construction): content words OR-joined as quoted phrases. Words
/// >3 chars, plus short tokens containing a digit (r2, s3, b2) which
/// > the >3 rule would otherwise silently drop.
pub fn fts_match_expr(query: &str) -> Option<String> {
    let lower = query.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            cur.push(ch);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let kept: Vec<&String> = words
        .iter()
        .filter(|w| {
            let n = w.chars().count();
            n > 3 || (n >= 2 && w.chars().any(|c| c.is_ascii_digit()))
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(
            kept.iter()
                .map(|w| format!("\"{w}\""))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

/// One fused candidate (the Python's per-chunk fusion dict).
#[derive(Debug, Clone, PartialEq)]
pub struct FusedEntry {
    pub chunk_id: i64,
    pub score: f64,
    pub fts_rank: Option<usize>,
    pub vector_rank: Option<usize>,
}

impl FusedEntry {
    pub fn min_rank(&self) -> usize {
        match (self.fts_rank, self.vector_rank) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => usize::MAX,
        }
    }

    /// `rank_sources` in the Python's insertion order (fts, vector).
    pub fn rank_sources_json(&self) -> Json {
        let mut m = Map::new();
        if let Some(r) = self.fts_rank {
            m.insert("fts".to_string(), Json::from(r));
        }
        if let Some(r) = self.vector_rank {
            m.insert("vector".to_string(), Json::from(r));
        }
        Json::Object(m)
    }
}

/// Fuse the two branch pools: score += 1/(k+rank) per branch, then
/// order by (-score, best rank, chunk_id) — the Python's sort key.
/// Also returns the per-chunk vector distance map the envelope
/// renders (every vector-pool chunk, not only the kept ones).
pub fn fuse(
    fts: &[BranchHit],
    vector: &[BranchHit],
) -> (Vec<FusedEntry>, HashMap<i64, Option<f64>>) {
    let mut entries: Vec<FusedEntry> = Vec::new();
    let mut by_id: HashMap<i64, usize> = HashMap::new();
    let mut get_or_insert = |entries: &mut Vec<FusedEntry>, cid: i64| -> usize {
        if let Some(&i) = by_id.get(&cid) {
            return i;
        }
        let i = entries.len();
        entries.push(FusedEntry {
            chunk_id: cid,
            score: 0.0,
            fts_rank: None,
            vector_rank: None,
        });
        by_id.insert(cid, i);
        i
    };
    for (i, hit) in fts.iter().enumerate() {
        let rank = i + 1;
        let idx = get_or_insert(&mut entries, hit.chunk_id);
        entries[idx].score += 1.0 / (RRF_K + rank) as f64;
        entries[idx].fts_rank = Some(rank);
    }
    let mut vec_dist: HashMap<i64, Option<f64>> = HashMap::new();
    for (i, hit) in vector.iter().enumerate() {
        let rank = i + 1;
        let idx = get_or_insert(&mut entries, hit.chunk_id);
        entries[idx].score += 1.0 / (RRF_K + rank) as f64;
        entries[idx].vector_rank = Some(rank);
        vec_dist.insert(hit.chunk_id, hit.dist);
    }
    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.min_rank().cmp(&b.min_rank()))
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    (entries, vec_dist)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::round6;

    fn hit(cid: i64, dist: Option<f64>) -> BranchHit {
        BranchHit {
            chunk_id: cid,
            dist,
        }
    }

    #[test]
    fn match_expr_rules() {
        assert_eq!(
            fts_match_expr("how do we back up the database").as_deref(),
            Some("\"back\" OR \"database\"")
        );
        // short digit tokens survive; plain short words do not
        assert_eq!(
            fts_match_expr("is r2 down or s3").as_deref(),
            Some("\"r2\" OR \"down\" OR \"s3\"")
        );
        assert_eq!(fts_match_expr("how do we"), None);
        assert_eq!(fts_match_expr(""), None);
        // case folds; punctuation separates
        assert_eq!(
            fts_match_expr("Back-up, DATABASE!").as_deref(),
            Some("\"back\" OR \"database\"")
        );
    }

    #[test]
    fn rrf_known_decompositions() {
        // 1/(10+1) + 1/(10+1) = 0.181818 (fixture hybrid_default top hit)
        let (fused, _) = fuse(&[hit(1, None)], &[hit(1, Some(0.001))]);
        assert_eq!(round6(fused[0].score), 0.181818);
        // 1/(10+2) + 1/(10+1) = 0.174242 (production-verified example)
        let (fused, _) = fuse(
            &[hit(9, None), hit(1, None)],
            &[hit(1, Some(0.001)), hit(9, Some(0.5))],
        );
        assert_eq!(fused[0].chunk_id, 1);
        assert_eq!(round6(fused[0].score), 0.174242);
        assert_eq!(
            fused[0].rank_sources_json().to_string(),
            "{\"fts\":2,\"vector\":1}"
        );
        // single-branch: 1/(10+1) = 0.090909
        let (fused, _) = fuse(&[], &[hit(7, Some(0.9))]);
        assert_eq!(round6(fused[0].score), 0.090909);
        // 1/(10+3) + 1/(10+84) = 0.087561 (production-verified example)
        let mut vec: Vec<BranchHit> = (0..83).map(|i| hit(1000 + i, Some(0.5))).collect();
        vec.push(hit(42, Some(0.5)));
        let (fused, dist) = fuse(&[hit(1, None), hit(2, None), hit(42, None)], &vec);
        let e42 = fused.iter().find(|e| e.chunk_id == 42).unwrap();
        assert_eq!(round6(e42.score), 0.087561);
        assert_eq!(dist.get(&42), Some(&Some(0.5)));
    }

    #[test]
    fn fuse_sort_tie_breaks() {
        // equal fused scores fall to best rank, then chunk_id
        let (fused, _) = fuse(
            &[hit(5, None), hit(3, None)],
            &[hit(3, Some(0.1)), hit(5, Some(0.2))],
        );
        // both chunks: 1/11+1/12 vs 1/12+1/11 -- identical sums; chunk 3
        // has best rank 1 (vector), chunk 5 has best rank 1 (fts):
        // still tied -> chunk_id order
        assert_eq!(fused[0].score, fused[1].score);
        assert_eq!(fused[0].min_rank(), fused[1].min_rank());
        assert_eq!(fused[0].chunk_id, 3);
        assert_eq!(fused[1].chunk_id, 5);
    }
}
