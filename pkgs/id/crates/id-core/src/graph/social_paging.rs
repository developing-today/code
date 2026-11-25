//! Paged, resumable social traversal.
//!
//! Issue #13095 slice S2 (remainder #14334); implementation design
//! `doc/2026-11-24T05-55-00Z_design_id13095_social-object-envelope`
//! §4 "Traversal contract"; parent design #9128 §7.
//!
//! [`super::social::traverse`] answers one bounded read and marks
//! everything past the budget as truncated. This module answers the
//! same walk one page at a time: when a node or byte budget stops a
//! page, the page carries a [`TraversalCursor`], and the next call
//! with that cursor resumes exactly where the page stopped.
//!
//! The walk is the same iterative depth-first order as `traverse`,
//! so the pages of one walk, put end to end, give the same positions
//! an unbounded `traverse` would. The cursor keeps the complete
//! visited set (design open question 2: lossless, never a Bloom
//! filter), so a resumed page neither repeats an expanded object nor
//! loses one, and a cycle across a page boundary still ends in a
//! back-reference.
//!
//! Bounds are inclusive and per page: a page may expand exactly
//! `max_visited` objects and charge exactly `max_bytes` bytes; the
//! object that would exceed either starts the next page. Depth is
//! absolute from the root, and a position past `max_depth` is folded
//! as [`TraversalMark::TruncatedDepth`], as in `traverse`; folding is
//! a display decision, not something a cursor continues. An object
//! whose own cost exceeds `max_bytes` can never fit on any page, so it
//! is a typed [`PagingError::ItemExceedsBudget`] instead of a cursor
//! that could never make progress.

use std::collections::HashSet;

use super::social::{
    SocialObject, TraversalEntry, TraversalMark, TraversalProfile, authorized_index, entry_cost,
};
use super::social_envelope::SOCIAL_SCHEMA_VERSION;
use super::term::ObjectRef;

/// Where a paged traversal resumes. Opaque: it is the walk's own
/// state, never an authorization token, and it is honoured only for
/// the root, profile and schema version it was issued under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalCursor {
    root: ObjectRef,
    profile: TraversalProfile,
    schema_version: u64,
    visited: HashSet<ObjectRef>,
    stack: Vec<(ObjectRef, usize)>,
}

impl TraversalCursor {
    /// The root the cursor's walk started from.
    #[must_use]
    pub const fn root(&self) -> &ObjectRef {
        &self.root
    }

    /// How many objects the walk has expanded so far, over all pages.
    #[must_use]
    pub fn visited_count(&self) -> usize {
        self.visited.len()
    }

    /// How many positions are still waiting to be walked.
    #[must_use]
    pub const fn pending(&self) -> usize {
        self.stack.len()
    }
}

/// One page of a traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalPage {
    /// The page's positions, in walk order. A page never holds
    /// [`TraversalMark::TruncatedBudget`]: where a budget stops the
    /// page, [`Self::cursor`] continues it instead.
    pub entries: Vec<TraversalEntry>,
    /// Pending positions whose object is no longer present or
    /// authorized (possible only on a resumed page). They are
    /// reported here and not followed.
    pub missing: Vec<ObjectRef>,
    /// Where the next page starts, or `None` when the walk is done.
    pub cursor: Option<TraversalCursor>,
}

/// Why a page could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PagingError {
    /// The root is not among the authorized objects.
    #[error("traversal root is missing or not authorized")]
    MissingRoot,
    /// The cursor was issued for another root, profile or schema
    /// version; the caller starts a fresh walk.
    #[error("traversal cursor does not belong to this walk")]
    CursorMismatch,
    /// One object costs more than a whole page's byte budget.
    #[error("one object exceeds the page byte budget")]
    ItemExceedsBudget,
}

/// Walk one page of the authorized subtree under `root`, starting
/// fresh when `cursor` is `None` and resuming from it otherwise.
///
/// # Errors
///
/// [`PagingError::MissingRoot`] when `root` is not authorized,
/// [`PagingError::CursorMismatch`] for a cursor of another walk, and
/// [`PagingError::ItemExceedsBudget`] when the next object to expand
/// could never fit on a page.
pub fn traverse_page(
    objects: &[SocialObject],
    root: &ObjectRef,
    profile: &TraversalProfile,
    authorized: impl Fn(&SocialObject) -> bool,
    cursor: Option<TraversalCursor>,
) -> Result<TraversalPage, PagingError> {
    let (by_id, children) = authorized_index(objects, authorized);
    if !by_id.contains_key(root) {
        return Err(PagingError::MissingRoot);
    }
    let (mut visited, mut stack) = match cursor {
        None => (HashSet::new(), vec![(*root, 0)]),
        Some(cursor) => {
            if cursor.root != *root
                || cursor.profile != *profile
                || cursor.schema_version != SOCIAL_SCHEMA_VERSION
            {
                return Err(PagingError::CursorMismatch);
            }
            (cursor.visited, cursor.stack)
        }
    };

    let mut entries = Vec::new();
    let mut missing = Vec::new();
    let mut expanded = 0usize;
    let mut bytes = 0usize;
    while let Some((id, depth)) = stack.pop() {
        if visited.contains(&id) {
            entries.push(TraversalEntry {
                object_id: id,
                depth,
                mark: TraversalMark::BackReference,
            });
            continue;
        }
        if depth > profile.max_depth {
            entries.push(TraversalEntry {
                object_id: id,
                depth,
                mark: TraversalMark::TruncatedDepth,
            });
            continue;
        }
        let Some(object) = by_id.get(&id) else {
            missing.push(id);
            continue;
        };
        let cost = entry_cost(object);
        if cost > profile.max_bytes {
            return Err(PagingError::ItemExceedsBudget);
        }
        if expanded >= profile.max_visited || bytes + cost > profile.max_bytes {
            // This position starts the next page.
            stack.push((id, depth));
            return Ok(TraversalPage {
                entries,
                missing,
                cursor: Some(TraversalCursor {
                    root: *root,
                    profile: *profile,
                    schema_version: SOCIAL_SCHEMA_VERSION,
                    visited,
                    stack,
                }),
            });
        }
        visited.insert(id);
        expanded += 1;
        bytes += cost;
        entries.push(TraversalEntry {
            object_id: id,
            depth,
            mark: TraversalMark::Expanded,
        });
        if let Some(kids) = children.get(&id) {
            for kid in kids.iter().rev() {
                stack.push((*kid, depth + 1));
            }
        }
    }
    Ok(TraversalPage {
        entries,
        missing,
        cursor: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::graph::social::{
        DEFAULT_MAX_BYTES, DEFAULT_MAX_DEPTH, DEFAULT_MAX_VISITED, SocialObjectKind, traverse,
    };
    use crate::id_shape::{ContentId, HashProfile};
    use crate::signed_envelope::EnvelopeId;

    fn oid(n: u32) -> ObjectRef {
        let mut digest = [0u8; 32];
        digest[..4].copy_from_slice(&n.to_be_bytes());
        ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
            HashProfile::RawBlake3_256,
            digest,
        )))
    }

    fn obj(id: u32, parent: Option<u32>, author_len: usize) -> SocialObject {
        SocialObject {
            object_id: oid(id),
            kind: if parent.is_some() {
                SocialObjectKind::Comment
            } else {
                SocialObjectKind::Topic
            },
            author: "a".repeat(author_len),
            created_claim: 0,
            parent_ref: parent.map(oid),
            content_ref: oid(1_000_000 + id),
            schema_version: SOCIAL_SCHEMA_VERSION,
            succeeds_ref: None,
            target_ref: None,
        }
    }

    /// A chain 0 <- 1 <- 2 ... of `len` objects.
    fn chain(len: u32) -> Vec<SocialObject> {
        (0..len).map(|i| obj(i, i.checked_sub(1), 1)).collect()
    }

    /// A root 0 with `n` direct children 1..=n.
    fn star(n: u32, author_len: usize) -> Vec<SocialObject> {
        let mut objects = vec![obj(0, None, author_len)];
        objects.extend((1..=n).map(|i| obj(i, Some(0), author_len)));
        objects
    }

    fn all(_: &SocialObject) -> bool {
        true
    }

    /// Every page of one walk, end to end.
    fn walk(
        objects: &[SocialObject],
        profile: &TraversalProfile,
    ) -> (Vec<TraversalPage>, Vec<TraversalEntry>) {
        let mut pages = Vec::new();
        let mut cursor = None;
        loop {
            let page = traverse_page(objects, &oid(0), profile, all, cursor).unwrap();
            cursor = page.cursor.clone();
            pages.push(page);
            if cursor.is_none() {
                break;
            }
            assert!(pages.len() < 10_000, "walk must make progress");
        }
        let entries = pages.iter().flat_map(|p| p.entries.clone()).collect();
        (pages, entries)
    }

    fn expanded(page: &TraversalPage) -> usize {
        page.entries
            .iter()
            .filter(|e| e.mark == TraversalMark::Expanded)
            .count()
    }

    #[test]
    fn test_depth_bound_is_exact() {
        let objects = chain(u32::try_from(DEFAULT_MAX_DEPTH).unwrap() + 3);
        let page =
            traverse_page(&objects, &oid(0), &TraversalProfile::default(), all, None).unwrap();
        assert!(page.cursor.is_none());
        let deepest = page
            .entries
            .iter()
            .rev()
            .find(|e| e.mark == TraversalMark::Expanded);
        assert_eq!(deepest.unwrap().depth, DEFAULT_MAX_DEPTH);
        let folded: Vec<_> = page
            .entries
            .iter()
            .filter(|e| e.mark == TraversalMark::TruncatedDepth)
            .collect();
        assert_eq!(folded.len(), 1, "only the first position past the bound");
        assert_eq!(folded[0].depth, DEFAULT_MAX_DEPTH + 1);
    }

    #[test]
    fn test_node_bound_is_exact_and_resumes() {
        let n = u32::try_from(DEFAULT_MAX_VISITED).unwrap();
        let profile = TraversalProfile::default();
        // Root plus n - 1 children fills one page exactly.
        let (pages, _) = walk(&star(n - 1, 1), &profile);
        assert_eq!(pages.len(), 1);
        assert_eq!(expanded(&pages[0]), DEFAULT_MAX_VISITED);

        // One more object starts a second page holding just it.
        let objects = star(n, 1);
        let (pages, entries) = walk(&objects, &profile);
        assert_eq!(pages.len(), 2);
        assert_eq!(expanded(&pages[0]), DEFAULT_MAX_VISITED);
        assert_eq!(expanded(&pages[1]), 1);
        assert_eq!(pages[1].entries[0].object_id, oid(n));
        let cursor = pages[0].cursor.as_ref().unwrap();
        assert_eq!(cursor.visited_count(), DEFAULT_MAX_VISITED);
        assert_eq!(cursor.pending(), 1);
        assert_eq!(cursor.root(), &oid(0));

        // The pages end to end equal one unbounded walk.
        let unbounded = TraversalProfile {
            max_visited: usize::MAX,
            ..profile
        };
        assert_eq!(
            entries,
            traverse(&objects, &oid(0), &unbounded, all).entries
        );
    }

    #[test]
    fn test_byte_bound_is_exact_and_resumes() {
        // 1,024 objects of 8,192 charged bytes fill 8 MiB exactly.
        let author_len = 8192 - 64;
        let profile = TraversalProfile::default();
        let (pages, _) = walk(&star(1023, author_len), &profile);
        assert_eq!(pages.len(), 1);
        assert_eq!(expanded(&pages[0]) * 8192, DEFAULT_MAX_BYTES);

        let (pages, entries) = walk(&star(1024, author_len), &profile);
        assert_eq!(pages.len(), 2);
        assert_eq!(expanded(&pages[0]) * 8192, DEFAULT_MAX_BYTES);
        assert_eq!(expanded(&pages[1]), 1);
        let ids: HashSet<_> = entries.iter().map(|e| e.object_id).collect();
        assert_eq!(ids.len(), 1025, "no object lost or repeated");
        assert_eq!(entries.len(), 1025);
    }

    #[test]
    fn test_oversized_item_is_a_typed_error() {
        let objects = vec![obj(0, None, DEFAULT_MAX_BYTES)];
        assert_eq!(
            traverse_page(&objects, &oid(0), &TraversalProfile::default(), all, None),
            Err(PagingError::ItemExceedsBudget)
        );
        // Exactly the budget still fits.
        let objects = vec![obj(0, None, DEFAULT_MAX_BYTES - 64)];
        let page =
            traverse_page(&objects, &oid(0), &TraversalProfile::default(), all, None).unwrap();
        assert_eq!(expanded(&page), 1);
    }

    #[test]
    fn test_cycle_across_a_page_boundary_ends_in_a_back_reference() {
        // 0 -> 1 -> 2 -> 3, and 0 also claims 3 as its parent.
        let mut objects = chain(4);
        objects[0].parent_ref = Some(oid(3));
        let profile = TraversalProfile {
            max_visited: 2,
            ..TraversalProfile::default()
        };
        let (pages, entries) = walk(&objects, &profile);
        assert_eq!(pages.len(), 2);
        let marks: Vec<_> = entries.iter().map(|e| (e.object_id, e.mark)).collect();
        assert_eq!(
            marks,
            [
                (oid(0), TraversalMark::Expanded),
                (oid(1), TraversalMark::Expanded),
                (oid(2), TraversalMark::Expanded),
                (oid(3), TraversalMark::Expanded),
                (oid(0), TraversalMark::BackReference),
            ]
        );
    }

    #[test]
    fn test_cursor_is_bound_to_its_walk() {
        let objects = star(5, 1);
        let small = TraversalProfile {
            max_visited: 2,
            ..TraversalProfile::default()
        };
        let cursor = traverse_page(&objects, &oid(0), &small, all, None)
            .unwrap()
            .cursor
            .unwrap();
        let other_profile = TraversalProfile {
            max_visited: 3,
            ..small
        };
        assert_eq!(
            traverse_page(&objects, &oid(0), &other_profile, all, Some(cursor.clone())),
            Err(PagingError::CursorMismatch)
        );
        assert_eq!(
            traverse_page(&objects, &oid(1), &small, all, Some(cursor)),
            Err(PagingError::CursorMismatch)
        );
        assert_eq!(
            traverse_page(&objects, &oid(99), &small, all, None),
            Err(PagingError::MissingRoot)
        );
    }

    #[test]
    fn test_object_withdrawn_between_pages_is_reported_not_followed() {
        let objects = star(3, 1);
        let small = TraversalProfile {
            max_visited: 2,
            ..TraversalProfile::default()
        };
        let first = traverse_page(&objects, &oid(0), &small, all, None).unwrap();
        // Child 2 stops being authorized before the next page.
        let page = traverse_page(
            &objects,
            &oid(0),
            &small,
            |o| o.object_id != oid(2),
            first.cursor,
        )
        .unwrap();
        assert_eq!(page.missing, [oid(2)]);
        assert_eq!(
            page.entries.iter().map(|e| e.object_id).collect::<Vec<_>>(),
            [oid(3)]
        );
        assert!(page.cursor.is_none());
    }
}
