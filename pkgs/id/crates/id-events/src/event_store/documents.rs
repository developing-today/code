//! Documents over the event store (Phase 5, P5.4 and P5.6).
//!
//! A document is its `document.put` event. The event applies like any content event, and the store
//! keeps it either way. Its content is a blob that the store does not hold: the header names it by
//! hash, and the bytes come from iroh-blobs on request. Tags on a document are tag events in the
//! same store, read by [`EventStore::tag_claims`], so they need nothing here.
//!
//! A removal (P5.6) is a `document.remove` event in the same store. It is kept like any other
//! event, and it does not change the document or its stored tags. [`EventStore::documents`]
//! marks the document with it when the removal is effective: signed by the document's own
//! author, or by a device key the author has authorized in the document's group (P8.3) whose
//! effective role there is Write or above — an agent acting for the owner (decision on #195,
//! 2026-11-16). A removal by anyone else is stored and has no effect.
//!
//! An effective removal stops new content for its document: the tag fold
//! ([`EventStore::tag_claims`]) drops a `tag.set` on the removed document when the removal is
//! in the tag's causal past — the tag's author had seen the removal when signing it. Tags
//! concurrent with the removal (their causal past does not contain it) are kept, and
//! `tag.clear` markers still fold: a clear adds no content. Content is retained (design
//! decision 4): the document, its blob, and its earlier tags stay readable, and the blob gate
//! is unchanged for already-stored content. A document names no other document, so tags are
//! the only new content a removal can stop.

use std::collections::{BTreeMap, HashSet};

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::resolve::{self, author_of};
use super::{
    ACTIONS, APPLIED, DEVICE_OPS, EVENTS, EventStore, FORKS, PROOF_HEADERS, StoreError, corrupt,
};
use crate::groups::{Projection, Role as GroupRole};
use id_core::waiting::AuthorKey;
use id_store::documents::{DOCUMENT_KIND, DocumentRecord, REMOVAL_KIND};

/// Whether a removal signed by `signer` is effective against a document whose author is
/// `author` and whose `document.put` names `group` (`None` when it names none): the author
/// themself, or an agent acting for the author — a device key the author has authorized in
/// that group (P8.3) whose effective role there is Write or above. The settled projection is
/// intentional, as in the tag fold's device authority (#5656): the authorization is read
/// from the store's current group state, not replayed at the removal's causal position.
fn removal_is_effective(
    signer: &AuthorKey,
    author: &AuthorKey,
    group: Option<Hash>,
    projection: Option<&Projection>,
) -> bool {
    if signer == author {
        return true;
    }
    let (Some(group), Some(projection)) = (group, projection) else {
        return false;
    };
    projection.active_devices(&group, author).contains(signer)
        && matches!(projection.device_role(&group, signer), Some(role) if role >= GroupRole::Write)
}

/// The effective removals among `stored`: for each removed document's event ID, the IDs of
/// the removals that mark it (see [`removal_is_effective`]), sorted by event ID. Shared by
/// [`EventStore::documents`] (which reports the lowest) and the tag fold (which stops any
/// tag that had seen one of them).
///
/// # Errors
///
/// Returns [`StoreError::Corrupt`] for a stored removal that names no document.
pub(super) fn removal_marks(
    stored: &[resolve::Stored],
    applied: &HashSet<Hash>,
    projection: Option<&Projection>,
) -> Result<BTreeMap<Hash, Vec<Hash>>, StoreError> {
    // The author and named group of each applied document.
    let mut documents: BTreeMap<Hash, (AuthorKey, Option<Hash>)> = BTreeMap::new();
    for item in stored {
        let event = &item.event;
        if !applied.contains(&event.id) {
            continue;
        }
        if event.header.extensions.kind == DOCUMENT_KIND {
            documents.insert(event.id, (author_of(event), event.header.extensions.group));
        }
    }
    let mut marks: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
    for item in stored {
        let event = &item.event;
        if !applied.contains(&event.id) {
            continue;
        }
        if event.header.extensions.kind != REMOVAL_KIND {
            continue;
        }
        let Some(target) = event.header.extensions.removes else {
            return Err(corrupt(
                "a stored removal event names no document".to_owned(),
            ));
        };
        let Some((author, group)) = documents.get(&target) else {
            continue;
        };
        if removal_is_effective(&author_of(event), author, *group, projection) {
            marks.entry(target).or_default().push(event.id);
        }
    }
    for ids in marks.values_mut() {
        ids.sort_by_key(|id| *id.as_bytes());
        ids.dedup();
    }
    Ok(marks)
}

impl EventStore {
    /// The documents in the store: each applied `document.put`, sorted by event ID. A document
    /// carries the ID of its lowest effective removal, if one has been signed for it (P5.6):
    /// a removal by the document's author, or by a device key the author authorized in the
    /// document's group at Write scope or above. A removal by anyone else has no effect on
    /// the record.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored document
    /// event that carries no document, or a stored removal that names no document.
    pub fn documents(&self) -> Result<Vec<DocumentRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let device_ops = txn.open_table(DEVICE_OPS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load_with_decoder(
                &events,
                &actions,
                &device_ops,
                &forks,
                &proof_headers,
                |bytes| self.decode_stored_event(bytes),
            )?
        };
        let residue = super::compaction::load_residue_read(&txn)?;
        let resolved = resolve::resolve(&stored, &forks, &headers, &residue)?;
        let marks = removal_marks(&stored, &resolved.applied, resolved.projection.as_ref())?;

        let mut records = Vec::new();
        for item in &stored {
            let event = &item.event;
            if !resolved.applied.contains(&event.id) {
                continue;
            }
            if event.header.extensions.kind != DOCUMENT_KIND {
                continue;
            }
            let Some(document) = event.header.extensions.document.clone() else {
                return Err(corrupt(
                    "a stored document event carries no document".to_owned(),
                ));
            };
            records.push(DocumentRecord {
                id: event.id,
                author: author_of(event),
                document,
                removal: marks.get(&event.id).and_then(|ids| ids.first()).copied(),
            });
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
            let event = self.decode_stored_event(bytes.value())?;
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
