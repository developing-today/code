//! Group operations through the production ingest path (P2.3).
//!
//! `SharedEventStore::ingest` takes an event's header and its body, and reads the membership
//! action from the body. These tests reach the store only through that call, the path the network
//! takes, and never through `ingest_with_action`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::chain::{ChainExt, Event, sign};
use id::event_store::{EventStore, Ingest, Refusal};
use id::membership::AuthId;
use id::membership::body::{CLAIM, encode_body};
use id::rejection::Reason;
use id::store_traits::SharedEventStore;
use id::waiting::WaitingLimits;
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum, SigningKey};

/// The group operation kind in an event's header.
const GROUP_OP: &str = "group.op";

fn key(who: char) -> SigningKey {
    SigningKey::from_bytes(&[who as u8; 32])
}

fn auth(who: char) -> AuthId {
    AuthId::Key(*key(who).verifying_key().as_bytes())
}

fn open() -> (tempfile::TempDir, SharedEventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, SharedEventStore::new(store))
}

/// A signed event and the body it commits to, as the network delivers them.
struct Sent {
    event: Event,
    body: Option<Vec<u8>>,
}

impl Sent {
    fn ingest(&self, shared: &SharedEventStore) -> Ingest {
        shared
            .ingest(&self.event.bytes, self.body.as_deref(), 0)
            .unwrap()
    }
}

/// Signs each author's events as one chain: the sequence numbers and backlinks follow from the
/// order of signing.
#[derive(Default)]
struct Chains {
    last: BTreeMap<char, (SeqNum, Hash)>,
}

impl Chains {
    /// Signs the next event of `who`. It names `after` and, when there is one, its body.
    fn sign(
        &mut self,
        who: char,
        group: Option<Hash>,
        after: &[Hash],
        kind: &str,
        body: Option<Vec<u8>>,
    ) -> Sent {
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after: after.to_vec(),
            tag: None,
            document: None,
            removes: None,
            chat: None,
        };
        let event = sign(&key(who), seq, backlink, body.as_deref(), ext).unwrap();
        self.last.insert(who, (seq, event.id));
        Sent { event, body }
    }

    /// A group operation by `who`, with its action in the body.
    fn op(
        &mut self,
        who: char,
        group: Hash,
        after: &[Hash],
        action: &GroupAction<AuthId, ()>,
    ) -> Sent {
        let body = encode_body(action).unwrap();
        self.sign(who, Some(group), after, GROUP_OP, Some(body))
    }

    /// The group's create, by A, B and C as managers. The group is the create's ID.
    fn create(&mut self) -> Sent {
        let action = GroupAction::Create {
            initial_members: ['A', 'B', 'C']
                .iter()
                .map(|who| (GroupMember::Individual(auth(*who)), Access::manage()))
                .collect(),
        };
        let body = encode_body(&action).unwrap();
        self.sign('A', None, &[], GROUP_OP, Some(body))
    }
}

fn add(who: char, access: Access<()>) -> GroupAction<AuthId, ()> {
    GroupAction::Add {
        member: GroupMember::Individual(auth(who)),
        access,
    }
}

fn remove(who: char) -> GroupAction<AuthId, ()> {
    GroupAction::Remove {
        member: GroupMember::Individual(auth(who)),
    }
}

/// The keys of `who`, sorted as the roster is.
fn members(who: &[char]) -> Vec<AuthId> {
    let mut keys: Vec<AuthId> = who.iter().map(|who| auth(*who)).collect();
    keys.sort();
    keys
}

/// Whether the store applies the event.
fn applied(shared: &SharedEventStore, id: &Hash) -> bool {
    shared.read(|store| Ok(store.event_applied(id)?)).unwrap()
}

/// The keys in the group's roster, sorted.
fn roster(shared: &SharedEventStore, group: &Hash) -> Vec<AuthId> {
    let mut keys: Vec<AuthId> = shared
        .read(|store| Ok(store.roster(group)?))
        .unwrap()
        .into_iter()
        .map(AuthId::Key)
        .collect();
    keys.sort();
    keys
}

/// The roster of the group the create made, when the test has one.
fn roster_of(shared: &SharedEventStore, create: &Sent) -> Vec<AuthId> {
    roster(shared, &create.event.id)
}

#[test]
fn test_an_unauthorised_group_operation_is_not_applied() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    assert!(matches!(create.ingest(&shared), Ingest::Accepted { .. }));
    // D is not a member, so D's add of E does not take effect.
    let group = create.event.id;
    let attempt = chains.op('D', group, &[group], &add('E', Access::manage()));
    assert_eq!(
        attempt.ingest(&shared),
        Ingest::Voided {
            id: attempt.event.id,
            released: Vec::new(),
        }
    );
    assert!(!applied(&shared, &attempt.event.id));
    assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C']));
}

#[test]
fn test_a_voided_operation_gives_the_voided_reason() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    // D is not a member, so the operation is voided at its place.
    let attempt = chains.op('D', group, &[group], &add('E', Access::manage()));
    let result = attempt.ingest(&shared);
    assert!(matches!(result, Ingest::Voided { .. }), "{result:?}");
    assert_eq!(result.reason(), Some(Reason::Voided));
}

#[test]
fn test_an_authorised_group_operation_is_applied() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    let grant = chains.op('A', group, &[group], &add('D', Access::read()));
    assert_eq!(
        grant.ingest(&shared),
        Ingest::Accepted {
            id: grant.event.id,
            released: Vec::new(),
        }
    );
    assert!(applied(&shared, &grant.event.id));
    assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C', 'D']));
}

#[test]
fn test_a_strong_removal_invalidates_the_removed_members_later_operations() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    // A removes B. B's add names the removal, so B had already lost authority when it made it.
    let removal = chains.op('A', group, &[group], &remove('B'));
    assert!(matches!(removal.ingest(&shared), Ingest::Accepted { .. }));
    let late = chains.op('B', group, &[removal.event.id], &add('E', Access::manage()));
    assert_eq!(
        late.ingest(&shared),
        Ingest::Voided {
            id: late.event.id,
            released: Vec::new(),
        }
    );
    assert!(!applied(&shared, &late.event.id));
    assert_eq!(roster_of(&shared, &create), members(&['A', 'C']));
}

#[test]
fn test_a_truncated_group_operation_body_is_refused() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    let body = encode_body(&add('D', Access::read())).unwrap();
    let cut = body[..body.len() - 3].to_vec();
    assert!(
        cut.starts_with(CLAIM),
        "the cut body still claims a group operation"
    );
    let sent = chains.sign('A', Some(group), &[group], GROUP_OP, Some(cut));
    assert!(matches!(
        sent.ingest(&shared),
        Ingest::Refused(Refusal::Malformed(_))
    ));
    assert_eq!(
        shared
            .read(|store| Ok(store.event_by_id(&sent.event.id)?))
            .unwrap(),
        None,
        "a refused event is not stored"
    );
    assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C']));
}

#[test]
fn test_a_group_operation_body_of_the_wrong_version_is_refused() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    let mut body = encode_body(&add('D', Access::read())).unwrap();
    // The version byte follows the claim: 1 becomes 2.
    body[CLAIM.len()] = 2;
    let sent = chains.sign('A', Some(group), &[group], GROUP_OP, Some(body));
    assert!(matches!(
        sent.ingest(&shared),
        Ingest::Refused(Refusal::Malformed(_))
    ));
}

#[test]
fn test_a_body_that_does_not_match_its_payload_hash_is_refused() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    // The header commits to an add, and the body that arrives is a remove.
    let committed = encode_body(&add('D', Access::manage())).unwrap();
    let event = sign(
        &key('A'),
        1,
        Some(create.event.id),
        Some(committed.as_slice()),
        ChainExt {
            kind: GROUP_OP.to_owned(),
            group: Some(group),
            after: vec![group],
            tag: None,
            document: None,
            removes: None,
            chat: None,
        },
    )
    .unwrap();
    let swapped = encode_body(&remove('B')).unwrap();
    let outcome = shared.ingest(&event.bytes, Some(&swapped), 0).unwrap();
    assert!(matches!(outcome, Ingest::Refused(Refusal::Malformed(_))));
    assert!(!applied(&shared, &event.id));
    assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C']));
}

#[test]
fn test_content_with_a_body_that_is_not_a_group_operation_is_content() {
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let message = chains.sign(
        'B',
        None,
        &[create.event.id],
        "message",
        Some(b"hello".to_vec()),
    );
    assert_eq!(
        message.ingest(&shared),
        Ingest::Accepted {
            id: message.event.id,
            released: Vec::new(),
        }
    );
    assert!(applied(&shared, &message.event.id));
}

#[test]
fn test_a_group_operation_that_arrives_without_its_body_is_content() {
    // The body is what makes an event a membership change. Without it the event is content, so it
    // applies with no effect on the group. A header that commits to a body is not refused for
    // lacking one.
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    let attempt = chains.op('D', group, &[group], &add('E', Access::manage()));
    assert_eq!(
        shared.ingest(&attempt.event.bytes, None, 0).unwrap(),
        Ingest::Accepted {
            id: attempt.event.id,
            released: Vec::new(),
        }
    );
    assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C']));
}

// ----- Review of Phase 2 [P2-review] -----

#[test]
fn test_a_member_of_any_access_can_leave_through_the_production_path() {
    // Leaving is a removal by the member itself. p2panda-auth allows it for any active member, and
    // the fold does too, so the production path must not void it by the member's access.
    for access in [Access::pull(), Access::read(), Access::write()] {
        let (_dir, shared) = open();
        let mut chains = Chains::default();
        let create = chains.create();
        create.ingest(&shared);
        let group = create.event.id;
        let grant = chains.op('A', group, &[group], &add('D', access.clone()));
        assert!(matches!(grant.ingest(&shared), Ingest::Accepted { .. }));
        let leave = chains.op('D', group, &[grant.event.id], &remove('D'));
        assert_eq!(
            leave.ingest(&shared),
            Ingest::Accepted {
                id: leave.event.id,
                released: Vec::new(),
            },
            "a member with {access:?} leaves"
        );
        assert_eq!(roster_of(&shared, &create), members(&['A', 'B', 'C']));
    }
}

#[test]
fn test_a_removal_whose_body_arrived_late_still_takes_effect() {
    // The header commits to the body, so a later delivery with the body is the same event. A relay
    // that strips the body on the first delivery must not make the removal content for good.
    let (_dir, shared) = open();
    let mut chains = Chains::default();
    let create = chains.create();
    create.ingest(&shared);
    let group = create.event.id;
    let removal = chains.op('A', group, &[group], &remove('B'));
    assert!(matches!(
        shared.ingest(&removal.event.bytes, None, 0).unwrap(),
        Ingest::Accepted { .. }
    ));
    assert_eq!(
        removal.ingest(&shared),
        Ingest::Duplicate {
            id: removal.event.id,
        }
    );
    assert_eq!(roster_of(&shared, &create), members(&['A', 'C']));
}
