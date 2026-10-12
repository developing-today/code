//! The `DataScheme` adapter: one device's encryption state for one group, over `p2panda-encryption`.
//!
//! Membership comes from the group model. The caller passes the member set the model holds after
//! the event (the `members` of [`crate::groups::GroupView`], from [`crate::groups::fold`]). After
//! each local membership change the adapter checks the crate's roster against that set, so the two
//! cannot drift apart without an error.
//!
//! An add calls `Dcgka::add` with the full secret bundle, then runs the local steps that
//! `EncryptionGroup::add` runs. The crate keeps those steps private, and the lower-level call is
//! what a smaller bundle (`recent:N`) needs.
//!
//! The crate moves the group state into each call and does not return it on error. After a failed
//! call the session reports [`SchemeError::Poisoned`]. Preconditions are checked before the call,
//! so a rejected request does not poison the session.

// The crate's `GroupError` is large, and every crate call returns it. Boxing it would hide the crate's
// error from callers, so the lint is allowed for this module.
#![allow(clippy::result_large_err)]

use std::collections::BTreeSet;

use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::data_scheme::dcgka::{Dcgka, OperationOutput, ProcessInput};
use p2panda_encryption::data_scheme::group::{
    EncryptionGroup, GroupError, GroupOutput, GroupResult, GroupState,
};
use p2panda_encryption::data_scheme::group_secret::{GroupSecret, GroupSecretId};
use p2panda_encryption::key_bundle::{Lifetime, LongTermKeyBundle};
use p2panda_encryption::key_manager::{KeyManager, KeyManagerState};
use p2panda_encryption::key_registry::{KeyRegistry, KeyRegistryState};
use p2panda_encryption::traits::{GroupMessage, GroupMessageContent, Ordering as _, PreKeyManager};
use thiserror::Error;

use super::ordering::{Envelope, Orderer};
use super::roster::Roster;
use crate::groups::Key;

type Pki = KeyRegistry<VerifyingKey>;

type Group = GroupState<VerifyingKey, Hash, Pki, Roster, KeyManager, Orderer>;

type Step<T> = GroupResult<T, VerifyingKey, Hash, Pki, Roster, KeyManager, Orderer>;

/// What a device reads from one message it is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// The plaintext of an application message.
    Plaintext(Vec<u8>),
    /// A removal that took this device out of the group.
    Removed,
    /// An application message whose group secret this device does not hold. It was not given to the
    /// crate, so it can be offered again later, once a welcome or an update brings the secret.
    Withheld(Hash),
}

/// What the adapter can fail with.
#[derive(Debug, Error)]
pub enum SchemeError {
    /// A failed crate call consumed the session's state.
    #[error("the session is poisoned: a failed crate call consumed its state")]
    Poisoned,
    /// The device has neither created nor joined the group.
    #[error("the group has not been created or joined on this device")]
    NotWelcomed,
    /// An error from `p2panda-encryption`.
    #[error("data scheme: {0}")]
    Crate(String),
    /// A group-model key is not a valid verifying key.
    #[error("invalid identity key: {0}")]
    InvalidKey(String),
    /// The key is not a member in the group model, so it cannot be added.
    #[error("{0} is not in the group model, so it cannot be added")]
    NotInModel(String),
    /// The key is still a member in the group model, so it cannot be removed.
    #[error("{0} is still in the group model, so it cannot be removed")]
    StillInModel(String),
    /// The crate's roster and the group model hold different members.
    #[error("the encryption roster disagrees with the group model: {0}")]
    RosterMismatch(String),
}

/// One device's encryption state for one group.
#[derive(Debug)]
pub struct Session {
    me: VerifyingKey,
    state: Option<Group>,
}

impl Session {
    /// A device's state before it creates or joins the group. `registry` must hold the long-term
    /// bundles of every member who may take part.
    pub fn init(
        me: VerifyingKey,
        keys: KeyManagerState,
        registry: KeyRegistryState<VerifyingKey>,
    ) -> Self {
        let state: Group = EncryptionGroup::init(
            me,
            keys,
            registry,
            Roster::initial_state(),
            Orderer::init(me),
        );
        Self {
            me,
            state: Some(state),
        }
    }

    /// Whether this device has created or joined the group.
    pub fn is_welcomed(&self) -> bool {
        self.state.as_ref().is_some_and(|y| y.is_welcomed)
    }

    /// The current epoch: the ID of the newest group secret this device holds.
    pub fn epoch(&self) -> Option<GroupSecretId> {
        self.state.as_ref()?.secrets.latest().map(GroupSecret::id)
    }

    /// The members the crate's roster holds, as group-model keys.
    pub fn members(&self) -> Result<BTreeSet<Key>, SchemeError> {
        let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
        let members = EncryptionGroup::members(y).map_err(crate_error)?;
        Ok(members.into_iter().map(key_of).collect())
    }

    /// Creates the group with `model` as its members. This device must be one of them.
    pub fn create(&mut self, model: &BTreeSet<Key>, rng: &Rng) -> Result<Envelope, SchemeError> {
        if !model.contains(&key_of(self.me)) {
            return Err(SchemeError::NotInModel(short(key_of(self.me))));
        }
        let initial: Vec<VerifyingKey> = members_of(model)?;
        let message = self.step(|y| EncryptionGroup::create(y, initial, rng))?;
        self.check_roster(model)?;
        Ok(message)
    }

    /// Adds `added`, which `model` must already list, with a welcome that carries the full bundle.
    pub fn add(
        &mut self,
        model: &BTreeSet<Key>,
        added: Key,
        rng: &Rng,
    ) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        if !model.contains(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        let message = self.step(|mut y| {
            let (dcgka, output) = Dcgka::add(y.dcgka, added_key, &y.secrets, rng)?;
            y.dcgka = dcgka;
            publish(y, output)
        })?;
        self.check_roster(model)?;
        Ok(message)
    }

    /// Removes `removed`, which `model` must no longer list. The removal rotates the epoch.
    pub fn remove(
        &mut self,
        model: &BTreeSet<Key>,
        removed: Key,
        rng: &Rng,
    ) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        if model.contains(&removed) {
            return Err(SchemeError::StillInModel(short(removed)));
        }
        let removed_key = member_of(removed)?;
        let message = self.step(|y| EncryptionGroup::remove(y, removed_key, rng))?;
        self.check_roster(model)?;
        Ok(message)
    }

    /// Rotates the epoch by giving every current member a new secret.
    pub fn update(&mut self, rng: &Rng) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        self.step(|y| EncryptionGroup::update(y, rng))
    }

    /// Encrypts `plaintext` under the current epoch.
    pub fn send(&mut self, plaintext: &[u8], rng: &Rng) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        self.step(|y| EncryptionGroup::send(y, plaintext, rng))
    }

    /// Handles a message from the group. An application message whose secret this device does not
    /// hold is withheld and not given to the crate (P4.3).
    pub fn process(&mut self, message: &Envelope) -> Result<Vec<Received>, SchemeError> {
        if let GroupMessageContent::Application {
            group_secret_id, ..
        } = message.content()
        {
            // The crate fails the whole call, and loses the state, on a secret it does not hold.
            let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
            if !y.secrets.contains(&group_secret_id) {
                return Ok(vec![Received::Withheld(message.id())]);
            }
        }
        let outputs = self.step(|y| EncryptionGroup::receive(y, message))?;
        Ok(outputs
            .into_iter()
            .filter_map(|output| match output {
                GroupOutput::Application { plaintext } => Some(Received::Plaintext(plaintext)),
                GroupOutput::Removed => Some(Received::Removed),
                GroupOutput::Control(_) => None,
            })
            .collect())
    }

    const fn ensure_welcomed(&self) -> Result<(), SchemeError> {
        match &self.state {
            None => Err(SchemeError::Poisoned),
            Some(y) if !y.is_welcomed => Err(SchemeError::NotWelcomed),
            Some(_) => Ok(()),
        }
    }

    /// Runs one crate call. On error the state is gone, so the session is poisoned.
    fn step<T>(&mut self, op: impl FnOnce(Group) -> Step<T>) -> Result<T, SchemeError> {
        let y = self.state.take().ok_or(SchemeError::Poisoned)?;
        let (y, value) = op(y).map_err(crate_error)?;
        self.state = Some(y);
        Ok(value)
    }

    fn check_roster(&self, model: &BTreeSet<Key>) -> Result<(), SchemeError> {
        let roster = self.members()?;
        if roster == *model {
            return Ok(());
        }
        Err(SchemeError::RosterMismatch(format!(
            "the roster has {} members, the model {}",
            roster.len(),
            model.len()
        )))
    }
}

/// The local steps `EncryptionGroup` runs after a control operation: publish the control message,
/// then process it as its sender.
fn publish(mut y: Group, output: OperationOutput<VerifyingKey, Hash, Roster>) -> Step<Envelope> {
    let (orderer, message) =
        Orderer::next_control_message(y.orderer, &output.control_message, &output.direct_messages)
            .map_err(GroupError::Orderer)?;
    y.orderer = orderer;

    let (dcgka, _) = Dcgka::process(
        y.dcgka,
        ProcessInput {
            seq: message.id(),
            sender: message.sender(),
            control_message: output.control_message,
            direct_message: None,
        },
    )?;
    y.dcgka = dcgka;
    Ok((y, message))
}

/// A device's long-term keys: its identity and one pre-key.
pub fn new_keys(
    identity: &SecretKey,
    lifetime: Lifetime,
    rng: &Rng,
) -> Result<KeyManagerState, SchemeError> {
    let keys = KeyManager::init(identity).map_err(crate_error)?;
    KeyManager::rotate_prekey(keys, lifetime, rng).map_err(crate_error)
}

/// The bundle other members need to reach this device.
pub fn key_bundle(keys: &KeyManagerState) -> Result<LongTermKeyBundle, SchemeError> {
    KeyManager::prekey_bundle(keys).map_err(crate_error)
}

/// A registry that holds each member's bundle.
pub fn registry(
    bundles: &[(VerifyingKey, LongTermKeyBundle)],
) -> Result<KeyRegistryState<VerifyingKey>, SchemeError> {
    bundles
        .iter()
        .try_fold(KeyRegistry::init(), |registry, (member, bundle)| {
            KeyRegistry::add_longterm_bundle(registry, *member, bundle.clone()).map_err(crate_error)
        })
}

/// The group-model key of a device.
pub fn key_of(member: VerifyingKey) -> Key {
    *member.as_bytes()
}

fn member_of(key: Key) -> Result<VerifyingKey, SchemeError> {
    VerifyingKey::from_bytes(&key).map_err(|err| SchemeError::InvalidKey(err.to_string()))
}

fn members_of(model: &BTreeSet<Key>) -> Result<Vec<VerifyingKey>, SchemeError> {
    model.iter().map(|key| member_of(*key)).collect()
}

fn crate_error(err: impl std::fmt::Display) -> SchemeError {
    SchemeError::Crate(err.to_string())
}

fn short(key: Key) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", key[0], key[1], key[2], key[3])
}
