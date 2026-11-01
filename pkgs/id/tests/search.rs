//! Search by tag name and value over the live claims (Phase 5, P5.5; design §9.4).
//!
//! A search takes a name and a value match, and returns each subject with the live claims that
//! match. The subjects are event IDs: identities, and documents, whose `document.put` event ID is
//! the subject their tags name. The result is ordered by subject ID bytes, and each subject's
//! claims by claim ID bytes:
//!
//! 1. A contested tag, with two different values both live, returns both claims.
//! 2. A cleared claim is not returned, and a concurrent add the clear did not see is.
//! 3. A value matches as a whole field. The value `a` does not match `a\0b`, and the name `a`
//!    does not match the name `a\0b`. The 00 00 terminator makes that so.
//! 4. A bare tag matches only the bare search, and the empty value matches only itself.
//! 5. For random histories, the search equals a filter over the live set from
//!    [`EventStore::tag_claims`].

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::chain::{sign, ChainExt, Event};
use id::documents::{DocumentRef, DOCUMENT_KIND};
use id::event_store::{EventStore, Ingest};
use id::search::{search, search_store, search_store_terms, search_terms, Hit, Term, ValueMatch};
use id::tag_claims::{Claim, TagOp, CLEAR_KIND, SET_KIND};
use id::waiting::WaitingLimits;
use p2panda_core::{Hash, SeqNum, SigningKey};
use proptest::prelude::*;

const WRITERS: usize = 3;
const SUBJECTS: usize = 3;
const NAMES: [&str; 3] = ["a", "a\0b", "city"];
const CLAIM_VALUES: [Option<&str>; 5] = [None, Some("a"), Some("a\0b"), Some("ab"), Some("")];
const QUERY_VALUES: [ValueMatch<'static>; 6] = [
    ValueMatch::Any,
    ValueMatch::Bare,
    ValueMatch::Exact("a"),
    ValueMatch::Exact("a\0b"),
    ValueMatch::Exact("ab"),
    ValueMatch::Exact(""),
];

/// A signer with its own chain, as the tag tests have it.
struct Writer {
    key: SigningKey,
    seq: SeqNum,
    last: Option<Hash>,
}

impl Writer {
    fn new(n: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[n; 32]),
            seq: 0,
            last: None,
        }
    }

    fn send(&mut self, ext: ChainExt) -> Event {
        let event = sign(&self.key, self.seq, self.last, None, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }

    fn set(&mut self, subject: Hash, name: &str, value: Option<&str>, after: Vec<Hash>) -> Event {
        let op = TagOp::Set {
            subject,
            name: name.to_owned(),
            value: value.map(str::to_owned),
        };
        self.send(tag_ext(SET_KIND, op, after))
    }

    fn clear(&mut self, claims: &[Hash], after: Vec<Hash>) -> Event {
        let op = TagOp::Clear {
            claims: claims.to_vec(),
        };
        self.send(tag_ext(CLEAR_KIND, op, after))
    }

    /// A `document.put`. Its event ID is the document's subject.
    fn document(&mut self, bytes: &[u8]) -> Event {
        let document = DocumentRef::for_bytes("text/markdown", bytes).unwrap();
        self.send(ChainExt {
            kind: DOCUMENT_KIND.to_owned(),
            group: None,
            after: vec![],
            tag: None,
            document: Some(document),
            removes: None,
        })
    }
}

fn tag_ext(kind: &str, op: TagOp, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: kind.to_owned(),
        group: None,
        after,
        tag: Some(op),
        document: None,
        removes: None,
    }
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

/// Delivers one event's bytes, as sync does, and checks that it was not refused.
fn deliver(store: &mut EventStore, event: &Event) {
    let outcome = store.ingest(&event.bytes, 0).unwrap();
    assert!(
        !matches!(outcome, Ingest::Refused(_)),
        "refused: {outcome:?}"
    );
}

/// The subjects of `hits` with their claims' IDs, in the order the search returns them.
fn shape(hits: &[Hit]) -> Vec<(Hash, Vec<Hash>)> {
    hits.iter()
        .map(|hit| {
            (
                hit.subject,
                hit.claims.iter().map(|claim| claim.id).collect(),
            )
        })
        .collect()
}

fn fits(value: Option<&str>, query: ValueMatch<'_>) -> bool {
    match query {
        ValueMatch::Any => true,
        ValueMatch::Bare => value.is_none(),
        ValueMatch::Exact(text) => value == Some(text),
    }
}

/// The reference: a filter over the live set, grouped by subject, in the order the search
/// promises.
fn expected(live: &[Claim], name: &str, query: ValueMatch<'_>) -> Vec<(Hash, Vec<Hash>)> {
    let mut by_subject: BTreeMap<[u8; 32], (Hash, Vec<Hash>)> = BTreeMap::new();
    for claim in live {
        if claim.name == name && fits(claim.value.as_deref(), query) {
            by_subject
                .entry(*claim.subject.as_bytes())
                .or_insert_with(|| (claim.subject, Vec::new()))
                .1
                .push(claim.id);
        }
    }
    by_subject
        .into_values()
        .map(|(subject, mut ids)| {
            ids.sort_by_key(|id| *id.as_bytes());
            (subject, ids)
        })
        .collect()
}

fn sorted_by_subject(mut subjects: Vec<Hash>) -> Vec<Hash> {
    subjects.sort_by_key(|subject| *subject.as_bytes());
    subjects
}

#[test]
fn test_a_search_returns_each_identity_and_document_with_its_matching_claim() {
    let mut author = Writer::new(1);
    let document = author.document(b"minutes");
    let identity = Hash::digest(b"identity");
    let on_document = author.set(document.id, "city", Some("Seattle"), vec![document.id]);
    let on_identity = author.set(identity, "city", Some("Seattle"), vec![on_document.id]);
    let elsewhere = author.set(identity, "name", None, vec![on_identity.id]);

    let (_dir, mut store) = open();
    for event in [&document, &on_document, &on_identity, &elsewhere] {
        deliver(&mut store, event);
    }

    let hits = search_store(&store, "city", ValueMatch::Exact("Seattle")).unwrap();
    let subjects: Vec<Hash> = hits.iter().map(|hit| hit.subject).collect();
    assert_eq!(subjects, sorted_by_subject(vec![document.id, identity]));
    assert_eq!(
        shape(&hits),
        expected(
            &store.tag_claims().unwrap(),
            "city",
            ValueMatch::Exact("Seattle")
        ),
    );
    // The pure search over the same live claims gives the same answer.
    let live = store.tag_claims().unwrap();
    assert_eq!(
        shape(&search(&live, "city", ValueMatch::Exact("Seattle"))),
        shape(&hits)
    );
    assert_eq!(
        hits[0].claims[0].author,
        *author.key.verifying_key().as_bytes()
    );
}

#[test]
fn test_a_contested_tag_returns_both_values() {
    let subject = Hash::digest(b"identity");
    let mut a = Writer::new(1);
    let seattle = a.set(subject, "city", Some("Seattle"), vec![]);
    let mut b = Writer::new(2);
    // `b` did not see `seattle`, so both claims are live.
    let portland = b.set(subject, "city", Some("Portland"), vec![]);

    let (_dir, mut store) = open();
    deliver(&mut store, &seattle);
    deliver(&mut store, &portland);

    let hits = search_store(&store, "city", ValueMatch::Any).unwrap();
    assert_eq!(
        shape(&hits),
        vec![(subject, sorted_ids(vec![seattle.id, portland.id]))]
    );
    let only_seattle = search_store(&store, "city", ValueMatch::Exact("Seattle")).unwrap();
    assert_eq!(shape(&only_seattle), vec![(subject, vec![seattle.id])]);
    let only_portland = search_store(&store, "city", ValueMatch::Exact("Portland")).unwrap();
    assert_eq!(shape(&only_portland), vec![(subject, vec![portland.id])]);
}

#[test]
fn test_a_cleared_claim_is_not_returned() {
    let subject = Hash::digest(b"identity");
    let mut a = Writer::new(1);
    let seen = a.set(subject, "city", Some("Seattle"), vec![]);
    let kept = a.set(subject, "name", None, vec![seen.id]);
    let mut b = Writer::new(2);
    let cleared = b.clear(&[seen.id], vec![kept.id]);

    let (_dir, mut store) = open();
    for event in [&seen, &kept, &cleared] {
        deliver(&mut store, event);
    }

    assert!(search_store(&store, "city", ValueMatch::Any)
        .unwrap()
        .is_empty());
    assert!(search_store(&store, "city", ValueMatch::Exact("Seattle"))
        .unwrap()
        .is_empty());
    // The clear removes only the claim it names.
    let names = search_store(&store, "name", ValueMatch::Any).unwrap();
    assert_eq!(shape(&names), vec![(subject, vec![kept.id])]);
}

#[test]
fn test_a_concurrent_add_survives_a_clear_that_did_not_see_it() {
    let subject = Hash::digest(b"identity");
    let mut a = Writer::new(1);
    let seen = a.set(subject, "city", Some("Seattle"), vec![]);
    let mut b = Writer::new(2);
    let unseen_clear = b.clear(&[seen.id], vec![]);

    let (_dir, mut store) = open();
    for event in [&seen, &unseen_clear] {
        deliver(&mut store, event);
    }

    let hits = search_store(&store, "city", ValueMatch::Any).unwrap();
    assert_eq!(shape(&hits), vec![(subject, vec![seen.id])]);
}

#[test]
fn test_a_bare_tag_matches_only_the_bare_search_and_an_empty_value_only_itself() {
    let (s, t) = (Hash::digest(b"s"), Hash::digest(b"t"));
    let mut a = Writer::new(1);
    let bare = a.set(s, "flag", None, vec![]);
    let empty = a.set(t, "flag", Some(""), vec![bare.id]);

    let (_dir, mut store) = open();
    for event in [&bare, &empty] {
        deliver(&mut store, event);
    }

    let bare_hits = search_store(&store, "flag", ValueMatch::Bare).unwrap();
    assert_eq!(shape(&bare_hits), vec![(s, vec![bare.id])]);
    let empty_hits = search_store(&store, "flag", ValueMatch::Exact("")).unwrap();
    assert_eq!(shape(&empty_hits), vec![(t, vec![empty.id])]);
    let any = search_store(&store, "flag", ValueMatch::Any).unwrap();
    let mut both = vec![(s, vec![bare.id]), (t, vec![empty.id])];
    both.sort_by_key(|(subject, _)| *subject.as_bytes());
    assert_eq!(
        shape(&any),
        both,
        "any value returns both subjects, in subject order"
    );
}

#[test]
fn test_a_value_and_a_name_match_as_whole_fields_so_a_does_not_match_a_nul_b() {
    let (one, two, three, four) = (
        Hash::digest(b"s"),
        Hash::digest(b"t"),
        Hash::digest(b"u"),
        Hash::digest(b"v"),
    );
    let mut a = Writer::new(1);
    let short = a.set(one, "city", Some("a"), vec![]);
    let nul = a.set(two, "city", Some("a\0b"), vec![short.id]);
    let longer = a.set(three, "city", Some("ab"), vec![nul.id]);
    let short_name = a.set(four, "a", None, vec![longer.id]);
    let nul_name = a.set(four, "a\0b", None, vec![short_name.id]);

    let (_dir, mut store) = open();
    for event in [&short, &nul, &longer, &short_name, &nul_name] {
        deliver(&mut store, event);
    }

    let value_a = search_store(&store, "city", ValueMatch::Exact("a")).unwrap();
    assert_eq!(shape(&value_a), vec![(one, vec![short.id])]);
    let value_nul = search_store(&store, "city", ValueMatch::Exact("a\0b")).unwrap();
    assert_eq!(shape(&value_nul), vec![(two, vec![nul.id])]);
    let name_a = search_store(&store, "a", ValueMatch::Any).unwrap();
    assert_eq!(shape(&name_a), vec![(four, vec![short_name.id])]);
    let name_nul = search_store(&store, "a\0b", ValueMatch::Any).unwrap();
    assert_eq!(shape(&name_nul), vec![(four, vec![nul_name.id])]);
}

#[test]
fn test_hits_are_ordered_by_subject_then_claim_and_do_not_depend_on_delivery_order() {
    let mut a = Writer::new(1);
    let mut b = Writer::new(2);
    let subjects: Vec<Hash> = (0..5u8).map(|n| Hash::digest([n])).collect();
    let mut events: Vec<Event> = subjects
        .iter()
        .map(|subject| a.set(*subject, "city", Some("Seattle"), vec![]))
        .collect();
    // A second, concurrent claim on the first subject.
    events.push(b.set(subjects[0], "city", Some("Seattle"), vec![]));

    let (_dir, mut forward) = open();
    for event in &events {
        deliver(&mut forward, event);
    }
    let (_dir2, mut reverse) = open();
    for event in events.iter().rev() {
        deliver(&mut reverse, event);
    }

    let got = shape(&search_store(&forward, "city", ValueMatch::Any).unwrap());
    assert_eq!(
        got,
        shape(&search_store(&reverse, "city", ValueMatch::Any).unwrap())
    );
    let order: Vec<Hash> = got.iter().map(|(subject, _)| *subject).collect();
    assert_eq!(order, sorted_by_subject(subjects.clone()));
    let on_first = got
        .iter()
        .find(|(subject, _)| *subject == subjects[0])
        .unwrap();
    assert_eq!(on_first.1.len(), 2, "both claims on the first subject");
    assert!(on_first
        .1
        .windows(2)
        .all(|pair| pair[0].as_bytes() < pair[1].as_bytes()));
}

#[test]
fn test_a_search_matches_nothing_on_an_empty_store_or_an_unknown_name() {
    let (_dir, store) = open();
    assert!(search_store(&store, "city", ValueMatch::Any)
        .unwrap()
        .is_empty());

    let mut a = Writer::new(1);
    let claim = a.set(Hash::digest(b"s"), "city", Some("Seattle"), vec![]);
    let (_dir2, mut store) = open();
    deliver(&mut store, &claim);
    assert!(search_store(&store, "state", ValueMatch::Any)
        .unwrap()
        .is_empty());
}

/// The live claims' IDs, sorted by ID bytes.
fn sorted_ids(mut ids: Vec<Hash>) -> Vec<Hash> {
    ids.sort_by_key(|id| *id.as_bytes());
    ids
}

/// One random step: a clear or a set, the writer, the subject, the name, the value, and how many
/// recent events the author names in `after`.
type Step = (bool, usize, usize, usize, usize, usize);

fn step_strategy() -> impl Strategy<Value = Step> {
    (
        any::<bool>(),
        0..WRITERS,
        0..SUBJECTS,
        0..NAMES.len(),
        0..CLAIM_VALUES.len(),
        0..4usize,
    )
}

/// Builds a store from random steps. Each event is delivered as it is written, so every `after`
/// names an event the store holds.
fn build(steps: &[Step]) -> (tempfile::TempDir, EventStore) {
    let (dir, mut store) = open();
    let mut writers: Vec<Writer> = (0..WRITERS)
        .map(|n| Writer::new(u8::try_from(n).unwrap() + 1))
        .collect();
    let mut events: Vec<Hash> = Vec::new();
    let mut claims: Vec<Hash> = Vec::new();
    for &(clear, writer, subject, name, value, reach) in steps {
        let after: Vec<Hash> = events.iter().rev().take(reach).copied().collect();
        let event = if clear && !claims.is_empty() {
            writers[writer].clear(&[claims[value % claims.len()]], after)
        } else {
            let subject = Hash::digest([u8::try_from(subject).unwrap()]);
            let set = writers[writer].set(subject, NAMES[name], CLAIM_VALUES[value], after);
            claims.push(set.id);
            set
        };
        events.push(event.id);
        deliver(&mut store, &event);
    }
    (dir, store)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

    /// For random tag histories, the search equals a filter over the live set.
    #[test]
    fn test_the_search_equals_a_filter_over_the_live_set(
        steps in prop::collection::vec(step_strategy(), 0..24),
        name in 0..NAMES.len(),
        query in 0..QUERY_VALUES.len(),
    ) {
        let (_dir, store) = build(&steps);
        let live = store.tag_claims().unwrap();
        let (name, query) = (NAMES[name], QUERY_VALUES[query]);
        prop_assert_eq!(
            shape(&search_store(&store, name, query).unwrap()),
            expected(&live, name, query)
        );
    }
}

#[test]
fn test_search_by_author_returns_only_that_authors_claims() {
    let subject = Hash::digest(b"identity");
    let mut a = Writer::new(1);
    let seattle = a.set(subject, "city", Some("Seattle"), vec![]);
    let mut b = Writer::new(2);
    let portland = b.set(subject, "city", Some("Portland"), vec![]);

    let (_dir, mut store) = open();
    deliver(&mut store, &seattle);
    deliver(&mut store, &portland);

    let a_key = *a.key.verifying_key().as_bytes();
    let b_key = *b.key.verifying_key().as_bytes();
    let hits = search_store_terms(&store, &[Term::Author(a_key)]).unwrap();
    assert_eq!(shape(&hits), vec![(subject, vec![seattle.id])]);
    let hits = search_store_terms(&store, &[Term::Author(b_key)]).unwrap();
    assert_eq!(shape(&hits), vec![(subject, vec![portland.id])]);
    // The pure search over the same live claims gives the same answer.
    let live = store.tag_claims().unwrap();
    assert_eq!(
        shape(&search_terms(&live, &[Term::Author(a_key)], |_| None)),
        vec![(subject, vec![seattle.id])]
    );
    // An author with no claims matches nothing.
    assert!(search_store_terms(&store, &[Term::Author([9; 32])])
        .unwrap()
        .is_empty());
}

#[test]
fn test_search_terms_and_combines_terms_on_a_subject() {
    let one = Hash::digest(b"one");
    let two = Hash::digest(b"two");
    let mut a = Writer::new(1);
    let city_one = a.set(one, "city", Some("Seattle"), vec![]);
    let mut b = Writer::new(2);
    let name_one = b.set(one, "name", None, vec![]);
    let city_two = b.set(two, "city", Some("Seattle"), vec![]);

    let (_dir, mut store) = open();
    for event in [&city_one, &name_one, &city_two] {
        deliver(&mut store, event);
    }

    // Name AND author: only `one` has a Seattle claim by `a`; the hit carries
    // the claim that matched a term, not `b`'s unrelated name claim.
    let a_key = *a.key.verifying_key().as_bytes();
    let hits = search_store_terms(
        &store,
        &[
            Term::Name {
                name: "city",
                value: ValueMatch::Exact("Seattle"),
            },
            Term::Author(a_key),
        ],
    )
    .unwrap();
    assert_eq!(shape(&hits), vec![(one, vec![city_one.id])]);

    // Two name terms may be satisfied by different claims on the subject.
    let hits = search_store_terms(
        &store,
        &[
            Term::Name {
                name: "city",
                value: ValueMatch::Any,
            },
            Term::Name {
                name: "name",
                value: ValueMatch::Any,
            },
        ],
    )
    .unwrap();
    assert_eq!(
        shape(&hits),
        vec![(one, sorted_ids(vec![city_one.id, name_one.id]))]
    );

    // A subject missing one term is not a hit, and an empty term set matches
    // every subject with all of its claims.
    let hits = search_store_terms(&store, &[]).unwrap();
    let mut want = vec![
        (one, sorted_ids(vec![city_one.id, name_one.id])),
        (two, vec![city_two.id]),
    ];
    want.sort_by_key(|(subject, _)| *subject.as_bytes());
    assert_eq!(shape(&hits), want);
}

#[test]
fn test_search_by_subject_kind_matches_only_events_of_that_kind() {
    let mut author = Writer::new(1);
    let document = author.document(b"minutes");
    let identity = Hash::digest(b"identity");
    let on_document = author.set(document.id, "city", Some("Seattle"), vec![document.id]);
    let on_identity = author.set(identity, "city", Some("Seattle"), vec![on_document.id]);

    let (_dir, mut store) = open();
    for event in [&document, &on_document, &on_identity] {
        deliver(&mut store, event);
    }

    // Kind alone: the document subject matches, with all of its live claims;
    // the identity subject is not an event in the store, so it has no kind.
    let hits = search_store_terms(&store, &[Term::SubjectKind(DOCUMENT_KIND)]).unwrap();
    assert_eq!(shape(&hits), vec![(document.id, vec![on_document.id])]);

    // Kind AND name: both terms must hold on the same subject.
    let hits = search_store_terms(
        &store,
        &[
            Term::SubjectKind(DOCUMENT_KIND),
            Term::Name {
                name: "city",
                value: ValueMatch::Exact("Seattle"),
            },
        ],
    )
    .unwrap();
    assert_eq!(shape(&hits), vec![(document.id, vec![on_document.id])]);
    let hits = search_store_terms(
        &store,
        &[
            Term::SubjectKind(DOCUMENT_KIND),
            Term::Name {
                name: "missing",
                value: ValueMatch::Any,
            },
        ],
    )
    .unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_search_terms_resolves_subject_kind_at_most_once_per_subject() {
    use std::cell::Cell;
    use std::collections::HashMap as Map;

    // Three subjects, each with one live claim.
    let mut author = Writer::new(1);
    let one = Hash::digest(b"counting-one");
    let two = Hash::digest(b"counting-two");
    let three = Hash::digest(b"counting-three");
    let on_one = author.set(one, "city", Some("Seattle"), vec![]);
    let on_two = author.set(two, "city", Some("Seattle"), vec![on_one.id]);
    let on_three = author.set(three, "city", Some("Seattle"), vec![on_two.id]);

    let (_dir, mut store) = open();
    for event in [&on_one, &on_two, &on_three] {
        deliver(&mut store, event);
    }
    let live = store.tag_claims().unwrap();

    // Subjects one and two resolve to a kind; three resolves to none.
    let kinds: Map<Hash, Option<String>> = [
        (one, Some("kind-a".to_string())),
        (two, Some("kind-a".to_string())),
        (three, None),
    ]
    .into_iter()
    .collect();
    let calls = Cell::new(0usize);
    let lookup = |subject: &Hash| {
        calls.set(calls.get() + 1);
        kinds.get(subject).cloned().unwrap_or(None)
    };

    // Three kind terms (one duplicated) over three subjects: without the memo
    // that is nine lookups; the documented contract is at most one per subject.
    let hits = search_terms(
        &live,
        &[
            Term::SubjectKind("kind-a"),
            Term::SubjectKind("kind-a"),
            Term::SubjectKind("kind-b"),
        ],
        &lookup,
    );
    assert_eq!(calls.get(), 3);
    // AND semantics are unchanged: no subject is both kind-a and kind-b.
    assert!(hits.is_empty());

    // A single kind term gives the same subjects as before, still one call each.
    calls.set(0);
    let hits = search_terms(&live, &[Term::SubjectKind("kind-a")], &lookup);
    assert_eq!(calls.get(), 3);
    assert_eq!(
        shape(&hits),
        vec![(one, vec![on_one.id]), (two, vec![on_two.id])]
    );
}

#[test]
fn test_search_terms_without_kind_succeeds_even_when_subject_event_is_missing() {
    let mut author = Writer::new(1);
    // Subject is a raw hash whose event was never delivered to the store.
    let uningested_subject = Hash::digest(b"nonexistent-event-subject");
    let on_nonexistent = author.set(uningested_subject, "city", Some("Seattle"), vec![]);

    let (_dir, mut store) = open();
    deliver(&mut store, &on_nonexistent);

    // Verify the subject's event is indeed missing in the store.
    assert!(store.event_by_id(&uningested_subject).unwrap().is_none());

    // Searching by name/value or author must succeed without kind terms.
    let a_key = *author.key.verifying_key().as_bytes();
    let hits = search_store_terms(
        &store,
        &[
            Term::Name {
                name: "city",
                value: ValueMatch::Exact("Seattle"),
            },
            Term::Author(a_key),
        ],
    )
    .unwrap();
    assert_eq!(
        shape(&hits),
        vec![(uningested_subject, vec![on_nonexistent.id])]
    );

    // Empty term set must also succeed without querying subject kinds.
    let hits = search_store_terms(&store, &[]).unwrap();
    assert_eq!(
        shape(&hits),
        vec![(uningested_subject, vec![on_nonexistent.id])]
    );

    // Adding a kind term requires resolving the kind, so the uningested subject won't match.
    let hits = search_store_terms(&store, &[Term::SubjectKind(DOCUMENT_KIND)]).unwrap();
    assert!(hits.is_empty());
}
