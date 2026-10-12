//! Tests for the `DataScheme` adapter: unit tests per operation, and a three-member integration.
//!
//! Everyone in a cast gets keys and a session up front. Control messages go to every member of the
//! cast, since a joiner needs the control history to take the welcome. Application messages go only
//! to the members who may read them.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey as IdentitySecret;
use p2panda_encryption::data_scheme::group_secret::GroupSecretId;
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::KeyManagerState;
use p2panda_encryption::traits::GroupMessage;
use proptest::prelude::*;

use super::*;
use crate::groups::{
    Body, Create, Event, GroupView, Key, MemberOp, Policy, Role, Scheme, Template, fold,
};

fn person(seed: u8) -> (Key, VerifyingKey) {
    let verifying = SigningKey::from_bytes(&[seed; 32]).verifying_key();
    (key_of(verifying), verifying)
}

fn id(seed: u8) -> Key {
    person(seed).0
}

fn text(message: &str) -> Received {
    Received::Plaintext(message.as_bytes().to_vec())
}

/// A group policy that names the `datascheme` scheme at `version`.
fn scheme_policy(version: u32) -> Policy {
    Policy {
        scheme: Some(Scheme {
            name: DATASCHEME.to_owned(),
            version,
        }),
        ..Policy::default()
    }
}

/// A view that lists `members` as Admins. For the checks that run before any fold.
fn view_of(members: &BTreeSet<Key>) -> GroupView {
    GroupView {
        policy: scheme_policy(SCHEME_V1),
        members: members.iter().map(|key| (*key, Role::Admin)).collect(),
        friendship: None,
        present: BTreeSet::new(),
        dissolved_by: None,
    }
}

/// Every person's keys and one session per person, all sharing one registry of bundles.
struct Cast {
    rng: Rng,
    sessions: BTreeMap<Key, Session>,
    /// The group's signed events, in order. The first is the create, and the group model is their
    /// fold.
    events: Vec<Event>,
    /// The member set the group model holds after the last operation.
    model: BTreeSet<Key>,
}

impl Cast {
    fn new(seeds: &[u8]) -> Self {
        let rng = Rng::default();
        let mut bundles = Vec::new();
        let mut keys: BTreeMap<Key, (VerifyingKey, KeyManagerState)> = BTreeMap::new();
        for seed in seeds {
            let (key, verifying) = person(*seed);
            let identity = IdentitySecret::from_rng(&rng).expect("identity");
            let state = new_keys(&identity, Lifetime::default(), &rng).expect("keys");
            bundles.push((verifying, key_bundle(&state).expect("bundle")));
            keys.insert(key, (verifying, state));
        }
        let registry = registry(&bundles).expect("registry");
        let sessions = keys
            .into_iter()
            .map(|(key, (verifying, state))| {
                let session = Session::init(
                    verifying,
                    state,
                    registry.clone(),
                    &scheme_policy(SCHEME_V1),
                )
                .expect("session");
                (key, session)
            })
            .collect();
        Self {
            rng,
            sessions,
            events: Vec::new(),
            model: BTreeSet::new(),
        }
    }

    fn session(&self, who: u8) -> &Session {
        self.sessions.get(&id(who)).expect("session")
    }

    /// The group's view as the fold gives it after the events so far.
    fn view(&self) -> GroupView {
        let create = self.events.first().expect("a create").id;
        let projection = fold(&self.events).expect("fold");
        projection.group(&create).expect("the group").clone()
    }

    /// Appends a membership event by `by` to the group's log, and returns the view after it.
    fn log(&mut self, by: u8, body: Body) -> GroupView {
        let group = self.events.first().map(|create| create.id);
        let after = self.events.last().map(|last| last.id).into_iter().collect();
        let name = format!("event {}", self.events.len());
        self.events.push(Event {
            id: Hash::digest(name.as_bytes()),
            author: id(by),
            after,
            group,
            body,
        });
        let view = self.view();
        self.model = view.members.keys().copied().collect();
        view
    }

    fn create(&mut self, by: u8, members: &[u8]) -> Envelope {
        let others = members
            .iter()
            .filter(|seed| **seed != by)
            .map(|seed| (id(*seed), Role::Admin))
            .collect();
        let create = Event {
            id: Hash::digest(b"event 0"),
            author: id(by),
            after: Vec::new(),
            group: None,
            body: Body::Create(Create {
                template: Template::Custom,
                policy: scheme_policy(SCHEME_V1),
                members: others,
                admins_on_create: false,
            }),
        };
        self.events = vec![create];
        let view = self.view();
        self.model = view.members.keys().copied().collect();
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .create(&view, &self.rng)
            .expect("create")
    }

    fn add(&mut self, by: u8, who: u8) -> Envelope {
        let view = self.log(
            by,
            Body::Member(MemberOp::Add {
                member: id(who),
                role: Role::Admin,
            }),
        );
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .add(&view, id(who), &self.rng)
            .expect("add")
    }

    fn remove(&mut self, by: u8, who: u8) -> Envelope {
        let view = self.log(
            by,
            Body::Member(MemberOp::Remove {
                member: id(who),
                successor: None,
            }),
        );
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .remove(&view, id(who), &self.rng)
            .expect("remove")
    }

    /// A join through `add_from_join`: the add, then the immediate update.
    fn join(&mut self, by: u8, who: u8) -> Vec<Envelope> {
        let view = self.log(
            by,
            Body::Member(MemberOp::Add {
                member: id(who),
                role: Role::Admin,
            }),
        );
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .add_from_join(&view, id(who), &self.rng)
            .expect("add_from_join")
    }

    fn update(&mut self, by: u8) -> Envelope {
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .update(&self.rng)
            .expect("update")
    }

    fn send(&mut self, by: u8, message: &str) -> Envelope {
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .send(message.as_bytes(), &self.rng)
            .expect("send")
    }

    /// Hands `message` to each of `to`, in order, and returns what each one read.
    fn deliver(&mut self, message: &Envelope, to: &[u8]) -> Vec<Vec<Received>> {
        to.iter()
            .map(|seed| {
                self.sessions
                    .get_mut(&id(*seed))
                    .expect("session")
                    .process(message)
                    .expect("process")
            })
            .collect()
    }

    fn members_of(&self, who: u8) -> BTreeSet<Key> {
        self.session(who).members().expect("members")
    }

    fn epoch(&self, who: u8) -> Option<GroupSecretId> {
        self.session(who).epoch()
    }
}

// --- create ---------------------------------------------------------------------------------------

#[test]
fn test_create_gives_every_member_the_model_roster() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    for who in [1, 2, 3] {
        assert!(cast.session(who).is_welcomed(), "member {who} is welcomed");
        assert_eq!(cast.members_of(who), cast.model, "member {who} roster");
    }
    assert_eq!(cast.epoch(1), cast.epoch(2));
    assert_eq!(cast.epoch(2), cast.epoch(3));
}

#[test]
fn test_create_rejects_a_device_outside_the_model() {
    let mut cast = Cast::new(&[1, 2]);
    let view = view_of(&std::iter::once(id(2)).collect());
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .create(&view, &rng);
    assert!(matches!(result, Err(SchemeError::NotInModel(_))));
    assert!(
        !cast.session(1).is_welcomed(),
        "a rejected create leaves the session usable"
    );
}

#[test]
fn test_operations_before_create_are_rejected_without_poisoning() {
    let mut cast = Cast::new(&[1, 2]);
    let rng = Rng::default();
    let session = cast.sessions.get_mut(&id(1)).expect("session");
    assert!(matches!(
        session.send(b"early", &rng),
        Err(SchemeError::NotWelcomed)
    ));
    assert!(matches!(
        session.update(&rng),
        Err(SchemeError::NotWelcomed)
    ));
    let view = view_of(&[id(1), id(2)].into_iter().collect());
    assert!(
        session.create(&view, &rng).is_ok(),
        "the session still creates"
    );
}

// --- add ------------------------------------------------------------------------------------------

#[test]
fn test_add_is_applied_by_the_adder_and_the_existing_members() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    let add = cast.add(1, 4);
    cast.deliver(&add, &[2, 3]);

    for who in [1, 2, 3] {
        assert_eq!(
            cast.members_of(who),
            cast.model,
            "member {who} roster after add"
        );
    }
}

#[test]
fn test_add_rejects_a_key_the_model_does_not_list() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2]);
    cast.deliver(&create, &[2]);

    let view = cast.view();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add(&view, id(3), &rng);
    assert!(matches!(result, Err(SchemeError::NotInModel(_))));
    assert_eq!(cast.members_of(1), cast.model, "the roster is untouched");
}

// --- update ---------------------------------------------------------------------------------------

#[test]
fn test_update_rotates_the_epoch_for_every_member() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let before = cast.epoch(1);

    let update = cast.update(2);
    cast.deliver(&update, &[1, 3]);
    assert_ne!(cast.epoch(1), before, "the update rotates the epoch");
    assert_eq!(cast.epoch(1), cast.epoch(2));
    assert_eq!(cast.epoch(2), cast.epoch(3));

    let message = cast.send(1, "after the update");
    let read = cast.deliver(&message, &[2, 3]);
    assert_eq!(
        read,
        vec![
            vec![text("after the update")],
            vec![text("after the update")]
        ]
    );
}

// --- remove ---------------------------------------------------------------------------------------

#[test]
fn test_remove_takes_the_member_out_and_it_cannot_read_later_messages() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    let remove = cast.remove(1, 3);
    let read = cast.deliver(&remove, &[2, 3]);
    assert_eq!(
        read[1],
        vec![Received::Removed],
        "the removed member learns it was removed"
    );
    assert_eq!(
        cast.members_of(1),
        cast.model,
        "the remover's roster drops the member"
    );

    let message = cast.send(1, "after the removal");
    assert_eq!(
        cast.deliver(&message, &[2]),
        vec![vec![text("after the removal")]]
    );
    let to_removed = cast.deliver(&message, &[3]);
    assert_eq!(
        to_removed,
        vec![vec![Received::Withheld(message.id())]],
        "the removed member cannot read a message sent under the new epoch"
    );
}

#[test]
fn test_remove_rejects_a_member_the_model_still_lists() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    let view = cast.view();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .remove(&view, id(3), &rng);
    assert!(matches!(result, Err(SchemeError::StillInModel(_))));
}

// --- integration: three members -------------------------------------------------------------------

#[test]
fn test_three_members_create_send_update_and_remove_in_step() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    let first = cast.send(2, "first");
    assert_eq!(
        cast.deliver(&first, &[1, 3]),
        vec![vec![text("first")], vec![text("first")]]
    );

    let update = cast.update(3);
    cast.deliver(&update, &[1, 2]);
    let second = cast.send(1, "second");
    assert_eq!(
        cast.deliver(&second, &[2, 3]),
        vec![vec![text("second")], vec![text("second")]]
    );

    let remove = cast.remove(1, 2);
    let read = cast.deliver(&remove, &[2, 3]);
    assert_eq!(read[0], vec![Received::Removed]);
    assert_eq!(cast.members_of(1), cast.model);
    assert_eq!(
        cast.members_of(3),
        cast.model,
        "the remaining member agrees"
    );
    assert_eq!(
        cast.epoch(1),
        cast.epoch(3),
        "the remaining members share an epoch"
    );

    let third = cast.send(3, "third");
    assert_eq!(cast.deliver(&third, &[1]), vec![vec![text("third")]]);
}

// --- a joiner (P4.2) ------------------------------------------------------------------------------

#[test]
fn test_joiner_reads_messages_sent_after_the_add() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);

    let add = cast.add(1, 4);
    cast.deliver(&add, &[2, 3, 4]);
    assert!(cast.session(4).is_welcomed(), "the add welcomes the joiner");
    assert_eq!(
        cast.members_of(4),
        cast.model,
        "the joiner's roster matches the model"
    );

    let message = cast.send(2, "after the join");
    assert_eq!(
        cast.deliver(&message, &[1, 3, 4]),
        vec![
            vec![text("after the join")],
            vec![text("after the join")],
            vec![text("after the join")]
        ]
    );
}

#[test]
fn test_five_members_converge_through_add_update_and_remove() {
    let mut cast = Cast::new(&[1, 2, 3, 4, 5]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4, 5]);

    let add_four = cast.add(1, 4);
    cast.deliver(&add_four, &[2, 3, 4, 5]);
    let add_five = cast.add(2, 5);
    cast.deliver(&add_five, &[1, 3, 4, 5]);

    let update = cast.update(5);
    cast.deliver(&update, &[1, 2, 3, 4]);
    for who in [1, 2, 3, 4, 5] {
        assert_eq!(
            cast.members_of(who),
            cast.model,
            "member {who} roster after joins"
        );
        assert_eq!(cast.epoch(who), cast.epoch(1), "member {who} epoch");
    }

    let before = cast.send(4, "before the removal");
    cast.deliver(&before, &[1, 2, 3, 5]);

    let remove = cast.remove(1, 2);
    let read = cast.deliver(&remove, &[2, 3, 4, 5]);
    assert_eq!(read[0], vec![Received::Removed]);

    let after = cast.send(5, "after the removal");
    assert_eq!(
        cast.deliver(&after, &[1, 3, 4]),
        vec![
            vec![text("after the removal")],
            vec![text("after the removal")],
            vec![text("after the removal")]
        ]
    );
    assert_eq!(
        cast.epoch(3),
        cast.epoch(5),
        "the remaining members share an epoch"
    );
    assert_eq!(cast.members_of(5), cast.model);
}

// --- from_join (P4.3) -----------------------------------------------------------------------------

#[test]
fn test_from_join_rejects_a_key_the_model_does_not_list() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);

    let view = cast.view();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add_from_join(&view, id(4), &rng);
    assert!(matches!(result, Err(SchemeError::NotInModel(_))));
    assert_eq!(cast.members_of(1), cast.model, "the roster is untouched");
}

#[test]
fn test_from_join_welcomes_the_joiner_into_a_new_epoch() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);
    let before = cast.epoch(1);

    let messages = cast.join(1, 4);
    assert_eq!(
        messages.len(),
        2,
        "the join is an add and an immediate update"
    );
    cast.deliver(&messages[0], &[2, 3, 4]);
    cast.deliver(&messages[1], &[2, 3, 4]);

    assert!(cast.session(4).is_welcomed(), "the joiner is welcomed");
    assert_eq!(
        cast.members_of(4),
        cast.model,
        "the joiner's roster matches the model"
    );
    assert_ne!(
        cast.epoch(4),
        before,
        "the joiner holds the epoch the join started"
    );
    assert_eq!(cast.epoch(4), cast.epoch(1));
}

#[test]
fn test_joiner_cannot_read_a_message_from_before_its_join_and_reads_one_after() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);
    let before = cast.send(2, "before the join");
    cast.deliver(&before, &[1, 3]);

    let messages = cast.join(1, 4);
    cast.deliver(&messages[0], &[2, 3, 4]);

    let gap = cast.send(2, "between the add and the update");
    assert_eq!(
        cast.deliver(&gap, &[4]),
        vec![vec![Received::Withheld(gap.id())]],
        "a message sent before the update is under the old epoch, and is withheld"
    );

    cast.deliver(&messages[1], &[2, 3, 4]);
    let after = cast.send(2, "after the update");
    assert_eq!(
        cast.deliver(&after, &[4]),
        vec![vec![text("after the update")]]
    );

    assert_eq!(
        cast.deliver(&before, &[4]),
        vec![vec![Received::Withheld(before.id())]],
        "the joiner does not read a message sent before its join"
    );
}

// --- wire (P4.4) ----------------------------------------------------------------------------------

/// The CBOR length of the add that welcomes a fourth member after `rotations` updates. Each update
/// adds one secret, and the welcome carries every secret the group holds.
fn add_welcome_len(rotations: usize) -> usize {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    for _ in 0..rotations {
        let update = cast.update(1);
        cast.deliver(&update, &[2, 3]);
    }
    let add = cast.add(1, 4);
    encode_cbor(&add).expect("encode the add").len()
}

#[test]
fn test_each_epoch_adds_about_forty_bytes_to_the_welcome() {
    // S12: serde_bytes on the two byte fields costs 40.0 bytes per epoch. The stock integer arrays
    // cost 146.5.
    let grown = add_welcome_len(110) - add_welcome_len(10);
    assert!(
        (3950..=4050).contains(&grown),
        "100 epochs grew the welcome by {grown} bytes, expected about 4000"
    );
}

#[test]
fn test_v1_bytes_decode_and_the_joiner_reads_them() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);

    let add = cast.add(1, 4);
    let bytes = encode_cbor(&add).expect("encode the add");
    let received: Envelope = decode_cbor(bytes.as_slice()).expect("decode the add");
    cast.deliver(&received, &[4]);
    assert!(
        cast.session(4).is_welcomed(),
        "the decoded welcome welcomes"
    );

    let message = cast.send(2, "over the wire");
    let bytes = encode_cbor(&message).expect("encode the message");
    let received: Envelope = decode_cbor(bytes.as_slice()).expect("decode the message");
    assert_eq!(
        cast.deliver(&received, &[4]),
        vec![vec![text("over the wire")]]
    );
}

#[test]
fn test_an_envelope_re_encodes_to_the_same_bytes() {
    // The bundle is sorted inside the welcome, so a decoded envelope is encoded again unchanged.
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    for _ in 0..3 {
        let update = cast.update(1);
        cast.deliver(&update, &[2, 3]);
    }
    let add = cast.add(1, 4);
    let first = encode_cbor(&add).expect("encode the add");
    let decoded: Envelope = decode_cbor(first.as_slice()).expect("decode the add");
    assert_eq!(encode_cbor(&decoded).expect("encode again"), first);
}

#[test]
fn test_a_welcome_with_a_v0_copy_is_refused() {
    // A relay that rewrites the welcome's copy to the stock version. The header still names v1.
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let forged = cast.add(1, 4).with_scheme(SCHEME_V1, 0);

    let result = cast
        .sessions
        .get_mut(&id(4))
        .expect("session")
        .process(&forged);
    assert!(
        matches!(
            result,
            Err(SchemeError::SchemeMismatch {
                expected: SCHEME_V1,
                found: 0
            })
        ),
        "the welcome's copy is checked against the signed policy: {result:?}"
    );
    assert!(
        !cast.session(4).is_welcomed(),
        "the refused welcome leaves the joiner unwelcomed"
    );
}

#[test]
fn test_a_header_that_disagrees_with_the_policy_is_refused() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let message = cast.send(1, "downgraded");
    let forged = message.with_scheme(0, 0);

    let result = cast
        .sessions
        .get_mut(&id(2))
        .expect("session")
        .process(&forged);
    assert!(
        matches!(
            result,
            Err(SchemeError::SchemeMismatch {
                expected: SCHEME_V1,
                found: 0
            })
        ),
        "the header is checked against the signed policy: {result:?}"
    );
}

#[test]
fn test_the_policy_names_the_scheme_this_build_speaks() {
    assert!(matches!(
        negotiate(&scheme_policy(SCHEME_V1)),
        Ok(SCHEME_V1)
    ));
    assert!(matches!(
        negotiate(&Policy::default()),
        Err(SchemeError::NoScheme)
    ));
    assert!(matches!(
        negotiate(&scheme_policy(0)),
        Err(SchemeError::UnsupportedScheme { version: 0, .. })
    ));
    assert!(matches!(
        negotiate(&scheme_policy(2)),
        Err(SchemeError::UnsupportedScheme { version: 2, .. })
    ));
    let other = Policy {
        scheme: Some(Scheme {
            name: "other".to_owned(),
            version: SCHEME_V1,
        }),
        ..Policy::default()
    };
    assert!(matches!(
        negotiate(&other),
        Err(SchemeError::UnsupportedScheme { .. })
    ));
}

#[test]
fn test_a_stock_group_is_refused_not_migrated() {
    // The stock layout (v0) is not shipped, so a group that names it is refused when a device
    // starts its session. Nothing is migrated (scheme-serde RESULTS, the recommendation).
    let (_, verifying) = person(1);
    let rng = Rng::default();
    let identity = IdentitySecret::from_rng(&rng).expect("identity");
    let keys = new_keys(&identity, Lifetime::default(), &rng).expect("keys");
    let registry = registry(&[]).expect("registry");
    let result = Session::init(verifying, keys, registry, &scheme_policy(0));
    assert!(
        matches!(
            result,
            Err(SchemeError::UnsupportedScheme { version: 0, .. })
        ),
        "a stock group is refused when the session starts"
    );
}

// --- the roster follows the fold (P4.13) ----------------------------------------------------------

#[test]
fn test_a_membership_change_through_the_group_path_updates_the_roster() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2]);
    cast.deliver(&create, &[2]);
    assert_eq!(cast.members_of(1), [id(1), id(2)].into_iter().collect());

    cast.add(1, 3);
    assert_eq!(
        cast.members_of(1),
        [id(1), id(2), id(3)].into_iter().collect(),
        "an add through the fold reaches the roster"
    );

    cast.remove(1, 2);
    assert_eq!(
        cast.members_of(1),
        [id(1), id(3)].into_iter().collect(),
        "a removal through the fold reaches the roster"
    );
}

#[test]
fn test_a_view_that_disagrees_with_the_roster_is_refused_and_poisons_the_session() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2]);
    cast.deliver(&create, &[2]);

    // The fold lists 4 as well, but the crate is only asked to add 3.
    let mut view = cast.view();
    view.members.insert(id(3), Role::Admin);
    view.members.insert(id(4), Role::Admin);
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add(&view, id(3), &rng);
    assert!(
        matches!(result, Err(SchemeError::RosterMismatch(_))),
        "a mismatch is refused: {result:?}"
    );
    assert!(
        matches!(cast.session(1).members(), Err(SchemeError::Poisoned)),
        "the session does not keep a roster the fold disagrees with"
    );
    assert!(matches!(
        cast.sessions
            .get_mut(&id(1))
            .expect("session")
            .send(b"after", &rng),
        Err(SchemeError::Poisoned)
    ));
}

#[test]
fn test_a_view_whose_policy_names_no_scheme_is_refused_without_poisoning() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2]);
    cast.deliver(&create, &[2]);

    let mut view = cast.view();
    view.policy = Policy::default();
    view.members.insert(id(3), Role::Admin);
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add(&view, id(3), &rng);
    assert!(matches!(result, Err(SchemeError::NoScheme)), "{result:?}");
    assert_eq!(
        cast.members_of(1),
        [id(1), id(2)].into_iter().collect(),
        "the refused change left the roster as it was"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Random adds, joins and removes through the group path. After each one, the creator's roster
    /// is the set of members the fold gives.
    #[test]
    fn test_roster_follows_the_fold_through_random_changes(
        ops in proptest::collection::vec((0u8..3, 2u8..6), 1..24),
    ) {
        let mut cast = Cast::new(&[1, 2, 3, 4, 5]);
        let create = cast.create(1, &[1, 2]);
        cast.deliver(&create, &[2]);
        for (kind, who) in ops {
            let present = cast.model.contains(&id(who));
            match kind {
                0 if !present => {
                    cast.add(1, who);
                }
                1 if !present => {
                    cast.join(1, who);
                }
                2 if present => {
                    cast.remove(1, who);
                }
                _ => continue,
            }
            let folded: BTreeSet<Key> = cast.view().members.keys().copied().collect();
            prop_assert_eq!(cast.members_of(1), folded, "after {} of {}", kind, who);
        }
    }
}

// --- property -------------------------------------------------------------------------------------

/// The seeds of the people the group model lists.
fn current_seeds(cast: &Cast, people: &[u8]) -> Vec<u8> {
    people
        .iter()
        .copied()
        .filter(|seed| cast.model.contains(&id(*seed)))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Random joins (from_join and full add), removals, updates and sends. At the end every member
    /// agrees on the roster and the epoch, every member reads the newest message, and every removed
    /// member is refused it.
    #[test]
    fn test_random_operations_keep_members_in_agreement(
        ops in proptest::collection::vec((0u8..5, any::<u8>()), 1..16),
    ) {
        const PEOPLE: [u8; 5] = [1, 2, 3, 4, 5];
        let mut cast = Cast::new(&PEOPLE);
        let create = cast.create(1, &[1, 2, 3]);
        cast.deliver(&create, &[2, 3, 4, 5]);

        let mut waiting: Vec<u8> = vec![4, 5];
        let mut removed: Vec<u8> = Vec::new();
        for (step, (kind, pick)) in ops.into_iter().enumerate() {
            let current = current_seeds(&cast, &PEOPLE);
            let by = current[usize::from(pick) % current.len()];
            // Control messages reach everyone not removed, so a joiner has the history it needs.
            let audience: Vec<u8> = PEOPLE
                .iter()
                .copied()
                .filter(|seed| *seed != by && !removed.contains(seed))
                .collect();
            match kind {
                0 if !waiting.is_empty() => {
                    let who = waiting.remove(0);
                    let messages = cast.join(by, who);
                    cast.deliver(&messages[0], &audience);
                    cast.deliver(&messages[1], &audience);
                }
                1 if !waiting.is_empty() => {
                    let who = waiting.remove(0);
                    let add = cast.add(by, who);
                    cast.deliver(&add, &audience);
                }
                2 if current.len() > 2 => {
                    let others: Vec<u8> = current.iter().copied().filter(|seed| *seed != by).collect();
                    let victim = others[usize::from(pick) % others.len()];
                    let remove = cast.remove(by, victim);
                    cast.deliver(&remove, &audience);
                    removed.push(victim);
                }
                3 => {
                    let update = cast.update(by);
                    cast.deliver(&update, &audience);
                }
                _ => {
                    let text = format!("message {step}");
                    let message = cast.send(by, &text);
                    let readers: Vec<u8> = current.iter().copied().filter(|seed| *seed != by).collect();
                    for read in cast.deliver(&message, &readers) {
                        prop_assert_eq!(read, vec![Received::Plaintext(text.as_bytes().to_vec())]);
                    }
                }
            }
        }

        let members = current_seeds(&cast, &PEOPLE);
        for who in &members {
            prop_assert_eq!(cast.members_of(*who), cast.model.clone(), "member {} roster", who);
        }
        let epochs: BTreeSet<_> = members.iter().map(|who| cast.epoch(*who)).collect();
        prop_assert_eq!(epochs.len(), 1, "every member holds the same epoch");

        let last = cast.send(members[0], "last");
        let readers: Vec<u8> = members.iter().copied().filter(|seed| *seed != members[0]).collect();
        for read in cast.deliver(&last, &readers) {
            prop_assert_eq!(read, vec![text("last")]);
        }
        for gone in &removed {
            let read = cast.deliver(&last, &[*gone]);
            prop_assert_eq!(read, vec![vec![Received::Withheld(last.id())]], "removed member {}", gone);
        }
    }
}
