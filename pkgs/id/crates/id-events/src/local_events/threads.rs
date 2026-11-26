//! Thread resolution and change proposals over the node's event store (#7849, decision
//! record `doc/2026-11-08_decision_id5874_thread_resolution.md`).
//!
//! A resolution, proposal, rejection or withdrawal is the signed body of one event of its
//! own kind in the thread's conversation group, carried in the header as a chat anchor is.
//! Writing keeps the v1 channel-membership rule every chat-family write uses: only a
//! member of the group signs one. Reading folds the group's thread events with the
//! accepting edits this store's notes carry (a note revision that cites a proposal), so
//! acceptance is the edit itself and never a separate approval write (§3–§4).
//!
//! Which keys may resolve a thread, and which may write an object, is the fold's
//! [`ThreadAuthority`]. [`LocalEvents::thread_projection_with`] takes one from the caller;
//! [`LocalEvents::thread_projection`] uses this node's own answer: a member of the group may
//! resolve its threads, and a key may write an object when it created it or this store
//! holds a revision of it by that key. That second rule is the conservative reading of the
//! note model, which admits a revision by any key: a key that has not edited an object
//! cannot reject a proposal on it until it does.
//!
//! [`LocalEvents::thread_view`] is the projection this node shows its user: the same fold,
//! cut to what this node may see ([`ThreadProjection::visible_to`]).

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use p2panda_core::Hash;

use super::{LocalEvents, now_ms};
use crate::event_store::Ingest;
use id_core::waiting::AuthorKey;
use id_store::chain::{ChainExt, sign};
use id_store::chat_anchor::ObjectRef;
use id_store::notes::accepting_edits;
use id_store::thread_resolution::{
    ThreadAuthority, ThreadEvent, ThreadEventRecord, ThreadProjection, project_threads,
};

/// This node's answer to the fold's authority questions for one group (see the module
/// documentation).
struct NodeAuthority {
    roster: HashSet<AuthorKey>,
    writers: BTreeMap<ObjectRef, HashSet<AuthorKey>>,
}

impl ThreadAuthority for NodeAuthority {
    fn may_resolve(&self, _thread: &Hash, author: &AuthorKey) -> bool {
        self.roster.contains(author)
    }

    fn may_write(&self, object: &ObjectRef, author: &AuthorKey) -> bool {
        object.author == *author
            || self
                .writers
                .get(object)
                .is_some_and(|writers| writers.contains(author))
    }
}

impl LocalEvents {
    /// Sends `event` as the thread event of its kind in the conversation `group`, and
    /// returns the event's ID.
    ///
    /// Admission is refused before anything is signed when the body breaks its own rules,
    /// or when the node's key is not a member of the group. Whether the act counts (a
    /// resolver's or writer's authority, a proposal the rejection names) is the fold's
    /// decision on read, so a sync peer that bypasses this gate gains nothing.
    ///
    /// # Errors
    ///
    /// Fails if the body is refused by [`ThreadEvent::check`], the node is not a member,
    /// or the store cannot record the event.
    pub fn send_thread_event(&mut self, group: Hash, event: ThreadEvent) -> Result<Hash> {
        event.check()?;
        let author = self.author();
        if !self
            .read_store(|store| store.roster(&group))?
            .contains(&author)
        {
            bail!(
                "the node's key is not a member of the channel group, so its thread write is refused at admission"
            );
        }
        let kind = event.kind();
        let after = self.bounded_after(&[])?;
        let chain = self
            .read_store(|store| store.chain(&author))?
            .unwrap_or_default();
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: Some(group),
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: Some(event),
        };
        let signed = sign(&self.key, chain.next_seq, chain.head, None, ext)?;
        match self.store.ingest(&signed.bytes, None, now_ms())? {
            Ingest::Accepted { id, .. } => Ok(id),
            refused => bail!("the store did not apply the {kind} event: {refused:?}"),
        }
    }

    /// The thread events of `group` this node may see, as the fold reads them.
    ///
    /// # Errors
    ///
    /// Fails if the node is not a member of the group, or the store cannot be read.
    pub fn thread_records(&self, group: Hash) -> Result<Vec<ThreadEventRecord>> {
        self.read_store(|store| store.thread_records(&group, &self.author()))
    }

    /// The folded thread projection of `group` under `authority`: the group's thread
    /// events this node may see, with every accepting edit this store's notes carry.
    ///
    /// # Errors
    ///
    /// Fails if the node is not a member of the group, or the store cannot be read or its
    /// notes do not fold.
    pub fn thread_projection_with(
        &self,
        group: Hash,
        authority: &dyn ThreadAuthority,
    ) -> Result<ThreadProjection> {
        let records = self.thread_records(group)?;
        let edits = accepting_edits(&self.notes()?);
        Ok(project_threads(records, edits, authority))
    }

    /// The folded thread projection of `group` under this node's own authority answer
    /// (see the module documentation).
    ///
    /// # Errors
    ///
    /// As for [`Self::thread_projection_with`].
    pub fn thread_projection(&self, group: Hash) -> Result<ThreadProjection> {
        let roster: HashSet<AuthorKey> = self
            .read_store(|store| store.roster(&group))?
            .into_iter()
            .collect();
        let mut writers: BTreeMap<ObjectRef, HashSet<AuthorKey>> = BTreeMap::new();
        for note in self.notes()? {
            if let Some(object) = note.object() {
                writers
                    .entry(object)
                    .or_default()
                    .extend(note.revisions.iter().map(|revision| revision.author));
            }
        }
        self.thread_projection_with(group, &NodeAuthority { roster, writers })
    }

    /// The thread projection of `group` as this node, its viewer, may see it: the fold of
    /// [`Self::thread_projection`] passed through [`ThreadProjection::visible_to`], so a
    /// display of it keeps the no-leak rule.
    ///
    /// The viewer reads every thread of the group, since the read is already cut to the
    /// thread events a member may see. It reads an object on the rule a talk page uses: an
    /// event this store has applied and, when that event names a group, one this node is a
    /// member of. Anything naming another object is dropped whole, and the view keeps no
    /// refused event IDs.
    ///
    /// # Errors
    ///
    /// As for [`Self::thread_projection`].
    pub fn thread_view(&self, group: Hash) -> Result<ThreadProjection> {
        let projection = self.thread_projection(group)?;
        let can_read_object = |object: &ObjectRef| self.visible_object(object.object_id).is_ok();
        Ok(projection.visible_to(&|_: &Hash| true, &can_read_object))
    }
}
