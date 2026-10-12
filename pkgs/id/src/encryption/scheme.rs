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

use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::hpke::{hpke_open, hpke_seal};
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::data_scheme::dcgka::{Dcgka, OperationOutput, ProcessInput};
use p2panda_encryption::data_scheme::group::{
    EncryptionGroup, GroupError, GroupOutput, GroupResult, GroupState,
};
use p2panda_encryption::data_scheme::group_secret::{GroupSecret, GroupSecretId, SecretBundle};
use p2panda_encryption::key_bundle::{Lifetime, LongTermKeyBundle};
use p2panda_encryption::key_manager::{KeyManager, KeyManagerState};
use p2panda_encryption::key_registry::{KeyRegistry, KeyRegistryState};
use p2panda_encryption::traits::{
    GroupMessage, GroupMessageContent, IdentityRegistry, Ordering as _, PreKeyManager,
};
use thiserror::Error;

use super::history::{HistoryAnswer, HistoryMode, HistoryRequest};
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
    /// An application message whose group secret this device does not hold. The adapter keeps it,
    /// and reads it once the secret arrives: a welcome or an update, or lazy history (P4.6).
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
    /// An answer carried a secret the request did not ask for, so it is refused.
    #[error("the history answer carries a secret the request did not ask for")]
    UnaskedSecret,
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
    /// Application messages this device cannot read yet, in arrival order. They stay out of the
    /// crate until their group secret arrives (P4.5).
    held: Vec<Envelope>,
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
            held: Vec::new(),
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

    /// How many group secrets this device holds, so how many epochs it can read.
    pub fn epoch_count(&self) -> usize {
        self.state.as_ref().map_or(0, |y| y.secrets.len())
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
        self.add_with_history(model, added, HistoryMode::Full, rng)
    }

    /// Adds `added`, which `model` must already list, with a welcome that carries the secrets
    /// `mode` allows: every secret for `Full`, the newest N for `Recent(N)` (P4.5).
    ///
    /// The bundle is cut here, before `Dcgka::add`, so the crate never sees a secret outside the
    /// window. The joiner then holds exactly the window and refuses older messages.
    pub fn add_with_history(
        &mut self,
        model: &BTreeSet<Key>,
        added: Key,
        mode: HistoryMode,
        rng: &Rng,
    ) -> Result<Envelope, SchemeError> {
        self.ensure_welcomed()?;
        if !model.contains(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        let bundle = {
            let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
            mode.welcome_bundle(&y.secrets)
        };
        let message = self.step(|mut y| {
            let (dcgka, output) = Dcgka::add(y.dcgka, added_key, &bundle, rng)?;
            y.dcgka = dcgka;
            publish(y, output)
        })?;
        self.check_roster(model)?;
        Ok(message)
    }

    /// Adds `added` by a join: a welcome that carries no earlier secret, then an immediate update.
    ///
    /// The joiner learns nothing the group had before the add, so it cannot read messages sent
    /// before its join. The update gives the group a secret the joiner does learn, and it must be
    /// delivered too. Returns the add and the update, in the order they are delivered.
    pub fn add_from_join(
        &mut self,
        model: &BTreeSet<Key>,
        added: Key,
        rng: &Rng,
    ) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        if !model.contains(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        let add = self.step(|y| EncryptionGroup::add_from_join(y, added_key, rng))?;
        let update = self.step(|y| EncryptionGroup::update(y, rng))?;
        self.check_roster(model)?;
        Ok(vec![add, update])
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

    /// Handles a message from the group.
    ///
    /// An application message whose secret this device does not hold is kept here, outside the
    /// crate, and reported as [`Received::Withheld`] (P4.3). The crate fails a whole call on a secret
    /// it does not hold, so it is never given such a message. A welcome brings the secrets, and the
    /// call that does so also reads the held messages it unlocks, in the order they arrived (P4.5).
    pub fn process(&mut self, message: &Envelope) -> Result<Vec<Received>, SchemeError> {
        let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
        if !Self::can_read(y, message) {
            if !self.held.iter().any(|held| held.id() == message.id()) {
                self.held.push(message.clone());
            }
            return Ok(vec![Received::Withheld(message.id())]);
        }
        let mut read = self.receive(message)?;
        read.extend(self.release()?);
        Ok(read)
    }

    /// The secrets this device holds messages for but cannot read, as a request to a member (P4.6).
    /// `None` when nothing held is unreadable for want of a secret.
    pub fn request_history(&self) -> Option<HistoryRequest> {
        let y = self.state.as_ref()?;
        let mut wanted = BTreeSet::new();
        for message in &self.held {
            if let GroupMessageContent::Application {
                group_secret_id, ..
            } = message.content()
                && !y.secrets.contains(&group_secret_id)
            {
                wanted.insert(group_secret_id);
            }
        }
        (!wanted.is_empty()).then(|| HistoryRequest::new(self.me, wanted.into_iter().collect()))
    }

    /// Answers a joiner's request with the secrets this device holds among those asked for, sealed
    /// to the requester's identity key (P4.6).
    ///
    /// Only a member answers, and only for a member: a device outside the group, or a request from
    /// outside the roster, gets `None`. So does a request for no secret this device holds.
    pub fn answer_history(
        &self,
        request: &HistoryRequest,
    ) -> Result<Option<HistoryAnswer>, SchemeError> {
        let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
        let roster = self.members()?;
        if !roster.contains(&key_of(self.me)) || !roster.contains(&key_of(request.requester)) {
            return Ok(None);
        }
        let Some(identity) = <Pki as IdentityRegistry<VerifyingKey, _>>::identity_key(
            &y.dcgka.pki,
            &request.requester,
        )
        .map_err(crate_error)?
        else {
            return Ok(None);
        };
        let secrets: Vec<GroupSecret> = request
            .wanted
            .iter()
            .filter_map(|id| y.secrets.get(id).cloned())
            .collect();
        if secrets.is_empty() {
            return Ok(None);
        }
        let plaintext = encode_cbor(&secrets).map_err(crate_error)?;
        let aad = encode_cbor(request).map_err(crate_error)?;
        let sealed = hpke_seal(&identity, Some(HISTORY_INFO), Some(&aad), &plaintext)
            .map_err(crate_error)?;
        Ok(Some(HistoryAnswer::new(sealed)))
    }

    /// Opens a member's answer to `request` with this device's identity key, and holds the secrets it
    /// carries (P4.6). Returns what the held messages now read.
    ///
    /// An answer for another request does not open, and a secret the request did not name is
    /// refused before anything is held.
    pub fn open_history(
        &mut self,
        request: &HistoryRequest,
        answer: &HistoryAnswer,
        identity: &SecretKey,
    ) -> Result<Vec<Received>, SchemeError> {
        self.ensure_welcomed()?;
        let aad = encode_cbor(request).map_err(crate_error)?;
        let plaintext = hpke_open(answer.sealed(), identity, Some(HISTORY_INFO), Some(&aad))
            .map_err(crate_error)?;
        let secrets: Vec<GroupSecret> = decode_cbor(plaintext.as_slice()).map_err(crate_error)?;
        if secrets
            .iter()
            .any(|secret| !request.wanted.contains(&secret.id()))
        {
            return Err(SchemeError::UnaskedSecret);
        }
        self.step(|y| {
            let y = EncryptionGroup::update_secrets(y, |bundle| {
                secrets.into_iter().fold(bundle, SecretBundle::insert)
            });
            Ok((y, ()))
        })?;
        self.release()
    }

    /// Whether the crate may take `message` now: a control message always, an application message
    /// once the device holds its group secret.
    fn can_read(y: &Group, message: &Envelope) -> bool {
        match message.content() {
            GroupMessageContent::Control(_) => true,
            GroupMessageContent::Application {
                group_secret_id, ..
            } => y.secrets.contains(&group_secret_id),
        }
    }

    /// Gives the crate each held message whose secret the device now holds, in arrival order.
    fn release(&mut self) -> Result<Vec<Received>, SchemeError> {
        let held = std::mem::take(&mut self.held);
        let (ready, waiting): (Vec<Envelope>, Vec<Envelope>) = match self.state.as_ref() {
            Some(y) => held
                .into_iter()
                .partition(|message| Self::can_read(y, message)),
            None => (Vec::new(), held),
        };
        self.held = waiting;
        let mut read = Vec::new();
        for message in &ready {
            read.extend(self.receive(message)?);
        }
        Ok(read)
    }

    /// Hands `message` to the crate and reads its outputs.
    fn receive(&mut self, message: &Envelope) -> Result<Vec<Received>, SchemeError> {
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

    /// Hands `message` to the crate with no secret check. The check is what keeps pre-welcome history
    /// out of the crate (P4.5), so only the reproduction of the crate's refusal uses this.
    #[cfg(test)]
    pub(super) fn process_unchecked(
        &mut self,
        message: &Envelope,
    ) -> Result<Vec<Received>, SchemeError> {
        self.receive(message)
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

/// The associated text of every history answer, so an answer opens only as one (P4.6).
const HISTORY_INFO: &[u8] = b"id/history-answer/v1";

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
