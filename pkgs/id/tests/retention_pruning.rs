//! Integration tests for retention and pruning behind a causal cut (Phase 8, P8.2).
//!
//! Pruning removes events behind a causal cut while protecting:
//! 1. Every event in the causal past of open removals (needed for membership voidness check).
//! 2. Open frontiers and active backlink chains.
//!
//! Invariants (`check_invariants`) and query folds (roster, live tag claims, documents)
//! produce identical results before and after pruning.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::Path;

use id::chain::{ChainExt, Event, sign};
use id::documents::{DOCUMENT_KIND, DocumentRef};
use id::event_store::EventStore;
use id::membership::AuthId;
use id::membership::body::encode_body;
use id::notes::{CREATE_KIND as NOTE_CREATE_KIND, NoteOp, REVISE_KIND as NOTE_REVISE_KIND};
use id::tag_claims::{CLEAR_KIND, SET_KIND, TagOp};
use id::waiting::WaitingLimits;
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum, SigningKey};

const GROUP_OP: &str = "group.op";

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn auth(n: u8) -> AuthId {
    AuthId::Key(*key(n).verifying_key().as_bytes())
}

fn open(dir: &Path) -> EventStore {
    EventStore::open(dir, WaitingLimits::default(), 0).unwrap()
}

#[derive(Default)]
struct Chains {
    last: BTreeMap<u8, (SeqNum, Hash)>,
}

impl Chains {
    fn new() -> Self {
        Self::default()
    }

    fn sign(
        &mut self,
        who: u8,
        group: Option<Hash>,
        after: &[Hash],
        kind: &str,
        body: Option<Vec<u8>>,
        tag: Option<TagOp>,
        document: Option<DocumentRef>,
    ) -> (Event, Option<Vec<u8>>) {
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after: after.to_vec(),
            tag,
            document,
            removes: None,
            note: None,
        };
        let event = sign(&key(who), seq, backlink, body.as_deref(), ext).unwrap();
        self.last.insert(who, (seq, event.id));
        (event, body)
    }

    fn group_op(
        &mut self,
        who: u8,
        group: Option<Hash>,
        after: &[Hash],
        action: &GroupAction<AuthId, ()>,
    ) -> (Event, Option<Vec<u8>>) {
        let body = encode_body(action).unwrap();
        self.sign(who, group, after, GROUP_OP, Some(body), None, None)
    }

    fn document(
        &mut self,
        who: u8,
        group: Hash,
        after: &[Hash],
        doc_hash: Hash,
    ) -> (Event, Option<Vec<u8>>) {
        let doc = DocumentRef {
            media_type: "text/plain".to_owned(),
            content: doc_hash,
            size: 100,
        };
        self.sign(
            who,
            Some(group),
            after,
            DOCUMENT_KIND,
            None,
            None,
            Some(doc),
        )
    }

    fn note_create(&mut self, who: u8, content: Hash) -> (Event, Option<Vec<u8>>) {
        let doc = DocumentRef {
            media_type: "text/plain".to_owned(),
            content,
            size: 100,
        };
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: NOTE_CREATE_KIND.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: Some(NoteOp::Create { content: doc }),
        };
        let event = sign(&key(who), seq, backlink, None, ext).unwrap();
        self.last.insert(who, (seq, event.id));
        (event, None)
    }

    fn note_revise(
        &mut self,
        who: u8,
        after: &[Hash],
        note: Hash,
        parents: &[Hash],
        content: Hash,
    ) -> (Event, Option<Vec<u8>>) {
        let doc = DocumentRef {
            media_type: "text/plain".to_owned(),
            content,
            size: 100,
        };
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: NOTE_REVISE_KIND.to_owned(),
            group: None,
            after: after.to_vec(),
            tag: None,
            document: None,
            removes: None,
            note: Some(NoteOp::Revise {
                note,
                content: doc,
                parents: parents.to_vec(),
            }),
        };
        let event = sign(&key(who), seq, backlink, None, ext).unwrap();
        self.last.insert(who, (seq, event.id));
        (event, None)
    }

    fn tag_set(
        &mut self,
        who: u8,
        group: Hash,
        after: &[Hash],
        subject: Hash,
        name: &str,
    ) -> (Event, Option<Vec<u8>>) {
        let op = TagOp::Set {
            subject,
            name: name.to_owned(),
            value: None,
        };
        self.sign(who, Some(group), after, SET_KIND, None, Some(op), None)
    }

    fn tag_clear(
        &mut self,
        who: u8,
        group: Hash,
        after: &[Hash],
        claims: Vec<Hash>,
    ) -> (Event, Option<Vec<u8>>) {
        let op = TagOp::Clear { claims };
        self.sign(who, Some(group), after, CLEAR_KIND, None, Some(op), None)
    }
}

fn deliver(store: &mut EventStore, event: &Event, body: Option<&[u8]>) {
    store.ingest_with_body(&event.bytes, body, 0).unwrap();
}

#[test]
fn test_retention_pruning_respects_open_removals_and_frontiers() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut chains = Chains::new();

    // 1. Author 1 (Alice) creates group G
    let (create, create_body) = chains.group_op(
        1,
        None,
        &[],
        &GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(auth(1)), Access::manage())],
        },
    );
    let group = create.id;
    deliver(&mut store, &create, create_body.as_deref());

    // 2. Alice adds Author 2 (Bob)
    let (add_bob, add_bob_body) = chains.group_op(
        1,
        Some(group),
        &[create.id],
        &GroupAction::Add {
            member: GroupMember::Individual(auth(2)),
            access: Access::read(),
        },
    );
    deliver(&mut store, &add_bob, add_bob_body.as_deref());

    // 3. Alice adds Author 3 (Charlie)
    let (add_charlie, add_charlie_body) = chains.group_op(
        1,
        Some(group),
        &[add_bob.id],
        &GroupAction::Add {
            member: GroupMember::Individual(auth(3)),
            access: Access::read(),
        },
    );
    deliver(&mut store, &add_charlie, add_charlie_body.as_deref());

    // 4. Bob publishes a document D1
    let (doc1, doc1_body) = chains.document(
        2,
        group,
        &[add_bob.id],
        Hash::digest(b"shared document content 1"),
    );
    deliver(&mut store, &doc1, doc1_body.as_deref());

    // 5. Bob sets a tag T1 on D1
    let (tag1, tag1_body) = chains.tag_set(2, group, &[doc1.id], doc1.id, "tag-bob-1");
    deliver(&mut store, &tag1, tag1_body.as_deref());

    // 6. Bob clears tag T1
    let (clear1, clear1_body) = chains.tag_clear(2, group, &[tag1.id], vec![tag1.id]);
    deliver(&mut store, &clear1, clear1_body.as_deref());

    // 7. Charlie publishes a document D2 and tag T2
    let (doc2, doc2_body) = chains.document(
        3,
        group,
        &[add_charlie.id],
        Hash::digest(b"shared document content 2"),
    );
    deliver(&mut store, &doc2, doc2_body.as_deref());

    let (tag2, tag2_body) = chains.tag_set(3, group, &[doc2.id], doc2.id, "tag-charlie-live");
    deliver(&mut store, &tag2, tag2_body.as_deref());

    // 8. Alice removes Bob. The removal names doc1 and doc2 in `after`, but did NOT see tag1 or clear1!
    let (removal, removal_body) = chains.group_op(
        1,
        Some(group),
        &[add_charlie.id, doc1.id, doc2.id],
        &GroupAction::Remove {
            member: GroupMember::Individual(auth(2)),
        },
    );
    deliver(&mut store, &removal, removal_body.as_deref());

    // 9. Alice publishes a later document D3 seeing the removal
    let (doc3, doc3_body) = chains.document(
        1,
        group,
        &[removal.id],
        Hash::digest(b"post-removal document 3"),
    );
    deliver(&mut store, &doc3, doc3_body.as_deref());

    // Verify baseline invariants and query states before pruning
    store.check_invariants().unwrap();

    let roster_before = store.roster(&group).unwrap();
    assert_eq!(
        roster_before,
        vec![
            *key(1).verifying_key().as_bytes(),
            *key(3).verifying_key().as_bytes()
        ]
    );

    let docs_before = store.documents().unwrap();
    assert_eq!(docs_before.len(), 3);

    let claims_before = store.tag_claims().unwrap();
    assert_eq!(claims_before.len(), 1);
    assert_eq!(claims_before[0].name, "tag-charlie-live");

    // Events in store before pruning
    assert!(store.event_by_id(&tag1.id).unwrap().is_some());
    assert!(store.event_by_id(&clear1.id).unwrap().is_some());
    assert!(store.event_by_id(&doc1.id).unwrap().is_some());
    assert!(store.event_by_id(&removal.id).unwrap().is_some());

    // Define the causal cut and the protected past:
    // Protected: open removal and open frontier
    let protected = vec![removal.id, doc3.id];
    // Cut boundary includes the old cleared tag branch
    let cut = vec![clear1.id, removal.id];

    // Prune behind the cut
    let pruned_count = store.prune_behind_cut(&cut, &protected).unwrap();
    assert!(
        pruned_count >= 1,
        "at least clear1 and tag1 should be pruned"
    );

    // Verify unneeded events were pruned
    assert!(store.event_by_id(&clear1.id).unwrap().is_none());
    assert!(store.event_by_id(&tag1.id).unwrap().is_none());

    // Verify the removal and its causal past (doc1, create, add_bob, add_charlie) survived
    assert!(store.event_by_id(&removal.id).unwrap().is_some());
    assert!(store.event_by_id(&doc1.id).unwrap().is_some());
    assert!(store.event_by_id(&doc2.id).unwrap().is_some());
    assert!(store.event_by_id(&create.id).unwrap().is_some());
    assert!(store.event_by_id(&add_bob.id).unwrap().is_some());
    assert!(store.event_by_id(&add_charlie.id).unwrap().is_some());
    assert!(store.event_by_id(&doc3.id).unwrap().is_some());

    // Invariants check still passes
    store.check_invariants().unwrap();

    // Roster and fold queries produce identical results
    let roster_after = store.roster(&group).unwrap();
    assert_eq!(roster_before, roster_after);

    let docs_after = store.documents().unwrap();
    assert_eq!(docs_before, docs_after);

    let claims_after = store.tag_claims().unwrap();
    assert_eq!(claims_before, claims_after);
}

#[test]
fn test_prune_behind_cut_empty_or_noop() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut chains = Chains::new();

    let (create, create_body) = chains.group_op(
        1,
        None,
        &[],
        &GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(auth(1)), Access::manage())],
        },
    );
    deliver(&mut store, &create, create_body.as_deref());

    // Empty cut prunes 0
    let count = store.prune_behind_cut(&[], &[create.id]).unwrap();
    assert_eq!(count, 0);

    // Protected matches cut prunes 0
    let count = store.prune_behind_cut(&[create.id], &[create.id]).unwrap();
    assert_eq!(count, 0);

    store.check_invariants().unwrap();
}

#[test]
fn test_prune_removes_unreferenced_author_chain_completely() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut chains = Chains::new();

    // Alice creates group
    let (create, create_body) = chains.group_op(
        1,
        None,
        &[],
        &GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(auth(1)), Access::manage())],
        },
    );
    let group = create.id;
    deliver(&mut store, &create, create_body.as_deref());

    // Author 4 (Dave) writes a standalone message/doc outside the group or in group
    let (doc_dave, doc_dave_body) =
        chains.document(4, group, &[create.id], Hash::digest(b"dave standalone doc"));
    deliver(&mut store, &doc_dave, doc_dave_body.as_deref());

    // Alice writes a document D2 seeing only create
    let (doc_alice, doc_alice_body) =
        chains.document(1, group, &[create.id], Hash::digest(b"alice doc"));
    deliver(&mut store, &doc_alice, doc_alice_body.as_deref());

    store.check_invariants().unwrap();
    assert!(store.event_by_id(&doc_dave.id).unwrap().is_some());
    assert!(
        store
            .chain(&*key(4).verifying_key().as_bytes())
            .unwrap()
            .is_some()
    );

    // Cut includes doc_dave, protected includes doc_alice
    let count = store
        .prune_behind_cut(&[doc_dave.id], &[doc_alice.id])
        .unwrap();
    assert_eq!(count, 1);

    // Dave's event is pruned and Dave's chain row is deleted
    assert!(store.event_by_id(&doc_dave.id).unwrap().is_none());
    assert!(
        store
            .chain(&*key(4).verifying_key().as_bytes())
            .unwrap()
            .is_none()
    );

    // Invariants still hold
    store.check_invariants().unwrap();
}

#[test]
fn test_active_backlink_walks_preserved_for_tail_prune() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut chains = Chains::new();

    // Alice creates group
    let (create, create_body) = chains.group_op(
        1,
        None,
        &[],
        &GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(auth(1)), Access::manage())],
        },
    );
    let group = create.id;
    deliver(&mut store, &create, create_body.as_deref());

    // Bob writes doc B0, then B1, then B2
    let (b0, b0_body) = chains.document(2, group, &[create.id], Hash::digest(b"b0"));
    deliver(&mut store, &b0, b0_body.as_deref());

    let (b1, b1_body) = chains.document(2, group, &[b0.id], Hash::digest(b"b1"));
    deliver(&mut store, &b1, b1_body.as_deref());

    let (b2, b2_body) = chains.document(2, group, &[b1.id], Hash::digest(b"b2"));
    deliver(&mut store, &b2, b2_body.as_deref());

    // Alice creates removal or anchor pointing to b1 (so b1, b0, create are in protected past)
    let (removal, removal_body) = chains.group_op(
        1,
        Some(group),
        &[b1.id],
        &GroupAction::Remove {
            member: GroupMember::Individual(auth(2)),
        },
    );
    deliver(&mut store, &removal, removal_body.as_deref());

    store.check_invariants().unwrap();

    // Cut is b2, protected is removal (which saw b1, but not b2)
    let count = store.prune_behind_cut(&[b2.id], &[removal.id]).unwrap();
    assert_eq!(count, 1, "b2 should be pruned");

    assert!(store.event_by_id(&b2.id).unwrap().is_none());
    assert!(store.event_by_id(&b1.id).unwrap().is_some());
    assert!(store.event_by_id(&b0.id).unwrap().is_some());

    // Bob's chain should now have head = b1 and next_seq = 2
    let bob_chain = store
        .chain(&*key(2).verifying_key().as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(bob_chain.next_seq, 2);
    assert_eq!(bob_chain.head, Some(b1.id));

    // Invariants check still passes
    store.check_invariants().unwrap();
}

#[test]
fn test_prune_preserves_note_parents_reachable_only_via_note_refs() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut chains = Chains::new();

    // Alice creates a note; the create event is the note's stable ID and first revision.
    let (create, create_body) = chains.note_create(1, Hash::digest(b"note v1"));
    deliver(&mut store, &create, create_body.as_deref());

    // Alice revises it normally: `after` names the create event.
    let (rev1, rev1_body) = chains.note_revise(
        1,
        &[create.id],
        create.id,
        &[create.id],
        Hash::digest(b"note v2"),
    );
    deliver(&mut store, &rev1, rev1_body.as_deref());

    // Bob revises again, but his event's `after` is empty: the ONLY references to the
    // note and to rev1 are inside the note operation itself.
    let (rev2, rev2_body) =
        chains.note_revise(2, &[], create.id, &[rev1.id], Hash::digest(b"note v3"));
    deliver(&mut store, &rev2, rev2_body.as_deref());

    // Charlie writes an unrelated event that saw rev2, keeping rev2 (and, through its
    // note references, its history) in the surviving frontier.
    let (anchor_event, anchor_body) = chains.sign(
        3,
        None,
        &[rev2.id],
        "message",
        Some(b"seen".to_vec()),
        None,
        None,
    );
    deliver(&mut store, &anchor_event, anchor_body.as_deref());

    store.check_invariants().unwrap();

    // Cut at rev1: rev1 and the create event are behind the cut and would be pruned
    // unless rev2's note references protect them.
    let count = store
        .prune_behind_cut(&[rev1.id], &[anchor_event.id])
        .unwrap();
    assert_eq!(
        count, 0,
        "note history reachable only via note refs must survive"
    );
    assert!(store.event_by_id(&create.id).unwrap().is_some());
    assert!(store.event_by_id(&rev1.id).unwrap().is_some());
    assert!(store.event_by_id(&rev2.id).unwrap().is_some());
    assert!(store.event_by_id(&anchor_event.id).unwrap().is_some());

    store.check_invariants().unwrap();
}
