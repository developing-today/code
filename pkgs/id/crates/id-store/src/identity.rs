//! Identity events and the user subject for tags (#160).
//!
//! A user's tag subject is an event ID, not a key: the ID of the user's first identity event
//! (owner decision on #160). An identity event is a signed event of kind [`IDENTITY_KIND`] that
//! an account publishes about itself; it carries no operation — the event's existence, its
//! author, and its position in the author's chain are the whole statement. A node publishes one
//! for its own key when its event store is opened (see `LocalEvents::open` in id-events), which
//! is this system's account-creation moment: the first local setup of a key.
//!
//! Drew's decision (2026-10-29, on #160): expect the identity event (Option A), fall back to
//! the earliest event in the key's log (Option B) — never gate on A. Identity must not require
//! an identity event in this service: people use direct accounts they create on their own, and
//! other systems integrate without one. [`resolve_subject`] is that rule: it returns the
//! identity event's ID when one exists in the author's log and the earliest event's ID when
//! none does, and it answers for every non-empty log.
//!
//! The key fingerprint is determinable from any message: every event's header names its
//! author's key, and [`pseudonym`] derives a deterministic display identity from that key
//! alone. When no identity event and no name can be found for a key, the display identity is
//! the pseudonym and the key's age is reported as a lower bound ([`age_lower_bound`]): the
//! earliest evidence a node has seen is the oldest the key can be proven to be, never the
//! oldest it is.

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;

use crate::chain::Event;

/// The kind of an identity event, as the header's `kind` names it.
pub const IDENTITY_KIND: &str = "identity";

/// Whether `kind` names an identity event.
#[must_use]
pub fn is_identity_kind(kind: &str) -> bool {
    kind == IDENTITY_KIND
}

/// An identity event: a signed statement by a key that it is an account's identity.
///
/// The event carries no operation. Its ID is the user subject for tags and aliases (#160).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityEvent {
    /// The identity event's ID. This is the user subject.
    pub id: Hash,
    /// The key that signed it: the account the event is about.
    pub author: AuthorKey,
}

impl IdentityEvent {
    /// The identity event in `event`, or `None` when `event` is of any other kind.
    #[must_use]
    pub fn from_event(event: &Event) -> Option<Self> {
        if !is_identity_kind(&event.header.extensions.kind) {
            return None;
        }
        Some(Self {
            id: event.id,
            author: *event.header.verifying_key.as_bytes(),
        })
    }
}

/// Where a resolved user subject came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectSource {
    /// Option A: the subject is the author's first identity event.
    IdentityEvent,
    /// Option B: no identity event exists in the log, so the subject is the author's
    /// earliest event of any kind.
    EarliestEvent,
}

/// A user's subject for tags and aliases: an event ID, and how it was derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserSubject {
    /// The subject: the event ID tags and aliases on this user name.
    pub subject: Hash,
    /// How the subject was derived.
    pub source: SubjectSource,
}

/// Resolves `author`'s user subject from their events, per Drew's rule on #160: the first
/// identity event when one exists (Option A), else the earliest event in the log (Option B).
///
/// `events` may hold any authors' events in any order; only `author`'s are considered, and
/// "first"/"earliest" are by sequence number in the author's own chain, which is the order
/// the author signed them in. Returns `None` only when the log holds no event by `author` at
/// all — resolution never fails for lack of an identity event.
#[must_use]
pub fn resolve_subject<'a>(
    events: impl IntoIterator<Item = &'a Event>,
    author: &AuthorKey,
) -> Option<UserSubject> {
    let mut earliest: Option<&Event> = None;
    let mut first_identity: Option<&Event> = None;
    for event in events {
        if event.header.verifying_key.as_bytes() != author {
            continue;
        }
        if earliest.is_none_or(|best| event.header.seq_num < best.header.seq_num) {
            earliest = Some(event);
        }
        if is_identity_kind(&event.header.extensions.kind)
            && first_identity.is_none_or(|best| event.header.seq_num < best.header.seq_num)
        {
            first_identity = Some(event);
        }
    }
    if let Some(event) = first_identity {
        return Some(UserSubject {
            subject: event.id,
            source: SubjectSource::IdentityEvent,
        });
    }
    earliest.map(|event| UserSubject {
        subject: event.id,
        source: SubjectSource::EarliestEvent,
    })
}

/// The deterministic pseudonym for a key with no known name: `user-` plus the first ten hex
/// digits of the key's fingerprint. The fingerprint of a public key is determinable from any
/// message the key signed, so every node derives the same pseudonym for the same key.
#[must_use]
pub fn pseudonym(author: &AuthorKey) -> String {
    let mut hex = String::with_capacity(64);
    for byte in author {
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        hex.push(char::from_digit(u32::from(byte & 0xf), 16).unwrap_or('0'));
    }
    format!("user-{}", &hex[..10])
}

/// How a key is displayed: a known name when one exists, else its pseudonym.
#[must_use]
pub fn display_identity(author: &AuthorKey, name: Option<&str>) -> String {
    match name {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => pseudonym(author),
    }
}

/// The lower bound on a key's age, in milliseconds: how long before `now_ms` the earliest
/// evidence of the key was seen (`first_seen_ms`). The key is *at least* this old; it may be
/// older, because the evidence is only the oldest this node has seen. Returns `None` when
/// there is no evidence or the evidence is dated after `now_ms`.
#[must_use]
pub fn age_lower_bound_ms(first_seen_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    first_seen_ms.and_then(|seen| now_ms.checked_sub(seen))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::chain::{ChainExt, sign};
    use p2panda_core::SigningKey;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&std::array::from_fn(|i| u8::try_from(i).unwrap()))
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn ext(kind: &str) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: None,
        }
    }

    fn event(key: &SigningKey, seq: u32, backlink: Option<Hash>, kind: &str) -> Event {
        sign(key, seq, backlink, None, ext(kind)).unwrap()
    }

    fn author_of(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    #[test]
    fn identity_event_extracts_only_from_identity_kind() {
        let key = key();
        let identity = event(&key, 0, None, IDENTITY_KIND);
        let extracted = IdentityEvent::from_event(&identity).unwrap();
        assert_eq!(extracted.id, identity.id);
        assert_eq!(extracted.author, author_of(&key));

        let other = event(&key, 0, None, "message");
        assert_eq!(IdentityEvent::from_event(&other), None);
    }

    #[test]
    fn subject_prefers_the_first_identity_event() {
        let key = key();
        let first = event(&key, 0, None, "message");
        let identity = event(&key, 1, Some(first.id), IDENTITY_KIND);
        let later_identity = event(&key, 2, Some(identity.id), IDENTITY_KIND);
        // Deliberately unordered, with a foreign author's earlier identity event mixed in.
        let foreign = event(&other_key(), 0, None, IDENTITY_KIND);
        let events = vec![later_identity, foreign, identity.clone(), first];
        let resolved = resolve_subject(&events, &author_of(&key)).unwrap();
        assert_eq!(resolved.subject, identity.id);
        assert_eq!(resolved.source, SubjectSource::IdentityEvent);
    }

    #[test]
    fn subject_falls_back_to_the_earliest_event() {
        let key = key();
        let first = event(&key, 0, None, "message");
        let second = event(&key, 1, Some(first.id), "message");
        let events = vec![second, first.clone()];
        let resolved = resolve_subject(&events, &author_of(&key)).unwrap();
        assert_eq!(resolved.subject, first.id);
        assert_eq!(resolved.source, SubjectSource::EarliestEvent);
    }

    #[test]
    fn subject_is_none_only_for_an_empty_log() {
        let key = key();
        let foreign = event(&other_key(), 0, None, IDENTITY_KIND);
        assert_eq!(resolve_subject(&[foreign], &author_of(&key)), None);
        assert_eq!(resolve_subject(&[], &author_of(&key)), None);
    }

    #[test]
    fn pseudonym_is_deterministic_from_the_key() {
        let author = author_of(&key());
        assert_eq!(pseudonym(&author), pseudonym(&author));
        assert!(pseudonym(&author).starts_with("user-"));
        assert_eq!(pseudonym(&author).len(), "user-".len() + 10);
        assert_ne!(pseudonym(&author), pseudonym(&author_of(&other_key())));
        assert_eq!(display_identity(&author, None), pseudonym(&author));
        assert_eq!(display_identity(&author, Some("")), pseudonym(&author));
        assert_eq!(display_identity(&author, Some("drew")), "drew");
    }

    #[test]
    fn age_lower_bound_is_never_more_than_the_evidence() {
        assert_eq!(age_lower_bound_ms(Some(1_000), 4_500), Some(3_500));
        assert_eq!(age_lower_bound_ms(None, 4_500), None);
        assert_eq!(age_lower_bound_ms(Some(5_000), 4_500), None);
    }
}
