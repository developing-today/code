//! Reading the `chunk_occurrences` provenance registry (#3125):
//! the rows the aggregator's promotion writes (aggregate.rs) are
//! resolved back to "every place this content was found" — the
//! read side rule 3 requires.
//!
//! The lookup key is the **canonical hash** (SHA-256 of the piece
//! text, the same digest the aggregator records), not a chunk id:
//! chunk ids are unique only within one shard, while the canonical
//! hash is global content identity. A lookup fans out over the
//! shards a caller's ceiling admits, reads each shard's
//! `chunk_occurrences_hash` index, and tolerates shards promoted
//! before the registry existed (no `chunk_occurrences` table):
//! they contribute no rows, never an error — a provenance read
//! must not take search down with it.

use serde_json::Value as Json;

use crate::error::{Error, Result};
use crate::shards::{ShardSet, Tier, TierCeiling};
use crate::types::{Stmt, Value};

/// One recorded place a chunk's content was found (#3125 rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkOccurrence {
    /// The shard that recorded the occurrence.
    pub shard: String,
    /// That shard's physical tier.
    pub tier: Tier,
    /// SHA-256 of the piece text (content identity, the lookup key).
    pub canonical_hash: String,
    /// The source locator: `repo@commit:path`, `system@host@date`,
    /// or `unit:<id>` for unit-staged items without a source.
    pub source: String,
    /// Byte offsets in the original staged item.
    pub span_start: i64,
    pub span_end: i64,
    /// When the aggregator observed the content.
    pub observed_at: String,
}

impl ChunkOccurrence {
    /// The wire form `get-chunk` attaches as one `occurrences`
    /// array element.
    pub fn to_json(&self) -> Json {
        serde_json::json!({
            "shard": self.shard,
            "tier": self.tier.name(),
            "canonical_hash": self.canonical_hash,
            "source": self.source,
            "span_start": self.span_start,
            "span_end": self.span_end,
            "observed_at": self.observed_at,
        })
    }
}

/// Every recorded occurrence of the content whose canonical hash
/// is `canonical_hash`, across the shards `ceiling` admits, in a
/// deterministic order (shard, source, span_start). Shards with
/// no `chunk_occurrences` table (promoted before the registry)
/// are skipped; a shard that answers with a genuine failure fails
/// the lookup — a partial provenance answer is as wrong as a
/// partial search answer.
pub fn lookup_by_hash(
    set: &ShardSet,
    ceiling: TierCeiling,
    canonical_hash: &str,
) -> Result<Vec<ChunkOccurrence>> {
    let mut out = Vec::new();
    for shard in set.select(ceiling, None)? {
        let Some(transport) = set.transport(&shard.name) else {
            return Err(Error::backend(format!(
                "occurrences: {} has no open transport",
                shard.name
            )));
        };
        let rows = match transport.pipeline(&[occurrence_stmt(canonical_hash)]) {
            Ok(mut batches) => batches.pop().unwrap_or_default(),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table") && msg.contains("chunk_occurrences") {
                    // A shard promoted before the registry (#3125
                    // slice 2): no rows to contribute.
                    continue;
                }
                return Err(Error::backend(format!("occurrences: {}: {e}", shard.name)));
            }
        };
        for r in rows {
            let (Some(source), Some(observed_at)) = (r.get_text("source"), r.get_text("observed_at"))
            else {
                return Err(Error::backend(format!(
                    "occurrences: {}: malformed occurrence row",
                    shard.name
                )));
            };
            out.push(ChunkOccurrence {
                shard: shard.name.clone(),
                tier: shard.tier,
                canonical_hash: canonical_hash.to_string(),
                source: source.to_string(),
                span_start: r.get_i64("span_start").unwrap_or_default(),
                span_end: r.get_i64("span_end").unwrap_or_default(),
                observed_at: observed_at.to_string(),
            });
        }
    }
    out.sort_by(|a, b| {
        a.shard
            .cmp(&b.shard)
            .then(a.source.cmp(&b.source))
            .then(a.span_start.cmp(&b.span_start))
    });
    Ok(out)
}

fn occurrence_stmt(canonical_hash: &str) -> Stmt {
    Stmt::new(
        "SELECT source, span_start, span_end, observed_at FROM chunk_occurrences \
         WHERE canonical_hash = ?1 ORDER BY source, span_start",
        vec![Value::text(canonical_hash)],
    )
}
