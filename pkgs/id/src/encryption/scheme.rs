//! The `DataScheme` adapter: one device's encryption state for one group, over `p2panda-encryption`.
//!
//! Membership comes from the group model. The caller passes the [`crate::groups::GroupView`] the
//! fold ([`crate::groups::fold`]) gives after the event, and the adapter takes its members from
//! that view. After each local membership change the adapter checks the crate's roster against the
//! view's members. A mismatch is a refusal, and it poisons the session, so the two cannot drift
//! apart without an error.
//!
//! The scheme version (P4.4) is the `scheme` of the group's signed policy. [`negotiate`] maps it to
//! the version this build speaks, and the session refuses any message whose header or direct copy
//! names another version. The stock layout (v0) is not shipped, so a stock group is refused, not
//! migrated (scheme-serde RESULTS).
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
use crate::groups::{GroupView, Key, Policy};

/// The name the group policy gives this scheme.
pub const DATASCHEME: &str = "datascheme";

/// The scheme version this build speaks: `serde_bytes` on the two byte fields of the vendored
/// crate (P4.4). Its welcome costs 40 bytes per epoch, where the stock layout costs 146.5.
pub const SCHEME_V1: u32 = 1;

/// The scheme version a group's signed policy names, if this build speaks it.
pub fn negotiate(policy: &Policy) -> Result<u32, SchemeError> {
    match &policy.scheme {
        None => Err(SchemeError::NoScheme),
        Some(scheme) if scheme.name == DATASCHEME && scheme.version == SCHEME_V1 => Ok(SCHEME_V1),
        Some(scheme) => Err(SchemeError::UnsupportedScheme {
            name: scheme.name.clone(),
            version: scheme.version,
        }),
    }
}

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
    /// The group's policy names no scheme, so the group is not encrypted.
    #[error("the group's policy names no scheme")]
    NoScheme,
    /// The group's policy names a scheme this build does not speak. A stock group (v0) is refused
    /// this way, and nothing is migrated.
    #[error("the group's policy names scheme {name} v{version}, which this build does not speak")]
    UnsupportedScheme {
        /// The scheme's name.
        name: String,
        /// The version the policy names.
        version: u32,
    },
    /// A message's header or direct copy names a version other than the group's signed policy.
    #[error("the message names scheme v{found}, but the group's signed policy names v{expected}")]
    SchemeMismatch {
        /// The version the signed policy names.
        expected: u32,
        /// The version the message names.
        found: u32,
    },
}

/// One device's encryption state for one group.
#[derive(Debug)]
pub struct Session {
    me: VerifyingKey,
    /// The scheme version the group's signed policy names.
    scheme: u32,
    state: Option<Group>,
}

impl Session {
    /// A device's state before it creates or joins the group. `registry` must hold the long-term
    /// bundles of every member who may take part. `policy` is the group's signed policy, and the
    /// session refuses a policy that names no scheme or one this build does not speak.
    pub fn init(
        me: VerifyingKey,
        keys: KeyManagerState,
        registry: KeyRegistryState<VerifyingKey>,
        policy: &Policy,
    ) -> Result<Self, SchemeError> {
        let scheme = negotiate(policy)?;
        let state: Group = EncryptionGroup::init(
            me,
            keys,
            registry,
            Roster::initial_state(),
            Orderer::init(me, scheme),
        );
        Ok(Self {
            me,
            scheme,
            state: Some(state),
        })
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
        roster_of(y)
    }

    /// Creates the group with the members `view` lists, as the group fold gives them. This device
    /// must be one of them.
    pub fn create(&mut self, view: &GroupView, rng: &Rng) -> Result<Envelope, SchemeError> {
        if !view.members.contains_key(&key_of(self.me)) {
            return Err(SchemeError::NotInModel(short(key_of(self.me))));
        }
        let initial: Vec<VerifyingKey> = members_of(&model_of(view))?;
        self.change(view, |y| EncryptionGroup::create(y, initial, rng))
    }

    /// Adds `added`, which `view` must already list, with a welcome that carries the full bundle.
    pub fn add(
        &mut self,
        view: &GroupView,
        added: Key,
        rng: &Rng,
    ) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        if !view.members.contains_key(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        self.change(view, |mut y| {
            let (dcgka, output) = Dcgka::add(y.dcgka, added_key, &y.secrets, rng)?;
            y.dcgka = dcgka;
            publish(y, output)
        })
    }

    /// Adds `added` by a join: a welcome that carries no earlier secret, then an immediate update.
    ///
    /// The joiner learns nothing the group had before the add, so it cannot read messages sent
    /// before its join. The update gives the group a secret the joiner does learn, and it must be
    /// delivered too. Returns the add and the update, in the order they are delivered.
    pub fn add_from_join(
        &mut self,
        view: &GroupView,
        added: Key,
        rng: &Rng,
    ) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        if !view.members.contains_key(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        let add = self.change(view, |y| EncryptionGroup::add_from_join(y, added_key, rng))?;
        let update = self.change(view, |y| EncryptionGroup::update(y, rng))?;
        Ok(vec![add, update])
    }

    /// Removes `removed`, which `view` must no longer list. The removal rotates the epoch.
    pub fn remove(
        &mut self,
        view: &GroupView,
        removed: Key,
        rng: &Rng,
    ) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        if view.members.contains_key(&removed) {
            return Err(SchemeError::StillInModel(short(removed)));
        }
        let removed_key = member_of(removed)?;
        self.change(view, |y| EncryptionGroup::remove(y, removed_key, rng))
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
        self.check_scheme(message)?;
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

    /// Refuses a message whose header or any direct copy names another version than the policy.
    /// The check runs before the crate sees the message, so a refusal leaves the state as it was.
    fn check_scheme(&self, message: &Envelope) -> Result<(), SchemeError> {
        let found = std::iter::once(message.scheme())
            .chain(message.direct_schemes())
            .find(|found| *found != self.scheme);
        match found {
            None => Ok(()),
            Some(found) => Err(SchemeError::SchemeMismatch {
                expected: self.scheme,
                found,
            }),
        }
    }

    /// Refuses a view whose policy names another scheme than the session's. A scheme change needs a
    /// new epoch (task-05 item 6), which is not built yet, so the session does not follow it.
    fn check_policy(&self, view: &GroupView) -> Result<(), SchemeError> {
        let scheme = negotiate(&view.policy)?;
        if scheme == self.scheme {
            return Ok(());
        }
        Err(SchemeError::SchemeMismatch {
            expected: self.scheme,
            found: scheme,
        })
    }

    /// Runs one membership change, then checks the crate's roster against the fold's members. The
    /// state is taken, as in [`Self::step`]. A mismatch therefore leaves the session poisoned: it
    /// cannot go on with a roster the fold does not give (P4.13).
    fn change<T>(
        &mut self,
        view: &GroupView,
        op: impl FnOnce(Group) -> Step<T>,
    ) -> Result<T, SchemeError> {
        self.check_policy(view)?;
        let y = self.state.take().ok_or(SchemeError::Poisoned)?;
        let (y, value) = op(y).map_err(crate_error)?;
        let roster = roster_of(&y)?;
        let model = model_of(view);
        if roster == model {
            self.state = Some(y);
            return Ok(value);
        }
        let only_roster: Vec<String> = roster.difference(&model).map(|key| short(*key)).collect();
        let only_fold: Vec<String> = model.difference(&roster).map(|key| short(*key)).collect();
        Err(SchemeError::RosterMismatch(format!(
            "in the roster only: [{}]; in the fold only: [{}]",
            only_roster.join(", "),
            only_fold.join(", ")
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

/// The members the crate's roster holds, as group-model keys.
fn roster_of(y: &Group) -> Result<BTreeSet<Key>, SchemeError> {
    let members = EncryptionGroup::members(y).map_err(crate_error)?;
    Ok(members.into_iter().map(key_of).collect())
}

/// The members the group fold gives: the keys of [`GroupView::members`].
fn model_of(view: &GroupView) -> BTreeSet<Key> {
    view.members.keys().copied().collect()
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
