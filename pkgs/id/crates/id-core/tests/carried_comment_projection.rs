//! Carried ancestor comments (slice A-GRAPH-9129-02, issue #10269;
//! design part A §8, statements S5.44, S5.45).
//!
//! Oracle: **model**. Fixed fixtures pin the transitive origin, the
//! old-version marker, shortest-path distance over diamond ancestry,
//! sibling-branch isolation and the access/moderation/deletion
//! filters; a naive layered-expansion model over the same DAG checks
//! the full result on a mixed fixture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use id_core::graph::comment_projection::{CommentFilters, carried_comments};
use id_core::graph::social::{SocialObject, SocialObjectKind};
use id_core::graph::term::{ObjectRef, Term};
use id_core::graph::version::{SucceedsEdge, Version, VersionDag};
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::EnvelopeId;

fn oid(n: u8) -> ObjectRef {
    ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
        HashProfile::RawBlake3_256,
        [n; 32],
    )))
}

fn dag(versions: &[u8], edges: &[(u8, u8)]) -> VersionDag {
    let mut dag = VersionDag::new();
    for n in versions {
        dag.add_version(Version {
            id: oid(*n),
            logical_object: Term::typed(7, b"doc".to_vec()),
            schema: 1,
        })
        .unwrap();
    }
    for (from, to) in edges {
        dag.admit(SucceedsEdge::new(oid(*from), oid(*to))).unwrap();
    }
    dag
}

fn comment(id: u8, target: Option<u8>) -> SocialObject {
    SocialObject {
        object_id: oid(id),
        kind: SocialObjectKind::Comment,
        author: "alice".to_owned(),
        created_claim: u64::from(id),
        parent_ref: target.map(oid),
        content_ref: oid(100 + id),
        schema_version: 1,
        succeeds_ref: None,
        target_ref: None,
    }
}

fn no_filters() -> CommentFilters {
    CommentFilters::default()
}

fn summary(
    dag: &VersionDag,
    comments: &[SocialObject],
    selected: u8,
    filters: &CommentFilters,
) -> Vec<(u8, u8, bool, usize)> {
    carried_comments(dag, comments, &oid(selected), filters, |_| true)
        .iter()
        .map(|carried| {
            let digest = |reference: ObjectRef| {
                let ObjectRef::Envelope(id) = reference else {
                    panic!("fixture refs are envelopes");
                };
                id.content_id().digest()[0]
            };
            (
                digest(carried.comment_id),
                digest(carried.origin_version),
                carried.old_version,
                carried.distance,
            )
        })
        .collect()
}

#[test]
fn chain_carry_keeps_transitive_origin_and_marks_old_versions() {
    let dag = dag(&[1, 2, 3], &[(1, 2), (2, 3)]);
    let comments = vec![
        comment(11, Some(1)),
        comment(12, Some(2)),
        comment(13, Some(3)),
    ];
    // The v1 comment shown on v3 still names v1 as its origin, two
    // steps back; only the comment on v3 itself is unmarked.
    assert_eq!(
        summary(&dag, &comments, 3, &no_filters()),
        vec![(13, 3, false, 0), (12, 2, true, 1), (11, 1, true, 2)]
    );
}

#[test]
fn sibling_branches_do_not_leak() {
    let dag = dag(&[1, 2, 3], &[(1, 2), (1, 3)]);
    let comments = vec![comment(11, Some(1)), comment(13, Some(3))];
    assert_eq!(
        summary(&dag, &comments, 2, &no_filters()),
        vec![(11, 1, true, 1)]
    );
    assert_eq!(
        summary(&dag, &comments, 3, &no_filters()),
        vec![(13, 3, false, 0), (11, 1, true, 1)]
    );
}

#[test]
fn diamond_ancestry_carries_once_at_shortest_distance() {
    let dag = dag(&[1, 2, 3, 4], &[(1, 2), (1, 3), (2, 4), (3, 4)]);
    // The same comment listed twice still appears once.
    let comments = vec![comment(11, Some(1)), comment(11, Some(1))];
    assert_eq!(
        summary(&dag, &comments, 4, &no_filters()),
        vec![(11, 1, true, 2)]
    );
}

#[test]
fn access_moderation_and_deletion_filters_still_apply() {
    let dag = dag(&[1, 2], &[(1, 2)]);
    let mut stranger = comment(14, Some(1));
    stranger.author = "mallory".to_owned();
    let comments = vec![
        comment(11, Some(1)),
        comment(12, Some(1)),
        comment(13, Some(1)),
        stranger,
        comment(15, Some(2)),
        comment(16, Some(1)),
    ];
    let filters = CommentFilters {
        hidden: HashSet::from([oid(12)]),
        deleted: HashSet::from([oid(13)]),
    };
    let got = carried_comments(&dag, &comments, &oid(2), &filters, |object| {
        object.author != "mallory"
    });
    let ids: Vec<u8> = got
        .iter()
        .map(|carried| {
            let ObjectRef::Envelope(id) = carried.comment_id else {
                panic!("fixture refs are envelopes");
            };
            id.content_id().digest()[0]
        })
        .collect();
    assert_eq!(ids, vec![15, 11, 16]);
}

#[test]
fn unrelated_and_malformed_comments_are_ignored() {
    let dag = dag(&[1, 2], &[(1, 2)]);
    let mut topic = comment(21, Some(1));
    topic.kind = SocialObjectKind::Topic;
    let comments = vec![
        topic,
        comment(22, None),
        comment(23, Some(9)),
        comment(24, Some(1)),
    ];
    assert_eq!(
        summary(&dag, &comments, 2, &no_filters()),
        vec![(24, 1, true, 1)]
    );
}

/// Naive model: eligible targets are the selected version plus
/// `VersionDag::ancestors`; distance is found by layered frontier
/// expansion from the selected version.
fn model_summary(
    dag: &VersionDag,
    comments: &[SocialObject],
    selected: u8,
) -> Vec<(u8, u8, bool, usize)> {
    let mut frontier = vec![oid(selected)];
    let mut distance: std::collections::HashMap<ObjectRef, usize> =
        frontier.iter().map(|reference| (*reference, 0)).collect();
    let mut steps = 0;
    loop {
        steps += 1;
        let mut next = Vec::new();
        for at in &frontier {
            for predecessor in dag.predecessors_of(at) {
                if !distance.contains_key(&predecessor) {
                    distance.insert(predecessor, steps);
                    next.push(predecessor);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for object in comments {
        if object.kind != SocialObjectKind::Comment {
            continue;
        }
        let Some(target) = object.parent_ref else {
            continue;
        };
        let Some(&steps) = distance.get(&target) else {
            continue;
        };
        if !seen.insert(object.object_id) {
            continue;
        }
        let digest = |reference: ObjectRef| {
            let ObjectRef::Envelope(id) = reference else {
                panic!("fixture refs are envelopes");
            };
            id.content_id().digest()[0]
        };
        out.push((digest(object.object_id), digest(target), steps > 0, steps));
    }
    out.sort_by_key(|(id, _, _, steps)| (*steps, *id));
    out
}

#[test]
fn mixed_fixture_matches_naive_model() {
    let dag = dag(
        &[1, 2, 3, 4, 5, 6],
        &[(1, 2), (1, 3), (2, 4), (3, 4), (4, 5), (3, 6)],
    );
    let comments = vec![
        comment(11, Some(1)),
        comment(12, Some(2)),
        comment(13, Some(3)),
        comment(14, Some(4)),
        comment(15, Some(5)),
        comment(16, Some(6)),
        comment(17, Some(3)),
    ];
    for selected in [4, 5, 6] {
        assert_eq!(
            summary(&dag, &comments, selected, &no_filters()),
            model_summary(&dag, &comments, selected),
            "selected version {selected}"
        );
    }
}
