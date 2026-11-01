//! Chat messages as signed events in a channel group (ID chat v1 core, #3799).
//!
//! A chat message is a `chat.message.v1` event in the channel's group, carried the way a
//! document is carried in [`crate::documents`]: the body travels in the event header, the
//! event is signed with its message, and sync moves the event alone. The channel's identity
//! is the signed group ID; names and nicknames stay local client aliases.
//!
//! The author of a message is the key that signed its event — never the transport peer and
//! never a server-assigned participant number. The Iroh `EndpointId` is a routing identity
//! and is not a substitute for the event signature.
//!
//! Messages follow the existing event causal model: a message names its causal parents in
//! `after` — the other authors' frontier heads — and its own author's previous event in
//! the header backlink (the store deliberately keeps the author's own head out of `after`;
//! see `local_events`). [`project`] folds delivered events into a timeline that respects
//! both kinds of parent link. Concurrent messages from different authors acquire no bridge- or arrival-defined
//! order; the projection's tie-break between concurrent messages is their event IDs, a
//! deterministic presentation order every replica computes identically from the events
//! alone. Duplicate delivery of an event changes nothing.
//!
//! Text is sanitized and bounded with the world-chat rules (see [`crate::world`]): control
//! characters are stripped and the text is at most [`MAX_TEXT_BYTES`] bytes. The send-rate
//! limit (50 per 10 seconds) is an admission rule of the sending path, not a property of a
//! stored message, so it is enforced where events are admitted, not here.
//!
//! A message may carry object references as links. Only `blob` and `event` are v1 link
//! kinds. A link names content by hash and names its author; it never embeds a capability.
//! Retrieval accepts a holder set (see [`ChatLink::holders`]) even though v1 usually knows
//! one holder, the author or the optional hint.
//!
//! Membership and history enforcement live with the group policy and the store read path
//! ([`crate::groups::policy`]): a reader applies the channel group's history policy to the
//! projected timeline, and a writer must be a member under the group policy before its
//! events are admitted. This module defines the message shape and the projection those
//! checks operate on. World chat behavior is untouched.

use std::collections::{BTreeMap, HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::waiting::AuthorKey;
use crate::world::sanitize_chat;

/// The kind of a `chat.message.v1` event, as the header's `kind` names it.
pub const CHAT_MESSAGE_KIND: &str = "chat.message.v1";

/// The most bytes a message's text may have: the world-chat limit.
pub const MAX_TEXT_BYTES: usize = 2_048;

/// The most links a message may carry.
pub const MAX_LINKS: usize = 16;

/// The most bytes a link's hint may have. A hint names where content may be found.
pub const MAX_HINT_BYTES: usize = 128;

/// Whether `kind` is the chat message kind, so its event carries a chat message.
#[must_use]
pub fn is_chat_message_kind(kind: &str) -> bool {
    kind == CHAT_MESSAGE_KIND
}

/// The kind of object a [`ChatLink`] references. Only these two kinds exist in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatLinkKind {
    /// Content bytes in the blob store, named by their BLAKE3 hash.
    Blob,
    /// Another event, named by its event ID.
    Event,
}

/// An object reference carried by a message: content by hash, its author, and an optional
/// hint naming where it may be retrieved. A link never embeds a capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatLink {
    /// What the reference points at.
    pub kind: ChatLinkKind,
    /// The BLAKE3 hash of the content (for `blob`) or the event ID (for `event`).
    pub hash: Hash,
    /// The key that authored the referenced object.
    pub author: AuthorKey,
    /// An optional retrieval hint, at most [`MAX_HINT_BYTES`] bytes.
    pub hint: Option<String>,
}

impl ChatLink {
    /// The holder set retrieval accepts for this link: the author, plus the hint when one
    /// is present and parses as a key. v1 usually yields exactly one holder; the API is a
    /// set so multi-provider retrieval (#2260/#2261) does not change this shape.
    #[must_use]
    pub fn holders(&self) -> Vec<AuthorKey> {
        // A hint names a place, not a key, in v1; it never widens the holder set beyond
        // the author until a provider defines its encoding.
        vec![self.author]
    }

    /// Checks the rules every link meets. The builder and the decoder both apply them.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::Hint`] when the hint is longer than [`MAX_HINT_BYTES`].
    pub fn check(&self) -> Result<(), ChatError> {
        if let Some(hint) = &self.hint
            && hint.len() > MAX_HINT_BYTES
        {
            return Err(ChatError::Hint(format!(
                "the hint is {} bytes, it must be at most {MAX_HINT_BYTES}",
                hint.len()
            )));
        }
        Ok(())
    }

    /// Checks `bytes` against a `blob` link: the bytes must hash to the link's hash.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::NotBlob`] for an `event` link and [`ChatError::Hash`] when the
    /// bytes do not hash to the link's hash.
    pub fn verify_blob(&self, bytes: &[u8]) -> Result<(), ChatError> {
        if self.kind != ChatLinkKind::Blob {
            return Err(ChatError::NotBlob);
        }
        if Hash::digest(bytes) != self.hash {
            return Err(ChatError::Hash);
        }
        Ok(())
    }
}

/// A chat message body: bounded sanitized text plus optional object links.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    /// The message text, sanitized with the world-chat rules and at most
    /// [`MAX_TEXT_BYTES`] bytes.
    pub text: String,
    /// Object references the message carries, at most [`MAX_LINKS`].
    #[serde(default)]
    pub links: Vec<ChatLink>,
}

impl ChatMessage {
    /// A message over `text` with `links`: the text is sanitized first, then checked, so a
    /// built message always passes [`ChatMessage::check`] on receipt.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::Text`] when the sanitized text is empty or too long, and the
    /// failing link's error when a link is refused.
    pub fn new(text: &str, links: Vec<ChatLink>) -> Result<Self, ChatError> {
        let message = Self {
            text: sanitize_chat(text),
            links,
        };
        message.check()?;
        Ok(message)
    }

    /// Checks the rules every message meets. A decoded message whose text is not exactly
    /// what the sanitizer would produce is refused: senders sanitize, so unsanitized text
    /// on the wire is not a message any conforming sender made.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::Text`] for empty, oversize, or unsanitized text,
    /// [`ChatError::Links`] for too many links, and the failing link's error otherwise.
    pub fn check(&self) -> Result<(), ChatError> {
        if self.text.is_empty() {
            return Err(ChatError::Text("the text is empty".to_owned()));
        }
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(ChatError::Text(format!(
                "the text is {} bytes, it must be at most {MAX_TEXT_BYTES}",
                self.text.len()
            )));
        }
        if sanitize_chat(&self.text) != self.text {
            return Err(ChatError::Text(
                "the text is not sanitized".to_owned(),
            ));
        }
        if self.links.len() > MAX_LINKS {
            return Err(ChatError::Links(format!(
                "the message carries {} links, it must carry at most {MAX_LINKS}",
                self.links.len()
            )));
        }
        for link in &self.links {
            link.check()?;
        }
        Ok(())
    }
}

/// Why a chat message or link was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// The text is empty, longer than [`MAX_TEXT_BYTES`], or not sanitized.
    #[error("{0}")]
    Text(String),
    /// The message carries more than [`MAX_LINKS`] links.
    #[error("{0}")]
    Links(String),
    /// A link hint is longer than [`MAX_HINT_BYTES`].
    #[error("{0}")]
    Hint(String),
    /// Blob bytes do not hash to the link's hash.
    #[error("the bytes do not hash to the link's hash")]
    Hash,
    /// Blob verification was asked of a link that is not a blob link.
    #[error("the link is not a blob link")]
    NotBlob,
}

/// A delivered chat event: its ID, the key that signed it, its causal parents, and its
/// message. Signature verification happens where events are admitted to the store; a
/// record here is an event the store has already accepted as signed by `author`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRecord {
    /// The event's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// The event IDs this message causally follows: the other authors' frontier heads.
    pub after: Vec<Hash>,
    /// The author's own previous event, from the event header's backlink. A causal parent
    /// like any `after` entry, kept in its own field because the store does.
    pub backlink: Option<Hash>,
    /// The message the event carries.
    pub message: ChatMessage,
}

/// Folds delivered records into the channel timeline: duplicates by event ID are dropped
/// (first delivery wins), and the order respects every `after` link whose parent is also
/// in the set. Concurrent records are ordered by event ID, so the result depends only on
/// the events, never on delivery order.
#[must_use]
pub fn project(records: impl IntoIterator<Item = ChatRecord>) -> Vec<ChatRecord> {
    let mut by_id: BTreeMap<[u8; 32], ChatRecord> = BTreeMap::new();
    for record in records {
        by_id.entry(*record.id.as_bytes()).or_insert(record);
    }
    // Kahn's algorithm over the causal links, always taking the smallest ready event ID:
    // a deterministic order that every replica derives from the same event set.
    let mut children: HashMap<[u8; 32], Vec<[u8; 32]>> = HashMap::new();
    let mut pending: HashMap<[u8; 32], usize> = HashMap::new();
    for record in by_id.values() {
        let mut parents = 0;
        let mut parent_ids: Vec<[u8; 32]> =
            record.after.iter().map(|parent| *parent.as_bytes()).collect();
        if let Some(backlink) = &record.backlink {
            parent_ids.push(*backlink.as_bytes());
        }
        for parent in parent_ids {
            if by_id.contains_key(&parent) {
                children
                    .entry(parent)
                    .or_default()
                    .push(*record.id.as_bytes());
                parents += 1;
            }
        }
        pending.insert(*record.id.as_bytes(), parents);
    }
    let mut ready: Vec<[u8; 32]> = pending
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut timeline = Vec::with_capacity(by_id.len());
    let mut emitted: HashSet<[u8; 32]> = HashSet::new();
    while let Some(index) = ready
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| *id)
        .map(|(index, _)| index)
    {
        let id = ready.remove(index);
        if !emitted.insert(id) {
            continue;
        }
        if let Some(record) = by_id.get(&id) {
            timeline.push(record.clone());
        }
        for child in children.get(&id).cloned().unwrap_or_default() {
            if let Some(count) = pending.get_mut(&child) {
                *count -= 1;
                if *count == 0 {
                    ready.push(child);
                }
            }
        }
    }
    // A cycle in `after` cannot occur in a store that verifies causal links; if one is
    // handed in anyway, append the remainder in ID order rather than dropping messages.
    if timeline.len() < by_id.len() {
        for record in by_id.values() {
            if !emitted.contains(record.id.as_bytes()) {
                timeline.push(record.clone());
            }
        }
    }
    timeline
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn author(seed: u8) -> AuthorKey {
        [seed; 32]
    }

    fn record(id: u8, after: Vec<u8>, text: &str) -> ChatRecord {
        ChatRecord {
            id: Hash::digest(&[id]),
            author: author(id),
            after: after.iter().map(|parent| Hash::digest(&[*parent])).collect(),
            backlink: None,
            message: ChatMessage::new(text, Vec::new()).unwrap(),
        }
    }

    #[test]
    fn test_message_sanitizes_and_bounds_text() {
        let message = ChatMessage::new("  hello\u{7} world  ", Vec::new()).unwrap();
        assert_eq!(message.text, "hello world");
        message.check().unwrap();

        let oversize = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(matches!(
            ChatMessage::new(&oversize, Vec::new()),
            Err(ChatError::Text(_))
        ));
        assert!(matches!(
            ChatMessage::new("   ", Vec::new()),
            Err(ChatError::Text(_))
        ));
    }

    #[test]
    fn test_decoded_message_must_be_sanitized() {
        let raw = ChatMessage {
            text: "has\u{7}control".to_owned(),
            links: Vec::new(),
        };
        assert!(matches!(raw.check(), Err(ChatError::Text(_))));
    }

    #[test]
    fn test_link_kinds_are_only_blob_and_event() {
        let parsed: Result<ChatLink, _> =
            serde_json::from_str(r#"{"kind":"file","hash":"0000000000000000000000000000000000000000000000000000000000000000","author":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"hint":null}"#);
        assert!(parsed.is_err());
    }

    #[test]
    fn test_blob_link_verifies_bytes() {
        let bytes = b"attachment bytes";
        let link = ChatLink {
            kind: ChatLinkKind::Blob,
            hash: Hash::digest(bytes),
            author: author(1),
            hint: None,
        };
        link.verify_blob(bytes).unwrap();
        assert!(matches!(link.verify_blob(b"other"), Err(ChatError::Hash)));

        let event_link = ChatLink {
            kind: ChatLinkKind::Event,
            ..link.clone()
        };
        assert!(matches!(
            event_link.verify_blob(bytes),
            Err(ChatError::NotBlob)
        ));
        assert_eq!(link.holders(), vec![author(1)]);
    }

    #[test]
    fn test_projection_dedupes_and_orders_causally() {
        // Delivered out of order, with a duplicate: 3 follows 2, 2 follows 1, and 4 is
        // concurrent with the chain.
        let delivered = vec![
            record(3, vec![2], "third"),
            record(2, vec![1], "second"),
            record(4, Vec::new(), "concurrent"),
            record(1, Vec::new(), "first"),
            record(2, vec![1], "second"),
        ];
        let timeline = project(delivered);
        let texts: Vec<&str> = timeline.iter().map(|r| r.message.text.as_str()).collect();
        assert_eq!(texts.len(), 4);
        let position = |text: &str| texts.iter().position(|t| *t == text).unwrap();
        assert!(position("first") < position("second"));
        assert!(position("second") < position("third"));

        // The same set in a different delivery order projects identically.
        let redelivered = vec![
            record(4, Vec::new(), "concurrent"),
            record(1, Vec::new(), "first"),
            record(2, vec![1], "second"),
            record(3, vec![2], "third"),
        ];
        let again = project(redelivered);
        let again_texts: Vec<&str> = again.iter().map(|r| r.message.text.as_str()).collect();
        assert_eq!(texts, again_texts);
    }

    #[test]
    fn test_projection_orders_same_author_chain_by_backlink() {
        // Same author, successive messages: the second names the first only as its
        // backlink, never in `after`. Delivered reversed, the chain still projects in
        // author order.
        let first = record(7, Vec::new(), "first from seven");
        let mut second = record(8, Vec::new(), "second from seven");
        second.author = first.author;
        second.backlink = Some(first.id);
        let timeline = project(vec![second, first]);
        let texts: Vec<&str> = timeline.iter().map(|r| r.message.text.as_str()).collect();
        assert_eq!(texts, vec!["first from seven", "second from seven"]);
    }
}
