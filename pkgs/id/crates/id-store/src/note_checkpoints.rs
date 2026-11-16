//! Note checkpoints for collaborative editing (#3819, Phase 3 of the #2048
//! Zettelkasten design): the checkpoint core that ties a live editing
//! session to the durable note graph [`crate::notes`] defines.
//!
//! The split this module enforces is the phase's first rule: editor steps
//! stay ephemeral (they live in the session, never in the graph), and what
//! crosses into the graph is a *checkpoint* — one signed `note.revise`
//! event (a [`NoteOp::Revise`]) naming the note's stable ID, the verified
//! blob content the editor had reached, and the parent revision(s) it
//! builds on. A checkpoint is published only at an explicit point (save,
//! blur, session end — the session owner's call), never per keystroke.
//!
//! Concurrency is the graph's, unchanged: two sessions that checkpoint
//! from the same head produce two heads, and nothing here picks a winner.
//! [`NoteCheckpoints::checkpoint_on_heads`] refuses a multi-head note —
//! joining heads is a decision, and it is made only through the explicit
//! [`NoteCheckpoints::merge`]. A session working from a stale head names
//! that head as its parent and forks, exactly as an offline edit should.
//!
//! The log is reconstructible: [`NoteCheckpoints::from_events`] rebuilds
//! it from the applied note events alone (a restart, a sync from a peer),
//! and the fold is order-independent, so the rebuilt heads are the heads
//! the live log had. Persistence and transport of the events are the
//! event store's business, as for every other kind; this core owns the
//! checkpoint rules and nothing else.

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;

use crate::documents::DocumentRef;
use crate::notes::{
    AppliedNote, CREATE_KIND, NoteError, NoteFoldError, NoteOp, NoteRecord, REVISE_KIND,
    check_note_op, fold_notes,
};

/// Why a checkpoint operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// The operation breaks the note rules ([`check_note_op`]).
    #[error(transparent)]
    Note(#[from] NoteError),
    /// The resulting event set does not fold ([`fold_notes`]).
    #[error(transparent)]
    Fold(#[from] NoteFoldError),
    /// The named note has no create event in the log.
    #[error("note {0} has no create event in the checkpoint log")]
    UnknownNote(Hash),
    /// A follow-on checkpoint was asked to build on the current heads
    /// of a note that has more than one: joining heads is an explicit
    /// merge, never a side effect of saving.
    #[error("note {note} has heads {heads:?}; a merge must name them explicitly")]
    ExplicitMergeRequired {
        /// The note whose heads would have been joined silently.
        note: Hash,
        /// The current heads, in ID order.
        heads: Vec<Hash>,
    },
    /// A merge was asked for on a note with fewer than two heads:
    /// there is nothing concurrent to join.
    #[error("note {note} has heads {heads:?}; a merge joins at least two")]
    NotConcurrent {
        /// The note the merge was asked for.
        note: Hash,
        /// The current heads, in ID order.
        heads: Vec<Hash>,
    },
}

/// The checkpoint log for collaborative notes: the applied note events
/// checkpoints are made of, folded on demand into per-note records and
/// heads. Events are appended only after the whole log still folds, so
/// a refused checkpoint leaves the log exactly as it was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteCheckpoints {
    events: Vec<AppliedNote>,
}

impl NoteCheckpoints {
    /// An empty log: no notes, no checkpoints.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds a log from applied note events — the restart/sync path:
    /// the events are the durable state, and the fold they produce is
    /// the state the live log had.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the events do not fold
    /// (duplicate IDs, a revision naming an unknown note or parent, a
    /// cycle).
    pub fn from_events(events: Vec<AppliedNote>) -> Result<Self, CheckpointError> {
        fold_notes(&events)?;
        Ok(Self { events })
    }

    /// The applied note events, in the order they were recorded. This
    /// is the durable form of the log: persist or sync these and
    /// [`Self::from_events`] rebuilds the rest.
    #[must_use]
    pub fn events(&self) -> &[AppliedNote] {
        &self.events
    }

    /// Every note in the log, folded, in note ID order.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the recorded events do
    /// not fold. Events are validated before they are recorded, so a
    /// log built through this type's methods does not produce this.
    pub fn notes(&self) -> Result<Vec<NoteRecord>, CheckpointError> {
        Ok(fold_notes(&self.events)?)
    }

    /// One note's folded record, if the log has its create event.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] on the same condition as
    /// [`Self::notes`].
    pub fn record(&self, note: Hash) -> Result<Option<NoteRecord>, CheckpointError> {
        Ok(self.notes()?.into_iter().find(|record| record.id == note))
    }

    /// A note's current heads, in ID order: the revisions no other
    /// revision names as a parent. Empty when the note is unknown.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] on the same condition as
    /// [`Self::notes`].
    pub fn heads(&self, note: Hash) -> Result<Vec<Hash>, CheckpointError> {
        Ok(self
            .record(note)?
            .map(|record| record.heads)
            .unwrap_or_default())
    }

    /// Establishes a note in the log: records the `note.create` event
    /// whose ID is the note's stable ID, carrying its first content.
    /// The create is also the note's first revision and its only head.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Note`] when the content breaks the
    /// document rules, and [`CheckpointError::Fold`] when the event ID
    /// is already in the log.
    pub fn create(
        &mut self,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let op = NoteOp::Create { content };
        check_note_op(CREATE_KIND, &op)?;
        self.append(AppliedNote {
            id: event_id,
            author,
            op,
        })?;
        self.record(event_id)
            .map(|record| record.expect("a just-created note folds"))
    }

    /// Publishes one checkpoint of `note`: a signed revision naming the
    /// parents the caller chose. Naming a head extends it; naming an
    /// older revision forks from it, and the fork is a head of its own —
    /// no checkpoint ever overwrites or retires another session's head.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::UnknownNote`] when the log has no
    /// create event for `note`, [`CheckpointError::Note`] when the
    /// revision breaks the note rules (no parents, a parent named
    /// twice, too many parents, a content rule), and
    /// [`CheckpointError::Fold`] when a parent is not a revision of
    /// this note or the event ID is already in the log. On any error
    /// the log is unchanged.
    pub fn checkpoint(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        parents: Vec<Hash>,
    ) -> Result<NoteRecord, CheckpointError> {
        if self.record(note)?.is_none() {
            return Err(CheckpointError::UnknownNote(note));
        }
        let op = NoteOp::Revise {
            note,
            content,
            parents,
        };
        check_note_op(REVISE_KIND, &op)?;
        self.append(AppliedNote {
            id: event_id,
            author,
            op,
        })?;
        self.record(note)
            .map(|record| record.expect("a checkpointed note folds"))
    }

    /// Publishes a checkpoint on the note's current head — the ordinary
    /// save path, for a note with exactly one head.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::ExplicitMergeRequired`], naming the
    /// heads, when the note has more than one head: saving must not
    /// silently join concurrent work. Returns the errors of
    /// [`Self::checkpoint`] otherwise, including
    /// [`CheckpointError::UnknownNote`] for an unknown note (which has
    /// no heads to build on).
    pub fn checkpoint_on_heads(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        match heads.len() {
            0 => Err(CheckpointError::UnknownNote(note)),
            1 => self.checkpoint(note, event_id, author, content, heads),
            _ => Err(CheckpointError::ExplicitMergeRequired { note, heads }),
        }
    }

    /// Explicitly merges a note's concurrent heads: one revision naming
    /// every current head as its parent, after which the merge is the
    /// note's only head. This is the only operation that retires more
    /// than one head, and it exists only as a deliberate call.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::NotConcurrent`], naming the heads,
    /// when the note has fewer than two heads, and the errors of
    /// [`Self::checkpoint`] otherwise.
    pub fn merge(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        if heads.len() < 2 {
            return Err(CheckpointError::NotConcurrent { note, heads });
        }
        self.checkpoint(note, event_id, author, content, heads)
    }

    /// Records one event after proving the whole log still folds with
    /// it; a failed proof records nothing.
    fn append(&mut self, event: AppliedNote) -> Result<(), CheckpointError> {
        let mut candidate = self.events.clone();
        candidate.push(event);
        fold_notes(&candidate)?;
        self.events = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn author(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    fn content(bytes: &[u8]) -> DocumentRef {
        DocumentRef::for_bytes("text/markdown", bytes).unwrap()
    }

    #[test]
    fn test_a_checkpoint_chains_on_the_head_and_the_note_id_is_stable() {
        let mut log = NoteCheckpoints::new();
        let created = log.create(hash(10), author(1), content(b"first")).unwrap();
        assert_eq!(created.id, hash(10));
        assert_eq!(created.heads, vec![hash(10)]);

        let saved = log
            .checkpoint_on_heads(hash(10), hash(11), author(1), content(b"second"))
            .unwrap();
        assert_eq!(
            saved.id,
            hash(10),
            "the note ID never becomes a revision ID"
        );
        assert_eq!(saved.revisions.len(), 2);
        assert_eq!(saved.heads, vec![hash(11)]);
        assert_eq!(saved.revision(hash(11)).unwrap().parents, vec![hash(10)]);
        assert!(
            saved
                .revision(hash(11))
                .unwrap()
                .content
                .verify_content(b"second")
                .is_ok()
        );
    }

    #[test]
    fn test_concurrent_checkpoints_stay_two_heads_until_an_explicit_merge() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        // Two sessions checkpoint from the same head: neither wins.
        log.checkpoint(
            hash(10),
            hash(11),
            author(1),
            content(b"alice"),
            vec![hash(10)],
        )
        .unwrap();
        let forked = log
            .checkpoint(
                hash(10),
                hash(12),
                author(2),
                content(b"bob"),
                vec![hash(10)],
            )
            .unwrap();
        assert_eq!(forked.heads, vec![hash(11), hash(12)]);

        // The ordinary save path refuses to join them silently, and
        // the refusal changes nothing.
        let err = log
            .checkpoint_on_heads(hash(10), hash(13), author(1), content(b"sneaky"))
            .unwrap_err();
        assert_eq!(
            err,
            CheckpointError::ExplicitMergeRequired {
                note: hash(10),
                heads: vec![hash(11), hash(12)],
            }
        );
        assert_eq!(log.heads(hash(10)).unwrap(), vec![hash(11), hash(12)]);

        // The explicit merge names both heads and becomes the one head.
        let merged = log
            .merge(hash(10), hash(13), author(1), content(b"joined"))
            .unwrap();
        assert_eq!(merged.heads, vec![hash(13)]);
        assert_eq!(
            merged.revision(hash(13)).unwrap().parents,
            vec![hash(11), hash(12)]
        );
        // And a merge with nothing concurrent left is refused.
        assert_eq!(
            log.merge(hash(10), hash(14), author(1), content(b"again"))
                .unwrap_err(),
            CheckpointError::NotConcurrent {
                note: hash(10),
                heads: vec![hash(13)],
            }
        );
    }

    #[test]
    fn test_a_checkpoint_from_a_stale_head_forks_instead_of_overwriting() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        log.checkpoint_on_heads(hash(10), hash(11), author(1), content(b"second"))
            .unwrap();
        // A session that never saw revision 11 checkpoints from 10:
        // its work is a second head, not a lost update to 11.
        let forked = log
            .checkpoint(
                hash(10),
                hash(12),
                author(2),
                content(b"offline"),
                vec![hash(10)],
            )
            .unwrap();
        assert_eq!(forked.heads, vec![hash(11), hash(12)]);
    }

    #[test]
    fn test_a_refused_checkpoint_leaves_the_log_unchanged() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        let before = log.events().to_vec();
        // A parent that is not a revision of this note.
        assert!(
            log.checkpoint(hash(10), hash(11), author(1), content(b"x"), vec![hash(99)])
                .is_err()
        );
        // No parents at all.
        assert!(
            log.checkpoint(hash(10), hash(11), author(1), content(b"x"), vec![])
                .is_err()
        );
        // An unknown note.
        assert_eq!(
            log.checkpoint_on_heads(hash(77), hash(11), author(1), content(b"x"))
                .unwrap_err(),
            CheckpointError::UnknownNote(hash(77))
        );
        assert_eq!(log.events(), before.as_slice());
        assert_eq!(log.heads(hash(10)).unwrap(), vec![hash(10)]);
    }

    #[test]
    fn test_the_log_rebuilds_from_its_events_with_the_same_heads() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        log.checkpoint(
            hash(10),
            hash(11),
            author(1),
            content(b"alice"),
            vec![hash(10)],
        )
        .unwrap();
        log.checkpoint(
            hash(10),
            hash(12),
            author(2),
            content(b"bob"),
            vec![hash(10)],
        )
        .unwrap();
        // Restart/sync: the events alone rebuild the log, and the
        // rebuild does not depend on the order they arrived in.
        let mut reversed = log.events().to_vec();
        reversed.reverse();
        let rebuilt = NoteCheckpoints::from_events(reversed).unwrap();
        assert_eq!(rebuilt.notes().unwrap(), log.notes().unwrap());
        assert_eq!(rebuilt.heads(hash(10)).unwrap(), vec![hash(11), hash(12)]);
    }
}
