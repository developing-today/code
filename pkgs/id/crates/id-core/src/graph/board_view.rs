//! Board view forest projection core (slice 1, #13096; design
//! `doc/2026-11-24T06-00-00Z_design_id13096_board_view_builder.md`
//! §5 slice 1).
//!
//! Pure projection over append-list reads: given a board object, its
//! `topics` append-list and each topic's `comments` append-list are
//! accepted entries only. Pending submissions are structurally absent:
//! the read type [`AcceptedEntry`] has no pending state, so "pending
//! never appears" holds by construction, not by filtering.
//!
//! Tree shape comes from `parent_ref`, sibling order from the accept
//! sequence (#9127 server order); sequences are stable server
//! identities. Tombstone filtering (slice 2), freshness tokens (slice
//! 3) and paging budgets / cursor continuation (slice 4) are out of
//! scope here and compose later without changing this assembly.

use std::collections::{HashMap, HashSet};

use super::social::{SocialObject, SocialObjectKind};
use super::term::ObjectRef;

/// One server-accepted append-list entry: its accept sequence plus
/// the object. There is no pending variant: a pending submission can
/// never be named here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedEntry {
    /// Accept sequence: the stable identity and sibling order key.
    pub seq: u64,
    /// The accepted object.
    pub object: SocialObject,
}

/// One rendered comment with its nested replies in append-seq order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentNode {
    /// Comments-list accept sequence (stable identity).
    pub seq: u64,
    /// The comment object.
    pub comment: SocialObject,
    /// Nested replies, append-seq order.
    pub children: Vec<Self>,
    /// True when `parent_ref` was unresolvable (absent, self, another
    /// topic, or part of a parent cycle): attached at the topic root
    /// so it is never silently dropped nor grafted wrongly (§2.4).
    pub orphaned: bool,
}

/// One rendered topic with its comment forest roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicNode {
    /// Topics-list accept sequence (stable identity).
    pub seq: u64,
    /// The topic object.
    pub topic: SocialObject,
    /// Comment forest roots, append-seq order.
    pub children: Vec<CommentNode>,
}

/// The projected thread view of one board (slice 1: topics +
/// forests; no tokens, no cursor — slices 3–4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardThreadView {
    /// The board object projected.
    pub board: ObjectRef,
    /// One node per in-page topic, topics-list seq order.
    pub topics: Vec<TopicNode>,
}

/// Project the board thread view (design §2.3 steps 1–5, slice-1 subset).
///
/// `topics` arrives already paged, so only in-page topics incur comment
/// reads; the caller reads each topic's comments (accepted only) and
/// hands them over by topic id.
///
/// Entries of the wrong kind are skipped (topics keep
/// [`SocialObjectKind::Topic`], comments keep
/// [`SocialObjectKind::Comment`]); duplicate listings of one object
/// id keep the first, matching [`super::social::traverse`].
#[must_use]
pub fn project_board(
    board: &ObjectRef,
    topics: &[AcceptedEntry],
    comments_by_topic: &HashMap<ObjectRef, Vec<AcceptedEntry>>,
) -> BoardThreadView {
    let mut topic_nodes: Vec<TopicNode> = topics
        .iter()
        .filter(|entry| entry.object.kind == SocialObjectKind::Topic)
        .map(|entry| {
            let empty = Vec::new();
            let comments = comments_by_topic
                .get(&entry.object.object_id)
                .unwrap_or(&empty);
            TopicNode {
                seq: entry.seq,
                topic: entry.object.clone(),
                children: assemble_topic(&entry.object.object_id, comments),
            }
        })
        .collect();
    topic_nodes.sort_by_key(|node| node.seq);
    BoardThreadView {
        board: *board,
        topics: topic_nodes,
    }
}

/// Assemble one topic's comment forest: index by object id, attach
/// each comment under its `parent_ref` (the topic id is a root), sort
/// siblings by accept seq. Unresolvable parents — absent, self,
/// cross-topic, or cyclic — attach at the root flagged `orphaned`.
fn assemble_topic(topic_id: &ObjectRef, entries: &[AcceptedEntry]) -> Vec<CommentNode> {
    let mut seq_of: HashMap<ObjectRef, u64> = HashMap::new();
    let mut obj_of: HashMap<ObjectRef, &SocialObject> = HashMap::new();
    for entry in entries {
        if entry.object.kind != SocialObjectKind::Comment {
            continue;
        }
        if obj_of.contains_key(&entry.object.object_id) {
            continue;
        }
        seq_of.insert(entry.object.object_id, entry.seq);
        obj_of.insert(entry.object.object_id, &entry.object);
    }

    // Child adjacency for resolvable same-topic parents only; every
    // other shape stays a root (flagged unless it names the topic).
    let mut kids: HashMap<ObjectRef, Vec<ObjectRef>> = HashMap::new();
    let mut roots: Vec<(ObjectRef, bool)> = Vec::new();
    for (id, object) in &obj_of {
        match object.parent_ref {
            None => roots.push((*id, false)),
            Some(parent) if parent == *topic_id => roots.push((*id, false)),
            Some(parent) if parent == *id => roots.push((*id, true)),
            Some(parent) if obj_of.contains_key(&parent) => {
                kids.entry(parent).or_default().push(*id);
            }
            Some(_) => roots.push((*id, true)),
        }
    }
    for list in kids.values_mut() {
        list.sort_by_key(|id| seq_of[id]);
    }
    roots.sort_by_key(|(id, _)| seq_of[id]);

    // Iterative depth-first build with a per-path set: a re-entrant
    // child (parent cycle) is left unplaced here and surfaces below
    // as a flagged root instead of recursing forever.
    let mut placed: HashSet<ObjectRef> = HashSet::new();
    let mut forest: Vec<CommentNode> = Vec::new();
    for (root_id, orphaned) in &roots {
        if placed.contains(root_id) {
            continue;
        }
        forest.push(build_subtree(
            *root_id,
            *orphaned,
            &obj_of,
            &seq_of,
            &kids,
            &mut placed,
            &mut HashSet::new(),
        ));
    }
    // Cycle members (and any other leftovers) are never silently
    // dropped: they surface at the root, flagged. Slice 4 renders
    // true back-references; until then the flag keeps the loss
    // visible.
    let mut leftovers: Vec<ObjectRef> = obj_of
        .keys()
        .filter(|id| !placed.contains(id))
        .copied()
        .collect();
    leftovers.sort_by_key(|id| seq_of[id]);
    for id in leftovers {
        if placed.contains(&id) {
            continue;
        }
        forest.push(build_subtree(
            id,
            true,
            &obj_of,
            &seq_of,
            &kids,
            &mut placed,
            &mut HashSet::new(),
        ));
    }
    forest.sort_by_key(|node| node.seq);
    forest
}

/// Build one subtree iteratively: explicit stack, so adversarial
/// depth cannot overflow the call stack. A child already on the
/// current path closes a parent cycle and is left unplaced (the
/// caller surfaces it as a flagged root).
fn build_subtree(
    root_id: ObjectRef,
    orphaned: bool,
    obj_of: &HashMap<ObjectRef, &SocialObject>,
    seq_of: &HashMap<ObjectRef, u64>,
    kids: &HashMap<ObjectRef, Vec<ObjectRef>>,
    placed: &mut HashSet<ObjectRef>,
    path: &mut HashSet<ObjectRef>,
) -> CommentNode {
    struct Frame {
        id: ObjectRef,
        orphaned: bool,
        children: Vec<CommentNode>,
        next_kid: usize,
    }
    let mut current = Frame {
        id: root_id,
        orphaned,
        children: Vec::new(),
        next_kid: 0,
    };
    let mut ancestors: Vec<Frame> = Vec::new();
    path.insert(root_id);
    loop {
        let ordered_kids: &[ObjectRef] = kids.get(&current.id).map_or(&[], Vec::as_slice);
        if current.next_kid < ordered_kids.len() {
            let kid = ordered_kids[current.next_kid];
            current.next_kid += 1;
            if !(placed.contains(&kid) || path.contains(&kid)) {
                path.insert(kid);
                let descended = Frame {
                    id: kid,
                    orphaned: false,
                    children: Vec::new(),
                    next_kid: 0,
                };
                ancestors.push(std::mem::replace(&mut current, descended));
            }
            continue;
        }
        path.remove(&current.id);
        placed.insert(current.id);
        let node = CommentNode {
            seq: seq_of[&current.id],
            comment: obj_of[&current.id].clone(),
            children: current.children,
            orphaned: current.orphaned,
        };
        match ancestors.pop() {
            Some(mut parent) => {
                parent.children.push(node);
                current = parent;
            }
            None => return node,
        }
    }
}
