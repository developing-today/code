//! Committed-catalog visibility gate: the read side of design #6057
//! §6.3 ("Prepare/commit visibility protocol"), implemented for issue
//! #12406.
//!
//! Physical shard enumeration ([`crate::shards::discover`]) is candidate
//! discovery only: a shard that exists is not automatically searchable.
//! A [`CommittedCatalog`] is the tier's active generation commit point:
//! discovery pinned to a catalog serves exactly the committed manifest
//! list, and refuses anything else. Fail-closed rules (both enforced
//! before any vector scan runs):
//!
//! * a physical shard absent from the catalog is never probed and never
//!   served (incomplete prepare stays invisible);
//! * a committed shard missing from the listing is reported, never
//!   silently skipped (a search must not quietly cover fewer shards
//!   than the committed generation names);
//! * a manifest disagreeing with its committed entry (`expected_chunks`
//!   or `embedding_model`) is refused (reconciliation in §6.4 must
//!   quarantine the mismatch first).
//!
//! Guarantees (design §8, items 9 and 10): no corpus text is read (the
//! only manifest keys compared are counts and model identity); nothing
//! here writes to any shard; the catalog snapshot id hashes the
//! committed generation, never live mutable row counts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Tier;

/// Format marker for a serialized committed catalog.
pub const CATALOG_FORMAT: &str = "atlas-committed-catalog/v1";

/// One shard publication a catalog generation commits to. Identity is
/// the shard name; `expected_chunks` and `embedding_model` pin the
/// manifest the fan-out must observe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedShard {
    pub name: String,
    pub tier: Tier,
    pub topic: String,
    pub part: u32,
    pub expected_chunks: Option<u64>,
    pub embedding_model: Option<String>,
}

/// A tier's active generation: the single visibility point. Discovery
/// pinned to this generation serves exactly these shards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedCatalog {
    pub format: String,
    pub generation: u64,
    pub shards: Vec<CommittedShard>,
}

impl CommittedCatalog {
    /// An empty generation (nothing committed yet: nothing is
    /// searchable through the gate).
    pub fn empty(generation: u64) -> CommittedCatalog {
        CommittedCatalog {
            format: CATALOG_FORMAT.to_string(),
            generation,
            shards: Vec::new(),
        }
    }

    /// The committed entry for `name`, if this generation publishes it.
    pub fn lookup(&self, name: &str) -> Option<&CommittedShard> {
        self.shards.iter().find(|s| s.name == name)
    }

    /// Canonical text of the committed manifest set: sorted
    /// `name tier expected_chunks embedding_model` lines. Sorted so
    /// entry order never moves the snapshot id.
    pub fn snapshot_material(&self) -> String {
        let mut by_name: BTreeMap<&str, &CommittedShard> = BTreeMap::new();
        for s in &self.shards {
            by_name.insert(s.name.as_str(), s);
        }
        by_name
            .values()
            .map(|s| {
                format!(
                    "{}\t{}\t{}\t{}",
                    s.name,
                    s.tier,
                    s.expected_chunks
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    s.embedding_model.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Hex sha256 over the format marker, the generation, and the
    /// committed manifest material. A live fan-out pins this id at
    /// query start (design §6.3 "Read"); result-cache keying must
    /// consume it rather than independently observing in-progress
    /// writes.
    pub fn snapshot_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(CATALOG_FORMAT.as_bytes());
        h.update([0]);
        h.update(self.generation.to_le_bytes());
        h.update([0]);
        h.update(self.snapshot_material().as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}
