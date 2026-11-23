//! Graph-backed [`MembershipFrontier`]: group authorization read from
//! the signed graph's resolved membership state (#3796 follow-on to
//! the relay gate in [`crate::relay_gate`]).
//!
//! [`AllowlistFrontier`](crate::relay_gate::AllowlistFrontier) is a
//! local stand-in: a hand-kept member list that can drift from the
//! graph. This backing answers from the event store instead —
//! [`EventStore::roster`] is the resolved fold of the group's signed
//! membership operations, the same read `local_events` gates document
//! writes on. A member with access at the end of the causal history
//! is listed; a removed member is not, so a removal takes effect at
//! the gate as soon as it is in the store.
//!
//! # The membership point
//!
//! Authorization is evaluated against the store's *resolved* state:
//! the fold of every applied group event the store holds. That is
//! the snapshot a store can offer today (the writer-set module in
//! id-events records the same boundary); membership as of an older
//! causal position — frontier-historical resolution — stays with the
//! membership layer's pinned expander and is not what a forwarding
//! gate needs: the gate's question is whether the origin may write
//! the group *now*, under everything the relay has accepted.
//!
//! # Identifier mapping
//!
//! The envelope names both sides as text; the graph names them as
//! typed IDs, and the mapping is the graph's own:
//!
//! - `origin` is the hex of the author's Ed25519 verifying key — the
//!   [`AuthorKey`](id_core::waiting::AuthorKey) the roster lists (the
//!   envelope builder writes exactly this form).
//! - `group` is the hex of the group ID — the BLAKE3 hash of the
//!   group's `group.create` event
//!   (`AuthId::Group`, design §5.5), parsed with [`Hash::from_str`].
//!
//! # Fail-closed
//!
//! The gate asks a yes/no question and the frontier never guesses: an
//! origin that is not 64 hex characters, a group that does not parse
//! as a hash, an unknown group, and a store that errors all authorize
//! nobody. Nested membership (a group inside a group) does not
//! authorize individual keys through this read: the roster lists key
//! members only, matching the store's roster projection.

use std::str::FromStr;

use id_events::event_store::EventStore;
use id_store::store_traits::SharedEventStore;
use p2panda_core::Hash;

use crate::hex::hex_to_array;
use crate::relay_gate::MembershipFrontier;

/// Group authorization backed by the signed graph: wraps the node's
/// shared event store and answers [`MembershipFrontier`] from its
/// resolved group rosters. Clones share the store, so a gate built on
/// a clone sees membership changes as they are ingested. The store
/// memoizes each group's resolved roster until the next committed
/// ingest (#13043), so a gate decision costs a roster lookup rather
/// than a whole-store fold per message.
#[derive(Clone, Debug)]
pub struct GraphFrontier {
    store: SharedEventStore<EventStore>,
}

impl GraphFrontier {
    /// A frontier over `store` (the node's shared event store).
    pub fn new(store: SharedEventStore<EventStore>) -> Self {
        Self { store }
    }
}

impl MembershipFrontier for GraphFrontier {
    fn is_authorized(&self, origin: &str, group: &str) -> bool {
        let Some(author) = hex_to_array::<32>(origin) else {
            return false;
        };
        let Ok(group) = Hash::from_str(group.trim()) else {
            return false;
        };
        self.store
            .read(|store| Ok(store.roster(&group)?.contains(&author)))
            .unwrap_or(false)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use id_core::membership::AuthId;
    use id_core::waiting::{AuthorKey, WaitingLimits};
    use id_store::chain::{ChainExt, Event, sign};
    use p2panda_auth::Access;
    use p2panda_auth::group::{GroupAction, GroupMember};

    use crate::chained_message::{ChainedContent, ChainedEnvelope, RelayedMessage};
    use crate::hex::hex_encode;
    use crate::relay_gate::{GateDecision, GateRefusal, RelayGate};

    // Oracle: the #2036 design record §4.2 (the gate asks the
    // frontier; the frontier answers from signed-graph membership)
    // and the event store's roster contract (resolved state; a
    // removed member is not listed).

    /// Dalek key: envelopes and the gate.
    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// p2panda key: group operations (the chain signer). The same
    /// seed gives the same author bytes as [`key`].
    fn pkey(seed: u8) -> p2panda_core::SigningKey {
        p2panda_core::SigningKey::from_bytes(&[seed; 32])
    }

    fn key_id(key: &SigningKey) -> String {
        hex_encode(&key.verifying_key().to_bytes())
    }

    fn pkey_id(key: &p2panda_core::SigningKey) -> String {
        hex_encode(key.verifying_key().as_bytes())
    }

    fn author(key: &p2panda_core::SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    fn individual(key: &p2panda_core::SigningKey) -> GroupMember<AuthId> {
        GroupMember::Individual(AuthId::Key(author(key)))
    }

    /// A signed group operation and its encoded body, built the way
    /// the event store's own tests build them (public constructors).
    fn group_op(
        key: &p2panda_core::SigningKey,
        seq: p2panda_core::SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        group: Option<Hash>,
        action: &GroupAction<AuthId, ()>,
    ) -> (Event, Vec<u8>) {
        let body = id_core::membership::body::encode_body(action).unwrap();
        let ext = ChainExt {
            kind: "group.op".to_owned(),
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
        };
        let event = sign(key, seq, backlink, Some(&body), ext).unwrap();
        (event, body)
    }

    fn ingest_op(store: &mut EventStore, event: &Event, body: &[u8]) {
        store.ingest_with_body(&event.bytes, Some(body), 0).unwrap();
    }

    fn open_store() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    /// Alice creates a group with Bob as a reader; the group ID is
    /// the create event's ID. Returns the store and the group.
    fn store_with_group(
        alice: &p2panda_core::SigningKey,
        bob: &p2panda_core::SigningKey,
    ) -> (tempfile::TempDir, EventStore, Hash) {
        let (dir, mut store) = open_store();
        let create = GroupAction::Create {
            initial_members: vec![
                (individual(alice), Access::manage()),
                (individual(bob), Access::read()),
            ],
        };
        let (create_event, create_body) = group_op(alice, 0, None, vec![], None, &create);
        let group = create_event.id;
        ingest_op(&mut store, &create_event, &create_body);
        assert!(store.event_applied(&group).unwrap());
        (dir, store, group)
    }

    #[test]
    fn resolved_roster_authorizes_members_only() {
        let (alice, bob, carol, dave) = (pkey(41), pkey(42), pkey(43), pkey(44));
        let (_dir, mut store, group) = store_with_group(&alice, &bob);

        // Alice (the creator, hence the group's Admin) adds Carol.
        let add_carol = GroupAction::Add {
            member: individual(&carol),
            access: Access::read(),
        };
        let (add_event, add_body) =
            group_op(&alice, 1, Some(group), vec![group], Some(group), &add_carol);
        ingest_op(&mut store, &add_event, &add_body);
        assert!(store.event_applied(&add_event.id).unwrap());

        let frontier = GraphFrontier::new(SharedEventStore::new(store));
        let group_text = group.to_string();
        assert!(frontier.is_authorized(&pkey_id(&alice), &group_text));
        assert!(frontier.is_authorized(&pkey_id(&bob), &group_text));
        assert!(frontier.is_authorized(&pkey_id(&carol), &group_text));
        // A stranger was never a member.
        assert!(!frontier.is_authorized(&pkey_id(&dave), &group_text));
        // A well-formed hash that names no group authorizes nobody.
        let other = Hash::digest(b"no such group").to_string();
        assert!(!frontier.is_authorized(&pkey_id(&alice), &other));
    }

    #[test]
    fn removed_member_is_refused() {
        let (alice, bob) = (pkey(51), pkey(52));
        let (_dir, mut store, group) = store_with_group(&alice, &bob);

        let remove_bob = GroupAction::Remove {
            member: individual(&bob),
        };
        let (remove_event, remove_body) = group_op(
            &alice,
            1,
            Some(group),
            vec![group],
            Some(group),
            &remove_bob,
        );
        ingest_op(&mut store, &remove_event, &remove_body);
        assert!(store.event_applied(&remove_event.id).unwrap());

        let frontier = GraphFrontier::new(SharedEventStore::new(store));
        let group_text = group.to_string();
        assert!(frontier.is_authorized(&pkey_id(&alice), &group_text));
        assert!(
            !frontier.is_authorized(&pkey_id(&bob), &group_text),
            "a removed member is not on the resolved roster"
        );
    }

    #[test]
    fn removal_ingested_after_the_frontier_opens_takes_effect() {
        // The store's roster memo (#13043) must not freeze membership
        // at first read: events ingested through the shared store
        // after the frontier is built move the store generation, so
        // the next gate question re-resolves.
        let (alice, bob, carol) = (pkey(81), pkey(82), pkey(83));
        let (_dir, store, group) = store_with_group(&alice, &bob);
        // Carol's unrelated group, ingested later through the handle.
        let carol_create = GroupAction::Create {
            initial_members: vec![(individual(&carol), Access::manage())],
        };
        let (carol_event, carol_body) = group_op(&carol, 0, None, vec![], None, &carol_create);

        let shared = SharedEventStore::new(store);
        let frontier = GraphFrontier::new(shared.clone());
        let group_text = group.to_string();
        assert!(frontier.is_authorized(&pkey_id(&bob), &group_text));

        // Unrelated ingest through the shared handle: the existing
        // members' answers are unchanged.
        shared
            .ingest(&carol_event.bytes, Some(&carol_body), 0)
            .unwrap();
        assert!(frontier.is_authorized(&pkey_id(&alice), &group_text));
        assert!(frontier.is_authorized(&pkey_id(&bob), &group_text));

        // The removal, ingested the same way, refuses Bob at once.
        let remove_bob = GroupAction::Remove {
            member: individual(&bob),
        };
        let (remove_event, remove_body) = group_op(
            &alice,
            1,
            Some(group),
            vec![group],
            Some(group),
            &remove_bob,
        );
        shared
            .ingest(&remove_event.bytes, Some(&remove_body), 0)
            .unwrap();
        assert!(frontier.is_authorized(&pkey_id(&alice), &group_text));
        assert!(
            !frontier.is_authorized(&pkey_id(&bob), &group_text),
            "a removal ingested after the frontier opened must take effect"
        );
    }

    #[test]
    fn malformed_ids_authorize_nobody() {
        let (_dir, store) = open_store();
        let frontier = GraphFrontier::new(SharedEventStore::new(store));
        let origin = key_id(&key(61));
        let group = Hash::digest(b"group").to_string();
        // Origins that are not a canonical 64-hex key.
        assert!(!frontier.is_authorized("not-hex", &group));
        assert!(!frontier.is_authorized(&origin[..62], &group));
        assert!(!frontier.is_authorized(&format!("+{}", &origin[1..]), &group));
        // Groups that do not parse as a hash — including the local
        // test-group spelling the allowlist stand-in uses.
        assert!(!frontier.is_authorized(&origin, "group-1"));
        assert!(!frontier.is_authorized(&origin, "not-a-hash"));
        assert!(!frontier.is_authorized(&origin, ""));
    }

    #[test]
    fn gate_forwards_for_graph_member_and_refuses_removed_member() {
        // The whole path: signed membership events in a real store,
        // read through the frontier by the production gate.
        let (alice, bob) = (pkey(71), pkey(72));
        let (_dir, mut store, group) = store_with_group(&alice, &bob);
        let remove_bob = GroupAction::Remove {
            member: individual(&bob),
        };
        let (remove_event, remove_body) = group_op(
            &alice,
            1,
            Some(group),
            vec![group],
            Some(group),
            &remove_bob,
        );
        ingest_op(&mut store, &remove_event, &remove_body);

        let frontier = GraphFrontier::new(SharedEventStore::new(store));
        let mut gate = RelayGate::new(&key(7), frontier);
        let group_text = group.to_string();
        let envelope = |origin: &SigningKey| {
            ChainedEnvelope::signed(
                origin,
                &group_text,
                0,
                Vec::new(),
                "durable",
                ChainedContent::Inline {
                    bytes: b"hello".to_vec(),
                },
                1_000,
            )
            .unwrap()
        };
        let member = gate.decide(&RelayedMessage::new(envelope(&key(71))), 2_000);
        assert!(
            matches!(member, GateDecision::Forward(_)),
            "a roster member's message must forward, got {member:?}"
        );
        let removed = gate.decide(&RelayedMessage::new(envelope(&key(72))), 2_001);
        assert!(
            matches!(
                removed,
                GateDecision::Refuse(GateRefusal::NotAuthorized { .. })
            ),
            "a removed member's message must be refused, got {removed:?}"
        );
        assert_eq!(gate.forwarded(), 1);
    }
}
