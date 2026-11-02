//! Notes as signed events: a stable logical identity over an immutable revision graph
//! (#3814, Phase 1 of the #2048 Zettelkasten design).
//!
//! A note is not one event. A `note.create` event establishes the note: its event ID is the
//! note's stable ID, distinct from every revision ID and independent of any filename or
//! alias. A `note.revise` event is one immutable revision: it names the note it revises, the
//! verified blob content it carries (a [`DocumentRef`], as a `document.put` names its
//! content), and the causal parent revision IDs it builds on.
//!
//! Concurrent revisions are distinct heads: nothing picks a winner. A merge is an explicit
//! `note.revise` that names both heads as its parents. [`fold_notes`] folds applied note
//! events into per-note history and heads; its output is sorted by ID at every level, so it
//! is the same whatever order the events arrived in.
//!
//! Existing `document.put` events are untouched: a note is a new kind pair, not a document
//! with extra rules. Deletion and erasure are out of this phase (the #2048 Q1 decision), and
//! no note operation removes anything.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::documents::DocumentRef;
use crate::waiting::AuthorKey;

/// The kind of a `note.create` event, as the header's `kind` names it. The event's own ID is
/// the note's stable ID.
pub const CREATE_KIND: &str = "note.create";

/// The kind of a `note.revise` event, which names its note, its content, and its causal
/// parent revisions in its header.
pub const REVISE_KIND: &str = "note.revise";

/// The most parent revisions one `note.revise` may name. A merge of more heads than this is
/// several merges.
pub const MAX_PARENTS: usize = 32;

/// What a note event does. The kind of the event says which form the operation takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteOp {
    /// Establishes a note with its first content. The carrying event's ID is the note's ID,
    /// and the event is also the note's first revision, with no parents.
    Create {
        /// The note's first content, named by hash and size as a blob.
        content: DocumentRef,
    },
    /// Adds one immutable revision to an existing note.
    Revise {
        /// The note's stable ID: the ID of its `note.create` event.
        note: Hash,
        /// This revision's content, named by hash and size as a blob.
        content: DocumentRef,
        /// The revisions this one builds on: at least one, none repeated. A first revision
        /// names the note's create event; a merge names every head it joins.
        parents: Vec<Hash>,
    },
}

impl NoteOp {
    /// The content the operation names, whichever form it takes.
    #[must_use]
    pub const fn content(&self) -> &DocumentRef {
        match self {
            Self::Create { content } | Self::Revise { content, .. } => content,
        }
    }
}

/// Why a note operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NoteError {
    /// The operation is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong note operation")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
    /// A field breaks a limit or a content rule.
    #[error("{0}")]
    Field(String),
}

/// Whether `kind` is a note kind, so its event carries a note operation.
#[must_use]
pub fn is_note_kind(kind: &str) -> bool {
    kind == CREATE_KIND || kind == REVISE_KIND
}

/// Checks a note operation against the kind of the event that carries it.
///
/// # Errors
///
/// Returns [`NoteError::KindMismatch`] when the operation is not the one `kind` names.
/// Returns [`NoteError::Field`] when the named content breaks [`DocumentRef::check`], or a
/// revision names no parents, more than [`MAX_PARENTS`] parents, or a parent twice.
pub fn check_note_op(kind: &str, op: &NoteOp) -> Result<(), NoteError> {
    match (kind, op) {
        (CREATE_KIND, NoteOp::Create { content }) => {
            content
                .check()
                .map_err(|err| NoteError::Field(err.to_string()))?;
            Ok(())
        }
        (
            REVISE_KIND,
            NoteOp::Revise {
                content, parents, ..
            },
        ) => {
            content
                .check()
                .map_err(|err| NoteError::Field(err.to_string()))?;
            if parents.is_empty() {
                return Err(NoteError::Field(
                    "a revision names at least one parent".to_owned(),
                ));
            }
            if parents.len() > MAX_PARENTS {
                return Err(NoteError::Field(format!(
                    "a revision names {} parents, it may name at most {MAX_PARENTS}",
                    parents.len()
                )));
            }
            let mut seen = HashSet::with_capacity(parents.len());
            if !parents.iter().all(|parent| seen.insert(parent)) {
                return Err(NoteError::Field(
                    "a revision names a parent twice".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Err(NoteError::KindMismatch {
            kind: kind.to_owned(),
        }),
    }
}

/// An applied note event, as the fold reads it: its ID, its signer, and its operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedNote {
    /// The event's ID. For a `note.create`, this is also the note's stable ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// What the event does.
    pub op: NoteOp,
}

/// One immutable revision of a note, as the fold records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionRecord {
    /// The revision's ID: the ID of the event that made it. For the first revision this is
    /// the note's own ID.
    pub id: Hash,
    /// The key that signed the revision's event.
    pub author: AuthorKey,
    /// The content the revision names. Bytes fetched for it are checked with
    /// [`DocumentRef::verify_content`] before they are used.
    pub content: DocumentRef,
    /// The revisions this one builds on, in ID order. Empty only for the first revision:
    /// the note's create.
    pub parents: Vec<Hash>,
}

/// A note in the store: its stable ID, every revision folded so far, and its current heads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRecord {
    /// The note's stable ID: the ID of its `note.create` event. It is not any later
    /// revision's ID, and it does not depend on a filename or alias.
    pub id: Hash,
    /// Every revision of the note, in revision ID order. The first is the create.
    pub revisions: Vec<RevisionRecord>,
    /// The revisions no other revision names as a parent, in ID order. Concurrent
    /// revisions are all heads until an explicit merge revision names them.
    pub heads: Vec<Hash>,
}

impl NoteRecord {
    /// The revision with ID `id`, if this note has it.
    #[must_use]
    pub fn revision(&self, id: Hash) -> Option<&RevisionRecord> {
        self.revisions.iter().find(|revision| revision.id == id)
    }
}

/// Why a set of applied note events could not be folded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NoteFoldError {
    /// Two applied events share one ID.
    #[error("two applied note events share the ID {0}")]
    DuplicateEvent(Hash),
    /// A revision names a note with no applied create event.
    #[error("revision {revision} names note {note}, which has no create event")]
    UnknownNote {
        /// The revision that names the missing note.
        revision: Hash,
        /// The note it names.
        note: Hash,
    },
    /// A revision names a parent that is not any note event in the fold.
    #[error("revision {revision} names parent {parent}, which is not a note event")]
    UnknownParent {
        /// The revision that names the missing parent.
        revision: Hash,
        /// The parent it names.
        parent: Hash,
    },
    /// A revision names a parent that is a revision of a different note.
    #[error("revision {revision} names parent {parent}, a revision of note {note}")]
    ParentOfAnotherNote {
        /// The revision that names the foreign parent.
        revision: Hash,
        /// The parent it names.
        parent: Hash,
        /// The note the parent belongs to.
        note: Hash,
    },
}

/// Folds applied note events into one record per note, in note ID order.
///
/// A revision is folded only once every parent it names is folded, so the result does not
/// depend on the order `events` are given in. Revisions and heads are in ID order.
///
/// # Errors
///
/// Returns [`NoteFoldError`] when two events share an ID, a revision names a note with no
/// create event in the set, or a revision names a parent that is unknown or belongs to
/// another note.
pub fn fold_notes(events: &[AppliedNote]) -> Result<Vec<NoteRecord>, NoteFoldError> {
    let mut by_id: HashMap<Hash, &AppliedNote> = HashMap::with_capacity(events.len());
    for event in events {
        if by_id.insert(event.id, event).is_some() {
            return Err(NoteFoldError::DuplicateEvent(event.id));
        }
    }
    // The note each revise event belongs to, by the revise event's ID. A create belongs to
    // the note it establishes: itself.
    let mut owner: HashMap<Hash, Hash> = HashMap::with_capacity(events.len());
    for event in events {
        match &event.op {
            NoteOp::Create { .. } => {
                owner.insert(event.id, event.id);
            }
            NoteOp::Revise { note, .. } => {
                owner.insert(event.id, *note);
            }
        }
    }
    let mut notes: Vec<NoteRecord> = Vec::new();
    for event in events {
        let NoteOp::Create { content } = &event.op else {
            continue;
        };
        let note_id = event.id;
        let mut revisions: Vec<RevisionRecord> = vec![RevisionRecord {
            id: note_id,
            author: event.author,
            content: content.clone(),
            parents: Vec::new(),
        }];
        let mut folded: HashSet<Hash> = HashSet::from([note_id]);
        let mut pending: Vec<&AppliedNote> = events
            .iter()
            .filter(|candidate| {
                matches!(&candidate.op, NoteOp::Revise { note, .. } if *note == note_id)
            })
            .collect();
        pending.sort_by_key(|candidate| *candidate.id.as_bytes());
        // A revision folds once its parents have folded; the pending list is re-scanned in
        // ID order until it is empty or nothing more can fold.
        while !pending.is_empty() {
            let mut progressed = false;
            let mut rest: Vec<&AppliedNote> = Vec::with_capacity(pending.len());
            for candidate in pending {
                let NoteOp::Revise {
                    content, parents, ..
                } = &candidate.op
                else {
                    continue;
                };
                for parent in parents {
                    match owner.get(parent) {
                        None => {
                            return Err(NoteFoldError::UnknownParent {
                                revision: candidate.id,
                                parent: *parent,
                            });
                        }
                        Some(parent_note) if *parent_note != note_id => {
                            return Err(NoteFoldError::ParentOfAnotherNote {
                                revision: candidate.id,
                                parent: *parent,
                                note: *parent_note,
                            });
                        }
                        _ => {}
                    }
                }
                if parents.iter().all(|parent| folded.contains(parent)) {
                    let mut named = parents.clone();
                    named.sort_by_key(|parent| *parent.as_bytes());
                    revisions.push(RevisionRecord {
                        id: candidate.id,
                        author: candidate.author,
                        content: content.clone(),
                        parents: named,
                    });
                    folded.insert(candidate.id);
                    progressed = true;
                } else {
                    rest.push(candidate);
                }
            }
            if !progressed {
                // Every remaining revision waits on a parent this note's fold never
                // produces: the first such parent in ID order names the failure.
                let stuck = rest[0];
                let NoteOp::Revise { parents, .. } = &stuck.op else {
                    unreachable!("pending holds only revise events");
                };
                let parent = parents
                    .iter()
                    .find(|parent| !folded.contains(parent))
                    .copied()
                    .unwrap_or(stuck.id);
                return Err(NoteFoldError::UnknownParent {
                    revision: stuck.id,
                    parent,
                });
            }
            pending = rest;
        }
        revisions.sort_by_key(|revision| *revision.id.as_bytes());
        let named_as_parent: HashSet<Hash> = revisions
            .iter()
            .flat_map(|revision| revision.parents.iter().copied())
            .collect();
        let heads: Vec<Hash> = revisions
            .iter()
            .map(|revision| revision.id)
            .filter(|id| !named_as_parent.contains(id))
            .collect();
        notes.push(NoteRecord {
            id: note_id,
            revisions,
            heads,
        });
    }
    // A revise whose note has no create in the set was never visited above.
    for event in events {
        if let NoteOp::Revise { note, .. } = &event.op {
            if !matches!(
                by_id.get(note).map(|create| &create.op),
                Some(NoteOp::Create { .. })
            ) {
                return Err(NoteFoldError::UnknownNote {
                    revision: event.id,
                    note: *note,
                });
            }
        }
    }
    notes.sort_by_key(|note| *note.id.as_bytes());
    Ok(notes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    fn create(id: u8, bytes: &[u8]) -> AppliedNote {
        AppliedNote {
            id: hash(id),
            author: author(1),
            op: NoteOp::Create {
                content: content(bytes),
            },
        }
    }

    fn revise(id: u8, note: u8, bytes: &[u8], parents: &[u8]) -> AppliedNote {
        AppliedNote {
            id: hash(id),
            author: author(1),
            op: NoteOp::Revise {
                note: hash(note),
                content: content(bytes),
                parents: parents.iter().map(|parent| hash(*parent)).collect(),
            },
        }
    }

    #[test]
    fn test_the_note_id_is_the_create_event_and_revisions_chain_under_it() {
        let events = vec![
            create(10, b"first"),
            revise(11, 10, b"second", &[10]),
            revise(12, 10, b"third", &[11]),
        ];
        let notes = fold_notes(&events).unwrap();
        assert_eq!(notes.len(), 1);
        let note = &notes[0];
        // The stable ID is the create event's, not any revision's.
        assert_eq!(note.id, hash(10));
        assert_eq!(note.revisions.len(), 3);
        assert_eq!(note.heads, vec![hash(12)]);
        let second = note.revision(hash(11)).unwrap();
        assert_eq!(second.parents, vec![hash(10)]);
        // The revision's content is verified the document way: the bytes it names pass,
        // other bytes do not.
        assert!(second.content.verify_content(b"second").is_ok());
        assert!(second.content.verify_content(b"tampered").is_err());
    }

    #[test]
    fn test_concurrent_revisions_stay_heads_until_an_explicit_merge() {
        let base = vec![
            create(10, b"first"),
            revise(11, 10, b"branch a", &[10]),
            revise(12, 10, b"branch b", &[10]),
        ];
        let notes = fold_notes(&base).unwrap();
        assert_eq!(notes[0].heads, vec![hash(11), hash(12)]);
        let mut merged = base.clone();
        merged.push(revise(13, 10, b"merged", &[12, 11]));
        let notes = fold_notes(&merged).unwrap();
        assert_eq!(notes[0].heads, vec![hash(13)]);
        // The merge names both heads as its parents, in ID order.
        assert_eq!(
            notes[0].revision(hash(13)).unwrap().parents,
            vec![hash(11), hash(12)]
        );
    }

    #[test]
    fn test_the_fold_is_the_same_whatever_order_events_arrive_in() {
        let events = vec![
            create(10, b"first"),
            revise(12, 10, b"third", &[11]),
            revise(11, 10, b"second", &[10]),
            create(20, b"other"),
            revise(21, 20, b"other second", &[20]),
        ];
        let forward = fold_notes(&events).unwrap();
        let mut reversed = events.clone();
        reversed.reverse();
        assert_eq!(forward, fold_notes(&reversed).unwrap());
        assert_eq!(forward.len(), 2);
    }

    #[test]
    fn test_malformed_operations_are_refused() {
        let good = content(b"x");
        // A create kind carrying a revise operation, and the reverse.
        assert!(matches!(
            check_note_op(
                CREATE_KIND,
                &NoteOp::Revise {
                    note: hash(1),
                    content: good.clone(),
                    parents: vec![hash(1)],
                }
            ),
            Err(NoteError::KindMismatch { .. })
        ));
        assert!(matches!(
            check_note_op(
                REVISE_KIND,
                &NoteOp::Create {
                    content: good.clone()
                }
            ),
            Err(NoteError::KindMismatch { .. })
        ));
        // No parents, a parent twice, and too many parents.
        for parents in [
            Vec::new(),
            vec![hash(1), hash(1)],
            (0..=MAX_PARENTS)
                .map(|n| hash(u8::try_from(n % 251).unwrap()))
                .collect(),
        ] {
            assert!(matches!(
                check_note_op(
                    REVISE_KIND,
                    &NoteOp::Revise {
                        note: hash(1),
                        content: good.clone(),
                        parents,
                    }
                ),
                Err(NoteError::Field(_))
            ));
        }
        // Content that breaks the document rules: an empty media type.
        let bad = DocumentRef {
            media_type: String::new(),
            content: Hash::digest(b"x"),
            size: 1,
        };
        assert!(matches!(
            check_note_op(CREATE_KIND, &NoteOp::Create { content: bad }),
            Err(NoteError::Field(_))
        ));
    }

    #[test]
    fn test_the_fold_refuses_unknown_notes_parents_and_foreign_parents() {
        // A revision of a note that was never created.
        assert_eq!(
            fold_notes(&[revise(11, 99, b"x", &[99])]),
            Err(NoteFoldError::UnknownNote {
                revision: hash(11),
                note: hash(99),
            })
        );
        // A revision naming a parent that is not a note event at all.
        assert_eq!(
            fold_notes(&[create(10, b"a"), revise(11, 10, b"x", &[77])]),
            Err(NoteFoldError::UnknownParent {
                revision: hash(11),
                parent: hash(77),
            })
        );
        // A revision naming another note's create as its parent.
        assert_eq!(
            fold_notes(&[
                create(10, b"a"),
                create(20, b"b"),
                revise(11, 10, b"x", &[20]),
            ]),
            Err(NoteFoldError::ParentOfAnotherNote {
                revision: hash(11),
                parent: hash(20),
                note: hash(20),
            })
        );
    }
}
