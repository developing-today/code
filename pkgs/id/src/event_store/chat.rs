//! Chat over the event store (#3799 remainder, #3950).
//!
//! A chat message is its `chat.message.v1` event in the channel group. The event applies
//! like any content event, and the store keeps it either way. [`EventStore::chat_records`]
//! is the record conversion the causal projection in [`crate::chat`] consumes: a stored
//! event becomes a [`ChatRecord`] with its `after` taken from the header extension and its
//! author's previous event taken from the header backlink (the store keeps the author's
//! own head out of `after`; see `local_events`).
//!
//! [`EventStore::chat_timeline`] is the policy-enforcing read: a reader who is not on the
//! group's roster reads nothing ([`StoreError::NotMember`]), and a member reads the
//! projected timeline cut by the group policy's history rule (see
//! [`crate::chat::visible_history`] for the exact semantics of each rule).

use std::collections::HashMap;

use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::resolve::{self, author_of};
use super::{ACTIONS, APPLIED, EVENTS, EventStore, FORKS, PROOF_HEADERS, StoreError, corrupt, decode_stored};
use crate::chat::{CHAT_MESSAGE_KIND, ChatRecord, project, visible_history};
use crate::groups::policy::{History, Policy};
use crate::membership::AuthId;
use crate::waiting::AuthorKey;

impl EventStore {
    /// The chat records of `group`: each applied `chat.message.v1` event that names the
    /// group, converted to a [`ChatRecord`], sorted by event ID. Events of any other kind,
    /// and chat events of other groups, are not records of this group.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored chat
    /// event that carries no message.
    pub fn chat_records(&self, group: &Hash) -> Result<Vec<ChatRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let applied_table = txn.open_table(APPLIED)?;

        let mut records = Vec::new();
        for row in events.iter()? {
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            if applied_table.get(event.id.as_bytes().as_slice())?.is_none() {
                continue;
            }
            let ext = &event.header.extensions;
            if ext.kind != CHAT_MESSAGE_KIND || ext.group != Some(*group) {
                continue;
            }
            let Some(message) = ext.chat.clone() else {
                return Err(corrupt("a stored chat event carries no message".to_owned()));
            };
            records.push(ChatRecord {
                id: event.id,
                author: author_of(&event),
                after: ext.after.clone(),
                backlink: event.header.backlink,
                message,
            });
        }
        records.sort_by_key(|record| *record.id.as_bytes());
        Ok(records)
    }

    /// The policy record of `group` in the resolved state, as last set by an Admin, or
    /// `None` when the store holds no group with that ID.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn group_policy(&self, group: &Hash) -> Result<Option<Policy>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load(&events, &actions, &forks, &proof_headers)?
        };
        let resolved = resolve::resolve(&stored, &forks, &headers)?;
        Ok(resolved
            .projection
            .as_ref()
            .and_then(|projection| projection.group(group))
            .map(|view| view.policy.clone()))
    }

    /// The chat timeline of `group` as `reader` may see it: the projected records, gated
    /// on membership and cut by the group's history policy.
    ///
    /// A reader who is not a member of the group in the resolved state is refused with
    /// [`StoreError::NotMember`]. Under a full-history policy a member sees the whole
    /// projected timeline. Under any other history rule the reader sees the part of the
    /// timeline that causally follows the event that made them a member (the create that
    /// listed them, or the add that named them), as [`visible_history`] defines.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotMember`] for a reader off the roster, a redb error if the
    /// read fails, or [`StoreError::Corrupt`].
    pub fn chat_timeline(
        &self,
        group: &Hash,
        reader: &AuthorKey,
    ) -> Result<Vec<ChatRecord>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load(&events, &actions, &forks, &proof_headers)?
        };
        let resolved = resolve::resolve(&stored, &forks, &headers)?;
        if !resolve::roster_of(&resolved, group).contains(reader) {
            return Err(StoreError::NotMember);
        }
        let policy = resolved
            .projection
            .as_ref()
            .and_then(|projection| projection.group(group))
            .map(|view| view.policy.clone())
            .unwrap_or_default();
        let timeline = project(self.chat_records(group)?);
        if policy.history == History::Full {
            return Ok(timeline);
        }
        // The causal parents of every applied event: its `after` and its backlink.
        let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
        for item in &stored {
            if !resolved.applied.contains(&item.event.id) {
                continue;
            }
            let mut event_parents = item.event.header.extensions.after.clone();
            if let Some(backlink) = item.event.header.backlink {
                event_parents.push(backlink);
            }
            parents.insert(item.event.id, event_parents);
        }
        let join = join_event(&stored, &resolved.applied, reader, &parents);
        Ok(visible_history(timeline, &policy.history, join.as_ref(), &parents))
    }
}

/// The event that made `reader` a member: a create listing them, or an add naming them.
/// When several qualify, the one with the smallest causal past is the join; ties break by
/// event ID, so the choice depends only on the stored events.
fn join_event(
    stored: &[resolve::Stored],
    applied: &std::collections::HashSet<Hash>,
    reader: &AuthorKey,
    parents: &HashMap<Hash, Vec<Hash>>,
) -> Option<Hash> {
    let names_reader = |member: &GroupMember<AuthId>| {
        matches!(member, GroupMember::Individual(AuthId::Key(key)) if key == reader)
    };
    let mut candidates: Vec<Hash> = Vec::new();
    for item in stored {
        if !applied.contains(&item.event.id) {
            continue;
        }
        let joined = match &item.action {
            Some(GroupAction::Create { initial_members }) => {
                initial_members.iter().any(|(member, _)| names_reader(member))
            }
            Some(GroupAction::Add { member, .. }) => names_reader(member),
            _ => false,
        };
        if joined {
            candidates.push(item.event.id);
        }
    }
    candidates.sort_by_key(|id| (past_size(*id, parents), *id.as_bytes()));
    candidates.into_iter().next()
}

/// How many events sit in the causal past of `id`, itself included, over `parents`.
fn past_size(id: Hash, parents: &HashMap<Hash, Vec<Hash>>) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        if seen.insert(current) {
            if let Some(event_parents) = parents.get(&current) {
                stack.extend(event_parents.iter().copied());
            }
        }
    }
    seen.len()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::chain::{ChainExt, sign};
    use crate::chat::ChatMessage;
    use crate::membership::body::encode_body;
    use crate::waiting::WaitingLimits;
    use p2panda_core::{SeqNum, SigningKey};

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    fn individual(key: &SigningKey) -> GroupMember<AuthId> {
        GroupMember::Individual(AuthId::Key(author(key)))
    }

    fn open() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    /// A signed group operation and its encoded body, as the production path carries them.
    fn group_op(
        key: &SigningKey,
        seq: SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        group: Option<Hash>,
        action: &GroupAction<AuthId, ()>,
    ) -> (crate::chain::Event, Vec<u8>) {
        let body = encode_body(action).unwrap();
        let ext = ChainExt {
            kind: "group.op".to_owned(),
            group,
            after,
            tag: None,
            document: None,
            removes: None,
            chat: None,
        };
        let event = sign(key, seq, backlink, Some(&body), ext).unwrap();
        (event, body)
    }

    /// A signed chat event in `group`.
    fn chat_event(
        key: &SigningKey,
        seq: SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        group: Hash,
        text: &str,
    ) -> crate::chain::Event {
        let ext = ChainExt {
            kind: CHAT_MESSAGE_KIND.to_owned(),
            group: Some(group),
            after,
            tag: None,
            document: None,
            removes: None,
            chat: Some(ChatMessage::new(text, Vec::new()).unwrap()),
        };
        sign(key, seq, backlink, None, ext).unwrap()
    }

    /// A store holding a channel created by Alice with Bob as a member.
    fn channel() -> (tempfile::TempDir, EventStore, SigningKey, SigningKey, Hash, Hash) {
        let (dir, mut store) = open();
        let (alice, bob) = (key(41), key(42));
        let create = GroupAction::Create {
            initial_members: vec![
                (individual(&alice), p2panda_auth::Access::manage()),
                (individual(&bob), p2panda_auth::Access::write()),
            ],
        };
        let (create_event, create_body) = group_op(&alice, 0, None, vec![], None, &create);
        let group = create_event.id;
        store
            .ingest_with_body(&create_event.bytes, Some(&create_body), 0)
            .unwrap();
        assert!(store.event_applied(&group).unwrap());
        (dir, store, alice, bob, group, create_event.id)
    }

    #[test]
    fn test_chat_records_convert_stored_events_and_bind_to_the_group() {
        let (_dir, mut store, alice, bob, group, create_id) = channel();

        let first = chat_event(&alice, 1, Some(create_id), vec![], group, "hello from alice");
        store.ingest(&first.bytes, 0).unwrap();
        let second = chat_event(&bob, 0, None, vec![create_id], group, "hello from bob");
        store.ingest(&second.bytes, 0).unwrap();

        // A chat event in another group is not a record of this one.
        let other_group = Hash::digest(b"another channel");
        let stray = chat_event(&alice, 2, Some(first.id), vec![], other_group, "elsewhere");
        store.ingest(&stray.bytes, 0).unwrap();

        let records = store.chat_records(&group).unwrap();
        assert_eq!(records.len(), 2);
        let by_id: HashMap<Hash, &ChatRecord> =
            records.iter().map(|record| (record.id, record)).collect();
        let first_record = by_id[&first.id];
        assert_eq!(first_record.author, author(&alice));
        assert_eq!(first_record.backlink, Some(create_id));
        assert_eq!(first_record.message.text, "hello from alice");
        let second_record = by_id[&second.id];
        assert_eq!(second_record.author, author(&bob));
        assert_eq!(second_record.after, vec![create_id]);
        assert_eq!(second_record.backlink, None);
    }

    #[test]
    fn test_two_members_exchange_through_two_stores_with_replay_and_reorder() {
        // Two independent members, each with their own store, exchange signed chat events:
        // the create reaches both, then each side's messages cross over. Delivery to Bob's
        // store is out of order (Alice's second message first) and duplicated; both stores
        // still project the same timeline.
        let (alice, bob) = (key(43), key(44));
        let (_dir_a, mut store_a) = open();
        let (_dir_b, mut store_b) = open();
        let create = GroupAction::Create {
            initial_members: vec![
                (individual(&alice), p2panda_auth::Access::manage()),
                (individual(&bob), p2panda_auth::Access::write()),
            ],
        };
        let (create_event, create_body) = group_op(&alice, 0, None, vec![], None, &create);
        let group = create_event.id;
        for store in [&mut store_a, &mut store_b] {
            store
                .ingest_with_body(&create_event.bytes, Some(&create_body), 0)
                .unwrap();
        }

        let alice_first = chat_event(&alice, 1, Some(group), vec![], group, "first from alice");
        store_a.ingest(&alice_first.bytes, 0).unwrap();
        let bob_first = chat_event(&bob, 0, None, vec![group], group, "first from bob");
        store_b.ingest(&bob_first.bytes, 0).unwrap();
        // Alice replies to Bob: her event names Bob's as its `after`.
        let alice_second = chat_event(
            &alice,
            2,
            Some(alice_first.id),
            vec![bob_first.id],
            group,
            "second from alice",
        );

        // Bob's store gets Alice's second message before her first, and her first twice.
        store_b.ingest(&alice_second.bytes, 0).unwrap();
        store_b.ingest(&alice_first.bytes, 0).unwrap();
        store_b.ingest(&alice_first.bytes, 0).unwrap();
        store_b.ingest(&alice_second.bytes, 0).unwrap();
        // Alice's store gets Bob's message, twice.
        store_a.ingest(&bob_first.bytes, 0).unwrap();
        store_a.ingest(&bob_first.bytes, 0).unwrap();
        store_a.ingest(&alice_second.bytes, 0).unwrap();

        let timeline_a = store_a.chat_timeline(&group, &author(&alice)).unwrap();
        let timeline_b = store_b.chat_timeline(&group, &author(&bob)).unwrap();
        let texts_a: Vec<&str> = timeline_a.iter().map(|r| r.message.text.as_str()).collect();
        let texts_b: Vec<&str> = timeline_b.iter().map(|r| r.message.text.as_str()).collect();
        assert_eq!(texts_a, texts_b);
        assert_eq!(texts_a.len(), 3);
        let position = |text: &str| texts_a.iter().position(|t| *t == text).unwrap();
        assert!(position("first from alice") < position("second from alice"));
        assert!(position("first from bob") < position("second from alice"));

        // A third key is not a member: it reads nothing, and the refusal is explicit.
        let carol = key(45);
        assert!(matches!(
            store_a.chat_timeline(&group, &author(&carol)),
            Err(StoreError::NotMember)
        ));
    }

    #[test]
    fn test_group_policy_comes_from_the_create() {
        let (_dir, store, _alice, _bob, group, _create) = channel();
        // The membership create bridges to a group create under the custom template with
        // the default policy record (see `resolve`), so the policy is the default: full
        // history for a member.
        assert_eq!(store.group_policy(&group).unwrap(), Some(Policy::default()));
        // A group the store never saw has no policy.
        let unseen = Hash::digest(b"no such group");
        assert_eq!(store.group_policy(&unseen).unwrap(), None);
    }
}
