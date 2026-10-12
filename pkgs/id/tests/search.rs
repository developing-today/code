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

use id::chain::{ChainExt, Event, sign};
use id::documents::{DOCUMENT_KIND, DocumentRef};
use id::event_store::{EventStore, Ingest};
use id::search::{Hit, ValueMatch, search, search_store};
use id::tag_claims::{CLEAR_KIND, Claim, SET_KIND, TagOp};
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

    assert!(
        search_store(&store, "city", ValueMatch::Any)
            .unwrap()
            .is_empty()
    );
    assert!(
        search_store(&store, "city", ValueMatch::Exact("Seattle"))
            .unwrap()
            .is_empty()
    );
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
    assert_eq!(
        shape(&any),
        vec![(s, vec![bare.id]), (t, vec![empty.id])],
        "any value returns both subjects, in subject order"
    );
}

#[test]
fn test_a_value_and_a_name_match_as_whole_fields_so_a_does_not_match_a_nul_b() {
    let (s, t, u, v) = (
        Hash::digest(b"s"),
        Hash::digest(b"t"),
        Hash::digest(b"u"),
        Hash::digest(b"v"),
    );
    let mut a = Writer::new(1);
    let short = a.set(s, "city", Some("a"), vec![]);
    let nul = a.set(t, "city", Some("a\0b"), vec![short.id]);
    let longer = a.set(u, "city", Some("ab"), vec![nul.id]);
    let short_name = a.set(v, "a", None, vec![longer.id]);
    let nul_name = a.set(v, "a\0b", None, vec![short_name.id]);

    let (_dir, mut store) = open();
    for event in [&short, &nul, &longer, &short_name, &nul_name] {
        deliver(&mut store, event);
    }

    let value_a = search_store(&store, "city", ValueMatch::Exact("a")).unwrap();
    assert_eq!(shape(&value_a), vec![(s, vec![short.id])]);
    let value_nul = search_store(&store, "city", ValueMatch::Exact("a\0b")).unwrap();
    assert_eq!(shape(&value_nul), vec![(t, vec![nul.id])]);
    let name_a = search_store(&store, "a", ValueMatch::Any).unwrap();
    assert_eq!(shape(&name_a), vec![(v, vec![short_name.id])]);
    let name_nul = search_store(&store, "a\0b", ValueMatch::Any).unwrap();
    assert_eq!(shape(&name_nul), vec![(v, vec![nul_name.id])]);
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
    let first = &got[0].1;
    assert_eq!(first.len(), 2, "both claims on the first subject");
    assert!(
        first
            .windows(2)
            .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
    );
}

#[test]
fn test_a_search_matches_nothing_on_an_empty_store_or_an_unknown_name() {
    let (_dir, store) = open();
    assert!(
        search_store(&store, "city", ValueMatch::Any)
            .unwrap()
            .is_empty()
    );

    let mut a = Writer::new(1);
    let claim = a.set(Hash::digest(b"s"), "city", Some("Seattle"), vec![]);
    let (_dir2, mut store) = open();
    deliver(&mut store, &claim);
    assert!(
        search_store(&store, "state", ValueMatch::Any)
            .unwrap()
            .is_empty()
    );
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
    let mut writers: Vec<Writer> = (0..WRITERS).map(|n| Writer::new(n as u8 + 1)).collect();
    let mut events: Vec<Hash> = Vec::new();
    let mut claims: Vec<Hash> = Vec::new();
    for &(clear, writer, subject, name, value, reach) in steps {
        let after: Vec<Hash> = events.iter().rev().take(reach).copied().collect();
        let event = if clear && !claims.is_empty() {
            writers[writer].clear(&[claims[value % claims.len()]], after)
        } else {
            let subject = Hash::digest([subject as u8]);
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
