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
use std::future::Future;
use std::time::Duration;

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

use super::history::{HistoryAnswer, HistoryError, HistoryMode, HistoryRequest, ask_member};
use super::ordering::{Envelope, MAX_PARENTS, Orderer};
use super::roster::Roster;
use crate::groups::{GroupView, Key, Policy};
use crate::rejection::Reason;

/// The name the group policy gives this scheme.
pub const DATASCHEME: &str = "datascheme";

/// The scheme version this build speaks: `serde_bytes` on the two byte fields of the vendored
/// crate (P4.4). Its welcome costs 40 bytes per epoch, where the stock layout costs 146.5.
pub const SCHEME_V1: u32 = 1;

/// The most application messages a session holds unread (#186). A removed or stalled peer
/// can keep sending messages this device cannot read, and each one is held until its group
/// secret arrives, so the held set needs a bound of its own: past it, a new arrival is
/// refused with [`SchemeError::HeldFull`]. Already-held messages are never evicted or
/// expired in-session - they leave only when their secret arrives and they are read.
pub const MAX_HELD_MESSAGES: usize = 1024;

/// The most CBOR-encoded bytes the held messages may total (#186): the sum of each held
/// envelope's encoded length, encoded with `encode_cbor` as `Envelope::new` encodes in
/// [`super::ordering`]. Tracked on the session as messages are held and released, so the
/// check never re-encodes the set.
pub const MAX_HELD_BYTES: usize = 16 * 1024 * 1024;

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
    /// An application message whose group secret this device does not hold. The adapter keeps it,
    /// and reads it once the secret arrives: a welcome or an update, or lazy history (P4.6).
    Withheld(Hash),
}

/// What the recovery procedure (N8) did. The CLI prints [`RECOVERY_STEPS`] next to it.
#[derive(Clone, Debug)]
pub struct Recovery {
    /// The removal, after any merge updates the heads needed first. It rotates the epoch, so the
    /// device no longer holds the new secrets.
    pub removal: Vec<Envelope>,
    /// The re-add: the welcome and the immediate update, in delivery order (P4.3).
    pub readd: Vec<Envelope>,
    /// The epoch before the procedure.
    pub epoch_before: Option<GroupSecretId>,
    /// The epoch after the procedure. It differs from `epoch_before`.
    pub epoch_after: Option<GroupSecretId>,
}

/// The steps of the recovery procedure, in the order [`Session::recover`] runs them.
pub const RECOVERY_STEPS: [&str; 2] = [
    "remove the device: the group rotates its secret, and the device cannot read what follows",
    "add the device back by a join: it reads messages from the new epoch, and nothing before it",
];

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
    /// A lazy history request got no answer it could use (P4.6). The held messages stay held.
    #[error(transparent)]
    History(#[from] HistoryError),
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
    /// The message would name more parents than a message may carry, so it is refused before the
    /// crate sees it. The session is not poisoned (P4.7).
    #[error("the message would name {count} parents, more than the cap of {cap}, so it is refused")]
    TooManyParents {
        /// The parents the message would name: the heads and the author's own latest.
        count: usize,
        /// The most parents a message may name.
        cap: usize,
    },
    /// The held set is at a cap, so a message this device cannot read yet is refused instead
    /// of held (#186). The check runs before any crate call, so the session is not poisoned,
    /// and no held message is dropped to make room: the refusal is of the new arrival only.
    #[error(
        "the session already holds {count} unreadable messages ({bytes} encoded bytes), and one more would pass the cap of {cap_count} messages or {cap_bytes} bytes, so it is refused"
    )]
    HeldFull {
        /// The messages the session already holds.
        count: usize,
        /// Their total CBOR-encoded length.
        bytes: usize,
        /// The most messages a session holds.
        cap_count: usize,
        /// The most encoded bytes the held messages may total.
        cap_bytes: usize,
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

impl SchemeError {
    /// The reason this error gives, when it is a refusal of an operation or a message. A crate
    /// failure, a bad key, and a policy or message that names a scheme this build does not
    /// speak are errors, not judgements, so they give none.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Poisoned | Self::RosterMismatch(_) => Some(Reason::Frozen),
            Self::NotWelcomed | Self::History(_) => Some(Reason::Waiting),
            Self::NotInModel(_)
            | Self::StillInModel(_)
            | Self::UnaskedSecret
            | Self::SchemeMismatch { .. } => Some(Reason::Voided),
            Self::NoScheme
            | Self::UnsupportedScheme { .. }
            | Self::Crate(_)
            | Self::InvalidKey(_)
            | Self::TooManyParents { .. }
            | Self::HeldFull { .. } => None,
        }
    }
}

/// One device's encryption state for one group.
#[derive(Debug)]
pub struct Session {
    me: VerifyingKey,
    /// The scheme version the group's signed policy names.
    scheme: u32,
    state: Option<Group>,
    /// Application messages this device cannot read yet, in arrival order. They stay out of the
    /// crate until their group secret arrives (P4.5). The set is bounded by
    /// [`MAX_HELD_MESSAGES`] and [`MAX_HELD_BYTES`] (#186), enforced by refusing NEW arrivals
    /// at the cap - a held message is never evicted or expired in-session, and leaves only
    /// through `release`, when its secret arrives. Each entry carries the message's
    /// CBOR-encoded length, recorded when it was held, so the byte total below is kept
    /// without ever re-encoding the set.
    held: Vec<(Envelope, usize)>,
    /// The held messages' total CBOR-encoded length, kept as they are held and released so
    /// the byte cap is checked without re-encoding the set per arrival.
    held_bytes: usize,
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
            held: Vec::new(),
            held_bytes: 0,
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

    /// How many group secrets this device holds, so how many epochs it can read.
    pub fn epoch_count(&self) -> usize {
        self.state.as_ref().map_or(0, |y| y.secrets.len())
    }

    /// The IDs of every group secret this device holds. Two members agree on the epochs when these
    /// sets are equal, even if their latest epoch differs (concurrent changes, P4.7).
    pub fn epoch_set(&self) -> BTreeSet<GroupSecretId> {
        self.state
            .as_ref()
            .map_or_else(BTreeSet::new, |y| y.secrets.ids().copied().collect())
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
    ) -> Result<Vec<Envelope>, SchemeError> {
        self.add_with_history(view, added, HistoryMode::Full, rng)
    }

    /// Adds `added`, which `view` must already list, with a welcome that carries the secrets
    /// `mode` allows: every secret for `Full`, the newest N for `Recent(N)` (P4.5).
    ///
    /// The bundle is cut here, before `Dcgka::add`, so the crate never sees a secret outside the
    /// window. The joiner then holds exactly the window and refuses older messages.
    ///
    /// Returns every message to broadcast: any merge updates first, then the add (P4.7).
    pub fn add_with_history(
        &mut self,
        view: &GroupView,
        added: Key,
        mode: HistoryMode,
        rng: &Rng,
    ) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        if !view.members.contains_key(&added) {
            return Err(SchemeError::NotInModel(short(added)));
        }
        let added_key = member_of(added)?;
        // The policy is checked before the merge, not only inside the change step: a refusal after
        // the merge would drop merge envelopes the session has already absorbed, and a later
        // publish would name a merge nobody received (P4.7).
        self.check_policy(view)?;
        let mut messages = self.merge_heads(rng)?;
        // The bundle is cut from the state before the change takes it, so the welcome carries the
        // window the mode allows and the roster check still runs on the fold's view.
        let bundle = {
            let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
            mode.welcome_bundle(&y.secrets)
        };
        messages.push(self.change(view, |mut y| {
            let (dcgka, output) = Dcgka::add(y.dcgka, added_key, &bundle, rng)?;
            y.dcgka = dcgka;
            publish(y, output)
        })?);
        Ok(messages)
    }

    /// Adds `added` by a join: a welcome that carries no earlier secret, then an immediate update.
    ///
    /// The joiner learns nothing the group had before the add, so it cannot read messages sent
    /// before its join. The update gives the group a secret the joiner does learn, and it must be
    /// delivered too. Returns the add and the update, in the order they are delivered, after any
    /// merge updates the heads needed first (P4.7).
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
        // The policy is checked before the merge, not only inside the change step: a refusal after
        // the merge would drop merge envelopes the session has already absorbed, and a later
        // publish would name a merge nobody received (P4.7).
        self.check_policy(view)?;
        let mut messages = self.merge_heads(rng)?;
        messages.push(self.change(view, |y| EncryptionGroup::add_from_join(y, added_key, rng))?);
        messages.push(self.change(view, |y| EncryptionGroup::update(y, rng))?);
        Ok(messages)
    }

    /// Removes `removed`, which `view` must no longer list. The removal rotates the epoch.
    ///
    /// Returns every message to broadcast: any merge updates first, then the removal (P4.7).
    pub fn remove(
        &mut self,
        view: &GroupView,
        removed: Key,
        rng: &Rng,
    ) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        if view.members.contains_key(&removed) {
            return Err(SchemeError::StillInModel(short(removed)));
        }
        let removed_key = member_of(removed)?;
        // The policy is checked before the merge, for the reason the add paths give (P4.7).
        self.check_policy(view)?;
        let mut messages = self.merge_heads(rng)?;
        messages.push(self.change(view, |y| EncryptionGroup::remove(y, removed_key, rng))?);
        Ok(messages)
    }

    /// Rotates the epoch by giving every current member a new secret.
    ///
    /// Returns every message to broadcast: the merge updates the heads needed first, if any, then
    /// the update itself (P4.7).
    pub fn update(&mut self, rng: &Rng) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        let mut messages = self.merge_heads(rng)?;
        messages.push(self.step(|y| EncryptionGroup::update(y, rng))?);
        Ok(messages)
    }

    /// Recovers a device whose key is suspect: removes `removed`, then adds `added` back by a join.
    /// A compromise (N8) keeps the device's key, so `added` is `removed`. A fork (decision 17)
    /// replaces the key, so `added` is the device's new key.
    ///
    /// `without` is the view after the removal and `with` the view after the re-add, so `with`
    /// lists `added` and `without` does not. The removal rotates the epoch, so the old key loses
    /// the secret it held. The re-add welcomes `added` into the new epoch without any earlier
    /// secret (P4.3). This session must be a manager's: the caller is the device that performs the
    /// procedure.
    pub fn recover(
        &mut self,
        without: &GroupView,
        with: &GroupView,
        removed: Key,
        added: Key,
        rng: &Rng,
    ) -> Result<Recovery, SchemeError> {
        let epoch_before = self.epoch();
        let removal = self.remove(without, removed, rng)?;
        let readd = self.add_from_join(with, added, rng)?;
        Ok(Recovery {
            removal,
            readd,
            epoch_before,
            epoch_after: self.epoch(),
        })
    }

    /// Encrypts `plaintext` under the current epoch.
    ///
    /// Returns every message to broadcast, in order: the merge updates the heads needed first, if
    /// any, then the application message (P4.7).
    pub fn send(&mut self, plaintext: &[u8], rng: &Rng) -> Result<Vec<Envelope>, SchemeError> {
        self.ensure_welcomed()?;
        let mut messages = self.merge_heads(rng)?;
        messages.push(self.step(|y| EncryptionGroup::send(y, plaintext, rng))?);
        Ok(messages)
    }

    /// Handles a message from the group.
    ///
    /// An application message whose secret this device does not hold is kept here, outside the
    /// crate, and reported as [`Received::Withheld`] (P4.3). The crate fails a whole call on a secret
    /// it does not hold, so it is never given such a message. A welcome brings the secrets, and the
    /// call that does so also reads the held messages it unlocks, in the order they arrived (P4.5).
    pub fn process(&mut self, message: &Envelope) -> Result<Vec<Received>, SchemeError> {
        self.check_scheme(message)?;
        let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
        if !Self::can_read(y, message) {
            if self.held.iter().any(|(held, _)| held.id() == message.id()) {
                return Ok(vec![Received::Withheld(message.id())]);
            }
            // The bound is checked here, before the push and before any crate call, so a
            // refusal neither poisons the session nor drops a held message (#186).
            let bytes = held_len(message)?;
            if self.held.len() >= MAX_HELD_MESSAGES || self.held_bytes + bytes > MAX_HELD_BYTES {
                return Err(SchemeError::HeldFull {
                    count: self.held.len(),
                    bytes: self.held_bytes,
                    cap_count: MAX_HELD_MESSAGES,
                    cap_bytes: MAX_HELD_BYTES,
                });
            }
            self.held.push((message.clone(), bytes));
            self.held_bytes += bytes;
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
        for (message, _) in &self.held {
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

    /// Asks a member for the secrets the held messages need, and releases the messages they unlock
    /// (P4.6). `ask` is the transport, as in [`ask_member`]: it reaches a member and returns that
    /// member's answer, or `None` when the member refuses. `identity` is this device's identity key,
    /// which the answer is sealed to.
    ///
    /// Nothing is asked when no held message waits on a secret. A refusal or a timeout leaves the
    /// messages held, so a later answer still releases them.
    pub async fn fetch_history<F, Fut>(
        &mut self,
        identity: &SecretKey,
        ask: F,
        wait: Duration,
    ) -> Result<Vec<Received>, SchemeError>
    where
        F: FnOnce(HistoryRequest) -> Fut,
        Fut: Future<Output = Option<HistoryAnswer>>,
    {
        let Some(request) = self.request_history() else {
            return Ok(Vec::new());
        };
        let answer = ask_member(request.clone(), ask, wait).await?;
        self.open_history(&request, &answer, identity)
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
        let (ready, waiting): (Vec<(Envelope, usize)>, Vec<(Envelope, usize)>) =
            match self.state.as_ref() {
                Some(y) => held
                    .into_iter()
                    .partition(|(message, _)| Self::can_read(y, message)),
                None => (Vec::new(), held),
            };
        // The byte total is re-summed from the encoded lengths recorded when the waiting
        // messages were held - the waiting set itself is never re-encoded (#186).
        self.held_bytes = waiting.iter().map(|(_, bytes)| bytes).sum();
        self.held = waiting;
        let mut read = Vec::new();
        for (message, _) in &ready {
            read.extend(self.receive(message)?);
        }
        Ok(read)
    }

    /// Hands `message` to the crate and reads its outputs.
    fn receive(&mut self, message: &Envelope) -> Result<Vec<Received>, SchemeError> {
        self.check_scheme(message)?;
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

    /// Publishes the merge updates the heads need before a change can name them all (P4.7).
    ///
    /// A merge is an update that names up to [`MAX_PARENTS`] heads, so the heads it leaves stay
    /// heads and the parent count falls by at least 31. A count above the cap therefore comes down
    /// in a bounded number of merges, and the change that follows names at most the cap. Nothing is
    /// merged while the device is not welcomed, since there is no head to merge, and nothing is
    /// merged when the heads already fit.
    ///
    /// The returned messages must reach the group before the change that follows: the change names
    /// the last merge as its author's latest, so a member that lacks it cannot deliver the change.
    fn merge_heads(&mut self, rng: &Rng) -> Result<Vec<Envelope>, SchemeError> {
        let mut merged = Vec::new();
        loop {
            let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
            if !y.is_welcomed || y.orderer.parent_count() <= MAX_PARENTS {
                return Ok(merged);
            }
            let message = self.step(|mut y| {
                y.orderer.begin_merge();
                EncryptionGroup::update(y, rng)
            })?;
            merged.push(message);
        }
    }

    /// Refuses a change whose parents still exceed [`MAX_PARENTS`] after the merges (P4.7). It runs
    /// before the crate call, since a call that fails inside the orderer consumes the state and
    /// poisons the session. Nothing is dropped from the parents: the change is refused.
    fn ensure_parents_fit(&self) -> Result<(), SchemeError> {
        let y = self.state.as_ref().ok_or(SchemeError::Poisoned)?;
        let count = y.orderer.parent_count();
        if count <= MAX_PARENTS {
            return Ok(());
        }
        Err(SchemeError::TooManyParents {
            count,
            cap: MAX_PARENTS,
        })
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
        self.ensure_parents_fit()?;
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

/// The CBOR-encoded length of one envelope, the held set's byte measure (#186). Encoded
/// with `encode_cbor`, as `Envelope::new` encodes in [`super::ordering`].
fn held_len(message: &Envelope) -> Result<usize, SchemeError> {
    Ok(encode_cbor(message).map_err(crate_error)?.len())
}

fn crate_error(err: impl std::fmt::Display) -> SchemeError {
    SchemeError::Crate(err.to_string())
}

fn short(key: Key) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", key[0], key[1], key[2], key[3])
}
