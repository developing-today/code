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

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::shards::{ShardSet, Tier, TierCeiling};
use crate::source_registry::resolve_source;
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
    /// When the lookup ran with a source registry attached
    /// (#3125 slice 6) and `source` is registered under a root,
    /// that root's id. `None` when no registry was attached or the
    /// locator is unregistered — the occurrence is then unattributed,
    /// never misattributed.
    pub root_id: Option<String>,
}

impl ChunkOccurrence {
    /// The wire form `get-chunk` attaches as one `occurrences`
    /// array element. `root_id` is present only when the lookup
    /// attributed the occurrence to a registered root, so the
    /// no-registry wire shape is unchanged.
    pub fn to_json(&self) -> Json {
        let mut v = serde_json::json!({
            "shard": self.shard,
            "tier": self.tier.name(),
            "canonical_hash": self.canonical_hash,
            "source": self.source,
            "span_start": self.span_start,
            "span_end": self.span_end,
            "observed_at": self.observed_at,
        });
        if let Some(root_id) = &self.root_id {
            v["root_id"] = Json::String(root_id.clone());
        }
        v
    }
}

/// Every recorded occurrence of the content whose canonical hash
/// is `canonical_hash`, across the shards `tier_ceiling` admits, in a
/// deterministic order (shard, source, span_start). Shards with
/// no `chunk_occurrences` table (promoted before the registry)
/// are skipped; a shard that answers with a genuine failure fails
/// the lookup — a partial provenance answer is as wrong as a
/// partial search answer.
///
/// `caller_ceiling` is the full Sensitivity ladder ceiling (the
/// intersection of the caller's grant and the disclosure context).
/// Each occurrence is joined to its referenced chunk and the chunk's
/// per-row sensitivity is checked against this ceiling — an admitted
/// physical shard can contain chunks classified above the caller's
/// ceiling (promotion names the shard from unit.sensitivity but
/// writes each chunk's effective sensitivity separately).
///
/// `registry` is the phase-4-catalog-side source registry (#3125
/// slice 6: `source_registry`'s tables in whichever database the
/// caller opens). When attached, each occurrence's `source` locator
/// is resolved to its registered root and carried as `root_id`;
/// unregistered locators stay unattributed (`None`), never guessed.
/// `None` keeps the pre-registry behavior exactly: unattributed rows
/// and no wire change. A registry that answers with a genuine
/// failure fails the lookup — same fail-closed philosophy as a
/// failing shard: a partial attribution presented as complete would
/// be as wrong as a partial search answer.
pub fn lookup_by_hash(
    set: &ShardSet,
    tier_ceiling: TierCeiling,
    caller_ceiling: Sensitivity,
    canonical_hash: &str,
    registry: Option<&Connection>,
) -> Result<Vec<ChunkOccurrence>> {
    let mut out = Vec::new();
    for shard in set.select(tier_ceiling, None)? {
        let Some(transport) = set.transport(&shard.name) else {
            return Err(Error::backend(format!(
                "occurrences: {} has no open transport",
                shard.name
            )));
        };
        // Join occurrences to chunks to enforce per-row sensitivity.
        // The occurrence row has shard_chunk_id -> chunks.id.
        let stmt = Stmt::new(
            "SELECT o.source, o.span_start, o.span_end, o.observed_at, c.sensitivity \
             FROM chunk_occurrences o \
             JOIN chunks c ON c.id = o.shard_chunk_id \
             WHERE o.canonical_hash = ?1 \
             ORDER BY o.source, o.span_start",
            vec![Value::text(canonical_hash)],
        );
        let rows = match transport.pipeline(&[stmt]) {
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
            let (Some(source), Some(observed_at), Some(chunk_sens)) = (
                r.get_text("source"),
                r.get_text("observed_at"),
                r.get_text("sensitivity"),
            ) else {
                return Err(Error::backend(format!(
                    "occurrences: {}: malformed occurrence row",
                    shard.name
                )));
            };
            // Fail-closed: unrecognized sensitivity -> SecretBearing (excluded by caller_ceiling)
            let chunk_sensitivity =
                Sensitivity::from_name(chunk_sens).unwrap_or(Sensitivity::SecretBearing);
            if chunk_sensitivity > caller_ceiling {
                // This occurrence references a chunk above the caller's ceiling — withhold.
                continue;
            }
            out.push(ChunkOccurrence {
                shard: shard.name.clone(),
                tier: shard.tier,
                canonical_hash: canonical_hash.to_string(),
                source: source.to_string(),
                span_start: r.get_i64("span_start").unwrap_or_default(),
                span_end: r.get_i64("span_end").unwrap_or_default(),
                observed_at: observed_at.to_string(),
                root_id: match registry {
                    Some(conn) => resolve_source(conn, &source)?.map(|r| r.root_id),
                    None => None,
                },
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
