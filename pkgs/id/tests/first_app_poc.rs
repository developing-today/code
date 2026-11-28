//! First ID app PoC (#3790, Phase 3 of #2031): sticky notes fused with the
//! did-it tracker in one shared group timeline, over the event store.
//!
//! The oracle is #2031's PoC bar, verbatim: two identities exchange notes
//! and did-it events; one identity's view correctly excludes a third party's
//! claims while keeping them retrievable on demand. The pieces composed
//! here are the already-landed library slices — documents + tag claims for
//! notes, the did-it schema/evaluator (#3788), writer-set projection
//! (#3789), and generic event-body persistence (#4837) — no app-local event
//! shapes and no new protocol.
//!
//! Notes are `document.put` events in the shared group; a logical note is
//! the `note.id` tag on each revision and an edit is a new document linked
//! by a `note.supersedes` tag, so concurrent heads would stay visible (this
//! scenario's edit is sequential). The did-it feed is `didit.*` events in
//! the same timeline; the counted view is the writer-set projection: only
//! records whose author is in A's writer set reach the pure evaluator.
//! C's record stays stored, is listed as not counted with its author, and
//! its body is retrieved on demand from A's own store — including after a
//! restart. A control evaluation over *all* records shows C's claim would
//! have satisfied the expectation had it been counted, which is what makes
//! the exclusion a real exclusion and not an absence.
//!
//! This is a proof of concept. No permanent feature is built on it without
//! a review round (#2031); see `examples/first-app-poc/README.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use id::{
    chain::{ChainExt, Event, sign},
    didit::{
        ActiveWindow, ENTITY_KIND, EXPECTATION_KIND, Entity, Expectation, HoursSpan, Period,
        RECORD_KIND, Record, VerdictState, Window as DiditWindow, decode_record_body,
        encode_entity_body, encode_expectation_body, encode_record_body, evaluate,
    },
    documents::{DOCUMENT_KIND, DocumentRef},
    event_store::{EventStore, Ingest},
    idset::{Entry, Excludes, WriterSet, view::expand_set},
    tag_claims::{SET_KIND, TagOp},
    waiting::WaitingLimits,
};
use p2panda_core::{Hash, SeqNum, SigningKey};

const CHICAGO: &str = "America/Chicago";

const NOTE_V1: &[u8] = b"# Shopping list\nmilk\n";
const NOTE_V2: &[u8] = b"# Shopping list\nmilk\neggs\n";
const NOTE_C: &[u8] = b"# C's private list\nnothing to see\n";

fn group() -> Hash {
    Hash::from([0x60; 32])
}

fn dog() -> Hash {
    Hash::from([0xD0; 32])
}

fn chicago(y: i32, m: u32, d: u32, hour: u32, minute: u32) -> i64 {
    let tz: Tz = CHICAGO.parse().unwrap();
    let naive = NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap();
    tz.from_local_datetime(&naive).single().unwrap().timestamp()
}

/// A signer with its own chain. Each event names the one before it.
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

    fn author(&self) -> [u8; 32] {
        *self.key.verifying_key().as_bytes()
    }

    fn send(&mut self, ext: ChainExt, body: Option<&[u8]>) -> Event {
        let event = sign(&self.key, self.seq, self.last, body, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }
}

fn ext(kind: &str, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: kind.to_owned(),
        group: Some(group()),
        after,
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
    }
}

fn document_event(writer: &mut Writer, doc: &DocumentRef, after: Vec<Hash>) -> Event {
    let mut e = ext(DOCUMENT_KIND, after);
    e.document = Some(doc.clone());
    writer.send(e, None)
}

fn tag_event(writer: &mut Writer, op: TagOp, after: Vec<Hash>) -> Event {
    let mut e = ext(SET_KIND, after);
    e.tag = Some(op);
    writer.send(e, None)
}

fn set(subject: Hash, name: &str, value: &str) -> TagOp {
    TagOp::Set {
        subject,
        name: name.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn open(dir: &std::path::Path) -> EventStore {
    EventStore::open(dir, WaitingLimits::default(), 0).unwrap()
}

/// Delivers one event, with its body when it has one, as sync does, and
/// checks that it was not refused.
fn deliver(store: &mut EventStore, event: &Event, body: Option<&[u8]>) {
    let outcome = store.ingest_with_body(&event.bytes, body, 0).unwrap();
    assert!(
        !matches!(outcome, Ingest::Refused(_)),
        "refused: {outcome:?}"
    );
}

/// One timeline entry: an applied event in the shared group.
struct TimelineEntry {
    author: [u8; 32],
    seq: u64,
    event: Event,
}

/// The shared group timeline of a store: every applied event that names
/// the group, in a deterministic (author, sequence) order.
fn timeline(store: &EventStore) -> Vec<TimelineEntry> {
    let mut entries = Vec::new();
    for (author, _chain) in store.chains().unwrap() {
        for event in store
            .applied_events(&author, None, None, usize::MAX)
            .unwrap()
        {
            if event.header.extensions.group == Some(group()) {
                entries.push(TimelineEntry {
                    author,
                    seq: u64::from(event.header.seq_num),
                    event,
                });
            }
        }
    }
    entries.sort_by(|a, b| (a.author, a.seq).cmp(&(b.author, b.seq)));
    entries
}

/// A logical note's current head, derived from documents + tag claims.
struct NoteHead {
    title: String,
    author: [u8; 32],
}

/// The notes view: logical notes by their `note.id` tag; a document that
/// another document's `note.supersedes` tag names is a past revision, not
/// a head. Titles are the first line of the verified content. The blob
/// bytes stand in for the blob store, as a fetch would supply them.
fn notes_view(
    store: &EventStore,
    blobs: &BTreeMap<iroh_blobs::Hash, Vec<u8>>,
) -> BTreeMap<String, NoteHead> {
    let in_timeline: BTreeSet<Hash> = timeline(store).iter().map(|e| e.event.id).collect();
    let mut note_id_of: BTreeMap<Hash, String> = BTreeMap::new();
    let mut superseded: BTreeSet<String> = BTreeSet::new();
    for claim in store.tag_claims().unwrap() {
        if !in_timeline.contains(&claim.subject) {
            continue;
        }
        match claim.name.as_str() {
            "note.id" => {
                if let Some(value) = &claim.value {
                    note_id_of.insert(claim.subject, value.clone());
                }
            }
            "note.supersedes" => {
                if let Some(value) = &claim.value {
                    superseded.insert(value.clone());
                }
            }
            _ => {}
        }
    }
    let mut heads = BTreeMap::new();
    for record in store.documents().unwrap() {
        if !in_timeline.contains(&record.id) {
            continue;
        }
        if superseded.contains(&record.id.to_string()) {
            continue;
        }
        let Some(note_id) = note_id_of.get(&record.id) else {
            continue;
        };
        let bytes = blobs.get(&record.document.blob_hash()).unwrap();
        record.document.verify_content(bytes).unwrap();
        let title = String::from_utf8_lossy(bytes)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        heads.insert(
            note_id.clone(),
            NoteHead {
                title,
                author: record.author,
            },
        );
    }
    heads
}

/// One did-it record as the view presents it: the decoded record, its
/// author, and whether the writer set counts it.
struct DiditRow {
    record: Record,
    author: [u8; 32],
    counted: bool,
}

/// The did-it view: every record stored in the timeline, decoded from its
/// persisted body, marked counted or not counted by the writer set.
fn didit_rows(store: &EventStore, counted: &impl Fn(&[u8; 32]) -> bool) -> Vec<DiditRow> {
    let mut rows = Vec::new();
    for entry in timeline(store) {
        if entry.event.header.extensions.kind != RECORD_KIND {
            continue;
        }
        let body = store.event_body(&entry.event.id).unwrap();
        let record = decode_record_body(&body).unwrap();
        rows.push(DiditRow {
            record,
            author: entry.author,
            counted: counted(&entry.author),
        });
    }
    rows
}

fn expectation_from(store: &EventStore) -> Expectation {
    let entry = timeline(store)
        .into_iter()
        .find(|e| e.event.header.extensions.kind == EXPECTATION_KIND)
        .expect("the expectation is in the timeline");
    let body = store.event_body(&entry.event.id).unwrap();
    id::didit::decode_expectation_body(&body).unwrap()
}

fn walk_record(id_byte: u8, at: i64) -> Record {
    Record {
        id: Hash::from([id_byte; 32]),
        subjects: vec![dog()],
        act: "walk".to_owned(),
        at,
        tz_offset_minutes: -360,
        details: Vec::new(),
        shared: None,
        measure: None,
        note: None,
        place: None,
    }
}

#[test]
fn test_first_app_poc_two_identities_exchange_and_a_third_partys_claims_are_excluded_but_retrievable()
 {
    let mut a = Writer::new(1);
    let mut b = Writer::new(2);
    let mut c = Writer::new(3);

    // The shared group's content, authored across the three identities.
    // A: the note's first revision, the did-it entity, the expectation
    // (three walks a day), and the first walk.
    let doc_v1 = DocumentRef::for_bytes("text/markdown", NOTE_V1).unwrap();
    let note_v1 = document_event(&mut a, &doc_v1, vec![]);
    let note_v1_id_tag = tag_event(
        &mut a,
        set(note_v1.id, "note.id", "note-1"),
        vec![note_v1.id],
    );

    let entity = Entity {
        entity: dog(),
        household: group(),
        kind: "dog".to_owned(),
        name: "Rex".to_owned(),
        aliases: Vec::new(),
        retired_at: None,
    };
    let entity_body = encode_entity_body(&entity).unwrap();
    let entity_event = a.send(ext(ENTITY_KIND, vec![]), Some(&entity_body));

    let expectation = Expectation {
        id: Hash::from([0xE1; 32]),
        subjects: vec![dog()],
        act: "walk".to_owned(),
        window: DiditWindow::CountPerPeriod {
            count: 3,
            period: Period::Day,
        },
        active: ActiveWindow::Hours {
            hours: vec![HoursSpan {
                from_minute: 420,
                to_minute: 1320,
                days: None,
            }],
            tz: CHICAGO.to_owned(),
        },
        supersedes: None,
        ends_at: None,
        note: None,
    };
    let expectation_body = encode_expectation_body(&expectation).unwrap();
    let expectation_event = a.send(ext(EXPECTATION_KIND, vec![]), Some(&expectation_body));

    let a_record = walk_record(0xA1, chicago(2026, 11, 22, 8, 0));
    let a_record_body = encode_record_body(&a_record).unwrap();
    let a_record_event = a.send(ext(RECORD_KIND, vec![]), Some(&a_record_body));

    // B: the note's second revision (superseding A's) and the second walk.
    let doc_v2 = DocumentRef::for_bytes("text/markdown", NOTE_V2).unwrap();
    let note_v2 = document_event(&mut b, &doc_v2, vec![note_v1.id]);
    let note_v2_id_tag = tag_event(
        &mut b,
        set(note_v2.id, "note.id", "note-1"),
        vec![note_v2.id],
    );
    let note_v2_supersedes_tag = tag_event(
        &mut b,
        set(note_v2.id, "note.supersedes", &note_v1.id.to_string()),
        vec![note_v2.id],
    );

    let b_record = walk_record(0xB1, chicago(2026, 11, 22, 9, 0));
    let b_record_body = encode_record_body(&b_record).unwrap();
    let b_record_event = b.send(ext(RECORD_KIND, vec![]), Some(&b_record_body));

    // C: a note of its own, and a third walk — a claim A's view must not
    // count, because C is not in A's writer set.
    let doc_c = DocumentRef::for_bytes("text/markdown", NOTE_C).unwrap();
    let note_c = document_event(&mut c, &doc_c, vec![]);
    let note_c_id_tag = tag_event(&mut c, set(note_c.id, "note.id", "note-2"), vec![note_c.id]);

    let c_record = walk_record(0xC1, chicago(2026, 11, 22, 10, 0));
    let c_record_body = encode_record_body(&c_record).unwrap();
    let c_record_event = c.send(ext(RECORD_KIND, vec![]), Some(&c_record_body));

    // Exchange: each identity's replica receives everyone's events, as
    // sync delivers them — headers by ingest, bodies with their events.
    let deliveries: Vec<(&Event, Option<&[u8]>)> = vec![
        (&note_v1, None),
        (&note_v1_id_tag, None),
        (&entity_event, Some(&entity_body)),
        (&expectation_event, Some(&expectation_body)),
        (&a_record_event, Some(&a_record_body)),
        (&note_v2, None),
        (&note_v2_id_tag, None),
        (&note_v2_supersedes_tag, None),
        (&b_record_event, Some(&b_record_body)),
        (&note_c, None),
        (&note_c_id_tag, None),
        (&c_record_event, Some(&c_record_body)),
    ];
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let mut store_a = open(dir_a.path());
    let mut store_b = open(dir_b.path());
    for (event, body) in &deliveries {
        deliver(&mut store_a, event, *body);
        deliver(&mut store_b, event, *body);
    }

    // The exchange converges: both replicas hold the same documents and
    // the same tag claims.
    assert_eq!(store_a.documents().unwrap(), store_b.documents().unwrap());
    assert_eq!(store_a.tag_claims().unwrap(), store_b.tag_claims().unwrap());

    // The blob bytes a fetch would supply, keyed by the hash each
    // document names.
    let blobs: BTreeMap<iroh_blobs::Hash, Vec<u8>> = [
        (doc_v1.blob_hash(), NOTE_V1.to_vec()),
        (doc_v2.blob_hash(), NOTE_V2.to_vec()),
        (doc_c.blob_hash(), NOTE_C.to_vec()),
    ]
    .into_iter()
    .collect();

    // Notes view: two logical notes. note-1's head is B's revision — A's
    // first revision is superseded, not a head — and C's note is present:
    // the writer set filters the did-it view, not the shared timeline.
    let notes = notes_view(&store_a, &blobs);
    assert_eq!(notes.len(), 2, "two logical notes: {:?}", notes.keys());
    assert_eq!(notes["note-1"].title, "# Shopping list");
    assert_eq!(notes["note-1"].author, b.author());
    assert_eq!(notes["note-2"].title, "# C's private list");
    assert_eq!(notes["note-2"].author, c.author());

    // A's writer set: A and B count; C does not.
    let set = WriterSet {
        set_id: Hash::from([0x90; 32]),
        version: 1,
        owner: a.author(),
        scope: Some("first-app-poc".to_owned()),
        entries: vec![
            Entry::Key {
                author: a.author(),
                include_history: true,
            },
            Entry::Key {
                author: b.author(),
                include_history: true,
            },
        ],
        excludes: Excludes(vec![]),
    };
    let expanded = expand_set(&set, &[], &Hash::from([0x12; 32]), &[], false).unwrap();
    assert!(expanded.contains(&a.author()));
    assert!(expanded.contains(&b.author()));
    assert!(!expanded.contains(&c.author()));

    // Did-it view: all three records are in the timeline; exactly C's is
    // not counted, and it is presented with its author.
    let rows = didit_rows(&store_a, &|author| expanded.contains(author));
    assert_eq!(rows.len(), 3, "every stored record is listed");
    let counted_records: Vec<Record> = rows
        .iter()
        .filter(|r| r.counted)
        .map(|r| r.record.clone())
        .collect();
    assert_eq!(counted_records.len(), 2);
    let uncounted: Vec<&DiditRow> = rows.iter().filter(|r| !r.counted).collect();
    assert_eq!(uncounted.len(), 1);
    assert_eq!(uncounted[0].author, c.author());
    assert_eq!(uncounted[0].record.id, c_record.id);

    // The counted view's verdict for the window containing `now`: two of
    // the three daily walks count, so the expectation is still due —
    // C's walk did not satisfy it. (The evaluator also reports completed
    // windows; the oracle is the live one.)
    let now = chicago(2026, 11, 22, 12, 0);
    let expectation_back = expectation_from(&store_a);
    let verdicts = evaluate(&[expectation_back.clone()], &counted_records, now).unwrap();
    let verdict = verdicts
        .iter()
        .find(|v| v.window_start <= now && now < v.window_end)
        .expect("a verdict covers now");
    assert_eq!(verdict.count, 2);
    assert_eq!(verdict.state, VerdictState::Due);

    // Control: had C's record been counted, the same expectation would be
    // satisfied. The exclusion above is what kept the view at two.
    let all_records: Vec<Record> = rows.iter().map(|r| r.record.clone()).collect();
    let control = evaluate(&[expectation_back], &all_records, now).unwrap();
    let control = control
        .iter()
        .find(|v| v.window_start <= now && now < v.window_end)
        .expect("a verdict covers now");
    assert_eq!(control.count, 3);
    assert_eq!(control.state, VerdictState::Satisfied);

    // Retrievable on demand, durably: after a restart of A's store, C's
    // record body is still there, byte-exact, decoding to C's claim.
    drop(store_a);
    let store_a = open(dir_a.path());
    let body = store_a.event_body(&c_record_event.id).unwrap();
    assert_eq!(body, c_record_body);
    assert_eq!(decode_record_body(&body).unwrap(), c_record);
}
