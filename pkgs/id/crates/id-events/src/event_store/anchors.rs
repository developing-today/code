//! Chat anchors over the event store (#5872, #5870, #6222).
//!
//! A `chat.anchor.v1` event is carried in its conversation group exactly as a
//! `chat.message.v1` event is: the operation travels in the signed header, and sync
//! moves the event alone. [`EventStore::anchor_records`] converts the applied anchor
//! events of a group into the [`AnchorRecord`]s the pure fold consumes. The conversion
//! keeps only the nearest anchor ancestors on each causal path, so an anchor event
//! separated from another by ordinary messages still folds as causally after it.
//!
//! The read is policy-enforcing like [`EventStore::chat_timeline`]: a reader who is not
//! on the group's roster sees nothing ([`StoreError::NotMember`]), and a reader under a
//! non-full history rule sees only anchors that causally follow the event that made them
//! a member.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;
use redb::ReadableDatabase;

use super::resolve::{self, author_of};
use super::{
    ACTIONS, APPLIED, DEVICE_OPS, EVENTS, EventStore, FORKS, PROOF_HEADERS, StoreError, corrupt,
};
use crate::groups::policy::History;
use id_core::waiting::AuthorKey;
use id_store::chat_anchor::{AnchorProjection, AnchorRecord, CHAT_ANCHOR_KIND, project_anchors};

impl EventStore {
    /// The anchor records of `group` that `reader` may see, sorted by event ID.
    ///
    /// Each record's `after` holds its nearest anchor ancestors: the events the fold
    /// needs to tell a causally later mutation from a concurrent one. Events of any
    /// other kind, and anchor events of other groups, are not records of this group.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotMember`] for a reader off the roster, a redb error if
    /// the read fails, or [`StoreError::Corrupt`] for a stored anchor event that carries
    /// no operation.
    pub fn anchor_records(
        &self,
        group: &Hash,
        reader: &AuthorKey,
    ) -> Result<Vec<AnchorRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let device_ops = txn.open_table(DEVICE_OPS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load(&events, &actions, &device_ops, &forks, &proof_headers)?
        };
        let residue = super::compaction::load_residue_read(&txn)?;
        let resolved = resolve::resolve(&stored, &forks, &headers, &residue)?;
        if !resolve::roster_of(&resolved, group).contains(reader) {
            return Err(StoreError::NotMember);
        }
        let policy = resolved
            .projection
            .as_ref()
            .and_then(|projection| projection.group(group))
            .map(|view| view.policy.clone())
            .unwrap_or_default();

        let mut applied_events = Vec::new();
        for item in &stored {
            if !resolved.applied.contains(&item.event.id) {
                continue;
            }
            applied_events.push(item.event.clone());
        }

        // The whole causal graph, so an anchor separated from another by ordinary
        // events is still ordered by the fold.
        let causal_parents: HashMap<Hash, Vec<Hash>> = applied_events
            .iter()
            .map(|event| {
                let mut parents = event.header.extensions.after.clone();
                if let Some(backlink) = event.header.backlink {
                    parents.push(backlink);
                }
                (event.id, parents)
            })
            .collect();
        let anchor_ids: HashSet<Hash> = applied_events
            .iter()
            .filter_map(|event| {
                let ext = &event.header.extensions;
                (ext.kind == CHAT_ANCHOR_KIND && ext.group == Some(*group)).then_some(event.id)
            })
            .collect();
        let join = if policy.history == History::Full {
            None
        } else {
            super::chat::join_event(&stored, &resolved.applied, group, reader, &causal_parents)
        };

        let mut records = Vec::new();
        for event in &applied_events {
            let ext = &event.header.extensions;
            if ext.kind != CHAT_ANCHOR_KIND || ext.group != Some(*group) {
                continue;
            }
            let Some(anchor) = ext.anchor.clone() else {
                return Err(corrupt(
                    "a stored anchor event carries no operation".to_owned(),
                ));
            };
            let mut ancestors = HashSet::new();
            let mut stack = causal_parents.get(&event.id).cloned().unwrap_or_default();
            let mut nearest = Vec::new();
            while let Some(parent) = stack.pop() {
                if !ancestors.insert(parent) {
                    continue;
                }
                if anchor_ids.contains(&parent) {
                    // Retain only the nearest anchor ancestor on each causal path; the
                    // fold walks them transitively.
                    nearest.push(parent);
                    continue;
                }
                if let Some(parents) = causal_parents.get(&parent) {
                    stack.extend(parents.iter().copied());
                }
            }
            nearest.sort_by_key(|ancestor| *ancestor.as_bytes());
            let record = AnchorRecord {
                id: event.id,
                author: author_of(event),
                conversation: *group,
                after: nearest,
                anchor: anchor.op.clone(),
            };
            if let Some(join) = join
                && !record_follows(&record, join, &causal_parents)
            {
                continue;
            }
            records.push(record);
        }
        records.sort_by_key(|record| *record.id.as_bytes());
        Ok(records)
    }

    /// The deterministic anchor projection of `group` as `reader` may see it.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`EventStore::anchor_records`].
    pub fn anchor_projection(
        &self,
        group: &Hash,
        reader: &AuthorKey,
    ) -> Result<AnchorProjection, StoreError> {
        Ok(project_anchors(self.anchor_records(group, reader)?))
    }
}

/// Whether `record` causally follows `ancestor`, over the whole applied event graph.
fn record_follows(
    record: &AnchorRecord,
    ancestor: Hash,
    parents: &HashMap<Hash, Vec<Hash>>,
) -> bool {
    let mut stack = parents.get(&record.id).cloned().unwrap_or_default();
    let mut seen: HashSet<Hash> = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == ancestor {
            return true;
        }
        if seen.insert(id) {
            if let Some(grandparents) = parents.get(&id) {
                stack.extend(grandparents.iter().copied());
            }
        }
    }
    false
}
