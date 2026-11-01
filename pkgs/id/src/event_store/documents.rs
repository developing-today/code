//! Documents over the event store (Phase 5, P5.4 and P5.6).
//!
//! A document is its `document.put` event. The event applies like any content event, and the store
//! keeps it either way. Its content is a blob that the store does not hold: the header names it by
//! hash, and the bytes come from iroh-blobs on request. Tags on a document are tag events in the
//! same store, read by [`EventStore::tag_claims`], so they need nothing here.
//!
//! A removal (P5.6) is a `document.remove` event in the same store. It is kept like any other event,
//! and it does not change the document or its tags. [`EventStore::documents`] marks the document
//! with it, when the document's own author signed it.

use std::collections::BTreeMap;

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::resolve::author_of;
use super::{APPLIED, EVENTS, EventStore, StoreError, corrupt, decode_stored};
use crate::documents::{DOCUMENT_KIND, DocumentRecord, REMOVAL_KIND};
use crate::waiting::AuthorKey;

impl EventStore {
    /// The documents in the store: each applied `document.put`, sorted by event ID. A document
    /// carries the ID of its author's removal, if the author has signed one (P5.6). A removal by
    /// anyone else has no effect on the record.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored document
    /// event that carries no document, or a stored removal that names no document.
    pub fn documents(&self) -> Result<Vec<DocumentRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;

        let mut records = Vec::new();
        // The lowest applied removal for each (document, author) pair. Its key is the document's
        // event ID and the author who signed the removal.
        let mut removals: BTreeMap<([u8; 32], AuthorKey), Hash> = BTreeMap::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            if applied_table.get(event.id.as_bytes().as_slice())?.is_none() {
                continue;
            }
            let kind = event.header.extensions.kind.as_str();
            if kind == REMOVAL_KIND {
                let Some(target) = event.header.extensions.removes else {
                    return Err(corrupt(
                        "a stored removal event names no document".to_owned(),
                    ));
                };
                let key = (*target.as_bytes(), author_of(&event));
                removals
                    .entry(key)
                    .and_modify(|lowest| {
                        if event.id.as_bytes() < lowest.as_bytes() {
                            *lowest = event.id;
                        }
                    })
                    .or_insert(event.id);
                continue;
            }
            if kind != DOCUMENT_KIND {
                continue;
            }
            let Some(document) = event.header.extensions.document.clone() else {
                return Err(corrupt(
                    "a stored document event carries no document".to_owned(),
                ));
            };
            records.push(DocumentRecord {
                id: event.id,
                author: author_of(&event),
                document,
                removal: None,
            });
        }
        for record in &mut records {
            record.removal = removals
                .get(&(*record.id.as_bytes(), record.author))
                .copied();
        }
        records.sort_by_key(|record| *record.id.as_bytes());
        Ok(records)
    }

    /// The blob references the gate index is built from: for every applied `document.put`
    /// its author has not removed, the document's blob hash and the group the event names
    /// (`None` when the event names none — such a document references its blob for no
    /// world). Sorted by blob hash for a stable index build.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored
    /// document event that carries no document, or a stored removal that names no document.
    pub fn document_references(&self) -> Result<Vec<(iroh_blobs::Hash, Option<Hash>)>, StoreError> {
        let records = self.documents()?;
        let removed: std::collections::BTreeSet<[u8; 32]> = records
            .iter()
            .filter(|record| record.removal.is_some())
            .map(|record| *record.id.as_bytes())
            .collect();
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;
        let mut references = Vec::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            if removed.contains(event.id.as_bytes()) {
                continue;
            }
            if applied_table.get(event.id.as_bytes().as_slice())?.is_none() {
                continue;
            }
            if event.header.extensions.kind != DOCUMENT_KIND {
                continue;
            }
            let Some(document) = &event.header.extensions.document else {
                return Err(corrupt(
                    "a stored document event carries no document".to_owned(),
                ));
            };
            references.push((document.blob_hash(), event.header.extensions.group));
        }
        references.sort_by_key(|(hash, _)| *hash.as_bytes());
        Ok(references)
    }
}
