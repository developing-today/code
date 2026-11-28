//! Bounded triple-pattern queries and cursors (slice A-GRAPH-9126-01,
//! issue #10221; design part A §5.1, statements S5.8, S5.10, S5.11).
//!
//! Oracle: **reference**. An independent pure matcher in this file
//! enumerates every tuple of readable assertions, checks the patterns
//! with its own variable bindings, orders rows by keys built from its own
//! encoding of the terms, and counts the steps the query should take.
//! The paged query must agree with it at a fixed frontier: cursor pages
//! concatenate to exactly the reference rows with no omission or
//! duplication, only the last page reports complete, and unauthorized or
//! over-budget queries fail instead of reporting complete.
//!
//! This suite includes the graph modules and their dependencies by path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::type_complexity,
    clippy::unnecessary_wraps,
    clippy::missing_const_for_fn
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

#[path = "../src/graph/cursor.rs"]
mod cursor;

#[path = "../src/graph/query.rs"]
mod query;

use std::collections::BTreeSet;

use cursor::Cursor;
use id_shape::{ContentId, HashProfile};
use proptest::prelude::*;
use query::{
    GraphReadPolicy, Profile, Query, QueryError, QueryPage, Snapshot, StoredAssertion, TermPattern,
    TriplePattern, Variable, run,
};
use signed_envelope::{EnvelopeId, LogicalGraphId};
use term::{Term, Triple};

// ---------------------------------------------------------------------
// Fixture helpers.

fn graph(n: u8) -> LogicalGraphId {
    LogicalGraphId::new([n; 32])
}

fn record(n: u8) -> EnvelopeId {
    EnvelopeId::from_content_id(ContentId::new(HashProfile::RawBlake3_256, [n; 32]))
}

fn t(tag: u8, byte: u8) -> Term {
    Term::typed(u64::from(tag), vec![byte])
}

fn var(name: &str) -> Variable {
    Variable::new(name).unwrap()
}

fn v(name: &str) -> Option<TermPattern> {
    Some(TermPattern::Var(var(name)))
}

fn exact(term: Term) -> Option<TermPattern> {
    Some(TermPattern::Exact(term))
}

/// Compact assertion: (graph, record, index, s, p, o, source).
type Spec = (u8, u8, u32, (u8, u8), (u8, u8), (u8, u8), Option<u8>);

fn stored(spec: Spec) -> StoredAssertion {
    let (g, r, index, s, p, o, source) = spec;
    StoredAssertion {
        record: record(r),
        index,
        graph: graph(g),
        triple: Triple::new(t(s.0, s.1), t(p.0, p.1), t(o.0, o.1)),
        source_record: source.map(record),
    }
}

struct Readable(BTreeSet<u8>);

impl GraphReadPolicy for Readable {
    fn may_read(&self, g: &LogicalGraphId) -> bool {
        self.0.contains(&g.as_bytes()[0])
    }
}

fn all_graphs() -> Readable {
    Readable((0..=255).collect())
}

fn query(patterns: Vec<TriplePattern>, projection: &[&str], limit: u32) -> Query {
    Query {
        patterns,
        projection: projection.iter().map(|n| var(n)).collect(),
        limit: Some(limit),
        cursor: None,
    }
}

/// Follows cursors to the end and returns every page.
fn all_pages(
    mut q: Query,
    snapshot: &Snapshot,
    policy: &dyn GraphReadPolicy,
    profile: &Profile,
) -> Result<Vec<QueryPage>, QueryError> {
    let mut pages = Vec::new();
    loop {
        let page = run(&q, snapshot, policy, profile)?;
        let next = page.next_cursor.clone();
        pages.push(page);
        match next {
            Some(cursor) => q.cursor = Some(cursor),
            None => return Ok(pages),
        }
    }
}

// ---------------------------------------------------------------------
// Independent reference matcher.

/// Own canonical encoding of a one-byte typed term, `[0, tag, h'byte']`,
/// valid for tags below 24.
fn ref_term_bytes(term: &Term) -> Vec<u8> {
    let Term::Typed { type_tag, bytes } = term else {
        panic!("reference handles typed terms only")
    };
    assert!(*type_tag < 24 && bytes.len() < 24);
    let mut out = vec![0x83, 0x00, *type_tag as u8, 0x40 | bytes.len() as u8];
    out.extend_from_slice(bytes);
    out
}

type RefKey = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, u32);

fn ref_key(a: &StoredAssertion) -> RefKey {
    (
        a.graph.as_bytes().to_vec(),
        ref_term_bytes(&a.triple.subject),
        ref_term_bytes(&a.triple.predicate),
        ref_term_bytes(&a.triple.object),
        a.record.content_id().digest().to_vec(),
        a.index,
    )
}

fn ref_bind(pos: Option<&TermPattern>, term: &Term, env: &mut Vec<(String, Term)>) -> bool {
    match pos {
        None => true,
        Some(TermPattern::Exact(want)) => want == term,
        Some(TermPattern::Var(name)) => {
            if let Some((_, bound)) = env.iter().find(|(n, _)| n == name.name()) {
                return bound == term;
            }
            env.push((name.name().to_owned(), term.clone()));
            true
        }
    }
}

/// Every consistent tuple for the first `upto` patterns, unordered.
fn ref_tuples(
    patterns: &[TriplePattern],
    upto: usize,
    readable: &[StoredAssertion],
) -> Vec<(Vec<usize>, Vec<(String, Term)>)> {
    let mut out = Vec::new();
    let n = readable.len();
    let total = n.pow(upto as u32);
    for mut code in 0..total {
        let mut chosen = Vec::with_capacity(upto);
        for _ in 0..upto {
            chosen.push(code % n);
            code /= n;
        }
        let mut env = Vec::new();
        let ok = patterns[..upto].iter().zip(&chosen).all(|(p, at)| {
            let a = &readable[*at];
            p.graph.is_none_or(|g| g == a.graph)
                && ref_bind(p.subject.as_ref(), &a.triple.subject, &mut env)
                && ref_bind(p.predicate.as_ref(), &a.triple.predicate, &mut env)
                && ref_bind(p.object.as_ref(), &a.triple.object, &mut env)
        });
        if ok {
            out.push((chosen, env));
        }
    }
    out
}

struct Expected {
    rows: Vec<(Vec<Term>, Vec<(EnvelopeId, u32, Option<EnvelopeId>)>)>,
    steps: u64,
}

fn reference(q: &Query, assertions: &[StoredAssertion], policy: &Readable) -> Expected {
    let mut readable: Vec<StoredAssertion> = assertions
        .iter()
        .filter(|a| policy.may_read(&a.graph))
        .cloned()
        .collect();
    readable.sort_by_key(ref_key);
    let n = readable.len() as u64;
    let steps = (0..q.patterns.len())
        .map(|i| ref_tuples(&q.patterns, i, &readable).len() as u64 * n)
        .sum();
    let mut tuples = ref_tuples(&q.patterns, q.patterns.len(), &readable);
    tuples.sort_by(|a, b| {
        let ka: Vec<RefKey> = a.0.iter().map(|i| ref_key(&readable[*i])).collect();
        let kb: Vec<RefKey> = b.0.iter().map(|i| ref_key(&readable[*i])).collect();
        ka.cmp(&kb)
    });
    let rows = tuples
        .into_iter()
        .map(|(chosen, env)| {
            let values = q
                .projection
                .iter()
                .map(|p| env.iter().find(|(n, _)| n == p.name()).unwrap().1.clone())
                .collect();
            let matched = chosen
                .iter()
                .map(|i| {
                    let a = &readable[*i];
                    (a.record, a.index, a.source_record)
                })
                .collect();
            (values, matched)
        })
        .collect();
    Expected { rows, steps }
}

fn flatten(pages: &[QueryPage]) -> Vec<(Vec<Term>, Vec<(EnvelopeId, u32, Option<EnvelopeId>)>)> {
    pages
        .iter()
        .flat_map(|p| &p.rows)
        .map(|r| {
            (
                r.values.clone(),
                r.matched
                    .iter()
                    .map(|m| (m.record, m.index, m.source_record))
                    .collect(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------
// Fixtures.

fn social() -> Vec<StoredAssertion> {
    // graph 1: alice(1) knows(9) bob(2), bob knows carol(3), alice likes(8) carol.
    // graph 2 (private): carol knows alice.
    [
        (1, 10, 0, (1, 1), (1, 9), (1, 2), None),
        (1, 10, 1, (1, 2), (1, 9), (1, 3), None),
        (1, 11, 0, (1, 1), (1, 8), (1, 3), Some(10)),
        (2, 12, 0, (1, 3), (1, 9), (1, 1), None),
    ]
    .into_iter()
    .map(stored)
    .collect()
}

#[test]
fn two_hop_join_returns_bindings_and_provenance_in_stable_order() {
    let snapshot = Snapshot::new(social());
    let q = query(
        vec![
            TriplePattern {
                subject: v("a"),
                predicate: exact(t(1, 9)),
                object: v("b"),
                ..TriplePattern::default()
            },
            TriplePattern {
                subject: v("b"),
                predicate: exact(t(1, 9)),
                object: v("c"),
                ..TriplePattern::default()
            },
        ],
        &["a", "c"],
        10,
    );
    let page = run(&q, &snapshot, &all_graphs(), &Profile::INITIAL).unwrap();
    // alice→bob→carol, bob→carol→alice (across graphs), carol→alice→bob.
    let got: Vec<Vec<Term>> = page.rows.iter().map(|r| r.values.clone()).collect();
    assert_eq!(
        got,
        vec![
            vec![t(1, 1), t(1, 3)],
            vec![t(1, 2), t(1, 1)],
            vec![t(1, 3), t(1, 2)],
        ]
    );
    assert!(page.complete);
    assert!(page.next_cursor.is_none());
    assert_eq!(page.cost.returned, 3);
    assert_eq!(page.cost.scanned, 4 + 3 * 4);
    assert_eq!(page.snapshot, snapshot.frontier());

    // Provenance: the "likes" row cites its source record.
    let likes = query(
        vec![TriplePattern {
            predicate: exact(t(1, 8)),
            ..TriplePattern::default()
        }],
        &[],
        10,
    );
    let page = run(&likes, &snapshot, &all_graphs(), &Profile::INITIAL).unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].matched[0].record, record(11));
    assert_eq!(page.rows[0].matched[0].source_record, Some(record(10)));
}

#[test]
fn unreadable_graphs_are_refused_by_name_and_invisible_otherwise() {
    let snapshot = Snapshot::new(social());
    let public = Readable(BTreeSet::from([1]));
    let named = query(
        vec![TriplePattern {
            graph: Some(graph(2)),
            ..TriplePattern::default()
        }],
        &[],
        10,
    );
    assert_eq!(
        run(&named, &snapshot, &public, &Profile::INITIAL),
        Err(QueryError::UnauthorizedGraph)
    );
    let any = query(
        vec![TriplePattern {
            subject: v("s"),
            ..TriplePattern::default()
        }],
        &["s"],
        10,
    );
    let page = run(&any, &snapshot, &public, &Profile::INITIAL).unwrap();
    assert_eq!(page.rows.len(), 3);
    // The hidden assertion does not show up in the cost either.
    assert_eq!(page.cost.scanned, 3);
}

#[test]
fn budget_and_limits_are_enforced_without_claiming_completeness() {
    let snapshot = Snapshot::new(social());
    let q = query(
        vec![TriplePattern::default(), TriplePattern::default()],
        &[],
        10,
    );
    let tight = Profile {
        max_steps: 19,
        ..Profile::INITIAL
    };
    assert_eq!(
        run(&q, &snapshot, &all_graphs(), &tight),
        Err(QueryError::BudgetExceeded { scanned: 20 })
    );
    let exact_fit = Profile {
        max_steps: 20,
        ..Profile::INITIAL
    };
    assert!(run(&q, &snapshot, &all_graphs(), &exact_fit).is_ok());

    let mut over = q.clone();
    over.limit = Some(Profile::INITIAL.row_ceiling + 1);
    assert_eq!(
        run(&over, &snapshot, &all_graphs(), &Profile::INITIAL)
            .unwrap_err()
            .class(),
        "invalid_pattern"
    );
    let mut zero = q.clone();
    zero.limit = Some(0);
    assert!(run(&zero, &snapshot, &all_graphs(), &Profile::INITIAL).is_err());
    let mut default = q;
    default.limit = None;
    let small = Profile {
        page_default: 5,
        ..Profile::INITIAL
    };
    let page = run(&default, &snapshot, &all_graphs(), &small).unwrap();
    assert_eq!(page.rows.len(), 5);
    assert!(!page.complete);
}

#[test]
fn malformed_queries_reject() {
    let snapshot = Snapshot::new(social());
    let empty = query(vec![], &[], 10);
    assert_eq!(
        run(&empty, &snapshot, &all_graphs(), &Profile::INITIAL)
            .unwrap_err()
            .class(),
        "invalid_pattern"
    );
    let unbound = query(vec![TriplePattern::default()], &["x"], 10);
    assert!(run(&unbound, &snapshot, &all_graphs(), &Profile::INITIAL).is_err());
    let too_many = query(vec![TriplePattern::default(); 9], &[], 10);
    assert!(run(&too_many, &snapshot, &all_graphs(), &Profile::INITIAL).is_err());
    for good in ["a_b", "_", "X9", &"x".repeat(64)] {
        assert!(Variable::new(good).is_ok(), "{good:?}");
    }
    for bad in ["", "has space", "dash-ed", &"x".repeat(65)] {
        assert!(Variable::new(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn cursors_are_bound_to_their_query_and_frontier() {
    let snapshot = Snapshot::new(social());
    let q = query(vec![TriplePattern::default()], &[], 1);
    let first = run(&q, &snapshot, &all_graphs(), &Profile::INITIAL).unwrap();
    let cursor = first.next_cursor.unwrap();
    assert!(!first.complete);

    let mut other = query(
        vec![TriplePattern {
            subject: v("s"),
            ..TriplePattern::default()
        }],
        &[],
        1,
    );
    other.cursor = Some(cursor.clone());
    assert_eq!(
        run(&other, &snapshot, &all_graphs(), &Profile::INITIAL),
        Err(QueryError::CursorMismatch)
    );

    let mut grown = social();
    grown.push(stored((1, 13, 0, (1, 4), (1, 9), (1, 1), None)));
    let newer = Snapshot::new(grown);
    let mut resumed = q.clone();
    resumed.cursor = Some(cursor.clone());
    assert_eq!(
        run(&resumed, &newer, &all_graphs(), &Profile::INITIAL),
        Err(QueryError::CursorExpired)
    );

    let mut forged = q.clone();
    forged.cursor = Some(Cursor::from_bytes(vec![0x01, 0x02]));
    assert_eq!(
        run(&forged, &snapshot, &all_graphs(), &Profile::INITIAL),
        Err(QueryError::CursorMismatch)
    );
    // Editing a real cursor's bytes does not move it to another query.
    let mut edited = cursor.as_bytes().to_vec();
    let last = edited.len() - 1;
    edited[last] ^= 0xff;
    forged.cursor = Some(Cursor::from_bytes(edited));
    assert_eq!(
        run(&forged, &snapshot, &all_graphs(), &Profile::INITIAL),
        Err(QueryError::CursorMismatch)
    );

    // The graph a pattern names is part of the query a cursor is for.
    let in_graph = |g: u8| {
        query(
            vec![TriplePattern {
                graph: Some(graph(g)),
                ..TriplePattern::default()
            }],
            &[],
            1,
        )
    };
    let g1_cursor = run(&in_graph(1), &snapshot, &all_graphs(), &Profile::INITIAL)
        .unwrap()
        .next_cursor
        .unwrap();
    let mut g2 = in_graph(2);
    g2.cursor = Some(g1_cursor);
    assert_eq!(
        run(&g2, &snapshot, &all_graphs(), &Profile::INITIAL),
        Err(QueryError::CursorMismatch)
    );

    // A different page size may continue the same query.
    let mut wider = q;
    wider.limit = Some(10);
    wider.cursor = Some(cursor);
    let rest = run(&wider, &snapshot, &all_graphs(), &Profile::INITIAL).unwrap();
    assert_eq!(rest.rows.len(), 3);
    assert!(rest.complete);
}

#[test]
fn frontier_ignores_arrival_order_and_duplicates() {
    let mut shuffled = social();
    shuffled.reverse();
    shuffled.push(social()[0].clone());
    let a = Snapshot::new(social());
    let b = Snapshot::new(shuffled);
    assert_eq!(a.frontier(), b.frontier());
    assert_eq!(b.len(), 4);
    assert!(!b.is_empty() && Snapshot::new(Vec::new()).is_empty());
    assert_ne!(
        a.frontier(),
        Snapshot::new(social()[..3].to_vec()).frontier()
    );
    // The frontier covers each assertion's index, not only its record.
    let mut moved = social();
    moved[0].index = 7;
    assert_ne!(a.frontier(), Snapshot::new(moved).frontier());
}

// ---------------------------------------------------------------------
// Property test against the reference matcher.

fn term_strategy() -> impl Strategy<Value = (u8, u8)> {
    (0_u8..2, 0_u8..2)
}

fn assertion_strategy() -> impl Strategy<Value = Spec> {
    (
        1_u8..3,
        0_u8..4,
        0_u32..3,
        term_strategy(),
        term_strategy(),
        term_strategy(),
        prop::option::of(0_u8..4),
    )
}

fn position_strategy() -> impl Strategy<Value = Option<TermPattern>> {
    prop_oneof![
        2 => Just(None),
        1 => term_strategy().prop_map(|(a, b)| exact(t(a, b))),
        3 => prop::sample::select(vec!["x", "y", "z"]).prop_map(v),
    ]
}

fn pattern_strategy() -> impl Strategy<Value = TriplePattern> {
    (
        prop::option::weighted(0.2, 1_u8..4),
        position_strategy(),
        position_strategy(),
        position_strategy(),
    )
        .prop_map(|(g, subject, predicate, object)| TriplePattern {
            graph: g.map(graph),
            subject,
            predicate,
            object,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn paged_query_matches_the_reference(
        specs in prop::collection::vec(assertion_strategy(), 3..12),
        patterns in prop::collection::vec(pattern_strategy(), 1..4),
        projection in prop::sample::subsequence(vec!["x", "y", "z"], 0..=3),
        readable in prop::collection::btree_set(1_u8..4, 1..=3),
        limit in 1_u32..5,
        max_steps in prop_oneof![1 => 0_u64..60, 3 => Just(u64::MAX)],
    ) {
        // Assertion ids are unique in a real store.
        let mut ids = BTreeSet::new();
        let assertions: Vec<StoredAssertion> = specs
            .into_iter()
            .filter(|s| ids.insert((s.1, s.2)))
            .map(stored)
            .collect();
        let snapshot = Snapshot::new(assertions.clone());
        let policy = Readable(readable);
        let profile = Profile { page_default: 3, row_ceiling: 4, max_steps };
        let mut q = query(patterns, &projection, limit);
        q.projection.retain(|p| {
            q.patterns.iter().any(|pat| {
                [&pat.subject, &pat.predicate, &pat.object]
                    .into_iter()
                    .any(|pos| matches!(pos, Some(TermPattern::Var(x)) if x == p))
            })
        });

        let named_hidden = q
            .patterns
            .iter()
            .filter_map(|p| p.graph.as_ref())
            .any(|g| !policy.may_read(g));
        let got = all_pages(q.clone(), &snapshot, &policy, &profile);
        if named_hidden {
            prop_assert_eq!(got.unwrap_err(), QueryError::UnauthorizedGraph);
            return Ok(());
        }
        let want = reference(&q, &assertions, &policy);
        if want.steps > max_steps {
            prop_assert_eq!(
                got.unwrap_err(),
                QueryError::BudgetExceeded { scanned: max_steps + 1 }
            );
            return Ok(());
        }
        let pages = got.unwrap();
        prop_assert_eq!(flatten(&pages), want.rows);
        for (i, page) in pages.iter().enumerate() {
            let last = i + 1 == pages.len();
            prop_assert_eq!(page.complete, last);
            prop_assert_eq!(page.next_cursor.is_none(), last);
            prop_assert!(page.rows.len() <= limit as usize);
            prop_assert_eq!(page.cost.returned as usize, page.rows.len());
            prop_assert_eq!(page.cost.scanned, want.steps);
            prop_assert_eq!(page.snapshot, snapshot.frontier());
            if !last {
                prop_assert_eq!(page.rows.len(), limit as usize);
            }
        }
    }
}
