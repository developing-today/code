//! Documents over the event store (Phase 5, P5.4).
//!
//! A document is its `document.put` event. The event applies like any content event, and the store
//! keeps it either way. Its content is a blob that the store does not hold: the header names it by
//! hash, and the bytes come from iroh-blobs on request. Tags on a document are tag events in the
//! same store, read by [`EventStore::tag_claims`], so they need nothing here.

use redb::{ReadableDatabase, ReadableTable};

use super::resolve::author_of;
use super::{APPLIED, EVENTS, EventStore, StoreError, corrupt, decode_stored};
use crate::documents::{DOCUMENT_KIND, DocumentRecord};

impl EventStore {
    /// The documents in the store: each applied `document.put`, sorted by event ID.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored document
    /// event that carries no document.
    pub fn documents(&self) -> Result<Vec<DocumentRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;

        let mut records = Vec::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            if event.header.extensions.kind != DOCUMENT_KIND
                || applied_table.get(event.id.as_bytes().as_slice())?.is_none()
            {
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
            });
        }
        records.sort_by_key(|record| *record.id.as_bytes());
        Ok(records)
    }
}
