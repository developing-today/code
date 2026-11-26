//! Thread resolution and change-proposal events over the event store (#7849,
//! decision record `doc/2026-11-08_decision_id5874_thread_resolution.md`).
//!
//! A `chat.resolution.v1`, `chat.proposal.v1`, `chat.proposal-rejection.v1` or
//! `chat.proposal-withdrawal.v1` event is carried in its conversation group exactly as
//! a `chat.anchor.v1` event is: the body travels in the signed header, and sync moves
//! the event alone. [`EventStore::thread_records`] converts the applied thread events
//! of a group into the [`ThreadEventRecord`]s the pure fold
//! (`id_store::thread_resolution::project_threads`) consumes. Each record's `after`
//! keeps only the nearest thread-event ancestors on each causal path, so a resolution
//! separated from an earlier one by ordinary messages still folds as causally after it.
//!
//! The read is policy-enforcing like [`EventStore::anchor_records`]: a reader who is
//! not on the group's roster sees nothing ([`StoreError::NotMember`]), and a reader
//! under a non-full history rule sees only thread events that causally follow the
//! event that made them a member.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::resolve::{self, author_of};
use super::{
    ACTIONS, APPLIED, DEVICE_OPS, EVENTS, EventStore, FORKS, PROOF_HEADERS, StoreError, corrupt,
};
use crate::groups::policy::History;
use id_core::waiting::AuthorKey;
use id_store::thread_resolution::{ThreadEventRecord, is_thread_event_kind};

impl EventStore {
    /// The thread event records of `group` that `reader` may see, sorted by event ID.
    ///
    /// Each record's `after` holds its nearest thread-event ancestors, which already
    /// include any through the author's own backlink, so `backlink` is left `None`.
    /// Events of any other kind, and thread events of other groups, are not records of
    /// this group.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotMember`] for a reader off the roster, a redb error if
    /// the read fails, or [`StoreError::Corrupt`] for a stored thread event that carries
    /// no body.
    pub fn thread_records(
        &self,
        group: &Hash,
        reader: &AuthorKey,
    ) -> Result<Vec<ThreadEventRecord>, StoreError> {
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
        if !resolve::roster_of(&resolved, group).contains(reader) {
            return Err(StoreError::NotMember);
        }
        let policy = resolved
            .projection
            .as_ref()
            .and_then(|projection| projection.group(group))
            .map(|view| view.policy.clone())
            .unwrap_or_default();

        let applied_events: Vec<_> = stored
            .iter()
            .filter(|item| resolved.applied.contains(&item.event.id))
            .map(|item| item.event.clone())
            .collect();

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
        let in_group = |kind: &str, event_group: Option<Hash>| {
            is_thread_event_kind(kind) && event_group == Some(*group)
        };
        let thread_ids: HashSet<Hash> = applied_events
            .iter()
            .filter_map(|event| {
                let ext = &event.header.extensions;
                in_group(&ext.kind, ext.group).then_some(event.id)
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
            if !in_group(&ext.kind, ext.group) {
                continue;
            }
            let Some(body) = ext.thread.clone() else {
                return Err(corrupt("a stored thread event carries no body".to_owned()));
            };
            if let Some(join) = join
                && !follows(event.id, join, &causal_parents)
            {
                continue;
            }
            let mut ancestors = HashSet::new();
            let mut stack = causal_parents.get(&event.id).cloned().unwrap_or_default();
            let mut nearest = Vec::new();
            while let Some(parent) = stack.pop() {
                if !ancestors.insert(parent) {
                    continue;
                }
                if thread_ids.contains(&parent) {
                    // The nearest thread ancestor on this path; the fold walks
                    // the rest transitively.
                    nearest.push(parent);
                    continue;
                }
                if let Some(parents) = causal_parents.get(&parent) {
                    stack.extend(parents.iter().copied());
                }
            }
            nearest.sort_by_key(|ancestor| *ancestor.as_bytes());
            records.push(ThreadEventRecord {
                id: event.id,
                author: author_of(event),
                after: nearest,
                backlink: None,
                event: body,
            });
        }
        records.sort_by_key(|record| *record.id.as_bytes());
        Ok(records)
    }
}

/// Whether `id` causally follows `ancestor`, over the whole applied event graph.
fn follows(id: Hash, ancestor: Hash, parents: &HashMap<Hash, Vec<Hash>>) -> bool {
    let mut stack = parents.get(&id).cloned().unwrap_or_default();
    let mut seen: HashSet<Hash> = HashSet::new();
    while let Some(next) = stack.pop() {
        if next == ancestor {
            return true;
        }
        if seen.insert(next) {
            if let Some(grandparents) = parents.get(&next) {
                stack.extend(grandparents.iter().copied());
            }
        }
    }
    false
}
