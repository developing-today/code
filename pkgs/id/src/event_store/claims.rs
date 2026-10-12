//! Tag claims over the store (Phase 5, P5.1 and P5.2).
//!
//! A tag event is applied like any content event: the resolution decides whether it applies, and
//! the store keeps it either way. Its body is kept in the `tag_ops` table, written when the event
//! arrives with the body its header commits to, so the claim and the clear can be read back.
//! [`EventStore::tag_claims`] reads the applied tag events with their bodies, and folds them with
//! the parent links of every event the store holds. A clear's causal past is read from those
//! links, so it covers the events the clear's author had seen.

use std::collections::HashMap;

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable, WriteTransaction};

use super::resolve::author_of;
use super::{
    APPLIED, EVENTS, EventStore, PROOF_HEADERS, StoreError, TAG_OPS, corrupt, decode_stored,
};
use crate::chain::Event;
use crate::tag_claims::{Applied, Claim, decode_op, is_tag_kind, live_claims};
use crate::tag_index::TagIndex;

/// Writes the body of a tag event, so the fold can read it. The body is the one the header commits
/// to, and the caller has checked that.
pub(super) fn store_tag_body(
    txn: &WriteTransaction,
    id: &Hash,
    body: &[u8],
) -> Result<(), StoreError> {
    let mut ops = txn.open_table(TAG_OPS)?;
    ops.insert(id.as_bytes().as_slice(), body)?;
    Ok(())
}

/// The events an event names: its `after` hashes, and its backlink.
fn parents_of(event: &Event) -> Vec<Hash> {
    event
        .header
        .extensions
        .after
        .iter()
        .copied()
        .chain(event.header.backlink)
        .collect()
}

impl EventStore {
    /// The live claims in the store: each applied `tag.set` that no applied `tag.clear` removes.
    ///
    /// A tag event whose body the store does not hold yet is not counted, so a claim or a clear
    /// shows only once its body has arrived.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored tag body
    /// that does not decode.
    pub fn tag_claims(&self) -> Result<Vec<Claim>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;
        let ops = txn.open_table(TAG_OPS)?;

        let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
        let mut applied = Vec::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            parents.insert(event.id, parents_of(&event));
            let kind = &event.header.extensions.kind;
            if !is_tag_kind(kind) || applied_table.get(event.id.as_bytes().as_slice())?.is_none() {
                continue;
            }
            let Some(body) = ops.get(event.id.as_bytes().as_slice())? else {
                continue;
            };
            let op = decode_op(kind, body.value())
                .map_err(|err| corrupt(format!("a stored tag body does not decode: {err}")))?;
            applied.push(Applied {
                id: event.id,
                author: author_of(&event),
                op,
            });
        }
        // A fork proof header is present, so a clear may name it, and it has parents. It never
        // applies, so it only extends the parent links.
        let headers = txn.open_table(PROOF_HEADERS)?;
        for row in headers.iter()? {
            let (_hash, bytes) = row?;
            let header = decode_stored(bytes.value())?;
            parents
                .entry(header.id)
                .or_insert_with(|| parents_of(&header));
        }
        Ok(live_claims(&applied, &parents))
    }

    /// The tag index over the live claims, built from [`EventStore::tag_claims`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`EventStore::tag_claims`].
    pub fn tag_index(&self) -> Result<TagIndex, StoreError> {
        Ok(TagIndex::new(&self.tag_claims()?))
    }
}
