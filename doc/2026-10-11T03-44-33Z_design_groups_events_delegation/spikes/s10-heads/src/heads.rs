//! Heads orderer for the S10 spike.
//!
//! The crate's `MessageOrderer` names, for each author, the latest control message it has seen,
//! and never removes an entry. This orderer names the heads of the author's view instead: the
//! processed messages that no processed message names, truncated to `cap` in (sender, seq) order.
//!
//! The readiness and waiting logic is copied from the crate's private `ordering` module, following
//! `MessageOrderer::queue` and `Orderer::process_pending` (test_utils/ordering.rs, ordering.rs).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::marker::PhantomData;

use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::{ControlMessage, DirectMessage, GroupSecretId};
use p2panda_encryption::test_utils::{MemberId, MessageId};
use p2panda_encryption::traits::{GroupMembership, GroupMessage, GroupMessageContent, Ordering};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct HeadsOrderer<DGM> {
    _marker: PhantomData<DGM>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadsState<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    pub my_id: MemberId,
    /// Most dependencies one message may name. `None` names every head.
    cap: Option<usize>,
    /// Whether application messages are heads too (every event), or only control messages.
    track_app: bool,
    /// Always name this member's own latest event first, so its own chain survives truncation.
    backlink: bool,
    /// The last event this member published or processed from itself.
    own_latest: Option<MessageId>,
    /// Heads of this member's view.
    heads: HashSet<MessageId>,
    next_seq: usize,
    welcome: bool,
    /// Every message queued so far. Nothing is pruned.
    messages: HashMap<MessageId, HeadsMessage<DGM>>,
    ready: HashSet<MessageId>,
    ready_queue: VecDeque<MessageId>,
    /// Missing dependency -> messages waiting on it, with their dependency lists.
    pending: HashMap<MessageId, HashSet<(MessageId, Vec<MessageId>)>>,
    /// Messages waiting for a dependency, mapped to the arrival count when they started waiting.
    waiting: HashMap<MessageId, usize>,
    arrivals: usize,
    max_waiting: usize,
    /// For each message that waited: how many other messages arrived before it was released.
    waits: Vec<usize>,
}

impl<DGM> HeadsOrderer<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    pub fn init(
        my_id: MemberId,
        cap: Option<usize>,
        track_app: bool,
        backlink: bool,
    ) -> HeadsState<DGM> {
        HeadsState {
            my_id,
            cap,
            track_app,
            backlink,
            own_latest: None,
            heads: HashSet::new(),
            next_seq: 0,
            welcome: false,
            messages: HashMap::new(),
            ready: HashSet::new(),
            ready_queue: VecDeque::new(),
            pending: HashMap::new(),
            waiting: HashMap::new(),
            arrivals: 0,
            max_waiting: 0,
            waits: Vec::new(),
        }
    }
}

impl<DGM> HeadsState<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    pub fn heads_len(&self) -> usize {
        self.heads.len()
    }

    /// Messages queued but not yet released (waiting for a dependency).
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    pub fn max_waiting(&self) -> usize {
        self.max_waiting
    }

    pub fn waits(&self) -> &[usize] {
        &self.waits
    }

    /// Dependencies a new message names: every head, or the first `cap` in (sender, seq) order.
    fn frontier(&self) -> Vec<MessageId> {
        let own = if self.backlink { self.own_latest } else { None };
        let mut ids: Vec<MessageId> = self
            .heads
            .iter()
            .copied()
            .filter(|id| Some(*id) != own)
            .collect();
        ids.sort_by_key(|id| (id.sender, id.seq));
        if let Some(cap) = self.cap {
            ids.truncate(cap.saturating_sub(usize::from(own.is_some())));
        }
        if let Some(own) = own {
            ids.insert(0, own);
        }
        ids
    }

    /// Updates the heads for a processed message: its dependencies are covered, and it is a head.
    /// Application messages only count when `track_app` is set.
    fn observe(&mut self, message: &HeadsMessage<DGM>) {
        let id = MessageId {
            sender: message.sender,
            seq: message.seq,
        };
        if message.sender == self.my_id {
            self.own_latest = Some(id);
        }
        let is_control = matches!(message.content, HeadsContent::System { .. });
        if !is_control && !self.track_app {
            return;
        }
        for dep in &message.previous {
            self.heads.remove(dep);
        }
        self.heads.insert(id);
    }

    fn is_ready(&self, deps: &[MessageId]) -> bool {
        deps.iter().all(|dep| self.ready.contains(dep))
    }

    fn mark_ready(&mut self, id: MessageId) -> bool {
        if !self.ready.insert(id) {
            return false;
        }
        if let Some(start) = self.waiting.remove(&id) {
            self.waits.push(self.arrivals - start);
        }
        self.ready_queue.push_back(id);
        true
    }

    /// Releases every message that waited for `key`, and, recursively, the ones waiting for those.
    fn release_dependents(&mut self, key: MessageId) {
        let Some(dependents) = self.pending.remove(&key) else {
            return;
        };
        for (next, deps) in dependents {
            if self.is_ready(&deps) && self.mark_ready(next) {
                self.release_dependents(next);
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadsMessage<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    seq: usize,
    sender: MemberId,
    /// The dependencies this message names. Same field name as the crate's test message.
    previous: Vec<MessageId>,
    content: HeadsContent<DGM>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HeadsContent<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    Application {
        ciphertext: Vec<u8>,
        group_secret_id: GroupSecretId,
        nonce: XAeadNonce,
    },
    System {
        control_message: ControlMessage<MemberId>,
        direct_messages: Vec<DirectMessage<MemberId, MessageId, DGM>>,
    },
}

impl<DGM> GroupMessage<MemberId, MessageId, DGM> for HeadsMessage<DGM>
where
    DGM: Clone + GroupMembership<MemberId, MessageId>,
{
    fn id(&self) -> MessageId {
        MessageId {
            sender: self.sender,
            seq: self.seq,
        }
    }

    fn sender(&self) -> MemberId {
        self.sender
    }

    fn content(&self) -> GroupMessageContent<MemberId> {
        match &self.content {
            HeadsContent::Application {
                ciphertext,
                group_secret_id,
                nonce,
            } => GroupMessageContent::Application {
                group_secret_id: *group_secret_id,
                nonce: *nonce,
                ciphertext: ciphertext.to_vec(),
            },
            HeadsContent::System {
                control_message, ..
            } => GroupMessageContent::Control(control_message.clone()),
        }
    }

    fn direct_messages(&self) -> Vec<DirectMessage<MemberId, MessageId, DGM>> {
        match &self.content {
            HeadsContent::Application { .. } => Vec::new(),
            HeadsContent::System {
                direct_messages, ..
            } => direct_messages.clone(),
        }
    }
}

#[derive(Debug)]
pub struct HeadsError(pub String);

impl fmt::Display for HeadsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HeadsError {}

impl<DGM> Ordering<MemberId, MessageId, DGM> for HeadsOrderer<DGM>
where
    DGM: fmt::Debug
        + Clone
        + GroupMembership<MemberId, MessageId>
        + Serialize
        + for<'a> Deserialize<'a>,
{
    type State = HeadsState<DGM>;

    type Error = HeadsError;

    type Message = HeadsMessage<DGM>;

    fn next_control_message(
        mut y: Self::State,
        control_message: &ControlMessage<MemberId>,
        direct_messages: &[DirectMessage<MemberId, MessageId, DGM>],
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        let message = HeadsMessage {
            seq: y.next_seq,
            sender: y.my_id,
            previous: y.frontier(),
            content: HeadsContent::System {
                control_message: control_message.to_owned(),
                direct_messages: direct_messages.to_owned(),
            },
        };
        y.next_seq += 1;
        y.observe(&message);
        Ok((y, message))
    }

    fn next_application_message(
        mut y: Self::State,
        group_secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        // The driver merges before this point. An application message never truncates.
        if let Some(cap) = y.cap
            && y.heads.len() > cap
        {
            return Err(HeadsError(format!(
                "frontier of {} heads exceeds cap {cap}",
                y.heads.len()
            )));
        }
        let message = HeadsMessage {
            seq: y.next_seq,
            sender: y.my_id,
            previous: y.frontier(),
            content: HeadsContent::Application {
                ciphertext,
                group_secret_id,
                nonce,
            },
        };
        y.next_seq += 1;
        y.observe(&message);
        Ok((y, message))
    }

    fn queue(mut y: Self::State, message: &Self::Message) -> Result<Self::State, Self::Error> {
        let id = message.id();
        y.messages.insert(id, message.clone());
        y.arrivals += 1;

        // Our own messages are never queued, so dependencies on ourselves are already met.
        let previous: Vec<MessageId> = message
            .previous
            .iter()
            .filter(|dep| dep.sender != y.my_id)
            .copied()
            .collect();

        if !y.is_ready(&previous) {
            y.waiting.insert(id, y.arrivals);
            y.max_waiting = y.max_waiting.max(y.waiting.len());
            for dep in &previous {
                if !y.ready.contains(dep) {
                    y.pending
                        .entry(*dep)
                        .or_default()
                        .insert((id, previous.clone()));
                }
            }
            return Ok(y);
        }

        y.mark_ready(id);
        y.release_dependents(id);
        Ok(y)
    }

    fn set_welcome(
        mut y: Self::State,
        _message: &Self::Message,
    ) -> Result<Self::State, Self::Error> {
        y.welcome = true;
        Ok(y)
    }

    fn next_ready_message(
        mut y: Self::State,
    ) -> Result<(Self::State, Option<Self::Message>), Self::Error> {
        if !y.welcome {
            return Ok((y, None));
        }
        let Some(id) = y.ready_queue.pop_front() else {
            return Ok((y, None));
        };
        let message = y
            .messages
            .get(&id)
            .cloned()
            .expect("ready ids map to messages");
        y.observe(&message);
        Ok((y, Some(message)))
    }
}
