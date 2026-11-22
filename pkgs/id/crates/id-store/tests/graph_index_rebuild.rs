//! Rebuildable SPO/POS/OSP graph indexes and bounded continuation scans
//! (slice A-GRAPH-9125-02, issue #10234; design part A §4.2).
//!
//! Oracle: **reference**. Graph records are signed and opened with the
//! slice A-GRAPH-9125-01 record code, and their assertions are the signed
//! truth. An independent matcher in this file filters those assertions as
//! typed terms (it never sees index keys or canonical bytes) at a fixed
//! frontier. Every supported leading-component scan, paged through its
//! continuations at several page sizes, must return exactly the matcher's
//! assertion IDs, and indexes built by shuffled batched appends and by a
//! rebuild from re-opened signed envelopes must agree row for row. Missing
//! tables and meta rows must make the index unavailable; extra, missing,
//! altered and undecodable rows and an unknown schema must be reported,
//! and a rebuild from the signed records must restore matching scans.
//!
//! The graph modules are not exported yet (slice 9110-09), so this test
//! includes them, and `src/graph_index.rs`, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

#[path = "../../id-core/src/canonical_cbor.rs"]
#[allow(dead_code, unreachable_pub)]
mod canonical_cbor;

#[path = "../../id-core/src/id_shape.rs"]
#[allow(dead_code, unreachable_pub)]
mod id_shape;

#[path = "../../id-core/src/hash_profile.rs"]
#[allow(dead_code, unreachable_pub)]
mod hash_profile;

#[path = "../../id-core/src/signed_envelope.rs"]
#[allow(dead_code, unreachable_pub)]
mod signed_envelope;

#[path = "../../id-core/src/graph/term.rs"]
#[allow(dead_code, unreachable_pub)]
mod term;

#[path = "../../id-core/src/graph/record.rs"]
#[allow(dead_code, unreachable_pub)]
mod record;

#[path = "../src/graph_index.rs"]
#[allow(dead_code, unreachable_pub)]
mod graph_index;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use graph_index::{
    AssertionKey, Continuation, GraphIndex, IndexError, IndexedTriple, Order, Pattern, ROW_CEILING,
    decode_row, plan, row_key,
};
use hash_profile::ProfileDispatch;
use id_core::tuple::{TupleEncoder, TupleValue, decode as decode_tuple};
use id_shape::{ContentId, HashProfile};
use proptest::prelude::*;
use record::{GraphRecord, open};
use redb::{Database, TableDefinition};
use signed_envelope::{Envelope, EnvelopeId, LogicalGraphId};
use term::{ObjectRef, Term, Triple};

const SPO: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_spo");
const POS: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_pos");
const OSP: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_osp");
const META: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("graph_index_meta");

const GRAPHS: [[u8; 32]; 3] = [[0xA1; 32], [0xB2; 32], [0xC3; 32]];
const PAGE_SIZES: [usize; 4] = [1, 2, 7, ROW_CEILING];

// ---------------------------------------------------------------------------
// Signed evidence.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

fn iri(text: &str) -> Term {
    Term::typed(1, text.as_bytes().to_vec())
}

fn content(byte: u8) -> Term {
    Term::Object(ObjectRef::Content(ContentId::new(
        HashProfile::RawBlake3_256,
        [byte; 32],
    )))
}

fn envelope(byte: u8) -> Term {
    Term::Object(ObjectRef::Envelope(EnvelopeId::from_content_id(
        ContentId::new(HashProfile::RawBlake3_256, [byte; 32]),
    )))
}

/// Small pools, so patterns hit many rows. Terms that differ only in type
/// tag or kind are deliberately present.
fn subjects() -> Vec<Term> {
    let mut out: Vec<Term> = (0..6).map(|i| iri(&format!("urn:s:{i}"))).collect();
    out.push(Term::typed(2, b"urn:s:0".to_vec()));
    out.push(envelope(7));
    out
}

fn predicates() -> Vec<Term> {
    ["name", "follows", "likes", "cites"]
        .iter()
        .map(|p| iri(p))
        .collect()
}

fn objects() -> Vec<Term> {
    let mut out = subjects();
    out.push(Term::typed(3, vec![0x00, 0xFF, 0x00]));
    out.push(Term::typed(3, vec![]));
    out.push(content(7));
    out.push(content(8));
    out
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

/// Signed records over two graphs, as canonical envelope bytes.
fn signed_records(seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng(seed);
    let (subjects, predicates, objects) = (subjects(), predicates(), objects());
    let count = 4 + rng.below(9);
    (0..count)
        .map(|r| {
            let triples = (0..=rng.below(6))
                .map(|_| {
                    Triple::new(
                        subjects[rng.below(subjects.len())].clone(),
                        predicates[rng.below(predicates.len())].clone(),
                        objects[rng.below(objects.len())].clone(),
                    )
                })
                .collect();
            let graph = LogicalGraphId::new(GRAPHS[rng.below(2)]);
            GraphRecord::new(graph, triples)
                .sign(&format!("ctx/{seed}/{r}"), "node-a", &key())
                .unwrap()
                .envelope
                .canonical_bytes()
                .unwrap()
        })
        .collect()
}

/// One signed triple, as the matcher sees it: typed terms, no bytes.
#[derive(Debug, Clone)]
struct Truth {
    key: AssertionKey,
    graph: [u8; 32],
    triple: Triple,
}

/// Re-opens every envelope (decode, verify, decode payload) and lists its
/// assertions. Envelopes that do not open contribute nothing.
fn evidence(envelopes: &[Vec<u8>]) -> Vec<Truth> {
    let dispatch = ProfileDispatch::default();
    let mut out = Vec::new();
    for bytes in envelopes {
        let Ok(env) = Envelope::decode(bytes) else {
            continue;
        };
        let Ok(opened) = open(env, &dispatch) else {
            continue;
        };
        for a in opened.assertions() {
            out.push(Truth {
                key: AssertionKey {
                    record: *a.record.content_id().digest(),
                    index: a.index,
                },
                graph: *a.graph.as_bytes(),
                triple: a.triple.clone(),
            });
        }
    }
    out
}

fn indexed(t: &Truth) -> IndexedTriple {
    IndexedTriple {
        key: t.key,
        graph: t.graph,
        subject: t.triple.subject.to_bytes().unwrap(),
        predicate: t.triple.predicate.to_bytes().unwrap(),
        object: t.triple.object.to_bytes().unwrap(),
    }
}

// ---------------------------------------------------------------------------
// The independent typed-set matcher.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TypedPattern {
    graph: [u8; 32],
    subject: Option<Term>,
    predicate: Option<Term>,
    object: Option<Term>,
}

fn matches(want: Option<&Term>, got: &Term) -> bool {
    want.is_none_or(|w| w == got)
}

fn matcher(truth: &[Truth], p: &TypedPattern) -> BTreeSet<AssertionKey> {
    truth
        .iter()
        .filter(|t| {
            t.graph == p.graph
                && matches(p.subject.as_ref(), &t.triple.subject)
                && matches(p.predicate.as_ref(), &t.triple.predicate)
                && matches(p.object.as_ref(), &t.triple.object)
        })
        .map(|t| t.key)
        .collect()
}

fn to_index_pattern(p: &TypedPattern) -> Pattern {
    let bytes = |t: &Option<Term>| t.as_ref().map(|t| t.to_bytes().unwrap());
    Pattern {
        graph: p.graph,
        subject: bytes(&p.subject),
        predicate: bytes(&p.predicate),
        object: bytes(&p.object),
    }
}

/// Every fixed/open combination in every graph, with terms taken from the
/// data and from outside it.
fn patterns(truth: &[Truth], seed: u64) -> Vec<TypedPattern> {
    let mut rng = Rng(seed ^ 0xFACE);
    let absent = iri("urn:absent");
    let mut out = Vec::new();
    for graph in GRAPHS {
        for mask in 0..8_u8 {
            for sample in 0..5 {
                let source = if sample == 4 || truth.is_empty() {
                    Triple::new(absent.clone(), absent.clone(), absent.clone())
                } else {
                    truth[rng.below(truth.len())].triple.clone()
                };
                out.push(TypedPattern {
                    graph,
                    subject: (mask & 1 != 0).then(|| source.subject.clone()),
                    predicate: (mask & 2 != 0).then(|| source.predicate.clone()),
                    object: (mask & 4 != 0).then(|| source.object.clone()),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Scanning through continuations.
// ---------------------------------------------------------------------------

fn scan_all(index: &GraphIndex, pattern: &Pattern, limit: usize) -> Vec<IndexedTriple> {
    let (order, prefix) = plan(pattern);
    let mut out: Vec<IndexedTriple> = Vec::new();
    let mut cursor: Option<Continuation> = None;
    loop {
        let page = index.scan(pattern, cursor.as_ref(), limit).unwrap();
        assert_eq!(page.order, order);
        assert!(page.rows.len() <= limit);
        let returned = page.rows.len() as u64;
        if page.complete() {
            assert_eq!(page.cost.scanned_rows, returned);
        } else {
            assert_eq!(page.rows.len(), limit, "a non-final page is full");
            assert_eq!(page.cost.scanned_rows, returned + 1);
        }
        let bytes: u64 = page
            .rows
            .iter()
            .map(|r| row_key(r, order).len() as u64)
            .sum();
        assert!(page.cost.scanned_bytes >= bytes);
        for row in &page.rows {
            assert!(row_key(row, order).starts_with(&prefix));
        }
        out.extend(page.rows);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    // Strictly increasing in the table's order: no repeats, no reordering
    // across page boundaries.
    for pair in out.windows(2) {
        assert!(row_key(&pair[0], order) < row_key(&pair[1], order));
    }
    out
}

fn check_against_matcher(index: &GraphIndex, truth: &[Truth], seed: u64) {
    for p in patterns(truth, seed) {
        let expected = matcher(truth, &p);
        let pattern = to_index_pattern(&p);
        let mut first: Option<Vec<IndexedTriple>> = None;
        for limit in PAGE_SIZES {
            let rows = scan_all(index, &pattern, limit);
            let got: BTreeSet<AssertionKey> = rows.iter().map(|r| r.key).collect();
            assert_eq!(got.len(), rows.len(), "a scan repeated an assertion");
            assert_eq!(got, expected, "pattern {p:?} at page size {limit}");
            match &first {
                None => first = Some(rows),
                Some(f) => assert_eq!(f, &rows, "page size changed the rows"),
            }
        }
    }
}

fn temp_index(dir: &Path, name: &str) -> (PathBuf, GraphIndex) {
    let path = dir.join(name);
    let index = GraphIndex::open(&path).unwrap();
    (path, index)
}

/// An index built by shuffled batched appends after an empty rebuild.
fn appended(path: &Path, truth: &[Truth], seed: u64) -> GraphIndex {
    let index = GraphIndex::open(path).unwrap();
    index.rebuild(std::iter::empty()).unwrap();
    let mut rows: Vec<IndexedTriple> = truth.iter().map(indexed).collect();
    let mut rng = Rng(seed ^ 0x5EED);
    rng.shuffle(&mut rows);
    let mut rest = rows.as_slice();
    while !rest.is_empty() {
        let take = 1 + rng.below(rest.len().min(5));
        index.append(rest[..take].iter().cloned()).unwrap();
        rest = &rest[take..];
    }
    index
}

/// An index rebuilt from re-opened signed envelopes, in shuffled order.
fn rebuilt(path: &Path, envelopes: &[Vec<u8>], seed: u64) -> GraphIndex {
    let mut envelopes = envelopes.to_vec();
    Rng(seed ^ 0xB0B).shuffle(&mut envelopes);
    let index = GraphIndex::open(path).unwrap();
    index
        .rebuild(evidence(&envelopes).iter().map(indexed))
        .unwrap();
    index
}

/// Runs `edit` on the raw redb file under a closed index.
fn tamper(path: &Path, edit: impl FnOnce(&redb::WriteTransaction)) {
    let db = Database::create(path).unwrap();
    let txn = db.begin_write().unwrap();
    edit(&txn);
    txn.commit().unwrap();
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn scans_match_the_typed_matcher(seed in any::<u64>()) {
        let envelopes = signed_records(seed);
        let truth = evidence(&envelopes);
        prop_assert!(!truth.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let a = appended(&dir.path().join("a.redb"), &truth, seed);
        let b = rebuilt(&dir.path().join("b.redb"), &envelopes, seed);

        let distinct: BTreeSet<AssertionKey> = truth.iter().map(|t| t.key).collect();
        let state_a = a.verify().unwrap();
        let state_b = b.verify().unwrap();
        prop_assert_eq!(state_a, state_b);
        prop_assert_eq!(state_a.count, distinct.len() as u64);

        check_against_matcher(&a, &truth, seed);
        check_against_matcher(&b, &truth, seed);
        for p in patterns(&truth, seed) {
            let pattern = to_index_pattern(&p);
            prop_assert_eq!(scan_all(&a, &pattern, 3), scan_all(&b, &pattern, 3));
        }
    }
}

#[test]
fn every_pattern_shape_has_a_leading_prefix() {
    let g = [1; 32];
    let t = |b: u8| Some(vec![b]);
    let cases = [
        ((None, None, None), Order::Spo, 0),
        ((t(1), None, None), Order::Spo, 1),
        ((t(1), t(2), None), Order::Spo, 2),
        ((t(1), t(2), t(3)), Order::Spo, 3),
        ((None, t(2), None), Order::Pos, 1),
        ((None, t(2), t(3)), Order::Pos, 2),
        ((None, None, t(3)), Order::Osp, 1),
        ((t(1), None, t(3)), Order::Osp, 2),
    ];
    for ((subject, predicate, object), order, fixed) in cases {
        let pattern = Pattern {
            graph: g,
            subject,
            predicate,
            object,
        };
        let (got, prefix) = plan(&pattern);
        assert_eq!(got, order);
        // The prefix is the graph plus exactly the fixed terms, each as a
        // two-byte-terminated tuple bytes field: tag, the byte, 00 00.
        assert_eq!(prefix.len(), (1 + 32 + 2) + 4 * fixed);
    }
}

#[test]
fn row_keys_with_an_out_of_range_index_are_corrupt() {
    // An assertion index is a u32; a key carrying a larger int is not a row
    // this index wrote, even though the tuple itself decodes.
    for index in [i64::from(u32::MAX) + 1, -1] {
        let key = TupleEncoder::new()
            .bytes(&[1; 32])
            .bytes(&[2])
            .bytes(&[3])
            .bytes(&[4])
            .bytes(&[5; 32])
            .int(index)
            .build();
        for order in Order::ALL {
            assert!(matches!(
                decode_row(&key, order),
                Err(IndexError::Corrupt { .. })
            ));
        }
    }
}

#[test]
fn rebuild_uses_only_records_that_open() {
    let mut envelopes = signed_records(42);
    let good = evidence(&envelopes);
    // Flip one payload byte of the last envelope: it no longer verifies,
    // so its triples are not signed truth and must not be indexed.
    let last = envelopes.last_mut().unwrap();
    let at = last.len() - 40;
    last[at] ^= 0x01;
    let truth = evidence(&envelopes);
    assert!(truth.len() < good.len());
    let dir = tempfile::tempdir().unwrap();
    let index = rebuilt(&dir.path().join("i.redb"), &envelopes, 42);
    assert_eq!(
        index.verify().unwrap().count,
        truth.iter().map(|t| t.key).collect::<BTreeSet<_>>().len() as u64
    );
    check_against_matcher(&index, &truth, 42);
}

#[test]
fn append_is_idempotent_and_order_independent() {
    let truth = evidence(&signed_records(7));
    let dir = tempfile::tempdir().unwrap();
    let a = appended(&dir.path().join("a.redb"), &truth, 1);
    let b = appended(&dir.path().join("b.redb"), &truth, 2);
    let state = a.verify().unwrap();
    assert_eq!(state, b.verify().unwrap());
    assert_eq!(a.append(truth.iter().map(indexed)).unwrap(), state);
    assert_eq!(a.verify().unwrap(), state);
    let empty = Pattern {
        graph: GRAPHS[0],
        subject: None,
        predicate: None,
        object: None,
    };
    assert_eq!(scan_all(&a, &empty, 4), scan_all(&b, &empty, 4));
}

/// #12011: concurrent appends of distinct rows must each build on the
/// committed count and digest, so none is lost and `verify` agrees with
/// a serial build of the same rows.
#[test]
fn concurrent_appends_keep_meta_consistent() {
    let truth = evidence(&signed_records(11));
    assert!(truth.len() >= 8, "need enough rows to split across threads");
    let dir = tempfile::tempdir().unwrap();
    let index = GraphIndex::open(dir.path().join("c.redb")).unwrap();
    index.rebuild(std::iter::empty()).unwrap();
    let rows: Vec<IndexedTriple> = truth.iter().map(indexed).collect();
    let threads = rows.len().min(24);
    let chunks: Vec<Vec<IndexedTriple>> = (0..threads)
        .map(|t| rows.iter().skip(t).step_by(threads).cloned().collect())
        .collect();
    let gate = std::sync::Barrier::new(threads);
    std::thread::scope(|scope| {
        for chunk in &chunks {
            let (index, gate) = (&index, &gate);
            scope.spawn(move || {
                gate.wait();
                for row in chunk {
                    index.append(std::iter::once(row.clone())).unwrap();
                }
            });
        }
    });
    let state = index.verify().unwrap();
    assert_eq!(state.count, rows.len() as u64);
    let serial = rebuilt(&dir.path().join("s.redb"), &signed_records(11), 3);
    assert_eq!(state, serial.verify().unwrap());
}

type Removal = fn(&redb::WriteTransaction);
type Edit = Box<dyn Fn(&redb::WriteTransaction)>;

#[test]
fn missing_tables_and_meta_make_the_index_unavailable() {
    let envelopes = signed_records(11);
    let truth = evidence(&envelopes);
    let dir = tempfile::tempdir().unwrap();
    let (path, index) = temp_index(dir.path(), "i.redb");
    let any = Pattern {
        graph: GRAPHS[0],
        subject: None,
        predicate: None,
        object: None,
    };

    // Never built.
    assert_eq!(
        index.scan(&any, None, 10),
        Err(IndexError::Unavailable("meta"))
    );
    assert_eq!(index.verify(), Err(IndexError::Unavailable("meta")));
    assert_eq!(
        index.append(truth.iter().map(indexed)),
        Err(IndexError::Unavailable("meta"))
    );
    drop(index);

    let cases: [(&str, Removal, IndexError); 4] = [
        (
            "osp table",
            |t| {
                t.delete_table(OSP).unwrap();
            },
            IndexError::Unavailable("osp"),
        ),
        (
            "pos table",
            |t| {
                t.delete_table(POS).unwrap();
            },
            IndexError::Unavailable("pos"),
        ),
        (
            "spo table",
            |t| {
                t.delete_table(SPO).unwrap();
            },
            IndexError::Unavailable("spo"),
        ),
        (
            "digest row",
            |t| {
                t.open_table(META).unwrap().remove("digest").unwrap();
            },
            IndexError::Unavailable("digest"),
        ),
    ];
    for (what, edit, expected) in cases {
        drop(rebuilt(&path, &envelopes, 11));
        tamper(&path, edit);
        let index = GraphIndex::open(&path).unwrap();
        // Unavailable for every pattern, not only the one whose table went.
        for p in patterns(&truth, 11).iter().take(40) {
            assert_eq!(
                index.scan(&to_index_pattern(p), None, 5),
                Err(expected.clone()),
                "{what}"
            );
        }
        assert_eq!(index.verify(), Err(expected.clone()), "{what}");
        index
            .rebuild(evidence(&envelopes).iter().map(indexed))
            .unwrap();
        index.verify().unwrap();
        check_against_matcher(&index, &truth, 11);
    }
}

#[test]
fn corruption_is_reported_and_rebuild_from_signed_records_recovers() {
    let envelopes = signed_records(23);
    let truth = evidence(&envelopes);
    let victim = indexed(&truth[0]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.redb");

    let ghost = IndexedTriple {
        key: AssertionKey {
            record: [0xEE; 32],
            index: 0,
        },
        ..victim.clone()
    };
    let altered = IndexedTriple {
        object: content(0x55).to_bytes().unwrap(),
        ..victim.clone()
    };
    let mut garbage = row_key(&victim, Order::Spo)[..36].to_vec();
    garbage.extend_from_slice(&[0x15, 0xFF, 0x77]);

    let row = |table, key: Vec<u8>, insert: bool| -> Edit {
        Box::new(move |t: &redb::WriteTransaction| {
            let mut table = t.open_table(table).unwrap();
            if insert {
                table.insert(key.as_slice(), ()).unwrap();
            } else {
                table.remove(key.as_slice()).unwrap();
            }
        })
    };
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "extra pos row",
            row(POS, row_key(&ghost, Order::Pos), true),
            "index_corrupt",
        ),
        (
            "missing osp row",
            row(OSP, row_key(&victim, Order::Osp), false),
            "index_corrupt",
        ),
        (
            "missing spo row",
            row(SPO, row_key(&victim, Order::Spo), false),
            "index_corrupt",
        ),
        (
            "altered spo row",
            Box::new({
                let (old, new) = (row_key(&victim, Order::Spo), row_key(&altered, Order::Spo));
                move |t: &redb::WriteTransaction| {
                    let mut table = t.open_table(SPO).unwrap();
                    table.remove(old.as_slice()).unwrap();
                    table.insert(new.as_slice(), ()).unwrap();
                }
            }),
            "index_corrupt",
        ),
        (
            "undecodable spo row",
            row(SPO, garbage.clone(), true),
            "index_corrupt",
        ),
        (
            "count row",
            Box::new(|t: &redb::WriteTransaction| {
                t.open_table(META)
                    .unwrap()
                    .insert("count", 1_u64.to_be_bytes().as_slice())
                    .unwrap();
            }),
            "index_corrupt",
        ),
        (
            "short count row",
            Box::new(|t: &redb::WriteTransaction| {
                t.open_table(META)
                    .unwrap()
                    .insert("count", [1_u8; 3].as_slice())
                    .unwrap();
            }),
            "index_corrupt",
        ),
        (
            "schema row",
            Box::new(|t: &redb::WriteTransaction| {
                t.open_table(META)
                    .unwrap()
                    .insert("schema", 2_u64.to_be_bytes().as_slice())
                    .unwrap();
            }),
            "unsupported_schema",
        ),
    ];
    for (what, edit, class) in cases {
        drop(rebuilt(&path, &envelopes, 23));
        tamper(&path, &*edit);
        let index = GraphIndex::open(&path).unwrap();
        let err = index.verify().unwrap_err();
        assert_eq!(err.class(), class, "{what}: {err}");
        index
            .rebuild(evidence(&envelopes).iter().map(indexed))
            .unwrap();
        index.verify().unwrap();
        check_against_matcher(&index, &truth, 23);
    }

    // A scan that reaches an undecodable row reports it rather than
    // skipping it or returning a wrong row.
    drop(rebuilt(&path, &envelopes, 23));
    tamper(&path, &*row(SPO, garbage, true));
    let index = GraphIndex::open(&path).unwrap();
    let graph = Pattern {
        graph: victim.graph,
        subject: None,
        predicate: None,
        object: None,
    };
    let err = index.scan(&graph, None, ROW_CEILING).unwrap_err();
    assert_eq!(err.class(), "index_corrupt");
    // Unsupported schema also stops scans.
    drop(index);
    drop(rebuilt(&path, &envelopes, 23));
    tamper(&path, |t| {
        t.open_table(META)
            .unwrap()
            .insert("schema", 2_u64.to_be_bytes().as_slice())
            .unwrap();
    });
    let index = GraphIndex::open(&path).unwrap();
    assert_eq!(
        index.scan(&graph, None, 5),
        Err(IndexError::UnsupportedSchema(2))
    );
}

#[test]
fn continuations_and_limits_are_bounded() {
    let envelopes = signed_records(5);
    let truth = evidence(&envelopes);
    let dir = tempfile::tempdir().unwrap();
    let index = rebuilt(&dir.path().join("i.redb"), &envelopes, 5);
    let g = truth[0].graph;
    let all = Pattern {
        graph: g,
        subject: None,
        predicate: None,
        object: None,
    };
    let by_subject = Pattern {
        subject: Some(truth[0].triple.subject.to_bytes().unwrap()),
        ..all.clone()
    };
    let other_graph = Pattern {
        graph: [0x99; 32],
        ..all.clone()
    };

    for limit in [0, ROW_CEILING + 1] {
        assert_eq!(
            index.scan(&all, None, limit),
            Err(IndexError::LimitOutOfRange(limit))
        );
    }
    index.scan(&all, None, ROW_CEILING).unwrap();

    let page = index.scan(&all, None, 1).unwrap();
    let cursor = page.next.expect("more than one row in the graph");
    // The continuation resumes its own scan only.
    // A narrower pattern whose prefix still contains the continuation's
    // last key: only the pattern fingerprint tells them apart.
    let narrower = Pattern {
        subject: Some(page.rows[0].subject.clone()),
        ..all.clone()
    };
    assert_eq!(
        index.scan(&narrower, Some(&cursor), 1),
        Err(IndexError::CursorMismatch)
    );
    let next = index.scan(&all, Some(&cursor), 1).unwrap();
    assert_ne!(next.rows, page.rows);
    for other in [&by_subject, &other_graph] {
        assert_eq!(
            index.scan(other, Some(&cursor), 1),
            Err(IndexError::CursorMismatch)
        );
    }
    for bytes in [
        vec![],
        vec![0x05],
        cursor.as_bytes()[..cursor.as_bytes().len() - 3].to_vec(),
    ] {
        let forged = Continuation::from_bytes(bytes);
        assert_eq!(
            index.scan(&all, Some(&forged), 1),
            Err(IndexError::CursorMismatch)
        );
    }
    // A continuation re-encoded with its last key moved to another graph:
    // the key lies outside the pattern's prefix.
    let fields = decode_tuple(cursor.as_bytes()).unwrap();
    let [
        TupleValue::Int(schema),
        TupleValue::Int(order),
        TupleValue::Bytes(print),
        TupleValue::Bytes(last),
    ] = <[TupleValue; 4]>::try_from(fields).unwrap()
    else {
        panic!("continuation layout changed");
    };
    let mut moved = last;
    moved[1] ^= 0x01;
    let outside = TupleEncoder::new()
        .int(schema)
        .int(order)
        .bytes(&print)
        .bytes(&moved)
        .build();
    assert_eq!(
        index.scan(&all, Some(&Continuation::from_bytes(outside)), 1),
        Err(IndexError::CursorMismatch)
    );
    // The same re-encoding with the key unchanged still resumes.
    let fields = decode_tuple(cursor.as_bytes()).unwrap();
    let [_, _, _, TupleValue::Bytes(last)] = <[TupleValue; 4]>::try_from(fields).unwrap() else {
        panic!("continuation layout changed");
    };
    let same = TupleEncoder::new()
        .int(schema)
        .int(order)
        .bytes(&print)
        .bytes(&last)
        .build();
    assert_eq!(
        index
            .scan(&all, Some(&Continuation::from_bytes(same)), 1)
            .unwrap(),
        next
    );
}
