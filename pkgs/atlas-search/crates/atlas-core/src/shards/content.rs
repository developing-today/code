//! Tier content stores (`atlas-core-<tier>-content`, design #5667
//! sections 3.3 / 4.2, issue #6664): the messages + sessions half of
//! the corpus, one database per tier, from which shard-served search
//! hydrates its top-N hits — by chunk `message_id` — instead of the
//! wrapped monolith backend.
//!
//! The ceiling mechanism is the scan shards' own, reused unchanged:
//!
//! * **Discovery** ([`discover_content_stores`]) is the service's
//!   maximum: a content store above the ceiling, or of a tier the
//!   connector holds no credential for, is *never connected* — it is
//!   recorded as a [`Rejection`](super::Rejection) without any
//!   connection attempt, exactly like an above-ceiling scan shard.
//! * A store that connects but cannot answer the hydration probe is
//!   likewise rejected with the reason recorded: degradation is
//!   explicit, never silent.
//! * Hydration failures after discovery are named errors (the store
//!   is named), never a silent fall-back to another tier's store.
//!
//! The successor switch lives on
//! [`ShardedBackend`](super::scan::ShardedBackend)
//! (`with_content_stores(.., enabled)`): the service's successor
//! protocol field (coordination issue #5668) maps onto it. The
//! shipped `--expand` neighbor-chunk semantics are untouched (Drew
//! ruling 2026-11-06): `neighbors` here is the same by-message
//! window query the monolith serves, answered from the content
//! store instead when the switch is on.

use std::sync::Arc;

use super::{
    Rejection, SHARD_PREFIX, ShardConnector, ShardEndpoint, ShardLister, ShardTransport, Tier,
    TierCeiling,
};
use crate::error::{Error, Result};
use crate::filters::{MESSAGE_SQL, NEIGHBORS_SQL, TURSO_SHAPE, details_sql};
use crate::types::{ChunkDetail, MessageRow, NeighborRow, Stmt, Value, detail_from_row};

/// The content store database name for a tier:
/// `atlas-core-<tier>-content`.
pub fn content_store_name(tier: Tier) -> String {
    format!("{SHARD_PREFIX}{}-content", tier.name())
}

/// `atlas-core-<tier>-content` -> tier. Scan shards
/// (`atlas-core-<tier>-<topic>-<nn>`) and anything else do not
/// parse: they are not content stores.
pub fn parse_content_store_name(name: &str) -> Option<Tier> {
    let rest = name.strip_prefix(SHARD_PREFIX)?;
    Tier::parse(rest.strip_suffix("-content")?)
}

/// One connected tier content store.
#[derive(Clone)]
pub struct ContentStore {
    pub name: String,
    pub tier: Tier,
    pub endpoint: ShardEndpoint,
    transport: Arc<dyn ShardTransport>,
}

impl ContentStore {
    fn run(&self, stmt: &Stmt) -> Result<Vec<crate::types::Row>> {
        self.transport
            .pipeline(std::slice::from_ref(stmt))
            .map_err(|e| Error::backend(format!("content store {}: {e}", self.name)))?
            .into_iter()
            .next()
            .ok_or_else(|| Error::backend(format!("content store {}: no result set", self.name)))
    }
}

/// The connected tier content stores, in ascending tier order.
pub struct ContentStores {
    stores: Vec<ContentStore>,
    /// The highest tier this set was discovered under (the
    /// service's maximum). Stores above it were never connected.
    discovery_ceiling: TierCeiling,
    /// Candidates that did not join the set, with the reason
    /// (above the ceiling, no credential, unreadable) — the
    /// explicit-degradation record.
    pub rejections: Vec<Rejection>,
}

impl ContentStores {
    pub fn stores(&self) -> &[ContentStore] {
        &self.stores
    }

    pub fn discovery_ceiling(&self) -> TierCeiling {
        self.discovery_ceiling
    }

    pub fn store_for(&self, tier: Tier) -> Option<&ContentStore> {
        self.stores.iter().find(|s| s.tier == tier)
    }

    /// Detail rows for explicit chunk ids, from the admitted tier
    /// content stores (messages + sessions joined by chunk
    /// `message_id`, the monolith's own `details_sql`). Chunk ids
    /// are globally unique across tiers (restored from one monolith
    /// with ids preserved), so the per-store results merge without
    /// ambiguity; ids no admitted store holds are simply missing,
    /// as the `SearchBackend` contract specifies.
    pub fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = details_sql(TURSO_SHAPE, ids.len());
        let args: Vec<Value> = ids.iter().map(|i| Value::Integer(*i)).collect();
        let mut out = Vec::new();
        for store in &self.stores {
            let rows = store.run(&Stmt::new(sql.clone(), args.clone()))?;
            out.extend(rows.iter().filter_map(detail_from_row));
        }
        Ok(out)
    }

    /// One message row by id, from the first admitted tier store
    /// that holds it (message ids are globally unique).
    pub fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        for store in &self.stores {
            let rows = store.run(&Stmt::new(MESSAGE_SQL, vec![Value::text(message_id)]))?;
            if let Some(r) = rows.first() {
                return Ok(Some(MessageRow {
                    id: r
                        .get_text("id")
                        .ok_or_else(|| {
                            Error::backend(format!(
                                "content store {}: message row has no id",
                                store.name
                            ))
                        })?
                        .to_string(),
                    role: r.get_text("role").map(str::to_string),
                    text: r.get_text("text").unwrap_or("").to_string(),
                }));
            }
        }
        Ok(None)
    }

    /// Same-message neighbor rows around (`message_id`, any
    /// `chunk_idx` in the window), ordered by chunk_idx — the
    /// shipped `--expand` query, answered from the content stores.
    pub fn neighbors(
        &self,
        message_id: &str,
        chunk_idx: i64,
        n: i64,
    ) -> Result<Vec<NeighborRow>> {
        for store in &self.stores {
            let rows = store.run(&Stmt::new(
                NEIGHBORS_SQL,
                vec![
                    Value::text(message_id),
                    Value::Integer(chunk_idx - n),
                    Value::Integer(chunk_idx + n),
                ],
            ))?;
            if !rows.is_empty() {
                return Ok(rows
                    .iter()
                    .filter_map(|r| {
                        Some(NeighborRow {
                            chunk_id: r.get_i64("id")?,
                            chunk_idx: r.get_i64("chunk_idx")?,
                            text: r.get_text("text")?.to_string(),
                        })
                    })
                    .collect());
            }
        }
        Ok(Vec::new())
    }
}

/// Resolve the tier content stores: list candidates, drop
/// everything that is not a content store, and *without
/// connecting* reject stores above the ceiling or without a
/// credential; connect the rest and probe each with a `messages`
/// count, recording unreadable stores as rejections (hydration is
/// an enhancement path: a bad store degrades explicitly at
/// discovery instead of failing searches later — but its absence
/// is always on record, in `rejections` and the backend notes).
pub fn discover_content_stores(
    lister: &dyn ShardLister,
    connector: &dyn ShardConnector,
    ceiling: TierCeiling,
) -> Result<ContentStores> {
    if !connector.can_open(ceiling.0) {
        return Err(Error::fatal(format!(
            "content-store discovery: ceiling `{}` needs a {}-tier credential ({}); none is available",
            ceiling.0,
            ceiling.0,
            ceiling.0.token_env()
        )));
    }
    let mut rejections = Vec::new();
    let mut stores = Vec::new();
    for ep in lister.list()? {
        let Some(tier) = parse_content_store_name(&ep.name) else {
            continue;
        };
        if !ceiling.admits(tier) {
            rejections.push(Rejection {
                name: ep.name,
                reason: format!("tier {tier} is above the ceiling {}", ceiling.0),
            });
            continue;
        }
        if !connector.can_open(tier) {
            rejections.push(Rejection {
                name: ep.name,
                reason: format!("no {tier}-tier credential ({})", tier.token_env()),
            });
            continue;
        }
        match connector.connect(&ep, tier) {
            Ok(t) => {
                let probe = t.pipeline(&[Stmt::new(
                    "SELECT COUNT(*) AS n FROM messages",
                    vec![],
                )]);
                match probe {
                    Ok(_) => stores.push(ContentStore {
                        name: ep.name.clone(),
                        tier,
                        endpoint: ep,
                        transport: t,
                    }),
                    Err(e) => rejections.push(Rejection {
                        name: ep.name,
                        reason: format!("content store unreadable: {e}"),
                    }),
                }
            }
            Err(e) => rejections.push(Rejection {
                name: ep.name,
                reason: format!("content store connection failed: {e}"),
            }),
        }
    }
    stores.sort_by_key(|s| s.tier.level());
    Ok(ContentStores {
        stores,
        discovery_ceiling: ceiling,
        rejections,
    })
}
