//! Note links as signed events: structured references between notes (Phase 2 of the
//! #2048 Zettelkasten design, #3816), and the per-store projection that derives
//! backlinks from them.
//!
//! A `note.link` event asserts one reference: it names the source note by its stable
//! ID, the source revision that asserts the reference, the target note by its stable
//! ID, the authored surface (the text as the author wrote it), and optionally the
//! exact target revision the reference pins. Links are resolved by ID, never by name
//! or surface, so two notes that share a name stay distinct targets and renaming a
//! target never moves a link.
//!
//! The projection is derived, never stored: [`project_links`] folds the applied link
//! events against the folded notes ([`crate::notes::fold_notes`]) and keeps exactly
//! the links asserted by a current head revision of their source note. Backlinks are
//! the inverse of that same edge set; there is no separately mutable backlink state
//! that could disagree with the links. Every list the projection returns is sorted,
//! so the result is the same whatever order the events arrived in, and concurrent
//! heads contribute their links side by side: no head is silently chosen over
//! another.
//!
//! Access filtering is a query-time view, not a second projection. The caller names
//! the notes a viewer may access, and a filtered query drops every edge that would
//! expose an inaccessible note: an edge to an inaccessible target is dropped whole
//! (its existence is target metadata), and a backlink from an inaccessible source is
//! dropped with its surface (the surface is the source's authored content). Nothing
//! about an inaccessible note — not its content, not its existence — crosses.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::notes::NoteRecord;
use crate::waiting::AuthorKey;

/// The kind of a `note.link` event, as the header's `kind` names it.
pub const LINK_KIND: &str = "note.link";

/// The most bytes a link's authored surface may have. The surface is text inside the
/// header, and the header decoder declares text at most 512 bytes long.
pub const MAX_SURFACE_BYTES: usize = 512;

/// What a `note.link` event does. The kind of the event says which form the operation
/// takes; today there is one form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteLinkOp {
    /// Asserts a reference from one note to another.
    Assert {
        /// The source note's stable ID: the ID of its `note.create` event.
        source: Hash,
        /// The revision of the source note that asserts this reference. The link is
        /// part of the projection only while this revision is a head of the source.
        revision: Hash,
        /// The target note's stable ID: the ID of its `note.create` event.
        target: Hash,
        /// The reference as authored: the surface text the author wrote. It is kept
        /// verbatim and never used to resolve the link.
        surface: String,
        /// The exact target revision the reference pins, if it pins one. A pinned
        /// reference keeps naming that revision as the target's heads move on.
        pin: Option<Hash>,
    },
}

/// Why a note link operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NoteLinkError {
    /// The operation is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong note link operation")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
    /// A field breaks a limit or a content rule.
    #[error("{0}")]
    Field(String),
}

/// Whether `kind` is a note link kind, so its event carries a note link operation.
#[must_use]
pub fn is_note_link_kind(kind: &str) -> bool {
    kind == LINK_KIND
}

/// Checks a note link operation against the kind of the event that carries it.
///
/// # Errors
///
/// Returns [`NoteLinkError::KindMismatch`] when the operation is not the one `kind`
/// names. Returns [`NoteLinkError::Field`] when the authored surface is empty, is
/// not NFC, or is longer than [`MAX_SURFACE_BYTES`]. Whether the named notes and
/// revisions exist — and whether a pin names a revision of the target — is the
/// projection's check, not this one: it needs the store's folded notes.
pub fn check_note_link_op(kind: &str, op: &NoteLinkOp) -> Result<(), NoteLinkError> {
    match (kind, op) {
        (LINK_KIND, NoteLinkOp::Assert { surface, .. }) => {
            if surface.is_empty() {
                return Err(NoteLinkError::Field(
                    "a link's authored surface is not empty".to_owned(),
                ));
            }
            if surface.len() > MAX_SURFACE_BYTES {
                return Err(NoteLinkError::Field(format!(
                    "a link's authored surface is {} bytes, it may be at most {MAX_SURFACE_BYTES}",
                    surface.len()
                )));
            }
            if !is_nfc(surface) {
                return Err(NoteLinkError::Field(
                    "a link's authored surface is NFC".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Err(NoteLinkError::KindMismatch {
            kind: kind.to_owned(),
        }),
    }
}

/// An applied note link event, as the projection reads it: its ID, its signer, and
/// its operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedLink {
    /// The event's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// What the event does.
    pub op: NoteLinkOp,
}

/// One projected link: a reference asserted by a head revision of its source note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRecord {
    /// The ID of the `note.link` event that asserted this link.
    pub id: Hash,
    /// The key that signed the link event.
    pub author: AuthorKey,
    /// The source note's stable ID.
    pub source: Hash,
    /// The source revision that asserts the link. Always a head of the source note
    /// in the projection this record came from.
    pub revision: Hash,
    /// The target note's stable ID.
    pub target: Hash,
    /// The reference as authored, verbatim.
    pub surface: String,
    /// The target revision the link pins, if it pins one.
    pub pin: Option<Hash>,
}

/// Why a set of applied link events could not be projected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkProjectionError {
    /// Two applied link events share one ID.
    #[error("two applied note link events share the ID {0}")]
    DuplicateEvent(Hash),
    /// A link names a source note with no applied create event.
    #[error("link {link} names source note {note}, which has no create event")]
    UnknownSource {
        /// The link that names the missing note.
        link: Hash,
        /// The note it names.
        note: Hash,
    },
    /// A link's asserting revision is not a revision of its source note.
    #[error("link {link} is asserted by {revision}, which is not a revision of note {note}")]
    RevisionNotOfSource {
        /// The link that names the revision.
        link: Hash,
        /// The revision it names.
        revision: Hash,
        /// The source note the revision was expected to belong to.
        note: Hash,
    },
    /// A link names a target note with no applied create event.
    #[error("link {link} names target note {target}, which has no create event")]
    UnknownTarget {
        /// The link that names the missing note.
        link: Hash,
        /// The note it names.
        target: Hash,
    },
    /// A link pins a revision that is not a revision of its target note.
    #[error("link {link} pins {pin}, which is not a revision of note {target}")]
    PinNotOfTarget {
        /// The link that carries the pin.
        link: Hash,
        /// The revision it pins.
        pin: Hash,
        /// The target note the pin was expected to belong to.
        target: Hash,
    },
}

/// The per-store link projection: the links asserted by the current head revisions
/// of their source notes, in a deterministic order. Backlinks are read off the same
/// edge set, never stored separately.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LinkProjection {
    edges: Vec<LinkRecord>,
}

impl LinkProjection {
    /// Every projected link, sorted by source, target, surface, and link ID.
    #[must_use]
    pub fn edges(&self) -> &[LinkRecord] {
        &self.edges
    }

    /// The projected links asserted by `source`, in projection order.
    #[must_use]
    pub fn links_from(&self, source: Hash) -> Vec<&LinkRecord> {
        self.edges
            .iter()
            .filter(|edge| edge.source == source)
            .collect()
    }

    /// The projected backlinks to `target`: every link whose target is `target`,
    /// in projection order. This is the inverse of [`LinkProjection::links_from`]
    /// over the same edges, derived on each call.
    #[must_use]
    pub fn backlinks_to(&self, target: Hash) -> Vec<&LinkRecord> {
        self.edges
            .iter()
            .filter(|edge| edge.target == target)
            .collect()
    }

    /// [`LinkProjection::links_from`] for one viewer: `visible` names every note
    /// the viewer may access. A source the viewer cannot access has no visible
    /// links, and an edge to an inaccessible target is dropped whole, so the
    /// target's existence does not leak through the link.
    #[must_use]
    pub fn links_from_visible(
        &self,
        source: Hash,
        visible: &HashSet<Hash>,
    ) -> Vec<&LinkRecord> {
        if !visible.contains(&source) {
            return Vec::new();
        }
        self.links_from(source)
            .into_iter()
            .filter(|edge| visible.contains(&edge.target))
            .collect()
    }

    /// [`LinkProjection::backlinks_to`] for one viewer: `visible` names every note
    /// the viewer may access. A target the viewer cannot access has no visible
    /// backlinks, and a backlink from an inaccessible source is dropped with its
    /// authored surface, which is the source's content.
    #[must_use]
    pub fn backlinks_to_visible(
        &self,
        target: Hash,
        visible: &HashSet<Hash>,
    ) -> Vec<&LinkRecord> {
        if !visible.contains(&target) {
            return Vec::new();
        }
        self.backlinks_to(target)
            .into_iter()
            .filter(|edge| visible.contains(&edge.source))
            .collect()
    }
}

/// Projects the applied link events against the folded notes.
///
/// A link is projected only while the revision that asserts it is a head of its
/// source note: revising a note restates its references, and a superseded
/// revision's links leave the projection with it. Links asserted by concurrent
/// heads are all projected, each naming the head that asserted it. The result is
/// sorted by (source, target, surface, link ID), so it does not depend on the
/// order `notes` or `links` are given in.
///
/// # Errors
///
/// Returns [`LinkProjectionError`] when two link events share an ID, a link names
/// a source or target note with no create in `notes`, a link's asserting revision
/// is not a revision of its source note, or a link pins a revision that is not a
/// revision of its target note.
pub fn project_links(
    notes: &[NoteRecord],
    links: &[AppliedLink],
) -> Result<LinkProjection, LinkProjectionError> {
    let mut by_id: HashSet<Hash> = HashSet::with_capacity(links.len());
    for link in links {
        if !by_id.insert(link.id) {
            return Err(LinkProjectionError::DuplicateEvent(link.id));
        }
    }
    let by_note: HashMap<Hash, &NoteRecord> = notes.iter().map(|note| (note.id, note)).collect();
    let mut edges: Vec<LinkRecord> = Vec::new();
    for link in links {
        let NoteLinkOp::Assert {
            source,
            revision,
            target,
            surface,
            pin,
        } = &link.op;
        let source_note = by_note.get(source).ok_or(LinkProjectionError::UnknownSource {
            link: link.id,
            note: *source,
        })?;
        if source_note.revision(*revision).is_none() {
            return Err(LinkProjectionError::RevisionNotOfSource {
                link: link.id,
                revision: *revision,
                note: *source,
            });
        }
        let target_note = by_note.get(target).ok_or(LinkProjectionError::UnknownTarget {
            link: link.id,
            target: *target,
        })?;
        if let Some(pin) = pin {
            if target_note.revision(*pin).is_none() {
                return Err(LinkProjectionError::PinNotOfTarget {
                    link: link.id,
                    pin: *pin,
                    target: *target,
                });
            }
        }
        if !source_note.heads.contains(revision) {
            continue;
        }
        edges.push(LinkRecord {
            id: link.id,
            author: link.author,
            source: *source,
            revision: *revision,
            target: *target,
            surface: surface.clone(),
            pin: *pin,
        });
    }
    edges.sort_by(|a, b| {
        (*a.source.as_bytes(), *a.target.as_bytes(), &a.surface, *a.id.as_bytes()).cmp(&(
            *b.source.as_bytes(),
            *b.target.as_bytes(),
            &b.surface,
            *b.id.as_bytes(),
        ))
    });
    Ok(LinkProjection { edges })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::documents::DocumentRef;
    use crate::notes::{AppliedNote, NoteOp, fold_notes};

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

    fn link(
        id: u8,
        source: u8,
        revision: u8,
        target: u8,
        surface: &str,
        pin: Option<u8>,
    ) -> AppliedLink {
        AppliedLink {
            id: hash(id),
            author: author(1),
            op: NoteLinkOp::Assert {
                source: hash(source),
                revision: hash(revision),
                target: hash(target),
                surface: surface.to_owned(),
                pin: pin.map(hash),
            },
        }
    }

    fn notes(events: &[AppliedNote]) -> Vec<NoteRecord> {
        fold_notes(events).unwrap()
    }

    // Notes 10 (source), 20 and 30 (targets), each a single create revision.
    fn three_notes() -> Vec<NoteRecord> {
        notes(&[
            create(10, b"source"),
            create(20, b"target one"),
            create(30, b"target two"),
        ])
    }

    #[test]
    fn test_a_link_targets_the_stable_id_and_keeps_its_surface_and_pin() {
        let folded = notes(&[
            create(10, b"source"),
            create(20, b"first"),
            revise(21, 20, b"second", &[20]),
        ]);
        // The pin names revision 20 even though the target's head is 21.
        let projection = project_links(&folded, &[link(40, 10, 10, 20, "[[Target]]", Some(20))])
            .unwrap();
        assert_eq!(projection.edges().len(), 1);
        let edge = &projection.edges()[0];
        assert_eq!(edge.target, hash(20));
        assert_eq!(edge.surface, "[[Target]]");
        assert_eq!(edge.pin, Some(hash(20)));
        assert_eq!(edge.revision, hash(10));
    }

    #[test]
    fn test_check_rejects_a_bad_surface_and_a_wrong_kind() {
        let ok = NoteLinkOp::Assert {
            source: hash(10),
            revision: hash(10),
            target: hash(20),
            surface: "fine".to_owned(),
            pin: None,
        };
        assert!(check_note_link_op(LINK_KIND, &ok).is_ok());
        assert!(matches!(
            check_note_link_op("note.create", &ok),
            Err(NoteLinkError::KindMismatch { .. })
        ));
        for surface in ["", "e\u{301}", &"x".repeat(MAX_SURFACE_BYTES + 1)] {
            let NoteLinkOp::Assert {
                source,
                revision,
                target,
                pin,
                ..
            } = ok.clone();
            let bad = NoteLinkOp::Assert {
                source,
                revision,
                target,
                surface: surface.to_owned(),
                pin,
            };
            assert!(
                matches!(
                    check_note_link_op(LINK_KIND, &bad),
                    Err(NoteLinkError::Field(_))
                ),
                "surface {surface:?} must be refused"
            );
        }
    }

    #[test]
    fn test_backlinks_are_the_inverse_of_the_projected_links() {
        let folded = three_notes();
        let links = vec![
            link(40, 10, 10, 20, "one", None),
            link(41, 30, 30, 20, "uno", None),
            link(42, 10, 10, 30, "two", None),
        ];
        let projection = project_links(&folded, &links).unwrap();
        let from_10: Vec<Hash> = projection
            .links_from(hash(10))
            .iter()
            .map(|edge| edge.target)
            .collect();
        assert_eq!(from_10, vec![hash(20), hash(30)]);
        let to_20: Vec<Hash> = projection
            .backlinks_to(hash(20))
            .iter()
            .map(|edge| edge.source)
            .collect();
        assert_eq!(to_20, vec![hash(10), hash(30)]);
        assert!(projection.backlinks_to(hash(10)).is_empty());
    }

    #[test]
    fn test_a_link_asserted_by_a_superseded_revision_leaves_the_projection() {
        // The source is revised; the new head restates only the second link.
        let folded = notes(&[
            create(10, b"v1"),
            revise(11, 10, b"v2", &[10]),
            create(20, b"a"),
            create(30, b"b"),
        ]);
        let links = vec![
            link(40, 10, 10, 20, "dropped", None),
            link(41, 10, 11, 30, "kept", None),
        ];
        let projection = project_links(&folded, &links).unwrap();
        assert_eq!(projection.edges().len(), 1);
        assert_eq!(projection.edges()[0].id, hash(41));
        assert!(projection.backlinks_to(hash(20)).is_empty());
    }

    #[test]
    fn test_concurrent_heads_contribute_their_links_side_by_side() {
        // Revisions 11 and 12 are concurrent heads of note 10.
        let folded = notes(&[
            create(10, b"base"),
            revise(11, 10, b"left", &[10]),
            revise(12, 10, b"right", &[10]),
            create(20, b"a"),
            create(30, b"b"),
        ]);
        let links = vec![
            link(40, 10, 11, 20, "from left", None),
            link(41, 10, 12, 30, "from right", None),
        ];
        let projection = project_links(&folded, &links).unwrap();
        assert_eq!(projection.edges().len(), 2);
        let heads: HashSet<Hash> = projection
            .edges()
            .iter()
            .map(|edge| edge.revision)
            .collect();
        assert_eq!(heads, HashSet::from([hash(11), hash(12)]));
    }

    #[test]
    fn test_a_pin_must_name_a_revision_of_the_target() {
        let folded = three_notes();
        // Revision 30 belongs to note 30, not to the target note 20.
        let bad = project_links(&folded, &[link(40, 10, 10, 20, "x", Some(30))]);
        assert!(matches!(
            bad,
            Err(LinkProjectionError::PinNotOfTarget { .. })
        ));
    }

    #[test]
    fn test_duplicate_surfaces_and_a_target_rename_do_not_move_links() {
        // Note 20 is renamed by a later revision; two links share one surface.
        let folded = notes(&[
            create(10, b"source"),
            create(20, b"Original name"),
            revise(21, 20, b"Renamed", &[20]),
            create(30, b"Original name too"),
        ]);
        let links = vec![
            link(40, 10, 10, 20, "Original name", None),
            link(41, 10, 10, 30, "Original name", None),
        ];
        let projection = project_links(&folded, &links).unwrap();
        assert_eq!(projection.edges().len(), 2);
        let targets: Vec<Hash> = projection
            .links_from(hash(10))
            .iter()
            .map(|edge| edge.target)
            .collect();
        assert_eq!(targets, vec![hash(20), hash(30)]);
        assert!(
            projection
                .edges()
                .iter()
                .all(|edge| edge.surface == "Original name")
        );
    }

    #[test]
    fn test_the_projection_does_not_depend_on_arrival_order() {
        let folded = three_notes();
        let links = vec![
            link(40, 10, 10, 20, "one", None),
            link(41, 30, 30, 20, "uno", None),
            link(42, 10, 10, 30, "two", None),
        ];
        let forward = project_links(&folded, &links).unwrap();
        let mut reversed_links = links.clone();
        reversed_links.reverse();
        let mut reversed_notes = folded.clone();
        reversed_notes.reverse();
        let backward = project_links(&reversed_notes, &reversed_links).unwrap();
        assert_eq!(forward, backward);
    }

    #[test]
    fn test_filtered_queries_expose_nothing_about_inaccessible_notes() {
        let folded = three_notes();
        let links = vec![
            link(40, 10, 10, 20, "one", None),
            link(41, 30, 30, 20, "secret source", None),
        ];
        let projection = project_links(&folded, &links).unwrap();
        // A viewer who may access notes 10 and 20 sees only the 10 -> 20 edge as a
        // backlink of 20: the edge from inaccessible note 30, surface included, is
        // dropped whole.
        let visible: HashSet<Hash> = HashSet::from([hash(10), hash(20)]);
        let backlinks = projection.backlinks_to_visible(hash(20), &visible);
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].source, hash(10));
        // A viewer who cannot access target 20 learns nothing from note 10's links:
        // the edge is dropped, not returned with its target redacted.
        let only_source: HashSet<Hash> = HashSet::from([hash(10)]);
        assert!(projection
            .links_from_visible(hash(10), &only_source)
            .is_empty());
        // And an inaccessible target has no visible backlinks at all.
        assert!(projection
            .backlinks_to_visible(hash(20), &only_source)
            .is_empty());
    }

    #[test]
    fn test_projection_rejects_structurally_bad_links() {
        let folded = three_notes();
        // Unknown source note.
        assert!(matches!(
            project_links(&folded, &[link(40, 99, 99, 20, "x", None)]),
            Err(LinkProjectionError::UnknownSource { .. })
        ));
        // The asserting revision (20) is not a revision of the source note (10).
        assert!(matches!(
            project_links(&folded, &[link(40, 10, 20, 30, "x", None)]),
            Err(LinkProjectionError::RevisionNotOfSource { .. })
        ));
        // Unknown target note.
        assert!(matches!(
            project_links(&folded, &[link(40, 10, 10, 99, "x", None)]),
            Err(LinkProjectionError::UnknownTarget { .. })
        ));
        // Two link events with one ID.
        let dup = vec![
            link(40, 10, 10, 20, "x", None),
            link(40, 10, 10, 30, "y", None),
        ];
        assert!(matches!(
            project_links(&folded, &dup),
            Err(LinkProjectionError::DuplicateEvent(_))
        ));
    }
}
