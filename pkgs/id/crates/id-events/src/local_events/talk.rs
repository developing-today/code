//! Talk-page posts over the node's event store (slice 2 of #5834, #14270).
//!
//! A post is a signed [`TalkPost`] (slice 1, #14274). This module writes one post as the
//! body of one [`TALK_POST_KIND`] content event, signed with the node's key, and reads an
//! object's talk page back from the events this store has applied. The record is the design
//! record's (`doc/2026-11-23T21-10-00Z_design_id5834_talk_page_changelog/`):
//!
//! - Nothing is edited in place (invariant 1). A correction is a new post that names the old
//!   one in `supersedes`. The read marks the old post with the post that superseded it, so a
//!   display can collapse it, and still returns it: the original stays stored and listed.
//! - Visibility is the only gate on writing (invariant 5). This node may post on, and read
//!   the talk page of, an object it can see. An object is an event this store has applied.
//!   An object that names a group is seen by the group's members, so a post on it carries
//!   that group and is written only by a member. The approval flow governs an object's
//!   effective version, never whether a post is allowed, so it plays no part here.
//! - A post is attributed to the identity that signed it. The post's author key is the
//!   signer of the event that carries it, and the read drops a post whose two keys differ.
//!
//! A post that arrives by sync is judged by the same rules on read: a post with no body, a
//! body that does not verify, a post about another object, a post outside the object's group,
//! or a post by a key that is not a member of that group is not on the page. An unknown
//! object and an object this node may not see are refused with the same error, so the
//! refusal never confirms that an object exists.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use p2panda_core::Hash;

use super::{LocalEvents, now_ms};
use crate::event_store::Ingest;
use id_core::id_shape::ContentId;
use id_store::chain::{ChainExt, sign};
use id_store::talk_page::{PostDraft, TALK_POST_KIND, TalkPost};

/// The refusal for an object this node cannot see, whether or not it exists.
const NOT_VISIBLE: &str = "the object is not visible to this node";

/// One post on an object's talk page, as this node reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalkEntry {
    /// The event that carries the post.
    pub event: Hash,
    /// The post's ID: the content hash of its signed record.
    pub post_id: ContentId,
    /// The signed post. Its author is the key that signed [`TalkEntry::event`].
    pub post: TalkPost,
    /// The later post, by the same author on the same object, that supersedes this one. A
    /// display may collapse a superseded post. The post itself stays stored and listed.
    pub superseded_by: Option<ContentId>,
}

impl TalkEntry {
    /// Whether a later post by the same author supersedes this one.
    #[must_use]
    pub const fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }
}

impl LocalEvents {
    /// Writes a post with `body` on the talk page of `object`, signed by this node, and
    /// returns it as the page lists it.
    ///
    /// `parent` names the post this one replies to, which must be on the same talk page.
    /// `supersedes` names an earlier post by this node on the same object that this post
    /// corrects. The earlier post is not changed: the page marks it superseded.
    ///
    /// # Errors
    ///
    /// Fails if this node cannot see `object`, the parent or superseded post is not on the
    /// page, the superseded post is another author's, the post is refused by the record rules
    /// (an empty or oversized body, for instance), or the store cannot record the event.
    pub fn post_talk(
        &mut self,
        object: Hash,
        parent: Option<ContentId>,
        supersedes: Option<ContentId>,
        body: &str,
    ) -> Result<TalkEntry> {
        let group = self.visible_object(object)?;
        let page = self.talk_posts(object, group)?;
        let post = TalkPost::sign(
            PostDraft {
                object_id: object.as_bytes().to_vec(),
                parent_post_id: parent,
                supersedes,
                authored_at: now_ms(),
                body: body.to_owned(),
            },
            &self.post_key(),
        )?;
        if let Some(parent) = parent
            && !page.iter().any(|entry| entry.post_id == parent)
        {
            bail!("the post this one replies to is not on the object's talk page");
        }
        if let Some(earlier) = supersedes {
            let earlier = page
                .iter()
                .find(|entry| entry.post_id == earlier)
                .ok_or_else(|| {
                    anyhow!("the post this one supersedes is not on the object's talk page")
                })?;
            post.check_supersedes(&earlier.post)?;
        }
        let post_id = post.post_id()?;
        let event = self.record_post(group, &post.canonical_bytes()?)?;
        Ok(TalkEntry {
            event,
            post_id,
            post,
            superseded_by: None,
        })
    }

    /// The talk page of `object` as this node may see it: every post on it, oldest first by
    /// the author's clock, each marked with the post that supersedes it, if any.
    ///
    /// # Errors
    ///
    /// Fails if this node cannot see `object`, or the store cannot be read.
    pub fn talk_page(&self, object: Hash) -> Result<Vec<TalkEntry>> {
        let group = self.visible_object(object)?;
        self.talk_posts(object, group)
    }

    /// The node's key as the Ed25519 key a talk post is signed with. It is the same key the
    /// node signs events with, so a post's author is the event's signer.
    fn post_key(&self) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&self.key.as_bytes())
    }

    /// Checks that this node can see `object`, and returns the object's group, if it has one.
    /// The object must be an applied event, and if it names a group, this node's key must be
    /// a member of that group.
    fn visible_object(&self, object: Hash) -> Result<Option<Hash>> {
        let (event, applied) = self
            .read_store(|store| Ok((store.event_by_id(&object)?, store.event_applied(&object)?)))?;
        let Some(event) = event.filter(|_| applied) else {
            bail!(NOT_VISIBLE);
        };
        let group = event.header.extensions.group;
        if let Some(group) = group
            && !self
                .read_store(|store| store.roster(&group))?
                .contains(&self.author())
        {
            bail!(NOT_VISIBLE);
        }
        Ok(group)
    }

    /// Every valid post on `object` among the applied talk events of `group` (or of no
    /// group), deduplicated by post ID and marked with what supersedes it.
    fn talk_posts(&self, object: Hash, group: Option<Hash>) -> Result<Vec<TalkEntry>> {
        let roster = match group {
            Some(group) => Some(self.read_store(|store| store.roster(&group))?),
            None => None,
        };
        // One read lock for the walk, so the events and their bodies come from one snapshot.
        let carried = self.read_store(|store| {
            let mut found = Vec::new();
            for (author, _) in store.chains()? {
                for event in store.applied_events(&author, None, None, usize::MAX)? {
                    let ext = &event.header.extensions;
                    if ext.kind == TALK_POST_KIND && ext.group == group {
                        let body = store.stored_body(&event.id)?;
                        found.push((event, body));
                    }
                }
            }
            Ok(found)
        })?;

        let mut entries = Vec::new();
        for (event, body) in carried {
            // A post whose body never arrived, or does not verify, is not on the page.
            let Some(post) = body.and_then(|body| TalkPost::from_canonical_bytes(&body).ok())
            else {
                continue;
            };
            let signer = *event.header.verifying_key.as_bytes();
            if post.object_id() != object.as_bytes() || *post.author() != signer {
                continue;
            }
            if roster
                .as_ref()
                .is_some_and(|roster| !roster.contains(&signer))
            {
                continue;
            }
            let Ok(post_id) = post.post_id() else {
                continue;
            };
            entries.push(TalkEntry {
                event: event.id,
                post_id,
                post,
                superseded_by: None,
            });
        }
        entries.sort_by(|a, b| {
            (a.post.authored_at(), a.event.as_bytes())
                .cmp(&(b.post.authored_at(), b.event.as_bytes()))
        });
        // The same signed post carried twice is one post: the first carrier in page order stands.
        let mut seen = std::collections::HashSet::new();
        entries.retain(|entry| seen.insert(entry.post_id));

        // A valid supersede marks the earlier post. When several posts supersede one, the
        // last in page order is the one a display shows in its place.
        let index: HashMap<ContentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(at, entry)| (entry.post_id, at))
            .collect();
        for later in 0..entries.len() {
            let Some(earlier) = entries[later]
                .post
                .supersedes()
                .and_then(|id| index.get(&id).copied())
            else {
                continue;
            };
            if entries[later]
                .post
                .check_supersedes(&entries[earlier].post)
                .is_ok()
            {
                entries[earlier].superseded_by = Some(entries[later].post_id);
            }
        }
        Ok(entries)
    }

    /// Signs one talk event carrying `body` with the node's key, after a bounded frontier of
    /// the other authors' heads, and ingests it with its body. The store must apply it.
    fn record_post(&mut self, group: Option<Hash>, body: &[u8]) -> Result<Hash> {
        let author = self.author();
        let after = self.bounded_after(&[])?;
        let chain = self
            .read_store(|store| store.chain(&author))?
            .unwrap_or_default();
        let ext = ChainExt {
            kind: TALK_POST_KIND.to_owned(),
            group,
            after,
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
        };
        let event = sign(&self.key, chain.next_seq, chain.head, Some(body), ext)?;
        match self.store.ingest(&event.bytes, Some(body), now_ms())? {
            Ingest::Accepted { id, .. } => Ok(id),
            refused => bail!("the store did not apply the {TALK_POST_KIND} event: {refused:?}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;

    use id_core::membership::AuthId;
    use id_core::membership::body::encode_body;
    use id_store::chat::ChatMessage;
    use p2panda_auth::group::{GroupAction, GroupMember};
    use p2panda_core::SigningKey;

    use super::*;
    use crate::event_store::Ingest;
    use id_store::chain::Event;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn ext(group: Option<Hash>, kind: &str) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group,
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

    /// A group create event (Alice manages, Bob writes) and its encoded body.
    fn create_group(alice: &SigningKey, bob: &SigningKey) -> (Event, Vec<u8>) {
        let member = |key: &SigningKey| {
            GroupMember::Individual(AuthId::Key(*key.verifying_key().as_bytes()))
        };
        let action = GroupAction::Create {
            initial_members: vec![
                (member(alice), p2panda_auth::Access::manage()),
                (member(bob), p2panda_auth::Access::write()),
            ],
        };
        let body = encode_body(&action).unwrap();
        let event = sign(alice, 0, None, Some(&body), ext(None, "group.op")).unwrap();
        (event, body)
    }

    fn node(dir: &Path, key: SigningKey, create: Option<&(Event, Vec<u8>)>) -> LocalEvents {
        let node = LocalEvents::open(dir, key).unwrap();
        if let Some((event, body)) = create {
            node.store.ingest(&event.bytes, Some(body), 0).unwrap();
        }
        node
    }

    /// Moves every applied event of `from`'s author, with its body, into `to`'s store, as a
    /// sync session would.
    fn exchange(from: &LocalEvents, to: &LocalEvents) {
        let carried = from
            .read_store(|store| {
                let mut carried = Vec::new();
                for event in store.applied_events(&from.author(), None, None, usize::MAX)? {
                    let body = store.stored_body(&event.id)?;
                    carried.push((event, body));
                }
                Ok(carried)
            })
            .unwrap();
        for (event, body) in carried {
            to.store
                .ingest(&event.bytes, body.as_deref(), now_ms())
                .unwrap();
        }
    }

    /// Signs `post` into an event of `signer`'s chain at `seq` in `group`, and ingests it
    /// into `into`'s store, as a peer that bypassed the write gate would deliver it.
    fn deliver(
        into: &LocalEvents,
        signer: &SigningKey,
        group: Option<Hash>,
        post: &TalkPost,
    ) -> Hash {
        let body = post.canonical_bytes().unwrap();
        let event = sign(signer, 0, None, Some(&body), ext(group, TALK_POST_KIND)).unwrap();
        match into
            .store
            .ingest(&event.bytes, Some(&body), now_ms())
            .unwrap()
        {
            Ingest::Accepted { id, .. } => id,
            other => panic!("the delivered event was not applied: {other:?}"),
        }
    }

    fn entry<'a>(page: &'a [TalkEntry], post_id: ContentId) -> &'a TalkEntry {
        page.iter()
            .find(|entry| entry.post_id == post_id)
            .expect("the post is on the page")
    }

    #[test]
    fn test_a_post_is_written_read_back_and_attributed_to_its_signer() {
        let dir = tempfile::tempdir().unwrap();
        let mut alice = node(dir.path(), key(71), None);
        let object = alice.create_note("text/plain", b"the object").unwrap();

        let written = alice
            .post_talk(object, None, None, "first thoughts")
            .unwrap();
        let page = alice.talk_page(object).unwrap();

        assert_eq!(page, vec![written.clone()]);
        assert_eq!(*written.post.author(), alice.author());
        assert_eq!(written.post.body(), Some("first thoughts"));
        assert_eq!(written.post.object_id(), object.as_bytes());
        assert_eq!(written.post_id, written.post.post_id().unwrap());
        // The carrying event is signed by the same key the post names as its author.
        let event = alice
            .read_store(|store| store.event_by_id(&written.event))
            .unwrap()
            .unwrap();
        assert_eq!(*event.header.verifying_key.as_bytes(), alice.author());
        assert_eq!(event.header.extensions.kind, TALK_POST_KIND);
        assert!(!written.is_superseded());
    }

    #[test]
    fn test_a_superseding_post_collapses_the_original_which_stays_stored() {
        let dir = tempfile::tempdir().unwrap();
        let mut alice = node(dir.path(), key(72), None);
        let object = alice.create_note("text/plain", b"the object").unwrap();

        let original = alice.post_talk(object, None, None, "teh typo").unwrap();
        let reply = alice
            .post_talk(object, Some(original.post_id), None, "a reply")
            .unwrap();
        let correction = alice
            .post_talk(object, None, Some(original.post_id), "the typo")
            .unwrap();
        let page = alice.talk_page(object).unwrap();

        assert_eq!(page.len(), 3, "a correction adds a post, it edits nothing");
        let stored = entry(&page, original.post_id);
        assert_eq!(stored.superseded_by, Some(correction.post_id));
        assert_eq!(
            stored.post.body(),
            Some("teh typo"),
            "the original stays stored"
        );
        assert!(!entry(&page, correction.post_id).is_superseded());
        assert_eq!(
            entry(&page, reply.post_id).post.parent_post_id(),
            Some(original.post_id)
        );
    }

    #[test]
    fn test_a_post_may_only_supersede_or_reply_to_a_post_on_the_page() {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut alice = node(dir_a.path(), key(73), None);
        let mut bob = node(dir_b.path(), key(74), None);
        let object = alice.create_note("text/plain", b"the object").unwrap();
        let other = alice.create_note("text/plain", b"another object").unwrap();
        let alices = alice.post_talk(object, None, None, "alice's post").unwrap();
        let elsewhere = alice.post_talk(other, None, None, "elsewhere").unwrap();
        exchange(&alice, &bob);

        // Bob sees Alice's post with the same ID and the same author.
        let seen = bob.talk_page(object).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].post_id, alices.post_id);
        assert_eq!(*seen[0].post.author(), alice.author());

        // Bob may reply to it, but may not supersede it: it is Alice's.
        assert!(
            bob.post_talk(object, Some(alices.post_id), None, "re")
                .is_ok()
        );
        assert!(
            bob.post_talk(object, None, Some(alices.post_id), "mine now")
                .is_err()
        );
        // A post on another object's page cannot be replied to or superseded from this one.
        assert!(
            alice
                .post_talk(object, Some(elsewhere.post_id), None, "re")
                .is_err()
        );
        assert!(
            alice
                .post_talk(object, None, Some(elsewhere.post_id), "fix")
                .is_err()
        );
        assert_eq!(alice.talk_page(object).unwrap().len(), 1);
    }

    #[test]
    fn test_an_unknown_object_is_not_visible_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut alice = node(dir.path(), key(75), None);
        let unknown = Hash::digest(b"never stored");

        let refused = alice.post_talk(unknown, None, None, "hello").unwrap_err();
        assert_eq!(refused.to_string(), NOT_VISIBLE);
        assert_eq!(
            alice.talk_page(unknown).unwrap_err().to_string(),
            NOT_VISIBLE
        );
        assert!(alice.read_store(|store| store.chains()).unwrap().is_empty());
    }

    #[test]
    fn test_a_replayed_post_under_another_signer_is_not_attributed_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut alice = node(dir.path(), key(76), None);
        let object = alice.create_note("text/plain", b"the object").unwrap();
        let real = alice
            .post_talk(object, None, None, "alice wrote this")
            .unwrap();

        // Mallory carries Alice's signed post in an event Mallory signed.
        let replayed = deliver(&alice, &key(77), None, &real.post);
        assert!(
            alice
                .read_store(|store| store.event_applied(&replayed))
                .unwrap()
        );

        let page = alice.talk_page(object).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(
            page[0].event, real.event,
            "only the author's own carrier stands"
        );
    }

    #[test]
    fn test_group_visibility_gates_writing_and_reading() {
        let (alice_key, bob_key, carol_key) = (key(81), key(82), key(83));
        let create = create_group(&alice_key, &bob_key);
        let group = create.0.id;
        let dirs = [
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        ];
        let mut alice = node(dirs[0].path(), alice_key, Some(&create));
        let mut bob = node(dirs[1].path(), bob_key, Some(&create));
        let mut carol = node(dirs[2].path(), carol_key.clone(), Some(&create));

        // The object is a message in the group: only the group's members see it.
        let object = alice
            .send_chat(group, ChatMessage::new("the object", Vec::new()).unwrap())
            .unwrap();
        exchange(&alice, &bob);
        exchange(&alice, &carol);

        // A non-member holds the object but cannot see it, so it may neither post nor read.
        assert_eq!(
            carol
                .post_talk(object, None, None, "carol here")
                .unwrap_err()
                .to_string(),
            NOT_VISIBLE
        );
        assert_eq!(
            carol.talk_page(object).unwrap_err().to_string(),
            NOT_VISIBLE
        );

        // Members post, and a member's post carries the object's group.
        let alices = alice.post_talk(object, None, None, "from alice").unwrap();
        let bobs = bob.post_talk(object, None, None, "from bob").unwrap();
        exchange(&bob, &alice);
        let carrier = alice
            .read_store(|store| store.event_by_id(&bobs.event))
            .unwrap()
            .unwrap();
        assert_eq!(carrier.header.extensions.group, Some(group));

        // A non-member's post delivered past the write gate, and a member's post carried
        // outside the object's group, are not on the page.
        let forged = TalkPost::sign(
            PostDraft {
                object_id: object.as_bytes().to_vec(),
                parent_post_id: None,
                supersedes: None,
                authored_at: now_ms(),
                body: "carol slipped in".to_owned(),
            },
            &ed25519_dalek::SigningKey::from_bytes(&carol_key.as_bytes()),
        )
        .unwrap();
        deliver(&alice, &carol_key, Some(group), &forged);
        let outside = TalkPost::sign(
            PostDraft {
                object_id: object.as_bytes().to_vec(),
                parent_post_id: None,
                supersedes: None,
                authored_at: now_ms(),
                body: "outside the group".to_owned(),
            },
            &ed25519_dalek::SigningKey::from_bytes(&key(84).as_bytes()),
        )
        .unwrap();
        deliver(&alice, &key(84), None, &outside);

        let page = alice.talk_page(object).unwrap();
        let ids: Vec<ContentId> = page.iter().map(|entry| entry.post_id).collect();
        assert_eq!(page.len(), 2);
        assert!(ids.contains(&alices.post_id));
        assert!(ids.contains(&bobs.post_id));
    }
}
