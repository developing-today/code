//! Tests for the `DataScheme` adapter: unit tests per operation, and a three-member integration.
//!
//! Everyone in a cast gets keys and a session up front. Control messages go to every member of the
//! cast, since a joiner needs the control history to take the welcome. Application messages go only
//! to the members who may read them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey as IdentitySecret;
use p2panda_encryption::data_scheme::group_secret::GroupSecretId;
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::KeyManagerState;
use p2panda_encryption::key_registry::KeyRegistryState;
use p2panda_encryption::traits::GroupMessage;
use proptest::prelude::*;
use proptest::test_runner::TestCaseResult;

use super::history::{HISTORY_WAIT, HistoryAnswer, HistoryError, ask_member};
use super::welcome::{CHUNK_HEADER_BYTES, Reassembly, WelcomeError, split};
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
    /// Each person's identity secret, to open the answers sealed to them.
    identities: BTreeMap<Key, IdentitySecret>,
    /// The group's signed events, in order. The first is the create, and the group model is their
    /// fold.
    events: Vec<Event>,
    /// The member set the group model holds after the last operation.
    model: BTreeSet<Key>,
    /// Each person's identity and keys, kept so a device can be rebuilt with the same keys.
    keys: BTreeMap<Key, (VerifyingKey, KeyManagerState)>,
    registry: KeyRegistryState<VerifyingKey>,
}

impl Cast {
    fn new(seeds: &[u8]) -> Self {
        let rng = Rng::default();
        let mut bundles = Vec::new();
        let mut keys: BTreeMap<Key, (VerifyingKey, KeyManagerState)> = BTreeMap::new();
        let mut identities = BTreeMap::new();
        for seed in seeds {
            let (key, verifying) = person(*seed);
            let identity = IdentitySecret::from_rng(&rng).expect("identity");
            let state = new_keys(&identity, Lifetime::default(), &rng).expect("keys");
            bundles.push((verifying, key_bundle(&state).expect("bundle")));
            keys.insert(key, (verifying, state));
            identities.insert(key, identity);
        }
        let registry = registry(&bundles).expect("registry");
        let sessions = keys
            .iter()
            .map(|(key, (verifying, state))| {
                let session = Session::init(
                    *verifying,
                    state.clone(),
                    registry.clone(),
                    &scheme_policy(SCHEME_V1),
                )
                .expect("session");
                (*key, session)
            })
            .collect();
        Self {
            rng,
            sessions,
            identities,
            events: Vec::new(),
            model: BTreeSet::new(),
            keys,
            registry,
        }
    }

    /// A fresh session for `who` with the keys it was created with. It holds no group secret.
    fn reinstall(&self, who: u8) -> Session {
        let (verifying, state) = self.keys.get(&id(who)).expect("keys");
        Session::init(
            *verifying,
            state.clone(),
            self.registry.clone(),
            &scheme_policy(SCHEME_V1),
        )
        .expect("session")
    }

    fn session(&self, who: u8) -> &Session {
        self.sessions.get(&id(who)).expect("session")
    }

    fn session_mut(&mut self, who: u8) -> &mut Session {
        self.sessions.get_mut(&id(who)).expect("session")
    }

    fn identity(&self, who: u8) -> &IdentitySecret {
        self.identities.get(&id(who)).expect("identity")
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

    /// An add that welcomes the joiner with the secrets `mode` allows.
    fn add_with_history(&mut self, by: u8, who: u8, mode: HistoryMode) -> Envelope {
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
            .add_with_history(&view, id(who), mode, &self.rng)
            .expect("add_with_history")
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

    fn epoch_set(&self, who: u8) -> BTreeSet<GroupSecretId> {
        self.session(who).epoch_set()
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

// --- history modes -------------------------------------------------------------------------------

/// Every window size and epoch count: a `recent:N` welcome succeeds, the joiner holds the newest
/// `min(N, K + 1)` epochs, and it reads a message sent after the join.
#[test]
fn test_recent_welcome_succeeds_for_every_window_and_epoch_count() {
    for window in [1, 8, 32, 128, 512] {
        for rotations in [0, 1, 5, 40] {
            let mode = HistoryMode::Recent(window);
            let mut cast = Cast::new(&[1, 2, 3, 4]);
            let create = cast.create(1, &[1, 2, 3]);
            cast.deliver(&create, &[2, 3, 4]);
            for _ in 0..rotations {
                let update = cast.update(1);
                cast.deliver(&update, &[2, 3, 4]);
            }

            let add = cast.add_with_history(1, 4, mode);
            cast.deliver(&add, &[2, 3, 4]);
            let label = format!("{mode} after {rotations} rotations");
            assert!(cast.session(4).is_welcomed(), "{label}: welcomed");
            assert_eq!(cast.members_of(4), cast.model, "{label}: roster");
            assert_eq!(cast.epoch(4), cast.epoch(1), "{label}: newest epoch");
            let epochs = usize::try_from(window).expect("window").min(rotations + 1);
            assert_eq!(cast.session(4).epoch_count(), epochs, "{label}: window");

            let message = cast.send(2, "after the join");
            assert_eq!(
                cast.deliver(&message, &[1, 3, 4]),
                vec![
                    vec![text("after the join")],
                    vec![text("after the join")],
                    vec![text("after the join")]
                ],
                "{label}: the joiner reads after its join"
            );
        }
    }
}

/// A full welcome carries every epoch, and the joiner holds all of them.
#[test]
fn test_full_welcome_carries_every_epoch() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);
    for _ in 0..5 {
        let update = cast.update(1);
        cast.deliver(&update, &[2, 3, 4]);
    }

    let add = cast.add_with_history(1, 4, HistoryMode::Full);
    cast.deliver(&add, &[2, 3, 4]);
    assert_eq!(
        cast.session(4).epoch_count(),
        6,
        "every epoch the adder holds"
    );
}

/// Builds `rotations` epochs for the group of 1, 2 and 3, and sends one message under each epoch.
/// Members 2 and 3 take every message as it goes. Returns every group message in send order, and
/// the application messages alone, oldest first.
fn history(cast: &mut Cast, rotations: usize) -> (Vec<Envelope>, Vec<Envelope>) {
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let mut all = vec![create];
    let mut apps = Vec::new();
    for epoch in 0..=rotations {
        let message = cast.send(1, &format!("epoch {epoch}"));
        cast.deliver(&message, &[2, 3]);
        all.push(message.clone());
        apps.push(message);
        if epoch < rotations {
            let update = cast.update(1);
            cast.deliver(&update, &[2, 3]);
            all.push(update);
        }
    }
    (all, apps)
}

/// The crate's refusal, reproduced. A joiner handed its pre-welcome history with no secret check
/// (the crate receives every message) fails its welcome whenever the window leaves an epoch out:
/// `recent:N` with N no larger than the rotations. Round 2 saw this in 80 of 120 runs, and never
/// in a run where the window held every epoch (P4.5, round 2 `welcome-recent` §E6).
#[test]
fn test_crate_refuses_the_welcome_when_pre_welcome_history_names_an_unknown_secret() {
    let windows = [Some(1), Some(8), Some(32), Some(128), Some(512), None];
    let mut runs = 0;
    let mut refused = 0;
    for rotations in [4, 20, 40] {
        for window in windows {
            let mode = window.map_or(HistoryMode::Full, HistoryMode::Recent);
            let mut cast = Cast::new(&[1, 2, 3, 4]);
            let (all, _) = history(&mut cast, rotations);
            for message in &all {
                cast.sessions
                    .get_mut(&id(4))
                    .expect("session")
                    .process_unchecked(message)
                    .expect("pre-welcome message");
            }
            let add = cast.add_with_history(1, 4, mode);
            let welcome = cast
                .sessions
                .get_mut(&id(4))
                .expect("session")
                .process_unchecked(&add);

            runs += 1;
            let label = format!("{mode} after {rotations} rotations");
            let expect_refused =
                window.is_some_and(|n| usize::try_from(n).expect("window") <= rotations);
            assert_eq!(welcome.is_err(), expect_refused, "{label}");
            if let Err(err) = welcome {
                refused += 1;
                assert!(
                    matches!(&err, SchemeError::Crate(text) if text.contains("unknown group secret")),
                    "{label}: {err}"
                );
            }
        }
    }
    assert_eq!(
        (refused, runs),
        (6, 18),
        "refused welcomes out of runs, by the pattern above"
    );
}

/// A recent joiner reads the window of history it was handed before its welcome, in the order it
/// arrived, and refuses the older messages. The welcome releases the window (P4.5). Every window
/// and epoch count is tried.
#[test]
fn test_recent_joiner_reads_its_window_of_history_handed_over_before_the_welcome() {
    for window in [1, 8, 32, 128, 512] {
        for rotations in [0, 1, 5, 40] {
            let mut cast = Cast::new(&[1, 2, 3, 4]);
            let (all, apps) = history(&mut cast, rotations);
            for message in &all {
                cast.deliver(message, &[4]);
            }

            let add = cast.add_with_history(1, 4, HistoryMode::Recent(window));
            let read = cast
                .sessions
                .get_mut(&id(4))
                .expect("session")
                .process(&add)
                .expect("welcome");

            let epochs = usize::try_from(window).expect("window").min(rotations + 1);
            let label = format!("recent:{window} after {rotations} rotations");
            let expected: Vec<Received> = (rotations + 1 - epochs..=rotations)
                .map(|epoch| text(&format!("epoch {epoch}")))
                .collect();
            assert_eq!(read, expected, "{label}: the window is released");
            if epochs <= rotations {
                assert_eq!(
                    cast.deliver(&apps[0], &[4]),
                    vec![vec![Received::Withheld(apps[0].id())]],
                    "{label}: the oldest epoch stays refused"
                );
            }
        }
    }
}

// --- lazy history ---------------------------------------------------------------------------------

/// A joiner with a `recent:N` welcome asks a member for the older epochs it was handed and cannot
/// read. The member answers, and the joiner reads them. A key sealed to someone else does not open.
#[test]
fn test_joiner_gets_its_older_epochs_from_a_member() {
    let mut cast = Cast::new(&[1, 2, 3, 4, 5]);
    let (all, _) = history(&mut cast, 10);
    for message in &all {
        cast.deliver(message, &[4]);
    }
    let add = cast.add_with_history(1, 4, HistoryMode::Recent(4));
    cast.deliver(&add, &[4]);
    assert_eq!(cast.session(4).epoch_count(), 4, "the window");

    let request = cast.session(4).request_history().expect("older epochs");
    assert_eq!(request.wanted.len(), 7, "epochs 0 to 6, outside the window");

    let answer = cast
        .session(1)
        .answer_history(&request)
        .expect("answer")
        .expect("a member answers");
    let outsider = cast.identity(5).clone();
    assert!(
        cast.session_mut(4)
            .open_history(&request, &answer, &outsider)
            .is_err(),
        "an answer does not open with another identity"
    );

    let identity = cast.identity(4).clone();
    let read = cast
        .session_mut(4)
        .open_history(&request, &answer, &identity)
        .expect("open");
    let expected: Vec<Received> = (0..=6)
        .map(|epoch| text(&format!("epoch {epoch}")))
        .collect();
    assert_eq!(read, expected, "the older messages, in send order");
    assert_eq!(cast.session(4).epoch_count(), 11, "every epoch now");
    assert!(
        cast.session(4).request_history().is_none(),
        "nothing left to ask for"
    );
}

/// Only a member answers, and only a member may ask. A device outside the group gets nothing, and a
/// member refuses a request from outside its roster.
#[test]
fn test_a_non_member_gets_no_history_and_a_non_member_answers_nothing() {
    let mut cast = Cast::new(&[1, 2, 3, 4, 5]);
    let (all, _) = history(&mut cast, 6);
    for message in &all {
        cast.deliver(message, &[4, 5]);
    }
    let add = cast.add_with_history(1, 4, HistoryMode::Recent(2));
    cast.deliver(&add, &[4]);

    let outsider_request = cast
        .session(5)
        .request_history()
        .expect("5 holds messages it cannot read");
    assert!(
        cast.session(1)
            .answer_history(&outsider_request)
            .expect("answer")
            .is_none(),
        "a member refuses a request from outside its roster"
    );

    let joiner_request = cast.session(4).request_history().expect("4 asks");
    assert!(
        cast.session(5)
            .answer_history(&joiner_request)
            .expect("answer")
            .is_none(),
        "a device outside the group answers nothing"
    );
}

/// The answer comes back through the transport, within the wait.
#[tokio::test]
async fn test_a_member_answers_through_the_transport() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (all, _) = history(&mut cast, 3);
    for message in &all {
        cast.deliver(message, &[4]);
    }
    let add = cast.add_with_history(1, 4, HistoryMode::Recent(1));
    cast.deliver(&add, &[4]);
    let request = cast.session(4).request_history().expect("older epochs");

    let member = cast.session(1);
    let answer = ask_member(
        request.clone(),
        |asked| std::future::ready(member.answer_history(&asked).expect("answer")),
        HISTORY_WAIT,
    )
    .await
    .expect("answered");
    let identity = cast.identity(4).clone();
    let read = cast
        .session_mut(4)
        .open_history(&request, &answer, &identity)
        .expect("open");
    assert_eq!(read.len(), 3, "the three older epochs");
}

/// A member that is offline never answers, and the wait ends in a timeout rather than a hang.
#[tokio::test]
async fn test_a_request_with_no_answer_times_out_cleanly() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (all, _) = history(&mut cast, 2);
    for message in &all {
        cast.deliver(message, &[4]);
    }
    let request = cast.session(4).request_history().expect("older epochs");

    let outcome = ask_member(
        request,
        |_| std::future::pending::<Option<HistoryAnswer>>(),
        Duration::from_millis(20),
    )
    .await;
    assert_eq!(
        outcome,
        Err(HistoryError::Timeout(Duration::from_millis(20)))
    );
}

// --- welcome chunks -------------------------------------------------------------------------------

/// A frame small enough that a welcome of a few epochs needs several chunks. A chunk is then
/// `FRAME - CHUNK_HEADER_BYTES` bytes of payload.
const SMALL_FRAME: usize = 512;

/// The group of 1, 2 and 3 after `rotations` updates, with 4 holding every control message. Returns
/// the full welcome for 4, and the update that was sent last.
fn full_welcome(cast: &mut Cast, rotations: usize) -> (Envelope, Envelope) {
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);
    let mut last = create;
    for _ in 0..rotations {
        last = cast.update(1);
        cast.deliver(&last, &[2, 3, 4]);
    }
    (cast.add(1, 4), last)
}

/// A full welcome above the frame cap is split into checked chunks, reassembled, and processed.
#[test]
fn test_a_full_welcome_above_the_cap_is_split_and_reassembled() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, _) = full_welcome(&mut cast, 20);

    let chunks = split(&add, SMALL_FRAME).expect("split");
    assert!(chunks.len() > 1, "a full welcome above the cap is split");
    for chunk in &chunks {
        let encoded = encode_cbor(chunk).expect("encode").len();
        assert!(
            encoded <= SMALL_FRAME,
            "chunk {} fits the frame",
            chunk.index
        );
        assert!(
            encoded - chunk.bytes.len() <= CHUNK_HEADER_BYTES,
            "the header of chunk {} is within its budget",
            chunk.index
        );
    }

    let mut joiner = Reassembly::default();
    let mut whole = None;
    for chunk in chunks {
        if let Some(envelope) = joiner.push(&chunk).expect("in order and intact") {
            whole = Some(envelope);
        }
    }
    let whole = whole.expect("the last chunk completes the welcome");
    joiner.finish().expect("complete");

    assert_eq!(
        cast.sessions
            .get_mut(&id(4))
            .expect("session")
            .process(&whole)
            .expect("welcome"),
        Vec::new()
    );
    assert!(
        cast.session(4).is_welcomed(),
        "the reassembled welcome welcomes"
    );
    assert_eq!(cast.session(4).epoch_count(), 21, "every epoch");
}

/// A welcome that fits one frame is one chunk, so every welcome takes the same path.
#[test]
fn test_a_welcome_within_one_frame_is_one_chunk() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, _) = full_welcome(&mut cast, 2);
    let chunks = split(&add, crate::world_session::MAX_FRAME_BYTES).expect("split");
    assert_eq!(chunks.len(), 1);
    let mut joiner = Reassembly::default();
    assert!(joiner.push(&chunks[0]).expect("one chunk").is_some());
}

/// A chunk whose bytes were altered in transit is refused as it arrives.
#[test]
fn test_a_corrupted_chunk_is_refused() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, _) = full_welcome(&mut cast, 20);
    let mut chunks = split(&add, SMALL_FRAME).expect("split");
    chunks[1].bytes[0] ^= 0x01;

    let mut joiner = Reassembly::default();
    assert!(joiner.push(&chunks.remove(0)).expect("chunk 0").is_none());
    assert_eq!(
        joiner.push(&chunks.remove(0)).err(),
        Some(WelcomeError::AlteredChunk { index: 1 })
    );
}

/// A missing chunk is refused: the next one that arrives is out of sequence, and a reassembly that
/// ends early is incomplete.
#[test]
fn test_a_missing_chunk_is_refused() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, _) = full_welcome(&mut cast, 20);
    let mut chunks = split(&add, SMALL_FRAME).expect("split");
    assert!(chunks.len() >= 3, "the test needs a middle chunk");

    let mut joiner = Reassembly::default();
    assert!(joiner.push(&chunks.remove(0)).expect("chunk 0").is_none());
    let skipped = chunks.remove(0);
    assert_eq!(skipped.index, 1);
    assert_eq!(
        joiner.push(&chunks.remove(0)).err(),
        Some(WelcomeError::OutOfSequence {
            expected: 1,
            got: 2
        })
    );
    assert!(matches!(
        joiner.finish(),
        Err(WelcomeError::Incomplete { received: 1, .. })
    ));
}

/// A chunk that checks and keeps its own digest, but carries altered bytes, passes each check and
/// fails the whole. The welcome is refused before it is decoded.
#[test]
fn test_an_altered_welcome_with_consistent_checks_is_refused_before_decoding() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, _) = full_welcome(&mut cast, 20);
    let mut chunks = split(&add, SMALL_FRAME).expect("split");
    let last = chunks.len() - 1;
    chunks[last].bytes[0] ^= 0x01;
    chunks[last].check = Hash::digest(&chunks[last].bytes);

    let mut joiner = Reassembly::default();
    let mut outcome = Ok(None);
    for chunk in chunks {
        outcome = joiner.push(&chunk);
    }
    assert_eq!(outcome.err(), Some(WelcomeError::AlteredWelcome));
}

/// A chunk of another welcome does not join this one.
#[test]
fn test_a_chunk_of_another_welcome_is_refused() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let (add, update) = full_welcome(&mut cast, 20);
    let mut chunks = split(&add, SMALL_FRAME).expect("split");
    let other = split(&update, SMALL_FRAME).expect("split");

    let mut joiner = Reassembly::default();
    assert!(joiner.push(&chunks.remove(0)).expect("chunk 0").is_none());
    assert_eq!(
        joiner
            .push(&other.into_iter().nth(1).expect("chunk 1 of the other"))
            .err(),
        Some(WelcomeError::OtherWelcome)
    );
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

// --- rotation (P4.8) ------------------------------------------------------------------------------

#[test]
fn test_the_epoch_rotates_on_create_update_and_remove_and_not_on_send() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let created = cast.epoch(1);

    let message = cast.send(1, "no rotation");
    cast.deliver(&message, &[2, 3]);
    assert_eq!(cast.epoch(1), created, "a send keeps the epoch");

    let update = cast.update(1);
    cast.deliver(&update, &[2, 3]);
    let updated = cast.epoch(1);
    assert_ne!(updated, created, "an explicit update rotates");

    let remove = cast.remove(1, 3);
    cast.deliver(&remove, &[2, 3]);
    assert_ne!(cast.epoch(1), updated, "a removal rotates");
    assert_eq!(
        cast.epoch(1),
        cast.epoch(2),
        "the members agree on the epoch"
    );
}

#[test]
fn test_an_idle_group_keeps_its_epoch_over_a_simulated_year() {
    // The adapter takes no clock and runs no timer, so the only thing that moves an epoch is a call
    // to create, update, add or remove. Each simulated hour below is a tick with no call at all.
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);
    let epoch = cast.epoch(1);

    for hour in 0..(24 * 365) {
        for who in [1, 2, 3] {
            assert_eq!(cast.epoch(who), epoch, "member {who} at hour {hour}");
        }
    }

    let update = cast.update(1);
    cast.deliver(&update, &[2, 3]);
    assert_ne!(
        cast.epoch(1),
        epoch,
        "the group still rotates when it is asked to"
    );
}

#[test]
fn test_recovery_puts_the_device_back_and_its_compromised_copy_cannot_read() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    // The attacker's copy of device 3 holds the state it had before the compromise was found.
    let mut compromised = cast.sessions.remove(&id(3)).expect("session");
    let before = cast.epoch(1);
    // The fold's views after the removal and after the re-add, in that order.
    let without = cast.log(
        1,
        Body::Member(MemberOp::Remove {
            member: id(3),
            successor: None,
        }),
    );
    let with = cast.log(
        1,
        Body::Member(MemberOp::Add {
            member: id(3),
            role: Role::Admin,
        }),
    );

    let recovery = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .recover(&without, &with, id(3), &cast.rng)
        .expect("recover");
    assert_eq!(recovery.epoch_before, before);
    assert_ne!(
        recovery.epoch_after, before,
        "the procedure rotates the epoch"
    );
    assert_eq!(recovery.readd.len(), 2, "a welcome and an immediate update");

    // The device comes back as a new session with its own keys and no group secret.
    let fresh = cast.reinstall(3);
    cast.sessions.insert(id(3), fresh);
    cast.deliver(&create, &[3]);
    cast.deliver(&recovery.removal, &[2, 3]);
    cast.deliver(&recovery.readd[0], &[2, 3]);
    cast.deliver(&recovery.readd[1], &[2, 3]);
    assert!(
        cast.session(3).is_welcomed(),
        "the re-add welcomes the device"
    );
    assert_eq!(
        cast.members_of(3),
        cast.model,
        "the device's roster matches"
    );

    let message = cast.send(1, "after the recovery");
    assert_eq!(
        cast.deliver(&message, &[2, 3]),
        vec![
            vec![text("after the recovery")],
            vec![text("after the recovery")]
        ]
    );

    assert_eq!(compromised.epoch(), recovery.epoch_before);
    assert_eq!(
        compromised.process(&message).expect("process"),
        vec![Received::Withheld(message.id())],
        "the compromised copy cannot read a message sent after the recovery"
    );
}

#[test]
fn test_recovery_rejects_a_removal_that_leaves_the_device_in_the_model() {
    let mut cast = Cast::new(&[1, 2, 3]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3]);

    let view = cast.view();
    let result =
        cast.sessions
            .get_mut(&id(1))
            .expect("session")
            .recover(&view, &view, id(3), &cast.rng);
    assert!(
        matches!(result, Err(SchemeError::StillInModel(_))),
        "a device the model still lists is not removed"
    );
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

// --- concurrency (P4.7) ---------------------------------------------------------------------------

/// `by` adds `who` from the group as `members` lists it, with nothing else delivered. The add is
/// concurrent with any other change made the same way.
fn concurrent_add(cast: &mut Cast, by: u8, who: u8, members: &[u8]) -> Envelope {
    let members: BTreeSet<Key> = members.iter().map(|seed| id(*seed)).collect();
    let rng = Rng::default();
    cast.session_mut(by)
        .add(&view_of(&members), id(who), &rng)
        .expect("concurrent add")
}

#[test]
fn test_two_concurrent_adds_converge_for_a_third_member() {
    let everyone: BTreeSet<Key> = [1, 2, 3, 4, 5].into_iter().map(id).collect();
    for order in [[0, 1], [1, 0]] {
        let mut cast = Cast::new(&[1, 2, 3, 4, 5]);
        let create = cast.create(1, &[1, 2, 3]);
        cast.deliver(&create, &[2, 3, 4, 5]);

        // Admins 2 and 3 each add a member from the same view, so neither add names the other.
        let adds = [
            concurrent_add(&mut cast, 2, 4, &[1, 2, 3, 4]),
            concurrent_add(&mut cast, 3, 5, &[1, 2, 3, 5]),
        ];
        let authors = [2, 3];

        // Member 1 is the third member and orders the two adds in `order`. The others hear them in
        // the other order, so no two members see the same arrival order.
        for member in [1, 2, 3, 4, 5] {
            let turn = if member == 2 || member == 5 {
                [order[1], order[0]]
            } else {
                order
            };
            for index in turn {
                if authors[index] != member {
                    cast.deliver(&adds[index], &[member]);
                }
            }
        }

        for who in [1, 2, 3, 4, 5] {
            assert_eq!(
                cast.members_of(who),
                everyone,
                "member {who} roster, order {order:?}"
            );
            assert_eq!(
                cast.epoch_set(who),
                cast.epoch_set(1),
                "member {who} epochs, order {order:?}"
            );
        }

        let message = cast.send(4, "after both adds");
        for read in cast.deliver(&message, &[1, 2, 3, 5]) {
            assert_eq!(read, vec![text("after both adds")]);
        }
    }
}

/// A seeded shuffle, so a proptest case replays one delivery order.
fn shuffled<T>(items: Vec<T>, seed: u64) -> Vec<T> {
    let mut keyed: Vec<(u64, T)> = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| (splitmix(seed ^ (index as u64)), item))
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, item)| item).collect()
}

fn splitmix(seed: u64) -> u64 {
    let x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// One change an admin makes in a round of concurrent changes.
#[derive(Clone, Copy, Debug)]
enum Change {
    Update,
    Send,
    /// The add of member 4. A round has at most one.
    Add,
}

/// Admins 1 to `admins` each make one change from the group as it was, before any of them hears
/// from another. Then each member hears the changes it did not make, one at a time, in its own
/// shuffled order, with the members taking turns. Afterwards every member must have the same roster
/// and epoch set, and read every send by another member. A refused message fails the round.
fn check_concurrent_round(changes: &[Change], shuffles: &[u64]) -> TestCaseResult {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);

    let mut made: Vec<(u8, Envelope)> = Vec::new();
    let mut sends: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut joined = false;
    for (author, change) in [1_u8, 2, 3].into_iter().zip(changes) {
        let envelope = match change {
            Change::Add if !joined => {
                joined = true;
                concurrent_add(&mut cast, author, 4, &[1, 2, 3, 4])
            }
            Change::Add | Change::Send => {
                let text = format!("from {author}");
                sends.push((author, text.as_bytes().to_vec()));
                cast.send(author, &text)
            }
            Change::Update => cast.update(author),
        };
        made.push((author, envelope));
    }

    let audience: Vec<u8> = if joined {
        vec![1, 2, 3, 4]
    } else {
        vec![1, 2, 3]
    };
    let mut queues: Vec<(u8, VecDeque<Envelope>)> = audience
        .iter()
        .map(|&member| {
            let theirs: Vec<Envelope> = made
                .iter()
                .filter(|(author, _)| *author != member)
                .map(|(_, envelope)| envelope.clone())
                .collect();
            let seed = shuffles[usize::from(member - 1)];
            (member, shuffled(theirs, seed).into())
        })
        .collect();

    let mut reads: BTreeMap<u8, Vec<Vec<u8>>> = BTreeMap::new();
    while queues.iter().any(|(_, queue)| !queue.is_empty()) {
        for (member, queue) in &mut queues {
            let Some(message) = queue.pop_front() else {
                continue;
            };
            let outputs = cast
                .session_mut(*member)
                .process(&message)
                .map_err(|err| TestCaseError::fail(format!("member {member}: {err}")))?;
            for output in outputs {
                if let Received::Plaintext(bytes) = output {
                    reads.entry(*member).or_default().push(bytes);
                }
            }
        }
    }

    let expected: BTreeSet<Key> = audience.iter().map(|seed| id(*seed)).collect();
    let mut every_send: Vec<Vec<u8>> = sends.iter().map(|(_, bytes)| bytes.clone()).collect();
    every_send.sort();
    for member in &audience {
        prop_assert_eq!(
            cast.members_of(*member),
            expected.clone(),
            "member {} roster",
            member
        );
        prop_assert_eq!(
            cast.epoch_set(*member),
            cast.epoch_set(audience[0]),
            "member {} epoch set",
            member
        );
        let mut heard = reads.remove(member).unwrap_or_default();
        heard.extend(
            sends
                .iter()
                .filter(|(author, _)| author == member)
                .map(|(_, bytes)| bytes.clone()),
        );
        heard.sort();
        prop_assert_eq!(heard, every_send.clone(), "member {} reads", member);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// Two or three admins make concurrent updates and sends (P4.7).
    #[test]
    fn test_concurrent_updates_and_sends_converge(
        admins in 2usize..=3,
        kinds in proptest::collection::vec(any::<bool>(), 3),
        shuffles in proptest::collection::vec(any::<u64>(), 4),
    ) {
        let changes: Vec<Change> = kinds
            .iter()
            .map(|send| if *send { Change::Send } else { Change::Update })
            .take(admins)
            .collect();
        check_concurrent_round(&changes, &shuffles)?;
    }

    /// Two or three admins make concurrent sends, and one of them adds member 4 (P4.7).
    #[test]
    fn test_a_concurrent_add_and_sends_converge(
        admins in 2usize..=3,
        adder in 0usize..3,
        shuffles in proptest::collection::vec(any::<u64>(), 4),
    ) {
        let changes: Vec<Change> = (0..admins)
            .map(|index| if index == adder { Change::Add } else { Change::Send })
            .collect();
        check_concurrent_round(&changes, &shuffles)?;
    }
}

/// Open (P4.7). An update made without knowing a concurrent add never reaches the joiner: the
/// joiner's welcome carries the secrets of the adder's view, and the update's secret is not sent to
/// it. The joiner then holds one epoch fewer than the members, and messages under that epoch are
/// withheld from it. This pins the gap as it is. A refusal, or a fetch of the missing secret through
/// lazy history (P4.6), would change this test.
#[test]
fn test_open_a_joiner_misses_an_update_made_without_knowing_it() {
    let mut cast = Cast::new(&[1, 2, 3, 4]);
    let create = cast.create(1, &[1, 2, 3]);
    cast.deliver(&create, &[2, 3, 4]);

    let add = concurrent_add(&mut cast, 2, 4, &[1, 2, 3, 4]);
    let update = cast.update(1);
    cast.deliver(&add, &[1, 3, 4]);
    cast.deliver(&update, &[2, 3, 4]);

    assert!(
        cast.epoch_set(4).is_subset(&cast.epoch_set(1)),
        "the joiner holds only epochs the members hold"
    );
    assert!(
        cast.epoch_set(4).len() < cast.epoch_set(1).len(),
        "the joiner misses the update's epoch"
    );
    let message = cast.send(1, "under the update's epoch");
    assert_eq!(
        cast.deliver(&message, &[4]),
        vec![vec![Received::Withheld(message.id())]]
    );
}

// --- cost (P4.11) ---------------------------------------------------------------------------------

/// The `index`th person of a benchmark group. [`person`] takes a one-byte seed, which runs out at
/// 256 people, so the benchmark spreads the index over the key bytes.
fn benchmark_person(index: usize) -> (Key, VerifyingKey) {
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
    let verifying = SigningKey::from_bytes(&bytes).verifying_key();
    (key_of(verifying), verifying)
}

/// The committer's cost for one group size: the create, and the mean of a few updates.
#[derive(Debug)]
struct UpdateCost {
    members: usize,
    create_bytes: usize,
    create_ms: f64,
    update_bytes: usize,
    update_ms: f64,
    updates: usize,
}

impl UpdateCost {
    fn report(&self) -> String {
        format!(
            "members {:>5}: create {:>9} B in {:>9.1} ms; update {:>9} B, {:>8.2} ms (mean of {})",
            self.members,
            self.create_bytes,
            self.create_ms,
            self.update_bytes,
            self.update_ms,
            self.updates,
        )
    }
}

/// Builds a group of `members` on one committer's device, and measures the committer's side. The
/// other members are not built, since the cost of an update is the committer's to pay: it encrypts
/// the new secret to each member and processes its own message. The bytes are the postcard size of
/// the envelope, which is what a transport would carry.
fn measure_update_cost(members: usize, updates: usize) -> UpdateCost {
    let rng = Rng::default();
    let mut bundles = Vec::with_capacity(members);
    let mut model = BTreeSet::new();
    let mut committer = None;
    for index in 0..members {
        let (key, verifying) = benchmark_person(index);
        let identity = IdentitySecret::from_rng(&rng).expect("identity");
        let state = new_keys(&identity, Lifetime::default(), &rng).expect("keys");
        bundles.push((verifying, key_bundle(&state).expect("bundle")));
        model.insert(key);
        if index == 0 {
            committer = Some((verifying, state));
        }
    }
    let (me, keys) = committer.expect("a committer");
    let mut session = Session::init(
        me,
        keys,
        registry(&bundles).expect("registry"),
        &scheme_policy(SCHEME_V1),
    )
    .expect("session");

    let started = Instant::now();
    let create = session.create(&view_of(&model), &rng).expect("create");
    let create_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let create_bytes = wire_bytes(&create);

    // One update before the timed ones, so the first-commit cost is not counted twice.
    session.update(&rng).expect("warm-up update");
    let mut sent = Vec::with_capacity(updates);
    let started = Instant::now();
    for _ in 0..updates {
        sent.push(session.update(&rng).expect("update"));
    }
    let count = f64::from(u32::try_from(updates).expect("a few updates"));
    let update_ms = started.elapsed().as_secs_f64() * 1_000.0 / count;
    let update_bytes = sent.iter().map(wire_bytes).sum::<usize>() / updates;

    UpdateCost {
        members,
        create_bytes,
        create_ms,
        update_bytes,
        update_ms,
        updates,
    }
}

fn wire_bytes(envelope: &Envelope) -> usize {
    postcard::to_allocvec(envelope).expect("encode").len()
}

#[test]
fn test_the_update_cost_measurement_grows_with_the_members() {
    let small = measure_update_cost(4, 1);
    let large = measure_update_cost(8, 1);
    assert!(small.update_bytes > 0, "an update is sent");
    assert!(
        large.update_bytes > small.update_bytes,
        "an update carries a message for each member"
    );
}

/// P4.11: what one `DataScheme` update costs the committer at 100, 1,000 and 5,000 members, with our
/// adapter. It records the numbers and fails on none of them. Run it in release:
/// `cargo test --release --lib encryption::tests::bench_update_cost -- --ignored --nocapture`.
#[test]
#[ignore = "release benchmark for P4.11: cargo test --release --lib encryption::tests::bench_update_cost -- --ignored --nocapture"]
fn bench_update_cost() {
    for members in [100, 1_000, 5_000] {
        println!("{}", measure_update_cost(members, 5).report());
    }
}
