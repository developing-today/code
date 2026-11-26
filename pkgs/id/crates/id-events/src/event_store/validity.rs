//! Live validity (writer-set spec §4.6; the last part of #3789).
//!
//! [`crate::idset::project::project`] evaluates `applied` against a pinned
//! resolution `R`, so a saved total never moves. [`EventStore::validity_now`]
//! is the other, explicitly named operation: it asks whether one event is in
//! force under the store's *current* resolution, so a reader can learn that
//! a claim already in a total has since been forked or voided without the
//! total being rewritten.
//!
//! The answers follow `resolve.rs`'s own rules. `APPLIED` is rebuilt inside
//! every ingest transaction, so a stored event that is not applied has
//! already been judged: at or above its author's fork point it is
//! [`Validity::Forked`] (rule 1), and otherwise its authority was voided
//! (rules 2–4), which is [`Validity::Voided`].

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};

use super::{
    APPLIED, EventStore, FORKS, HASHES, PROOF_IDS, StoreError, lowest_proof, split_position,
};

/// Whether an event is in force under the current resolution (§4.6). A
/// separate answer space from [`crate::idset::project::WhyNotCounted`]:
/// this reports on a claim that may already be in a total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    /// Stored and applied.
    Applies,
    /// At or above its author's fork point, or kept only as one of a fork
    /// proof's two headers. Never applies.
    Forked,
    /// Stored and resolved, but not in force: the authority it stood on was
    /// voided (a refused or voided group operation, or content depending on
    /// one by an author who lost access at its position). No fork involved.
    Voided,
    /// Not resolvable from this store: not stored, or still waiting for a
    /// dependency. Never a synonym for `Forked`.
    Unknown,
}

impl EventStore {
    /// The live validity of the event with this ID (§4.6).
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn validity_now(&self, id: &Hash) -> Result<Validity, StoreError> {
        let key = id.as_bytes().as_slice();
        let txn = self.db.begin_read()?;
        if txn.open_table(APPLIED)?.get(key)?.is_some() {
            return Ok(Validity::Applies);
        }
        if let Some(position) = txn.open_table(HASHES)?.get(key)? {
            let (author, seq) = split_position(position.value())?;
            let forks = txn.open_table(FORKS)?;
            return Ok(match lowest_proof(&forks, &author)? {
                Some(proof) if seq >= proof.seq => Validity::Forked,
                _ => Validity::Voided,
            });
        }
        if txn.open_table(PROOF_IDS)?.get(key)?.is_some() {
            return Ok(Validity::Forked);
        }
        Ok(Validity::Unknown)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id_core::membership::AuthId;
    use id_core::waiting::{AuthorKey, WaitingLimits};
    use id_store::chain::{ChainExt, Event, sign};
    use p2panda_auth::group::{GroupAction, GroupMember};
    use p2panda_core::{SeqNum, SigningKey};

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    fn ext(kind: &str, group: Option<Hash>, after: Vec<Hash>) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
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
        }
    }

    fn open_temp() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    fn plain(key: &SigningKey, seq: SeqNum, backlink: Option<Hash>, after: Vec<Hash>) -> Event {
        sign(key, seq, backlink, None, ext("didit.record", None, after)).unwrap()
    }

    fn group_op(
        key: &SigningKey,
        seq: SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        group: Option<Hash>,
        action: &GroupAction<AuthId, ()>,
    ) -> (Event, Vec<u8>) {
        let body = id_core::membership::body::encode_body(action).unwrap();
        let event = sign(
            key,
            seq,
            backlink,
            Some(&body),
            ext("group.op", group, after),
        )
        .unwrap();
        (event, body)
    }

    fn individual(key: &SigningKey) -> GroupMember<AuthId> {
        GroupMember::Individual(AuthId::Key(author(key)))
    }

    #[test]
    fn applied_absent_and_waiting_events() {
        let (_dir, mut store) = open_temp();
        let alice = key(2);
        let first = plain(&alice, 0, None, vec![]);
        store.ingest(&first.bytes, 0).unwrap();
        assert_eq!(store.validity_now(&first.id).unwrap(), Validity::Applies);
        assert_eq!(
            store.validity_now(&Hash::from([66; 32])).unwrap(),
            Validity::Unknown
        );
        // An event naming a dependency the store lacks waits: present, and
        // neither applied nor refused yet.
        let waiting = plain(&key(3), 0, None, vec![Hash::from([67; 32])]);
        store.ingest(&waiting.bytes, 0).unwrap();
        assert!(store.waiting_event(&waiting.id).unwrap().is_some());
        assert_eq!(store.validity_now(&waiting.id).unwrap(), Validity::Unknown);
    }

    #[test]
    fn a_later_fork_proof_flips_counted_events_to_forked() {
        let (_dir, mut store) = open_temp();
        let alice = key(16);
        let mut chain: Vec<Event> = Vec::new();
        for seq in 0..4 {
            let event = plain(&alice, seq, chain.last().map(|event| event.id), vec![]);
            store.ingest(&event.bytes, 0).unwrap();
            chain.push(event);
        }
        assert!(
            chain
                .iter()
                .all(|event| store.validity_now(&event.id).unwrap() == Validity::Applies)
        );
        let rival = sign(
            &alice,
            2,
            Some(chain[1].id),
            None,
            ext("rival", None, vec![]),
        )
        .unwrap();
        store.ingest(&rival.bytes, 0).unwrap();
        let answers: Vec<Validity> = chain
            .iter()
            .map(|event| store.validity_now(&event.id).unwrap())
            .collect();
        assert_eq!(
            answers,
            vec![
                Validity::Applies,
                Validity::Applies,
                Validity::Forked,
                Validity::Forked
            ]
        );
        assert_eq!(store.validity_now(&rival.id).unwrap(), Validity::Forked);
    }

    #[test]
    fn a_refused_group_operation_is_voided_not_forked() {
        let (_dir, mut store) = open_temp();
        let (alice, bob, carol) = (key(31), key(32), key(33));
        let create = GroupAction::Create {
            initial_members: vec![
                (individual(&alice), p2panda_auth::Access::manage()),
                (individual(&bob), p2panda_auth::Access::manage()),
                (individual(&carol), p2panda_auth::Access::read()),
            ],
        };
        let (create_event, create_body) = group_op(&alice, 0, None, vec![], None, &create);
        store
            .ingest_with_body(&create_event.bytes, Some(&create_body), 0)
            .unwrap();
        // Bob, a manager but not the Admin, changes Carol's role: the group
        // fold refuses it, so it is stored and resolved but not in force.
        let promote = GroupAction::Promote {
            member: individual(&carol),
            access: p2panda_auth::Access::write(),
        };
        let (bob_event, bob_body) = group_op(
            &bob,
            0,
            None,
            vec![create_event.id],
            Some(create_event.id),
            &promote,
        );
        store
            .ingest_with_body(&bob_event.bytes, Some(&bob_body), 0)
            .unwrap();
        assert_eq!(
            store.validity_now(&create_event.id).unwrap(),
            Validity::Applies
        );
        assert_eq!(store.validity_now(&bob_event.id).unwrap(), Validity::Voided);
        assert!(store.fork_proof(&author(&bob)).unwrap().is_none());
    }
}
