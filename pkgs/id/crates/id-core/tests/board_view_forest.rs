//! Board view forest projection core (slice 1, #13096).
//!
//! Oracle: **fixtures**. Fixed append-list fixtures pin the tree shape
//! from `parent_ref`, the sibling order from accept sequence, the
//! structural absence of pending entries, orphan flagging and the
//! no-drop guarantee over parent cycles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use id_core::graph::board_view::{AcceptedEntry, project_board};
use id_core::graph::social::{SocialObject, SocialObjectKind};
use id_core::graph::term::ObjectRef;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::EnvelopeId;

fn oid(n: u8) -> ObjectRef {
    ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
        HashProfile::RawBlake3_256,
        [n; 32],
    )))
}

fn topic(id: u8) -> SocialObject {
    SocialObject {
        object_id: oid(id),
        kind: SocialObjectKind::Topic,
        author: "alice".to_owned(),
        created_claim: u64::from(id),
        parent_ref: None,
        content_ref: oid(100 + id),
        schema_version: 1,
        succeeds_ref: None,
        target_ref: None,
    }
}

fn comment(id: u8, parent: Option<u8>) -> SocialObject {
    SocialObject {
        object_id: oid(id),
        kind: SocialObjectKind::Comment,
        author: "alice".to_owned(),
        created_claim: u64::from(id),
        parent_ref: parent.map(oid),
        content_ref: oid(100 + id),
        schema_version: 1,
        succeeds_ref: None,
        target_ref: None,
    }
}

const fn entry(seq: u64, object: SocialObject) -> AcceptedEntry {
    AcceptedEntry { seq, object }
}

/// Shape summary: `(seq, id-byte, orphaned, [(child summaries...)])`.
fn shape(nodes: &[id_core::graph::board_view::CommentNode]) -> Vec<(u64, u8, bool, Vec<u64>)> {
    nodes
        .iter()
        .map(|node| {
            let digest = {
                let ObjectRef::Envelope(id) = node.comment.object_id else {
                    panic!("fixture refs are envelopes");
                };
                id.content_id().digest()[0]
            };
            (
                node.seq,
                digest,
                node.orphaned,
                node.children.iter().map(|kid| kid.seq).collect(),
            )
        })
        .collect()
}

#[test]
fn nested_comments_render_in_append_order_at_each_level() {
    let board = oid(1);
    let topics = vec![entry(7, topic(10))];
    // Input deliberately out of seq order; sibling order must follow
    // the accept sequence, not input order.
    let comments = vec![
        entry(30, comment(13, Some(11))),
        entry(10, comment(11, Some(10))),
        entry(20, comment(12, Some(10))),
        entry(40, comment(14, Some(12))),
    ];
    let mut by_topic = HashMap::new();
    by_topic.insert(oid(10), comments);
    let view = project_board(&board, &topics, &by_topic);
    assert_eq!(view.board, board);
    assert_eq!(view.topics.len(), 1);
    assert_eq!(view.topics[0].seq, 7);
    let roots = shape(&view.topics[0].children);
    assert_eq!(roots.len(), 2);
    // Roots in seq order: 11 (seq 10) then 12 (seq 20).
    assert_eq!(roots[0].0, 10);
    assert_eq!(roots[0].1, 11);
    assert!(!roots[0].2);
    assert_eq!(roots[0].3, vec![30]);
    assert_eq!(roots[1].0, 20);
    assert_eq!(roots[1].1, 12);
    assert_eq!(roots[1].3, vec![40]);
    // Nested reply 13 sits under 11; 14 under 12.
    let first = &view.topics[0].children[0];
    assert_eq!(first.children.len(), 1);
    assert_eq!(first.children[0].seq, 30);
    assert!(!first.children[0].orphaned);
}

#[test]
fn topics_render_in_seq_order_and_non_topics_are_skipped() {
    let board = oid(1);
    let topics = vec![
        entry(9, topic(21)),
        entry(3, topic(20)),
        entry(5, comment(30, None)),
    ];
    let by_topic = HashMap::new();
    let view = project_board(&board, &topics, &by_topic);
    let seqs: Vec<u64> = view.topics.iter().map(|node| node.seq).collect();
    assert_eq!(seqs, vec![3, 9]);
}

#[test]
fn unresolvable_parents_attach_flagged_at_the_root() {
    let board = oid(1);
    let topics = vec![entry(1, topic(10))];
    let comments = vec![
        entry(1, comment(11, Some(10))),
        entry(2, comment(12, Some(99))),
        entry(3, comment(13, Some(13))),
        entry(4, comment(14, Some(21))),
        entry(5, comment(15, None)),
    ];
    let mut by_topic = HashMap::new();
    by_topic.insert(oid(10), comments);
    let view = project_board(&board, &topics, &by_topic);
    let roots = shape(&view.topics[0].children);
    // 11 and 15 are clean roots; 12 (absent parent), 13 (self
    // parent) and 14 (cross-topic parent 21) are flagged orphans.
    let flagged: Vec<u8> = roots
        .iter()
        .filter(|node| node.2)
        .map(|node| node.1)
        .collect();
    assert_eq!(flagged, vec![12, 13, 14]);
    assert_eq!(roots.len(), 5);
}

#[test]
fn parent_cycle_members_surface_flagged_instead_of_hanging() {
    let board = oid(1);
    let topics = vec![entry(1, topic(10))];
    let comments = vec![
        entry(1, comment(11, Some(12))),
        entry(2, comment(12, Some(11))),
    ];
    let mut by_topic = HashMap::new();
    by_topic.insert(oid(10), comments);
    let view = project_board(&board, &topics, &by_topic);
    // Neither member is reachable from a clean root, so both surface
    // via the leftover pass: one flagged root carrying the other.
    // Nothing is dropped and the builder terminates.
    let roots = &view.topics[0].children;
    assert_eq!(roots.len(), 1);
    assert!(roots[0].orphaned);
    assert_eq!(roots[0].children.len(), 1);
}

#[test]
fn duplicate_listings_keep_the_first_and_pending_has_no_shape() {
    let board = oid(1);
    let topics = vec![entry(1, topic(10))];
    // A repeated listing of one comment id keeps the first accept
    // seq; a pending submission cannot be named at all — the read
    // type [`AcceptedEntry`] has no pending state, so this fixture
    // can only list accepted entries.
    let comments = vec![
        entry(2, comment(11, Some(10))),
        entry(9, comment(11, Some(10))),
    ];
    let mut by_topic = HashMap::new();
    by_topic.insert(oid(10), comments);
    let view = project_board(&board, &topics, &by_topic);
    assert_eq!(view.topics[0].children.len(), 1);
    assert_eq!(view.topics[0].children[0].seq, 2);
}
