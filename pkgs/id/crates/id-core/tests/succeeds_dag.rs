//! `succeeds` version DAG (slice A-GRAPH-9129-01, issue #10223; design
//! part A §8, statements S5.41–S5.43).
//!
//! Oracle: **model**. Fixed fixtures pin the stored direction (v1 → v2),
//! cycle rejection, pending ancestry, branch heads, owner selection and
//! the explicit legacy `Supersede` mapping. A property test then drives
//! the DAG and an independent model, a dense adjacency matrix with
//! transitive closure recomputed from scratch after every step, through
//! the same random operations and requires the same outcome, accepted
//! edge set, pending set and heads at every step.
//!
//! `graph/mod.rs` does not export these modules yet (slice 9110-09 wires
//! them), so this test includes them, and the modules they build on, by
//! path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/graph/term.rs"]
#[allow(dead_code)]
mod term;

#[path = "../src/graph/version.rs"]
mod version;

use id_shape::{ContentId, HashProfile};
use proptest::prelude::*;
use signed_envelope::EnvelopeId;
use term::{ObjectRef, Term, Triple};
use version::{
    Admission, HeadView, LegacySupersede, SucceedsEdge, Version, VersionDag, VersionError,
    adapt_supersede, succeeds_predicate,
};

fn vid(n: u8) -> ObjectRef {
    ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
        HashProfile::RawBlake3_256,
        [n; 32],
    )))
}

fn object(name: &str) -> Term {
    Term::typed(7, name.as_bytes().to_vec())
}

fn version(n: u8, name: &str) -> Version {
    Version {
        id: vid(n),
        logical_object: object(name),
        schema: 1,
    }
}

fn edge(from: u8, to: u8) -> SucceedsEdge {
    SucceedsEdge::new(vid(from), vid(to))
}

fn dag_with(versions: &[u8], name: &str) -> VersionDag {
    let mut dag = VersionDag::new();
    for n in versions {
        assert!(dag.add_version(version(*n, name)).unwrap().is_empty());
    }
    dag
}

#[test]
fn stored_direction_is_predecessor_succeeds_successor() {
    let triple = edge(1, 2).to_triple();
    assert_eq!(triple.subject, Term::Object(vid(1)));
    assert_eq!(triple.predicate, succeeds_predicate());
    assert_eq!(triple.object, Term::Object(vid(2)));
    let bytes = triple.to_bytes().unwrap();
    let back = SucceedsEdge::from_triple(&Triple::from_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(back, edge(1, 2));

    let mut dag = dag_with(&[1, 2], "doc");
    assert_eq!(dag.admit(edge(1, 2)), Ok(Admission::Accepted));
    assert_eq!(dag.edges(), vec![edge(1, 2)]);
    assert_eq!(dag.successors_of(&vid(1)), vec![vid(2)]);
    assert_eq!(dag.predecessors_of(&vid(2)), vec![vid(1)]);
    assert_eq!(dag.heads(&object("doc")), vec![vid(2)]);
}

#[test]
fn non_succeeds_triples_reject() {
    let other = Triple::new(
        Term::Object(vid(1)),
        Term::typed(1, b"id/replies-to".to_vec()),
        Term::Object(vid(2)),
    );
    assert_eq!(
        SucceedsEdge::from_triple(&other).unwrap_err().class(),
        "malformed"
    );
    let literal = Triple::new(
        Term::Object(vid(1)),
        succeeds_predicate(),
        Term::typed(7, b"v2".to_vec()),
    );
    assert!(matches!(
        SucceedsEdge::from_triple(&literal),
        Err(VersionError::NotSucceeds(_))
    ));
}

#[test]
fn self_edge_cross_object_and_schema_mismatch_reject() {
    let mut dag = dag_with(&[1, 2], "doc");
    dag.add_version(version(3, "other")).unwrap();
    let mut v4 = version(4, "doc");
    v4.schema = 2;
    dag.add_version(v4).unwrap();

    assert_eq!(
        dag.admit(edge(1, 1)).unwrap_err().class(),
        "invalid_successor"
    );
    assert_eq!(dag.admit(edge(1, 3)), Err(VersionError::WrongLogicalObject));
    assert_eq!(
        dag.admit(edge(1, 4)).unwrap_err().class(),
        "invalid_successor"
    );
    assert!(dag.edges().is_empty());
    assert!(dag.pending().is_empty());
}

#[test]
fn complete_cycles_reject_and_leave_the_graph_unchanged() {
    let mut dag = dag_with(&[1, 2, 3], "doc");
    assert_eq!(dag.admit(edge(1, 2)), Ok(Admission::Accepted));
    assert_eq!(dag.admit(edge(2, 3)), Ok(Admission::Accepted));
    assert_eq!(dag.admit(edge(3, 1)), Err(VersionError::LineageCycle));
    assert_eq!(dag.admit(edge(2, 1)), Err(VersionError::LineageCycle));
    assert_eq!(dag.edges(), vec![edge(1, 2), edge(2, 3)]);
    assert_eq!(dag.ancestors(&vid(3)), vec![vid(1), vid(2)]);
    // Re-admitting an accepted edge is idempotent.
    assert_eq!(dag.admit(edge(1, 2)), Ok(Admission::Accepted));
    assert_eq!(dag.edges().len(), 2);
}

#[test]
fn incomplete_ancestry_stays_pending_until_the_version_arrives() {
    let mut dag = dag_with(&[1, 2], "doc");
    dag.admit(edge(1, 2)).unwrap();
    assert_eq!(
        dag.admit(edge(2, 9)),
        Ok(Admission::Pending {
            missing: vec![vid(9)]
        })
    );
    // An edge that would close a cycle through the missing version is
    // pending too, not guessed at.
    assert_eq!(
        dag.admit(edge(9, 1)),
        Ok(Admission::Pending {
            missing: vec![vid(9)]
        })
    );
    assert_eq!(dag.pending(), &[edge(2, 9), edge(9, 1)]);
    assert_eq!(dag.heads(&object("doc")), vec![vid(2)]);

    let resolved = dag.add_version(version(9, "doc")).unwrap();
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].edge, edge(2, 9));
    assert_eq!(resolved[0].outcome, Ok(Admission::Accepted));
    assert_eq!(resolved[1].edge, edge(9, 1));
    assert_eq!(resolved[1].outcome, Err(VersionError::LineageCycle));
    assert!(dag.pending().is_empty());
    assert_eq!(dag.heads(&object("doc")), vec![vid(9)]);
}

#[test]
fn branches_keep_every_head_and_only_an_owner_selection_picks_one() {
    let mut dag = dag_with(&[1, 2, 3], "doc");
    dag.admit(edge(1, 2)).unwrap();
    dag.admit(edge(1, 3)).unwrap();
    let doc = object("doc");
    assert_eq!(dag.heads(&doc), vec![vid(2), vid(3)]);
    assert_eq!(
        dag.head_view(&doc, None),
        HeadView::Branches(vec![vid(2), vid(3)])
    );
    assert_eq!(
        dag.head_view(&doc, Some(&vid(3))),
        HeadView::Selected {
            head: vid(3),
            alternatives: vec![vid(2)]
        }
    );
    // A selection that is not a current head is ignored.
    assert_eq!(
        dag.head_view(&doc, Some(&vid(1))),
        HeadView::Branches(vec![vid(2), vid(3)])
    );

    // A merge version succeeding both branches makes one head again.
    dag.add_version(version(4, "doc")).unwrap();
    dag.admit(edge(2, 4)).unwrap();
    dag.admit(edge(3, 4)).unwrap();
    assert_eq!(dag.head_view(&doc, None), HeadView::Single(vid(4)));
    assert_eq!(dag.predecessors_of(&vid(4)), vec![vid(2), vid(3)]);
    assert_eq!(dag.head_view(&object("absent"), None), HeadView::Empty);
}

#[test]
fn a_version_id_cannot_change_its_object_or_schema() {
    let mut dag = dag_with(&[1], "doc");
    assert!(dag.add_version(version(1, "doc")).unwrap().is_empty());
    assert_eq!(
        dag.add_version(version(1, "other")).unwrap_err().class(),
        "invalid_successor"
    );
}

#[test]
fn legacy_supersede_maps_only_when_proven_equivalent() {
    let record = LegacySupersede {
        previous: vid(1),
        replacement: vid(2),
    };
    let v1 = version(1, "post");
    let v2 = version(2, "post");
    assert_eq!(adapt_supersede(&record, &v1, &v2, true), Ok(edge(1, 2)));
    // The record itself is unchanged; the adapter only reads it.
    assert_eq!(record.previous, vid(1));

    let not = |result: Result<SucceedsEdge, VersionError>| {
        assert_eq!(result.unwrap_err().class(), "not_equivalent");
    };
    not(adapt_supersede(&record, &v1, &v2, false));
    not(adapt_supersede(&record, &v2, &v1, true));
    not(adapt_supersede(&record, &v1, &version(2, "other"), true));
    let mut v2_schema = v2;
    v2_schema.schema = 9;
    not(adapt_supersede(&record, &v1, &v2_schema, true));
    let same = LegacySupersede {
        previous: vid(1),
        replacement: vid(1),
    };
    not(adapt_supersede(&same, &v1, &v1, true));
}

// ---------------------------------------------------------------------
// Independent model: dense matrices, closure recomputed every step.

const UNIVERSE: usize = 6;

#[derive(Debug, Clone)]
enum Op {
    Add(u8),
    Link(u8, u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Accepted,
    Pending(Vec<u8>),
    Rejected(&'static str),
}

/// Version `n` belongs to object `n % 2` and has schema `n / 4` (so 4
/// and 5 have a different schema from 0..=3).
fn model_version(n: u8) -> Version {
    Version {
        id: vid(n),
        logical_object: object(if n.is_multiple_of(2) { "even" } else { "odd" }),
        schema: u64::from(n / 4),
    }
}

#[derive(Default)]
struct Model {
    known: [bool; UNIVERSE],
    adj: [[bool; UNIVERSE]; UNIVERSE],
    pending: Vec<(u8, u8)>,
}

impl Model {
    fn closure(&self) -> [[bool; UNIVERSE]; UNIVERSE] {
        let mut reach = self.adj;
        for k in 0..UNIVERSE {
            for i in 0..UNIVERSE {
                for j in 0..UNIVERSE {
                    reach[i][j] |= reach[i][k] && reach[k][j];
                }
            }
        }
        reach
    }

    fn link(&mut self, a: u8, b: u8) -> Outcome {
        let (i, j) = (usize::from(a), usize::from(b));
        if a == b {
            return Outcome::Rejected("invalid_successor");
        }
        let missing: Vec<u8> = [a.min(b), a.max(b)]
            .into_iter()
            .filter(|n| !self.known[usize::from(*n)])
            .collect();
        if !missing.is_empty() {
            if !self.pending.contains(&(a, b)) {
                self.pending.push((a, b));
            }
            return Outcome::Pending(missing);
        }
        if a % 2 != b % 2 {
            return Outcome::Rejected("wrong_logical_object");
        }
        if a / 4 != b / 4 {
            return Outcome::Rejected("invalid_successor");
        }
        if self.adj[i][j] {
            return Outcome::Accepted;
        }
        if self.closure()[j][i] {
            return Outcome::Rejected("lineage_cycle");
        }
        self.adj[i][j] = true;
        Outcome::Accepted
    }

    fn add(&mut self, n: u8) -> Vec<((u8, u8), Outcome)> {
        if self.known[usize::from(n)] {
            return Vec::new();
        }
        self.known[usize::from(n)] = true;
        let waiting = std::mem::take(&mut self.pending);
        waiting
            .into_iter()
            .map(|(a, b)| ((a, b), self.link(a, b)))
            .collect()
    }

    fn heads(&self, parity: u8) -> Vec<u8> {
        (0..UNIVERSE as u8)
            .filter(|n| n % 2 == parity && self.known[usize::from(*n)])
            .filter(|n| !self.adj[usize::from(*n)].iter().any(|x| *x))
            .collect()
    }
}

fn index_of(reference: &ObjectRef) -> u8 {
    let ObjectRef::Envelope(id) = reference else {
        panic!("model uses envelope refs only")
    };
    id.content_id().digest()[0]
}

fn outcome(result: &Result<Admission, VersionError>) -> Outcome {
    match result {
        Ok(Admission::Accepted) => Outcome::Accepted,
        Ok(Admission::Pending { missing }) => {
            Outcome::Pending(missing.iter().map(index_of).collect())
        }
        Err(error) => Outcome::Rejected(error.class()),
    }
}

fn op() -> impl Strategy<Value = Op> {
    let n = 0..UNIVERSE as u8;
    prop_oneof![
        n.clone().prop_map(Op::Add),
        (n.clone(), n).prop_map(|(a, b)| Op::Link(a, b)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn dag_matches_the_independent_model(ops in prop::collection::vec(op(), 1..40)) {
        let mut dag = VersionDag::new();
        let mut model = Model::default();
        for step in &ops {
            match *step {
                Op::Add(n) => {
                    let got: Vec<((u8, u8), Outcome)> = dag
                        .add_version(model_version(n))
                        .unwrap()
                        .iter()
                        .map(|r| ((index_of(&r.edge.predecessor), index_of(&r.edge.successor)), outcome(&r.outcome)))
                        .collect();
                    prop_assert_eq!(got, model.add(n));
                }
                Op::Link(a, b) => {
                    prop_assert_eq!(outcome(&dag.admit(edge(a, b))), model.link(a, b));
                }
            }
            let mut want_edges = Vec::new();
            for i in 0..UNIVERSE {
                for j in 0..UNIVERSE {
                    if model.adj[i][j] {
                        want_edges.push((i as u8, j as u8));
                    }
                }
            }
            let got_edges: Vec<(u8, u8)> = dag
                .edges()
                .iter()
                .map(|e| (index_of(&e.predecessor), index_of(&e.successor)))
                .collect();
            prop_assert_eq!(got_edges, want_edges);
            let got_pending: Vec<(u8, u8)> = dag
                .pending()
                .iter()
                .map(|e| (index_of(&e.predecessor), index_of(&e.successor)))
                .collect();
            prop_assert_eq!(&got_pending, &model.pending);
            for parity in 0..2 {
                let name = if parity == 0 { "even" } else { "odd" };
                let got: Vec<u8> = dag.heads(&object(name)).iter().map(index_of).collect();
                prop_assert_eq!(got, model.heads(parity));
            }
            // The accepted graph is always acyclic.
            let reach = model.closure();
            prop_assert!((0..UNIVERSE).all(|i| !reach[i][i]));
        }
    }
}
