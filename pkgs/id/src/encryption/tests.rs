//! Tests for the `DataScheme` adapter: unit tests per operation, and a three-member integration.
//!
//! Everyone in a cast gets keys and a session up front. Control messages go to every member of the
//! cast, since a joiner needs the control history to take the welcome. Application messages go only
//! to the members who may read them.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use p2panda_core::{SigningKey, VerifyingKey};
use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey as IdentitySecret;
use p2panda_encryption::data_scheme::group_secret::GroupSecretId;
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::KeyManagerState;
use p2panda_encryption::key_registry::KeyRegistryState;
use p2panda_encryption::traits::GroupMessage;
use proptest::prelude::*;

use super::*;
use crate::groups::Key;

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

/// Every person's keys and one session per person, all sharing one registry of bundles.
struct Cast {
    rng: Rng,
    sessions: BTreeMap<Key, Session>,
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
        for seed in seeds {
            let (key, verifying) = person(*seed);
            let identity = IdentitySecret::from_rng(&rng).expect("identity");
            let state = new_keys(&identity, Lifetime::default(), &rng).expect("keys");
            bundles.push((verifying, key_bundle(&state).expect("bundle")));
            keys.insert(key, (verifying, state));
        }
        let registry = registry(&bundles).expect("registry");
        let sessions = keys
            .iter()
            .map(|(key, (verifying, state))| {
                let session = Session::init(*verifying, state.clone(), registry.clone());
                (*key, session)
            })
            .collect();
        Self {
            rng,
            sessions,
            model: BTreeSet::new(),
            keys,
            registry,
        }
    }

    /// A fresh session for `who` with the keys it was created with. It holds no group secret.
    fn reinstall(&self, who: u8) -> Session {
        let (verifying, state) = self.keys.get(&id(who)).expect("keys");
        Session::init(*verifying, state.clone(), self.registry.clone())
    }

    fn session(&self, who: u8) -> &Session {
        self.sessions.get(&id(who)).expect("session")
    }

    fn create(&mut self, by: u8, members: &[u8]) -> Envelope {
        self.model = members.iter().map(|seed| id(*seed)).collect();
        let model = self.model.clone();
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .create(&model, &self.rng)
            .expect("create")
    }

    fn add(&mut self, by: u8, who: u8) -> Envelope {
        self.model.insert(id(who));
        let model = self.model.clone();
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .add(&model, id(who), &self.rng)
            .expect("add")
    }

    fn remove(&mut self, by: u8, who: u8) -> Envelope {
        self.model.remove(&id(who));
        let model = self.model.clone();
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .remove(&model, id(who), &self.rng)
            .expect("remove")
    }

    /// A join through `add_from_join`: the add, then the immediate update.
    fn join(&mut self, by: u8, who: u8) -> Vec<Envelope> {
        self.model.insert(id(who));
        let model = self.model.clone();
        self.sessions
            .get_mut(&id(by))
            .expect("session")
            .add_from_join(&model, id(who), &self.rng)
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
    let model: BTreeSet<Key> = std::iter::once(id(2)).collect();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .create(&model, &rng);
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
    let create_model: BTreeSet<Key> = [id(1), id(2)].into_iter().collect();
    assert!(
        session.create(&create_model, &rng).is_ok(),
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

    let model = cast.model.clone();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add(&model, id(3), &rng);
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

    let model = cast.model.clone();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .remove(&model, id(3), &rng);
    assert!(matches!(result, Err(SchemeError::StillInModel(_))));
}

// --- integration: three members -------------------------------------------------------------------

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
    let without: BTreeSet<Key> = [id(1), id(2)].into();
    let with = cast.model.clone();
    let before = cast.epoch(1);

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
    cast.model = with;
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

    let model = cast.model.clone();
    let result =
        cast.sessions
            .get_mut(&id(1))
            .expect("session")
            .recover(&model, &model, id(3), &cast.rng);
    assert!(
        matches!(result, Err(SchemeError::StillInModel(_))),
        "a device the model still lists is not removed"
    );
}

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

    let model = cast.model.clone();
    let rng = Rng::default();
    let result = cast
        .sessions
        .get_mut(&id(1))
        .expect("session")
        .add_from_join(&model, id(4), &rng);
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
    let mut session = Session::init(me, keys, registry(&bundles).expect("registry"));

    let started = Instant::now();
    let create = session.create(&model, &rng).expect("create");
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
