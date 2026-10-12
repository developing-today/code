//! Tag claims over the store (Phase 5, P5.1 and P5.2).
//!
//! A tag event is applied like any content event: the resolution decides whether it applies, and
//! the store keeps it either way. Its operation is in its header (#182), so the stored event
//! carries the claim or the clear. [`EventStore::tag_claims`] reads the applied tag events and
//! folds them with the parent links of every event the store holds. A clear's causal past is read
//! from those links, so it covers the events the clear's author had seen.

use std::collections::HashMap;

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::resolve::author_of;
use super::{APPLIED, EVENTS, EventStore, PROOF_HEADERS, StoreError, corrupt, decode_stored};
use crate::chain::Event;
use crate::tag_claims::{Applied, Claim, compacted_claims, is_tag_kind};
use crate::tag_index::TagIndex;

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
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored tag event
    /// that carries no tag operation for its kind.
    pub fn tag_claims(&self) -> Result<Vec<Claim>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;

        let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
        let mut applied = Vec::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            parents.insert(event.id, parents_of(&event));
            if !is_tag_kind(&event.header.extensions.kind)
                || applied_table.get(event.id.as_bytes().as_slice())?.is_none()
            {
                continue;
            }
            let Some(op) = event.header.extensions.tag.clone() else {
                return Err(corrupt(
                    "a stored tag event carries no tag operation".to_owned(),
                ));
            };
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
        Ok(compacted_claims(&applied, &parents))
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
