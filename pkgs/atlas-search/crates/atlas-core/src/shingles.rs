//! Token shingles and the Jaccard rescore signal for Atlas search
//! (issue #5733, design note of 2026-11-05, task 10487).
//!
//! Jaccard is a **third signal at rescore — never a retrieval
//! scorer**: retrieval stays dense cosine + BM25 fused with RRF, and
//! the Jaccard feature is computed only over the fused candidate
//! union (the top `jaccard_depth` entries). Two effects, per the
//! design:
//!
//! - **Near-exact boost**: a candidate whose shingle overlap with the
//!   query is very high is almost certainly *the* pasted/quoted text
//!   itself (an error message, a passage, a title). It is promoted
//!   decisively above every non-near-exact candidate.
//! - **Tie-break / confirmation**: among semantically-equal
//!   candidates, higher literal overlap wins.
//!
//! One normalizer is shared by the query side and the chunk side —
//! there is exactly one implementation, and every shingle set carries
//! [`NORMALIZER_VERSION`]: sets of different versions are never
//! compared (a version-mismatched candidate scores `J = 0` here and
//! is re-shingled lazily on next ingest/touch, per the design's
//! versioning rule).
//!
//! Candidate sets in this slice are computed from chunk text at query
//! time (the rescore stage only ever touches the candidate union, so
//! the cost stays bounded by `jaccard_depth`). The design's stored
//! sets — a side store keyed by content hash, written at ingest and
//! shared with the ingest-dedup half — remain the production
//! optimization and drop into [`rescore_fused`] unchanged, since it
//! consumes sets, not texts.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use crate::fusion::FusedEntry;

/// The normalizer version stamped on every shingle set. Bump on any
/// change to [`normalize_tokens`]; mixed-version sets never compare.
pub const NORMALIZER_VERSION: u32 = 1;

/// Word-shingle width (the design's 5-shingles).
pub const SHINGLE_WIDTH: usize = 5;

/// The design's near-exact tier threshold on Jaccard similarity,
/// used when `jaccard_threshold` is left at its 0.0 default.
pub const DEFAULT_NEAR_EXACT_THRESHOLD: f64 = 0.60;

/// Standard-tier weights (the design's exact formula):
/// `final = 0.85 * rrf_norm + 0.15 * J`.
pub const STANDARD_RRF_WEIGHT: f64 = 0.85;
/// See [`STANDARD_RRF_WEIGHT`].
pub const STANDARD_JACCARD_WEIGHT: f64 = 0.15;

/// Score window inside which two candidates tie-break on Jaccard,
/// then RRF, then chunk id (the design's 0.005).
pub const TIE_WINDOW: f64 = 0.005;

/// The design's shared normalizer: Unicode NFKC → lowercase → strip
/// boilerplate (fenced-code marker lines) → collapse whitespace →
/// tokenize on non-alphanumeric boundaries, keeping code identifiers
/// whole (underscores stay inside a token).
#[must_use]
pub fn normalize_tokens(text: &str) -> Vec<String> {
    let lowered: String = text.nfkc().collect::<String>().to_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in lowered.lines() {
        if line.trim_start().starts_with("```") {
            continue;
        }
        for ch in line.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                current.push(ch);
            } else if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    tokens
}

/// xxHash64 (seed 0) of a byte string — the design's shingle hash.
/// Implemented here to keep the exact algorithm pinned to the design
/// note; verified against the reference test vectors.
#[must_use]
pub fn xxhash64(data: &[u8]) -> u64 {
    const P1: u64 = 11_400_714_785_074_694_791;
    const P2: u64 = 14_029_467_366_897_019_727;
    const P3: u64 = 1_609_587_929_392_839_161;
    const P4: u64 = 9_650_029_242_287_828_579;
    const P5: u64 = 2_870_177_450_012_600_261;

    fn round(acc: u64, input: u64) -> u64 {
        const P1: u64 = 11_400_714_785_074_694_791;
        const P2: u64 = 14_029_467_366_897_019_727;
        acc.wrapping_add(input.wrapping_mul(P2))
            .rotate_left(31)
            .wrapping_mul(P1)
    }
    fn merge(h: u64, val: u64) -> u64 {
        const P1: u64 = 11_400_714_785_074_694_791;
        const P4: u64 = 9_650_029_242_287_828_579;
        let v = round(0, val);
        let h = h ^ v;
        h.wrapping_mul(P1).wrapping_add(P4)
    }
    fn read_u64(b: &[u8]) -> u64 {
        u64::from_le_bytes(b[..8].try_into().expect("8 bytes"))
    }
    fn read_u32(b: &[u8]) -> u32 {
        u32::from_le_bytes(b[..4].try_into().expect("4 bytes"))
    }

    let len = data.len() as u64;
    let mut pos = 0usize;
    let mut h: u64;
    if data.len() >= 32 {
        let mut v1 = P1.wrapping_add(P2);
        let mut v2 = P2;
        let mut v3 = 0u64;
        let mut v4 = 0u64.wrapping_sub(P1);
        while pos + 32 <= data.len() {
            v1 = round(v1, read_u64(&data[pos..]));
            v2 = round(v2, read_u64(&data[pos + 8..]));
            v3 = round(v3, read_u64(&data[pos + 16..]));
            v4 = round(v4, read_u64(&data[pos + 24..]));
            pos += 32;
        }
        h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = merge(h, v1);
        h = merge(h, v2);
        h = merge(h, v3);
        h = merge(h, v4);
    } else {
        h = P5;
    }
    h = h.wrapping_add(len);
    while pos + 8 <= data.len() {
        let k = round(0, read_u64(&data[pos..]));
        h ^= k;
        h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        pos += 8;
    }
    if pos + 4 <= data.len() {
        h ^= u64::from(read_u32(&data[pos..])).wrapping_mul(P1);
        h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        pos += 4;
    }
    for &byte in &data[pos..] {
        h ^= u64::from(byte).wrapping_mul(P5);
        h = h.rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^= h >> 32;
    h
}

/// One chunk's (or the query's) token-shingle set: deduplicated,
/// sorted xxHash64 values of its word 5-shingles, stamped with the
/// normalizer version that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShingleSet {
    /// The normalizer version that produced this set.
    pub version: u32,
    /// Sorted, deduplicated shingle hashes.
    pub hashes: Vec<u64>,
}

impl ShingleSet {
    /// Build the set for a text under the current normalizer. A text
    /// with fewer than [`SHINGLE_WIDTH`] tokens yields one shingle:
    /// its full token sequence (the design's rule). An empty text
    /// yields an empty set.
    #[must_use]
    pub fn from_text(text: &str) -> ShingleSet {
        Self::from_tokens(&normalize_tokens(text))
    }

    /// Build the set from pre-normalized tokens.
    #[must_use]
    pub fn from_tokens(tokens: &[String]) -> ShingleSet {
        let mut hashes: Vec<u64> = Vec::new();
        if tokens.len() < SHINGLE_WIDTH {
            if !tokens.is_empty() {
                hashes.push(xxhash64(tokens.join(" ").as_bytes()));
            }
        } else {
            for window in tokens.windows(SHINGLE_WIDTH) {
                hashes.push(xxhash64(window.join(" ").as_bytes()));
            }
        }
        hashes.sort_unstable();
        hashes.dedup();
        ShingleSet {
            version: NORMALIZER_VERSION,
            hashes,
        }
    }
}

/// Jaccard similarity of two sorted shingle-hash slices:
/// `|A ∩ B| / |A ∪ B|`, with two empty sets defined as 0 (no text,
/// no signal — never a near-exact match).
#[must_use]
pub fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j) = (0usize, 0usize);
    let mut intersection = 0usize;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                intersection += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// What a rescore pass did, for the search envelope's notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescoreReport {
    /// Candidates in the union that were rescored.
    pub rescored: usize,
    /// Candidates that landed in the near-exact tier.
    pub near_exact: usize,
}

/// Apply the design's rescore formula to the first `depth` entries
/// of a fused ranking, in place.
///
/// For each union candidate `c`: `rrf_norm(c)` is its fused score
/// divided by the union's maximum fused score; `J(c)` is the Jaccard
/// similarity of the query's shingle set against the candidate's
/// (0 for a missing or version-mismatched set). Near-exact tier:
/// `J(c) >= threshold` → `final = 1.0 + J(c)`, which always outranks
/// the standard tier's maximum. Standard tier:
/// `final = 0.85 * rrf_norm + 0.15 * J(c)`. Ordering: by `final`
/// descending; scores within [`TIE_WINDOW`] tie-break on `J`
/// descending, then `rrf_norm` descending, then chunk id ascending
/// (deterministic). Entries beyond `depth` are untouched and keep
/// their fused order after the union.
pub fn rescore_fused(
    entries: &mut Vec<FusedEntry>,
    query: &ShingleSet,
    sets: &HashMap<i64, ShingleSet>,
    depth: usize,
    threshold: f64,
) -> RescoreReport {
    let union_len = depth.min(entries.len());
    if union_len == 0 {
        return RescoreReport {
            rescored: 0,
            near_exact: 0,
        };
    }
    let max_rrf = entries[..union_len]
        .iter()
        .map(|e| e.score)
        .fold(0.0f64, f64::max);
    let mut scored: Vec<(FusedEntry, f64, f64, f64)> = Vec::with_capacity(union_len);
    let mut near_exact = 0usize;
    for entry in entries[..union_len].iter() {
        let rrf_norm = if max_rrf > 0.0 {
            entry.score / max_rrf
        } else {
            0.0
        };
        let j = match sets.get(&entry.chunk_id) {
            Some(set) if set.version == query.version => jaccard(&query.hashes, &set.hashes),
            _ => 0.0,
        };
        let final_score = if j >= threshold {
            near_exact += 1;
            1.0 + j
        } else {
            STANDARD_RRF_WEIGHT * rrf_norm + STANDARD_JACCARD_WEIGHT * j
        };
        scored.push((entry.clone(), final_score, j, rrf_norm));
    }
    scored.sort_by(|a, b| {
        let (a_final, a_j, a_rrf) = (a.1, a.2, a.3);
        let (b_final, b_j, b_rrf) = (b.1, b.2, b.3);
        if (a_final - b_final).abs() >= TIE_WINDOW {
            return b_final
                .partial_cmp(&a_final)
                .unwrap_or(std::cmp::Ordering::Equal);
        }
        b_j.partial_cmp(&a_j)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b_rrf
                    .partial_cmp(&a_rrf)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.0.chunk_id.cmp(&b.0.chunk_id))
    });
    for (slot, (mut entry, final_score, _, _)) in scored.into_iter().enumerate() {
        entry.score = final_score;
        entries[slot] = entry;
    }
    RescoreReport {
        rescored: union_len,
        near_exact,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn xxhash64_reference_vectors() {
        // The xxHash64 (seed 0) reference vectors.
        assert_eq!(xxhash64(b""), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxhash64(b"a"), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxhash64(b"abc"), 0x44BC_2CF5_AD77_0999);
    }

    #[test]
    fn normalizer_folds_case_width_and_fences() {
        // NFKC folds fullwidth to ASCII; case folds; the fence line
        // is boilerplate and drops; identifiers keep underscores.
        let tokens = normalize_tokens("```rust\nHello ＷＯＲＬＤ foo_bar\n```");
        assert_eq!(tokens, vec!["hello", "world", "foo_bar"]);
    }

    #[test]
    fn short_text_is_one_shingle() {
        let set = ShingleSet::from_text("alpha beta gamma");
        assert_eq!(set.hashes.len(), 1);
        assert_eq!(set.hashes[0], xxhash64(b"alpha beta gamma"));
        assert!(ShingleSet::from_text("").hashes.is_empty());
    }

    #[test]
    fn shingles_are_five_wide_deduped_sorted() {
        let set = ShingleSet::from_text("a b c d e f");
        assert_eq!(set.hashes.len(), 2);
        let mut sorted = set.hashes.clone();
        sorted.sort_unstable();
        assert_eq!(set.hashes, sorted);
        let repeated = ShingleSet::from_text("x x x x x x");
        assert_eq!(
            repeated.hashes.len(),
            1,
            "identical windows dedupe to one shingle"
        );
    }

    #[test]
    fn jaccard_exact_fractions() {
        assert_eq!(jaccard(&[1, 2, 3], &[2, 3, 4]), 0.5);
        assert_eq!(jaccard(&[1, 2], &[1, 2]), 1.0);
        assert_eq!(jaccard(&[1], &[2]), 0.0);
        assert_eq!(jaccard(&[], &[1]), 0.0);
        assert_eq!(jaccard(&[], &[]), 0.0);
    }

    fn entry(chunk_id: i64, score: f64) -> FusedEntry {
        FusedEntry {
            chunk_id,
            score,
            fts_rank: None,
            vector_rank: None,
        }
    }

    fn set(text: &str) -> ShingleSet {
        ShingleSet::from_text(text)
    }

    #[test]
    fn near_exact_candidate_is_promoted_decisively() {
        // The design's core claim: the pasted passage itself (chunk 2,
        // fused rank 2) outranks the merely topical fused winner once
        // its shingle overlap is near-exact.
        let query = set("the quick brown fox jumps over the lazy dog again");
        let mut entries = vec![entry(1, 0.030), entry(2, 0.020), entry(3, 0.010)];
        let sets = HashMap::from([
            (1, set("quick brown foxes are topical animals in stories")),
            (2, set("the quick brown fox jumps over the lazy dog again")),
            (3, set("something else entirely about dogs and foxes")),
        ]);
        let report = rescore_fused(
            &mut entries,
            &query,
            &sets,
            100,
            DEFAULT_NEAR_EXACT_THRESHOLD,
        );
        assert_eq!(report.near_exact, 1);
        assert_eq!(entries[0].chunk_id, 2);
        assert!(
            entries[0].score >= 1.0,
            "near-exact tier: {}",
            entries[0].score
        );
        assert!(entries[1].score < 1.0 && entries[2].score < 1.0);
    }

    #[test]
    fn standard_tier_blends_by_the_design_weights() {
        // J = 0 (disjoint): final = 0.85 * rrf_norm exactly.
        let query = set("alpha beta gamma delta epsilon zeta");
        let mut entries = vec![entry(1, 0.02), entry(2, 0.01)];
        let sets = HashMap::from([
            (1, set("one two three four five six")),
            (2, set("seven eight nine ten eleven twelve")),
        ]);
        rescore_fused(
            &mut entries,
            &query,
            &sets,
            100,
            DEFAULT_NEAR_EXACT_THRESHOLD,
        );
        assert!((entries[0].score - 0.85).abs() < 1e-12);
        assert!((entries[1].score - 0.85 * 0.5).abs() < 1e-12);
    }

    #[test]
    fn version_mismatched_set_scores_zero() {
        let query = set("the quick brown fox jumps over the lazy dog again");
        let mut stale = set("the quick brown fox jumps over the lazy dog again");
        stale.version = NORMALIZER_VERSION + 1;
        let mut entries = vec![entry(1, 0.02), entry(2, 0.01)];
        let sets = HashMap::from([(2, stale), (1, set("unrelated words here for chunk one"))]);
        let report = rescore_fused(
            &mut entries,
            &query,
            &sets,
            100,
            DEFAULT_NEAR_EXACT_THRESHOLD,
        );
        assert_eq!(report.near_exact, 0);
        assert_eq!(
            entries[0].chunk_id, 1,
            "no near-exact promotion across versions"
        );
    }

    #[test]
    fn depth_caps_the_union() {
        let query = set("the quick brown fox jumps over the lazy dog again");
        let mut entries = vec![entry(1, 0.03), entry(2, 0.02), entry(3, 0.01)];
        let sets = HashMap::from([(3, set("the quick brown fox jumps over the lazy dog again"))]);
        let report = rescore_fused(&mut entries, &query, &sets, 2, DEFAULT_NEAR_EXACT_THRESHOLD);
        assert_eq!(report.rescored, 2);
        assert_eq!(report.near_exact, 0, "chunk 3 is outside the union");
        assert_eq!(entries[2].chunk_id, 3);
        assert_eq!(
            entries[2].score, 0.01,
            "beyond-depth entries keep fused score and place"
        );
    }
}
