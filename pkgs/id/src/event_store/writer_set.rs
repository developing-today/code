//! Writer-set objects in the store (writer-set spec §§4–5; issue #6690,
//! follow-up to #5545).
//!
//! The pure projection lives in [`crate::idset::view`]: it runs over a pinned
//! snapshot and never consults live store state. This module is the wiring
//! that lets it run against what a store actually holds:
//!
//! - **Body retention.** Event headers commit to a body by hash but the store
//!   otherwise keeps no bodies. When a writer-set or rescind event is ingested
//!   with its body ([`EventStore::ingest_with_body`]) and the body is the one
//!   the header commits to, the body is retained here, keyed by event ID, so
//!   the object can be read back and checked against its header.
//! - **Reads.** [`EventStore::stored_writer_set`] and
//!   [`EventStore::stored_rescind`] decode a retained body and cross-check it
//!   against the stored header: kind, author/owner, sequence/version, and the
//!   payload hash commitment. A body that disagrees with its header is
//!   corruption, never a usable object.
//! - **Snapshots.** [`EventStore::group_members_snapshot`] builds the
//!   one-level [`GroupMembers`] a projection needs from the store's resolved
//!   group state (the same load + resolve path as [`EventStore::roster`]),
//!   pinned under the caller's `(frontier, resolution)` tuple.
//! - **Projection + discovery.** [`EventStore::expand_stored_writer_set`]
//!   expands one stored set at its own publishing frontier, and
//!   [`EventStore::discover_writer_sets`] is the subject-first discovery
//!   query over every stored set: filter by scope (the set's subject), each
//!   hit carrying the set and its expanded writers — both sides of the set,
//!   not the set alone.
//!
//! The snapshot a store can offer today is its *resolved* state: the fold of
//! every applied group event it holds. Frontier-historical resolution —
//! membership as of an older causal position — stays with the membership
//! layer's pinned expander ([`crate::idset::expand`]); the tuple recorded on
//! the snapshot names the caller's pin so a result is never presented as
//! resolved under a different snapshot than it was.

use std::collections::BTreeMap;

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use crate::groups::Role;
use crate::idset::view::{ExpandedWriters, GroupMembers, ProjectionError, expand_set};
use crate::idset::{IdsetError, Rescind, WriterSet, decode_rescind_body, decode_writer_body};
use crate::waiting::AuthorKey;

use super::{EventStore, StoreError, WRITER_SET_BODIES, resolve};

/// Why a stored writer-set object cannot be read or expanded.
#[derive(Debug, thiserror::Error)]
pub enum WriterSetStoreError {
    /// The underlying store read or write failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The retained body is not a valid object of its kind.
    #[error(transparent)]
    Body(#[from] IdsetError),
    /// No applied writer-set event has this ID.
    #[error("no stored writer-set event {0}")]
    NoSuchSet(Hash),
    /// The stored event or its retained body disagrees with the other: the
    /// event is not of the expected kind, its author is not the object's
    /// owner, its sequence is not the object's version, or its payload hash
    /// does not commit to the body.
    #[error("stored writer-set event {0} does not match its body")]
    Mismatch(Hash),
    /// The projection could not be evaluated over the store's snapshot.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

/// One discovery hit: a stored writer set and the writers it expands to
/// under the query's resolution snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredWriterSet {
    /// The event that published this version of the set.
    pub event_id: Hash,
    /// The set object, as decoded from its retained body.
    pub set: WriterSet,
    /// The set expanded at its publishing frontier under the query's
    /// resolution: who the set admits, after exclusions.
    pub expanded: ExpandedWriters,
}

impl EventStore {
    /// Retains the body of a writer-set or rescind event, keyed by the
    /// event's ID. Called from the ingest path once the body has been
    /// verified against the header's payload commitment; a body retained for
    /// an event that is later superseded by a fork proof is harmless, as
    /// every read cross-checks the body against the stored header and the
    /// applied state.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the write fails.
    pub(super) fn record_writer_body(&self, id: &Hash, body: &[u8]) -> Result<(), StoreError> {
        let txn = self.db.begin_write()?;
        {
            let mut bodies = txn.open_table(WRITER_SET_BODIES)?;
            bodies.insert(id.as_bytes().as_slice(), body)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// The retained body of the event with this ID, when one was ingested
    /// with the event. Only writer-set and rescind events retain bodies.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails.
    pub fn writer_set_body(&self, id: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read()?;
        let bodies = txn.open_table(WRITER_SET_BODIES)?;
        let row = bodies.get(id.as_bytes().as_slice())?;
        Ok(row.map(|body| body.value().to_vec()))
    }

    /// The stored writer set published by the event with this ID.
    ///
    /// Returns `None` when the event is not an applied writer-set event or
    /// no body was retained for it. A retained body that disagrees with its
    /// header — kind, owner, version, or the payload hash commitment — is an
    /// error, not an absent set.
    ///
    /// # Errors
    ///
    /// Returns [`WriterSetStoreError::Mismatch`] when the body and the header
    /// disagree, [`WriterSetStoreError::Body`] when the body does not decode,
    /// or a store error when a read fails.
    pub fn stored_writer_set(&self, id: &Hash) -> Result<Option<WriterSet>, WriterSetStoreError> {
        let Some((event, body)) = self.writer_object_parts(id, crate::idset::WRITER_KIND)? else {
            return Ok(None);
        };
        let set = decode_writer_body(&body)?;
        let author = resolve::author_of(&event);
        if set.owner != author || set.version != u64::from(event.header.seq_num) {
            return Err(WriterSetStoreError::Mismatch(*id));
        }
        Ok(Some(set))
    }

    /// The stored rescind published by the event with this ID, under the same
    /// rules as [`EventStore::stored_writer_set`]. A rescind carries no owner
    /// or version of its own — its set is named in the body — so the check is
    /// kind plus the payload hash commitment.
    ///
    /// # Errors
    ///
    /// The same errors as [`EventStore::stored_writer_set`].
    pub fn stored_rescind(&self, id: &Hash) -> Result<Option<Rescind>, WriterSetStoreError> {
        let Some((_event, body)) = self.writer_object_parts(id, crate::idset::RESCIND_KIND)? else {
            return Ok(None);
        };
        Ok(Some(decode_rescind_body(&body)?))
    }

    /// The event and retained body behind a writer-object read: the event
    /// must be stored, applied, and of `kind`, the body must be retained, and
    /// the header must commit to the body by payload hash.
    fn writer_object_parts(
        &self,
        id: &Hash,
        kind: &str,
    ) -> Result<Option<(crate::chain::Event, Vec<u8>)>, WriterSetStoreError> {
        let Some(event) = self.event_by_id(id)? else {
            return Ok(None);
        };
        if event.header.extensions.kind != kind || !self.event_applied(id)? {
            return Ok(None);
        }
        let Some(body) = self.writer_set_body(id)? else {
            return Ok(None);
        };
        if event.header.payload_hash != Some(Hash::digest(&body)) {
            return Err(WriterSetStoreError::Mismatch(*id));
        }
        Ok(Some((event, body)))
    }

    /// The one-level membership snapshot of `group` from the store's
    /// resolved state, pinned under `(frontier, resolution)`.
    ///
    /// Members and roles come from the group fold over every applied group
    /// event the store holds — the same resolution [`EventStore::roster`]
    /// reports, with roles kept. A group the store does not know resolves to
    /// an empty snapshot: the projection then admits nobody through that
    /// group entry, rather than failing the whole expansion.
    ///
    /// # Errors
    ///
    /// Returns a store error when the resolution cannot be loaded.
    pub fn group_members_snapshot(
        &self,
        group: &Hash,
        frontier: Vec<Hash>,
        resolution: &Hash,
    ) -> Result<GroupMembers, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(super::EVENTS)?;
            let actions = txn.open_table(super::ACTIONS)?;
            let device_ops = txn.open_table(super::DEVICE_OPS)?;
            let forks = txn.open_table(super::FORKS)?;
            let proof_headers = txn.open_table(super::PROOF_HEADERS)?;
            resolve::load(&events, &actions, &device_ops, &forks, &proof_headers)?
        };
        let resolved = resolve::resolve(&stored, &forks, &headers)?;
        let members: BTreeMap<AuthorKey, Role> = resolved
            .projection
            .as_ref()
            .and_then(|projection| projection.group(group))
            .map(|view| view.members.clone())
            .unwrap_or_default();
        Ok(GroupMembers::new(*group, frontier, *resolution, members))
    }

    /// Expands the stored writer set published by `id` at the set event's
    /// own publishing frontier (its signed `after` dependencies and
    /// backlink), under `resolution`.
    ///
    /// With `retro`, only entries marked `include_history` contribute, per
    /// the projection's retro rule; exclusions always apply.
    ///
    /// # Errors
    ///
    /// Returns [`WriterSetStoreError::NoSuchSet`] when no applied writer set
    /// has this ID, or the read/projection error that stopped the expansion.
    pub fn expand_stored_writer_set(
        &self,
        id: &Hash,
        resolution: &Hash,
        retro: bool,
    ) -> Result<ExpandedWriters, WriterSetStoreError> {
        let Some(set) = self.stored_writer_set(id)? else {
            return Err(WriterSetStoreError::NoSuchSet(*id));
        };
        let event = self
            .event_by_id(id)?
            .ok_or(WriterSetStoreError::NoSuchSet(*id))?;
        let mut frontier: Vec<Hash> = event.header.extensions.after.clone();
        if let Some(backlink) = event.header.backlink
            && !frontier.contains(&backlink)
        {
            frontier.push(backlink);
        }
        let mut groups = Vec::new();
        let mut seen = Vec::new();
        for entry in set.entries.iter().chain(set.excludes.0.iter()) {
            if let crate::idset::Entry::Group { id: group, .. } = entry
                && !seen.contains(group)
            {
                seen.push(*group);
                groups.push(self.group_members_snapshot(group, frontier.clone(), resolution)?);
            }
        }
        Ok(expand_set(&set, &frontier, resolution, &groups, retro)?)
    }

    /// The discovery query (spec §5): every stored writer set whose scope is
    /// `scope` — or every stored set when `scope` is `None` — each expanded
    /// at its publishing frontier under `resolution`. Hits are ordered by
    /// publishing event ID, so the result is deterministic for a given store
    /// state.
    ///
    /// A stored set whose expansion fails is reported as the error that
    /// stopped it: discovery surfaces its hits whole or says why it cannot,
    /// it never silently drops a set.
    ///
    /// # Errors
    ///
    /// Returns the first read or projection error that stops the scan.
    pub fn discover_writer_sets(
        &self,
        scope: Option<&str>,
        resolution: &Hash,
    ) -> Result<Vec<DiscoveredWriterSet>, WriterSetStoreError> {
        let mut ids = Vec::new();
        {
            let txn = self.db.begin_read().map_err(StoreError::from)?;
            let bodies = txn
                .open_table(WRITER_SET_BODIES)
                .map_err(StoreError::from)?;
            for row in bodies.iter().map_err(StoreError::from)? {
                let (key, _body) = row.map_err(StoreError::from)?;
                let bytes: [u8; 32] = key.value().try_into().map_err(|_| {
                    StoreError::Corrupt("a writer-set body key is not a hash".to_owned())
                })?;
                ids.push(Hash::from(bytes));
            }
        }
        ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        let mut hits = Vec::new();
        for id in ids {
            let Some(set) = self.stored_writer_set(&id)? else {
                continue;
            };
            if scope.is_some() && set.scope.as_deref() != scope {
                continue;
            }
            let expanded = self.expand_stored_writer_set(&id, resolution, false)?;
            hits.push(DiscoveredWriterSet {
                event_id: id,
                set,
                expanded,
            });
        }
        Ok(hits)
    }
}
