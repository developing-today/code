//! Social object graph traversal and feed ledger (slice
//! A-GRAPH-9128-01, issue #10252; design part A §7, statements
//! S5.30–S5.36, S5.40).
//!
//! Oracle: **model**. The implementation's iterative traversal is
//! checked against an independent recursive reference model over the
//! same fixtures (nested, cyclic, diamond, budget-limited), and the
//! feed ledger is checked against the design's pinned-snapshot
//! paging rules, including the two-authorities-at-`(e, 1)` collision.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};

use id_core::graph::social::{
    traverse, FeedError, FeedLedger, FeedScope, SocialObject, SocialObjectKind, SourcePosition,
    Traversal, TraversalMark, TraversalProfile,
};
use id_core::graph::term::ObjectRef;
use id_core::graph::version::ref_order_key;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::EnvelopeId;

fn oid(n: u8) -> ObjectRef {
    ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
        HashProfile::RawBlake3_256,
        [n; 32],
    )))
}

fn obj(id: u8, kind: SocialObjectKind, parent: Option<u8>) -> SocialObject {
    SocialObject {
        object_id: oid(id),
        kind,
        author: "alice".to_owned(),
        created_claim: u64::from(id),
        parent_ref: parent.map(oid),
        content_ref: oid(100 + id),
        schema_version: 1,
        succeeds_ref: None,
    }
}

fn marks(traversal: &Traversal) -> Vec<(u8, usize, TraversalMark)> {
    traversal
        .entries
        .iter()
        .map(|entry| {
            let ObjectRef::Envelope(id) = entry.object_id else {
                panic!("fixture refs are envelopes");
            };
            (id.content_id().digest()[0], entry.depth, entry.mark)
        })
        .collect()
}

/// Independent recursive reference model for the documented
/// traversal rules: authorized objects only, children in reference
/// order, back-reference on revisit, explicit depth/budget marks.
fn model_traverse(
    objects: &[SocialObject],
    root: &ObjectRef,
    profile: &TraversalProfile,
    authorized: impl Fn(&SocialObject) -> bool,
) -> Vec<(ObjectRef, usize, TraversalMark)> {
    let mut children: HashMap<ObjectRef, Vec<ObjectRef>> = HashMap::new();
    let mut known: HashSet<ObjectRef> = HashSet::new();
    for object in objects {
        if !authorized(object) {
            continue;
        }
        if !known.insert(object.object_id) {
            continue;
        }
        if let Some(parent) = object.parent_ref {
            children.entry(parent).or_default().push(object.object_id);
        }
    }
    for list in children.values_mut() {
        list.sort_by_key(|id| ref_order_key(id));
        list.dedup();
    }

    fn walk(
        id: ObjectRef,
        depth: usize,
        children: &HashMap<ObjectRef, Vec<ObjectRef>>,
        profile: &TraversalProfile,
        visited: &mut HashSet<ObjectRef>,
        bytes: &mut usize,
        out: &mut Vec<(ObjectRef, usize, TraversalMark)>,
    ) {
        if visited.contains(&id) {
            out.push((id, depth, TraversalMark::BackReference));
            return;
        }
        if depth > profile.max_depth {
            out.push((id, depth, TraversalMark::TruncatedDepth));
            return;
        }
        // Fixture authors are all "alice" (5 bytes).
        let cost = 32 + 32 + 5;
        if visited.len() >= profile.max_visited || *bytes + cost > profile.max_bytes {
            out.push((id, depth, TraversalMark::TruncatedBudget));
            return;
        }
        visited.insert(id);
        *bytes += cost;
        out.push((id, depth, TraversalMark::Expanded));
        if let Some(kids) = children.get(&id) {
            for kid in kids {
                walk(*kid, depth + 1, children, profile, visited, bytes, out);
            }
        }
    }

    let mut out = Vec::new();
    if known.contains(root) {
        let mut visited = HashSet::new();
        let mut bytes = 0;
        walk(
            *root,
            0,
            &children,
            profile,
            &mut visited,
            &mut bytes,
            &mut out,
        );
    }
    out
}

fn assert_matches_model(objects: &[SocialObject], root: u8, profile: &TraversalProfile) {
    let got = traverse(objects, &oid(root), profile, |_| true);
    let expected: Vec<(u8, usize, TraversalMark)> =
        model_traverse(objects, &oid(root), profile, |_| true)
            .into_iter()
            .map(|(id, depth, mark)| {
                let ObjectRef::Envelope(id) = id else {
                    panic!("fixture refs are envelopes");
                };
                (id.content_id().digest()[0], depth, mark)
            })
            .collect();
    assert_eq!(marks(&got), expected);
}

#[test]
fn nested_traversal_is_preorder_and_complete() {
    let objects = vec![
        obj(1, SocialObjectKind::Topic, None),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(3, SocialObjectKind::Comment, Some(1)),
        obj(4, SocialObjectKind::Comment, Some(2)),
        obj(5, SocialObjectKind::Reaction, Some(2)),
    ];
    let traversal = traverse(&objects, &oid(1), &TraversalProfile::default(), |_| true);
    assert!(traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (4, 2, TraversalMark::Expanded),
            (5, 2, TraversalMark::Expanded),
            (3, 1, TraversalMark::Expanded),
        ]
    );
    assert_matches_model(&objects, 1, &TraversalProfile::default());
}

#[test]
fn cycles_end_in_back_references() {
    let objects = vec![
        obj(1, SocialObjectKind::Topic, Some(3)),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(3, SocialObjectKind::Comment, Some(2)),
    ];
    let traversal = traverse(&objects, &oid(1), &TraversalProfile::default(), |_| true);
    assert!(traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (3, 2, TraversalMark::Expanded),
            (1, 3, TraversalMark::BackReference),
        ]
    );
    assert_matches_model(&objects, 1, &TraversalProfile::default());
}

#[test]
fn repeated_listings_of_one_object_expand_once() {
    // Objects are immutable: a second listing of object 4 is the
    // same object, not another relation, and must not produce a
    // back-reference.
    let objects = vec![
        obj(1, SocialObjectKind::Topic, None),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(4, SocialObjectKind::Comment, Some(2)),
        obj(4, SocialObjectKind::Comment, Some(2)),
    ];
    let traversal = traverse(&objects, &oid(1), &TraversalProfile::default(), |_| true);
    assert!(traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (4, 2, TraversalMark::Expanded),
        ]
    );
    assert_matches_model(&objects, 1, &TraversalProfile::default());
}

#[test]
fn depth_budget_truncates_explicitly() {
    let objects: Vec<SocialObject> = (1..=5u8)
        .map(|n| {
            obj(
                n,
                SocialObjectKind::Comment,
                if n == 1 { None } else { Some(n - 1) },
            )
        })
        .collect();
    let profile = TraversalProfile {
        max_depth: 2,
        ..TraversalProfile::default()
    };
    let traversal = traverse(&objects, &oid(1), &profile, |_| true);
    assert!(!traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (3, 2, TraversalMark::Expanded),
            (4, 3, TraversalMark::TruncatedDepth),
        ]
    );
    assert_matches_model(&objects, 1, &profile);
}

#[test]
fn node_budget_truncates_explicitly() {
    let objects = vec![
        obj(1, SocialObjectKind::Topic, None),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(3, SocialObjectKind::Comment, Some(1)),
        obj(4, SocialObjectKind::Comment, Some(1)),
        obj(5, SocialObjectKind::Comment, Some(1)),
    ];
    let profile = TraversalProfile {
        max_visited: 3,
        ..TraversalProfile::default()
    };
    let traversal = traverse(&objects, &oid(1), &profile, |_| true);
    assert!(!traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (3, 1, TraversalMark::Expanded),
            (4, 1, TraversalMark::TruncatedBudget),
            (5, 1, TraversalMark::TruncatedBudget),
        ]
    );
    assert_matches_model(&objects, 1, &profile);
}

#[test]
fn byte_budget_truncates_explicitly() {
    let objects = vec![
        obj(1, SocialObjectKind::Topic, None),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(3, SocialObjectKind::Comment, Some(1)),
    ];
    // Every fixture object charges 32 + 32 + len("alice") = 69 bytes.
    let profile = TraversalProfile {
        max_bytes: 2 * 69,
        ..TraversalProfile::default()
    };
    let traversal = traverse(&objects, &oid(1), &profile, |_| true);
    assert!(!traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (2, 1, TraversalMark::Expanded),
            (3, 1, TraversalMark::TruncatedBudget),
        ]
    );
    assert_matches_model(&objects, 1, &profile);
}

#[test]
fn unauthorized_objects_are_neither_returned_nor_traversed() {
    let objects = vec![
        obj(1, SocialObjectKind::Topic, None),
        obj(2, SocialObjectKind::Comment, Some(1)),
        obj(3, SocialObjectKind::Comment, Some(2)),
        obj(4, SocialObjectKind::Comment, Some(1)),
    ];
    let traversal = traverse(&objects, &oid(1), &TraversalProfile::default(), |object| {
        object.object_id != oid(2)
    });
    assert!(traversal.complete);
    assert_eq!(
        marks(&traversal),
        vec![
            (1, 0, TraversalMark::Expanded),
            (4, 1, TraversalMark::Expanded),
        ]
    );
}

#[test]
fn missing_root_is_an_empty_complete_traversal() {
    let objects = vec![obj(1, SocialObjectKind::Topic, None)];
    let traversal = traverse(&objects, &oid(9), &TraversalProfile::default(), |_| true);
    assert!(traversal.complete);
    assert!(traversal.entries.is_empty());
}

fn scope() -> FeedScope {
    FeedScope {
        server_id: "srv-main".to_owned(),
        tenant_id: "tenant-a".to_owned(),
        query_fingerprint: [7; 32],
    }
}

fn source(authority: &str, epoch: u64, sequence: u64, object: u8) -> SourcePosition {
    SourcePosition {
        source_authority: authority.to_owned(),
        source_epoch: epoch,
        source_sequence: sequence,
        source_object_id: oid(object),
    }
}

#[test]
fn colliding_source_positions_get_distinct_projection_sequences() {
    let mut ledger = FeedLedger::new(scope());
    // The design's example: two source attributes may each sit at
    // source epoch/sequence (e, 1); neither entry is omitted or
    // conflated.
    let first = ledger
        .append("a1".to_owned(), oid(10), source("srv-a", 1, 1, 10))
        .unwrap();
    let second = ledger
        .append("b1".to_owned(), oid(11), source("srv-b", 1, 1, 11))
        .unwrap();
    assert_eq!(first.last_projection_seq, 1);
    assert_eq!(second.last_projection_seq, 2);
    assert_eq!(
        ledger
            .append("a1".to_owned(), oid(12), source("srv-a", 1, 2, 12))
            .unwrap_err()
            .class(),
        "duplicate_entry"
    );
}

#[test]
fn since_and_before_page_the_pinned_snapshot() {
    let mut ledger = FeedLedger::new(scope());
    ledger
        .append("a1".to_owned(), oid(10), source("srv-a", 1, 1, 10))
        .unwrap();
    ledger
        .append("b1".to_owned(), oid(11), source("srv-b", 1, 1, 11))
        .unwrap();
    let cursor = ledger.cursor();
    // Appended after the cursor: a2 is beyond the pinned cutoff for
    // srv-a and stays out of the snapshot; b2 replays a source
    // position inside the cutoff and belongs to it.
    ledger
        .append("a2".to_owned(), oid(12), source("srv-a", 1, 2, 12))
        .unwrap();
    ledger
        .append("b2".to_owned(), oid(13), source("srv-b", 1, 1, 13))
        .unwrap();

    let since: Vec<String> = ledger
        .since(&cursor)
        .unwrap()
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect();
    assert_eq!(since, vec!["b2"]);
    let before: Vec<String> = ledger
        .before(&cursor)
        .unwrap()
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect();
    assert_eq!(before, vec!["a1"]);

    // Paging the whole snapshot from a head cursor: before() returns
    // every older entry in the same total order, since() is empty.
    let head = ledger.cursor();
    let older: Vec<String> = ledger
        .before(&head)
        .unwrap()
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect();
    assert_eq!(older, vec!["a1", "b1", "a2"]);
    assert!(ledger.since(&head).unwrap().is_empty());
}

#[test]
fn rotation_expires_cursors_and_foreign_scopes_mismatch() {
    let mut ledger = FeedLedger::new(scope());
    ledger
        .append("a1".to_owned(), oid(10), source("srv-a", 1, 1, 10))
        .unwrap();
    let cursor = ledger.cursor();

    let mut foreign = cursor.clone();
    foreign.tenant_id = "tenant-b".to_owned();
    assert_eq!(
        ledger.since(&foreign).unwrap_err(),
        FeedError::CursorMismatch
    );

    ledger.rotate_generation();
    assert_eq!(ledger.since(&cursor).unwrap_err(), FeedError::CursorExpired);
    // The high-water mark continues across the rotation.
    let next = ledger
        .append("a2".to_owned(), oid(11), source("srv-a", 1, 2, 11))
        .unwrap();
    assert_eq!(next.last_projection_seq, 2);
}

#[test]
fn exhaustion_stops_allocation_without_wrapping() {
    let mut ledger = FeedLedger::restored(scope(), 1, u64::MAX);
    let last = ledger
        .append("final".to_owned(), oid(10), source("srv-a", 1, 1, 10))
        .unwrap();
    assert_eq!(last.last_projection_seq, u64::MAX);
    assert_eq!(
        ledger
            .append("overflow".to_owned(), oid(11), source("srv-a", 1, 2, 11))
            .unwrap_err(),
        FeedError::SequenceExhausted
    );
    assert_eq!(ledger.entries().len(), 1);
}
