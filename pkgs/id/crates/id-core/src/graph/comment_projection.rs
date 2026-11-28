//! Carried ancestor comments for the V3 social projection (slice
//! A-GRAPH-9129-02, #10269; design part A §8, statements S5.44,
//! S5.45).
//!
//! Comments attach to the exact version they were authored against;
//! they never change identity, target or bytes when the content moves
//! on. This projection carries eligible ancestor comments forward onto
//! a selected descendant: each appears once, with its immutable
//! original target kept as its origin — preserved transitively, so a
//! comment authored on v1 still names v1 when it is shown on v3 — and
//! an old-version marker whenever that target is not the selected
//! version itself.
//!
//! Access, moderation and deletion filters still apply to carried
//! comments, and the projection contains only comments in the
//! selected version's own ancestry: sibling branches never leak.

use std::collections::{HashMap, HashSet, VecDeque};

use super::social::{SocialObject, SocialObjectKind};
use super::term::ObjectRef;
use super::version::{VersionDag, ref_order_key};

/// Projection-side filters over comment identities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentFilters {
    /// Comments hidden by moderation in this projection.
    pub hidden: HashSet<ObjectRef>,
    /// Comments deleted in this projection.
    pub deleted: HashSet<ObjectRef>,
}

/// One comment in a carried-forward view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarriedComment {
    /// The comment's immutable identity.
    pub comment_id: ObjectRef,
    /// The version the comment was authored against; never rewritten
    /// by the carry.
    pub original_target: ObjectRef,
    /// The comment's origin, preserved transitively: always the
    /// original target, however many versions the carry spans.
    pub origin_version: ObjectRef,
    /// True when the comment was authored against an older version
    /// than the selected one.
    pub old_version: bool,
    /// `succeeds` steps from the selected version back to the
    /// comment's original target (0 for comments on the selected
    /// version itself).
    pub distance: usize,
}

/// The default descendant view of `selected`: its own comments plus
/// every eligible ancestor comment carried forward.
///
/// Eligibility is decided against the selected version's ancestry in
/// `dag` (breadth-first over the reverse `succeeds` index, which also
/// fixes each target's shortest distance). A comment qualifies when
/// it is a [`SocialObjectKind::Comment`] whose parent is the selected
/// version or one of its ancestors, when neither filter names it, and
/// when the `authorized` access predicate accepts it. Duplicate
/// listings of one comment ID yield one entry. The result is ordered
/// by distance, then by the deterministic reference order, so the
/// selected version's own comments come first and nearer ancestors
/// before farther ones.
#[must_use]
pub fn carried_comments(
    dag: &VersionDag,
    comments: &[SocialObject],
    selected: &ObjectRef,
    filters: &CommentFilters,
    authorized: impl Fn(&SocialObject) -> bool,
) -> Vec<CarriedComment> {
    let mut distance: HashMap<ObjectRef, usize> = HashMap::new();
    let mut queue: VecDeque<ObjectRef> = VecDeque::new();
    distance.insert(*selected, 0);
    queue.push_back(*selected);
    while let Some(at) = queue.pop_front() {
        let steps = distance[&at];
        for predecessor in dag.predecessors_of(&at) {
            if !distance.contains_key(&predecessor) {
                distance.insert(predecessor, steps + 1);
                queue.push_back(predecessor);
            }
        }
    }

    let mut seen: HashSet<ObjectRef> = HashSet::new();
    let mut out: Vec<CarriedComment> = Vec::new();
    for comment in comments {
        if comment.kind != SocialObjectKind::Comment {
            continue;
        }
        let Some(target) = comment.parent_ref else {
            continue;
        };
        let Some(&steps) = distance.get(&target) else {
            continue;
        };
        if filters.hidden.contains(&comment.object_id)
            || filters.deleted.contains(&comment.object_id)
        {
            continue;
        }
        if !authorized(comment) {
            continue;
        }
        if !seen.insert(comment.object_id) {
            continue;
        }
        out.push(CarriedComment {
            comment_id: comment.object_id,
            original_target: target,
            origin_version: target,
            old_version: steps > 0,
            distance: steps,
        });
    }
    out.sort_by(|a, b| {
        (a.distance, ref_order_key(&a.comment_id)).cmp(&(b.distance, ref_order_key(&b.comment_id)))
    });
    out
}
