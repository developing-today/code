//! The `DataScheme` orderer and the envelope it delivers.
//!
//! The crate's orderer (`MessageOrderer`) is test-only, so the adapter brings its own with the same
//! rules. A message names the last control message the sender had delivered from each author, and
//! it is delivered once every message it names is delivered. Nothing is delivered before the device
//! is welcomed: a create or a welcome opens delivery, and earlier messages wait in the queue.
//!
//! Application messages are never named by other messages, so the adapter may leave one out
//! without stalling anything. [`super::scheme`] does that for messages whose group secret the
//! device does not hold.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use p2panda_core::cbor::encode_cbor;
use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::group_secret::GroupSecretId;
use p2panda_encryption::data_scheme::{ControlMessage, DirectMessage};
use p2panda_encryption::traits::{GroupMessage, GroupMessageContent, Ordering};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::roster::Roster;

/// One published group message: a control message with its direct messages, or an application
/// message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// Hash of the sequence number, the sender, the names and the content.
    id: Hash,
    sender: VerifyingKey,
    /// The last control message seen from each author when this was published.
    previous: Vec<Hash>,
    content: Content,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Content {
    Control {
        control: ControlMessage<VerifyingKey>,
        direct: Vec<DirectMessage<VerifyingKey, Hash, Roster>>,
    },
    Application {
        group_secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    },
}

impl Envelope {
    fn new(
        seq: u64,
        sender: VerifyingKey,
        previous: Vec<Hash>,
        content: Content,
    ) -> Result<Self, OrdererError> {
        let bytes = encode_cbor(&(seq, sender, &previous, &content))
            .map_err(|err| OrdererError::Encode(err.to_string()))?;
        Ok(Self {
            id: Hash::digest(&bytes),
            sender,
            previous,
            content,
        })
    }
}

impl GroupMessage<VerifyingKey, Hash, Roster> for Envelope {
    fn id(&self) -> Hash {
        self.id
    }

    fn sender(&self) -> VerifyingKey {
        self.sender
    }

    fn content(&self) -> GroupMessageContent<VerifyingKey> {
        match &self.content {
            Content::Control { control, .. } => GroupMessageContent::Control(control.clone()),
            Content::Application {
                group_secret_id,
                nonce,
                ciphertext,
            } => GroupMessageContent::Application {
                group_secret_id: *group_secret_id,
                nonce: *nonce,
                ciphertext: ciphertext.clone(),
            },
        }
    }

    fn direct_messages(&self) -> Vec<DirectMessage<VerifyingKey, Hash, Roster>> {
        match &self.content {
            Content::Control { direct, .. } => direct.clone(),
            Content::Application { .. } => Vec::new(),
        }
    }
}

/// An error from the orderer.
#[derive(Debug, Error)]
pub enum OrdererError {
    /// The message could not be encoded to compute its hash.
    #[error("cannot encode the message: {0}")]
    Encode(String),
}

/// The orderer. Its state is [`OrdererState`].
#[derive(Clone, Copy, Debug)]
pub struct Orderer;

impl Orderer {
    /// The state before a create or a welcome.
    pub const fn init(my_id: VerifyingKey) -> OrdererState {
        OrdererState {
            my_id,
            next_seq: 0,
            previous: BTreeMap::new(),
            messages: BTreeMap::new(),
            ready: BTreeSet::new(),
            waiting: BTreeMap::new(),
            deliverable: VecDeque::new(),
            welcomed: false,
        }
    }
}

/// The orderer's state: our sequence, the last control message from each author, and the messages
/// that are not delivered yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrdererState {
    my_id: VerifyingKey,
    next_seq: u64,
    /// The last control message delivered from each author, including our own.
    previous: BTreeMap<VerifyingKey, Hash>,
    /// Every message received, by ID.
    messages: BTreeMap<Hash, Envelope>,
    /// Messages that are ready: every message they name is ready. Our own published messages are
    /// ready, since we processed them locally.
    ready: BTreeSet<Hash>,
    /// Received messages that still lack a named message, with the names they lack.
    waiting: BTreeMap<Hash, Vec<Hash>>,
    /// Ready messages the group has not taken yet, oldest first.
    deliverable: VecDeque<Hash>,
    /// Set by a create or a welcome. Nothing is delivered before it.
    welcomed: bool,
}

impl OrdererState {
    /// Publishes our own message. It names the control messages we have seen, and it is ready.
    fn publish(&mut self, content: Content) -> Result<Envelope, OrdererError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let previous = self.previous.values().copied().collect();
        let envelope = Envelope::new(seq, self.my_id, previous, content)?;
        self.ready.insert(envelope.id);
        Ok(envelope)
    }

    /// Marks `id` ready, then every waiting message that it completes.
    fn make_ready(&mut self, id: Hash) {
        self.ready.insert(id);
        self.deliverable.push_back(id);
        while let Some(next) = self
            .waiting
            .iter()
            .find(|(_, missing)| missing.iter().all(|named| self.ready.contains(named)))
            .map(|(id, _)| *id)
        {
            self.waiting.remove(&next);
            self.ready.insert(next);
            self.deliverable.push_back(next);
        }
    }
}

impl Ordering<VerifyingKey, Hash, Roster> for Orderer {
    type State = OrdererState;

    type Error = OrdererError;

    type Message = Envelope;

    fn next_control_message(
        mut y: Self::State,
        control_message: &ControlMessage<VerifyingKey>,
        direct_messages: &[DirectMessage<VerifyingKey, Hash, Roster>],
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        let content = Content::Control {
            control: control_message.clone(),
            direct: direct_messages.to_vec(),
        };
        let envelope = y.publish(content)?;
        y.previous.insert(y.my_id, envelope.id);
        Ok((y, envelope))
    }

    fn next_application_message(
        mut y: Self::State,
        group_secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        let content = Content::Application {
            group_secret_id,
            nonce,
            ciphertext,
        };
        let envelope = y.publish(content)?;
        Ok((y, envelope))
    }

    fn queue(mut y: Self::State, message: &Self::Message) -> Result<Self::State, Self::Error> {
        if y.ready.contains(&message.id) || y.messages.contains_key(&message.id) {
            return Ok(y);
        }
        y.messages.insert(message.id, message.clone());

        let missing: Vec<Hash> = message
            .previous
            .iter()
            .filter(|named| !y.ready.contains(named))
            .copied()
            .collect();
        if missing.is_empty() {
            y.make_ready(message.id);
        } else {
            y.waiting.insert(message.id, missing);
        }
        Ok(y)
    }

    fn set_welcome(
        mut y: Self::State,
        _message: &Self::Message,
    ) -> Result<Self::State, Self::Error> {
        y.welcomed = true;
        Ok(y)
    }

    fn next_ready_message(
        mut y: Self::State,
    ) -> Result<(Self::State, Option<Self::Message>), Self::Error> {
        if !y.welcomed {
            return Ok((y, None));
        }
        let Some(id) = y.deliverable.pop_front() else {
            return Ok((y, None));
        };
        let Some(envelope) = y.messages.get(&id).cloned() else {
            return Ok((y, None));
        };
        if let Content::Control { .. } = envelope.content {
            y.previous.insert(envelope.sender, id);
        }
        Ok((y, Some(envelope)))
    }
}
