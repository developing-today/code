//! Ingest: how an event received from a peer is verified, decided, and stored.
//!
//! [`EventStore::ingest`] runs one event through these steps:
//!
//! 1. **Verify.** [`chain::decode`] checks the canonical encoding and the signature
//!    (through `AnyHeader::decode`, which uses `verify_strict`), then the extension's
//!    rules. Anything that fails is refused as [`Refusal::Malformed`].
//! 2. **Duplicate.** An event whose ID is stored, or is one of the author's fork
//!    proof headers, changes nothing.
//! 3. **Fork.** If the author already has an applied event at the event's sequence
//!    number, the two conflict. The chain freezes at the lower of the two sequence
//!    numbers, and both headers are stored as the proof.
//! 4. **Frozen.** A frozen author's events from the fork point on are refused.
//! 5. **Accept.** The next sequence number, whose backlink is the applied head, is
//!    applied once every `after` hash is applied.
//! 6. **Wait.** An event ahead of its chain, or with an `after` hash that is not
//!    applied yet, is held in the waiting pool under its caps. When the dependency is
//!    applied, the event is released and decided again.
//!
//! Each decision that writes runs in one redb write transaction. A refused event
//! writes nothing, and an accepted one is committed before any event it released is
//! decided.
//!
//! # Applied state and forks
//!
//! The applied prefix of a chain is the events below its next sequence number. A fork
//! moves the next sequence number down to the fork point. Events from the fork point
//! on stay stored, but they are not applied. When an event leaves the applied prefix,
//! the waiting pool forgets it, so events that wait on it wait again. The applied
//! state therefore depends on which events a store has received, not on the order in
//! which it received them. The property tests in `tests/event_store_properties.rs`
//! check that.
//!
//! Two cases still depend on arrival order, and the membership layer must handle
//! both. First, a fork does not undo the events applied on top of a later fork, so an
//! event applied before the fork stays applied. Second, a conflict whose backlink is
//! not the applied head is refused, not held, so a conflict that arrives before the
//! event it conflicts with can go unnoticed. The property tests generate neither: their
//! forks share the original's backlink, and their `after` hashes name only events a
//! fork cannot unapply.

use std::collections::VecDeque;

use p2panda_core::{Hash, SeqNum};
use redb::{ReadableTable, WriteTransaction};

use super::{
    CHAINS, Chain, EVENTS, EventStore, FORKS, HASHES, StoreError, corrupt, decode_proof,
    decode_stored, position_key, split_position, store_chain, store_event, store_fork,
};
use crate::chain::{self, Event};
use crate::waiting::{AuthorKey, WaitRefusal};

/// Why an event was refused. A refused event changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The bytes are not a valid event. The message names the rule that failed: a
    /// non-canonical encoding, a bad signature, or an extension that breaks its rules.
    Malformed(String),
    /// The author has forked at or below this sequence number, so later events are
    /// not applied.
    Frozen,
    /// The sequence number is the next one, but the backlink is not the applied head.
    BacklinkMismatch,
    /// The author already holds as many waiting events as the caps allow.
    WaitingFull,
}

/// What [`EventStore::ingest`] did with one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ingest {
    /// The event is stored and applied. `released` lists the waiting events that
    /// became applied because of it, in the order they applied.
    Accepted {
        /// The event's ID.
        id: Hash,
        /// The IDs of the waiting events it released, in application order.
        released: Vec<Hash>,
    },
    /// The event is already stored, or it is one of the author's fork proof headers.
    /// Nothing changed.
    Duplicate {
        /// The event's ID.
        id: Hash,
    },
    /// The author signed two different events at a sequence number that exists. The
    /// chain froze at the lower of the two sequence numbers, and both headers are
    /// stored as the fork proof.
    Fork {
        /// The author.
        author: AuthorKey,
        /// The fork point: the lowest sequence number where the author conflicts.
        seq: SeqNum,
    },
    /// The event waits for a dependency. It is held in memory and not stored. Sync
    /// sends it again if it is lost.
    Waiting {
        /// The event's ID.
        id: Hash,
    },
    /// The event was refused. Nothing changed.
    Refused(Refusal),
}

/// What the store decides for one event, before waiting.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Stored and applied.
    Accept,
    /// Already stored, or a proof header. Nothing written.
    Duplicate,
    /// A different event at `seq`, below the applied prefix, which ended at `applied`.
    /// Written as the fork.
    Fork { seq: SeqNum, applied: SeqNum },
    /// Waits for these dependencies. Nothing written.
    Wait(Vec<Hash>),
    /// Refused. Nothing written.
    Refuse(Refusal),
}

impl EventStore {
    /// Verifies one received event and applies what it allows.
    ///
    /// `now_ms` is the clock the waiting caps use. It is a parameter so tests can
    /// run on a fake clock.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error. A refusal is not an error: it comes back
    /// as [`Ingest::Refused`].
    pub fn ingest(&mut self, bytes: &[u8], now_ms: u64) -> Result<Ingest, StoreError> {
        let event = match chain::decode(bytes) {
            Ok(event) => event,
            Err(err) => return Ok(Ingest::Refused(Refusal::Malformed(err.to_string()))),
        };
        let id = event.id;
        let author = *event.header.verifying_key.as_bytes();
        match self.decide(&event)? {
            Decision::Accept => {
                let arrived = self.waiting.arrived(id, now_ms);
                let released = self.drain(arrived.into(), now_ms)?;
                Ok(Ingest::Accepted { id, released })
            }
            Decision::Duplicate => Ok(Ingest::Duplicate { id }),
            Decision::Fork { seq, applied } => {
                self.forget_unapplied(&author, seq, applied)?;
                Ok(Ingest::Fork { author, seq })
            }
            Decision::Wait(deps) => {
                let outcome = match self.waiting.insert(author, id, deps, event, now_ms) {
                    Ok(()) => Ingest::Waiting { id },
                    Err(WaitRefusal::AuthorFull) => Ingest::Refused(Refusal::WaitingFull),
                };
                self.drain(VecDeque::new(), now_ms)?;
                Ok(outcome)
            }
            Decision::Refuse(refusal) => Ok(Ingest::Refused(refusal)),
        }
    }

    /// Drops the waiting events that have reached the age cap at `now_ms`. Returns
    /// how many were dropped.
    pub fn expire_waiting(&mut self, now_ms: u64) -> usize {
        self.waiting.expire(now_ms).len()
    }

    /// The number of events held waiting, across all authors.
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    /// The number of events `author` has waiting.
    pub fn waiting_count(&self, author: &AuthorKey) -> usize {
        self.waiting.waiting_count(*author)
    }

    /// Decides `event` in one write transaction. A decision that writes nothing aborts
    /// the transaction.
    fn decide(&self, event: &Event) -> Result<Decision, StoreError> {
        let txn = self.db.begin_write()?;
        let decision = decide_in(&txn, event)?;
        if matches!(decision, Decision::Accept | Decision::Fork { .. }) {
            txn.commit()?;
        }
        Ok(decision)
    }

    /// Applies the events in `queue`, then each waiting event they release, until
    /// nothing more applies. Returns the IDs of the waiting events applied, in order.
    fn drain(&mut self, mut queue: VecDeque<Event>, now_ms: u64) -> Result<Vec<Hash>, StoreError> {
        let mut applied = Vec::new();
        queue.extend(self.waiting.take_ready());
        while let Some(event) = queue.pop_front() {
            let id = event.id;
            let author = *event.header.verifying_key.as_bytes();
            match self.decide(&event)? {
                Decision::Accept => {
                    applied.push(id);
                    queue.extend(self.waiting.arrived(id, now_ms));
                }
                Decision::Fork {
                    seq,
                    applied: before,
                } => self.forget_unapplied(&author, seq, before)?,
                Decision::Wait(deps) => {
                    // A refused insert drops the event, as an event past the cap is dropped.
                    let _ = self.waiting.insert(author, id, deps, event, now_ms);
                }
                Decision::Duplicate | Decision::Refuse(_) => {}
            }
            queue.extend(self.waiting.take_ready());
        }
        Ok(applied)
    }

    /// Tells the waiting pool that the events from `from` up to `applied` are no longer
    /// applied, so events that wait on them wait again.
    fn forget_unapplied(
        &mut self,
        author: &AuthorKey,
        from: SeqNum,
        applied: SeqNum,
    ) -> Result<(), StoreError> {
        for seq in from..applied {
            if let Some(event) = self.event(author, seq)? {
                self.waiting.forget(&event.id);
            }
        }
        Ok(())
    }
}

/// Decides `event` inside `txn`, and writes what the decision needs.
fn decide_in(txn: &WriteTransaction, event: &Event) -> Result<Decision, StoreError> {
    let author = *event.header.verifying_key.as_bytes();
    let seq = event.header.seq_num;
    if is_stored(txn, &event.id)? || is_proof_header(txn, &author, &event.id)? {
        return Ok(Decision::Duplicate);
    }
    let chain = read_chain(txn, &author)?;
    if seq < chain.next_seq {
        let stored = stored_event(txn, &author, seq)?;
        let head = match seq.checked_sub(1) {
            Some(before) => Some(stored_event(txn, &author, before)?.id),
            None => None,
        };
        store_fork(txn, &author, seq, &stored, event)?;
        store_chain(
            txn,
            &author,
            Chain {
                next_seq: seq,
                head,
                frozen: true,
            },
        )?;
        return Ok(Decision::Fork {
            seq,
            applied: chain.next_seq,
        });
    }
    if chain.frozen {
        return Ok(Decision::Refuse(Refusal::Frozen));
    }
    let after = &event.header.extensions.after;
    if seq == chain.next_seq {
        if seq > 0 && event.header.backlink != chain.head {
            return Ok(Decision::Refuse(Refusal::BacklinkMismatch));
        }
        let missing = unapplied(txn, after)?;
        if !missing.is_empty() {
            return Ok(Decision::Wait(missing));
        }
        store_event(txn, event)?;
        let next_seq = seq
            .checked_add(1)
            .ok_or_else(|| corrupt("a sequence number overflows"))?;
        store_chain(
            txn,
            &author,
            Chain {
                next_seq,
                head: Some(event.id),
                frozen: false,
            },
        )?;
        return Ok(Decision::Accept);
    }
    // Ahead of the chain: the backlink is the event that must come first.
    let mut deps = unapplied(txn, after)?;
    deps.extend(event.header.backlink);
    Ok(Decision::Wait(deps))
}

fn is_stored(txn: &WriteTransaction, id: &Hash) -> Result<bool, StoreError> {
    let hashes = txn.open_table(HASHES)?;
    let found = hashes.get(id.as_bytes().as_slice())?.is_some();
    Ok(found)
}

fn is_proof_header(
    txn: &WriteTransaction,
    author: &AuthorKey,
    id: &Hash,
) -> Result<bool, StoreError> {
    let forks = txn.open_table(FORKS)?;
    let Some(row) = forks.get(author.as_slice())? else {
        return Ok(false);
    };
    let proof = decode_proof(row.value())?;
    Ok(proof.first.id == *id || proof.second.id == *id)
}

fn read_chain(txn: &WriteTransaction, author: &AuthorKey) -> Result<Chain, StoreError> {
    let chains = txn.open_table(CHAINS)?;
    let row = chains.get(author.as_slice())?;
    match row {
        Some(row) => Chain::decode(row.value()),
        None => Ok(Chain::default()),
    }
}

fn stored_event(
    txn: &WriteTransaction,
    author: &AuthorKey,
    seq: SeqNum,
) -> Result<Event, StoreError> {
    let events = txn.open_table(EVENTS)?;
    let row = events.get(position_key(author, seq).as_slice())?;
    match row {
        Some(row) => decode_stored(row.value()),
        None => Err(corrupt("an applied sequence number has no stored event")),
    }
}

/// The `after` hashes that are not applied yet.
fn unapplied(txn: &WriteTransaction, after: &[Hash]) -> Result<Vec<Hash>, StoreError> {
    let mut missing = Vec::new();
    for hash in after {
        if !is_applied(txn, hash)? {
            missing.push(*hash);
        }
    }
    Ok(missing)
}

/// Whether the event with this ID is applied: stored, at a sequence number below its
/// author's applied prefix.
fn is_applied(txn: &WriteTransaction, hash: &Hash) -> Result<bool, StoreError> {
    let hashes = txn.open_table(HASHES)?;
    let position = hashes
        .get(hash.as_bytes().as_slice())?
        .map(|found| found.value().to_vec());
    let Some(position) = position else {
        return Ok(false);
    };
    let (author, seq) = split_position(&position)?;
    let chains = txn.open_table(CHAINS)?;
    let row = chains.get(author.as_slice())?;
    match row {
        Some(row) => Ok(Chain::decode(row.value())?.next_seq > seq),
        None => Ok(false),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::chain::{ChainExt, sign};
    use crate::waiting::WaitingLimits;
    use p2panda_core::SigningKey;

    const WEEK_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    /// A signed event: its sequence number, backlink, frontier, and kind.
    fn event(
        key: &SigningKey,
        seq: SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        kind: &str,
    ) -> Event {
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
        };
        sign(key, seq, backlink, None, ext).unwrap()
    }

    /// The first `count` events of `key`'s chain, each naming the one before.
    fn chain_of(key: &SigningKey, count: SeqNum) -> Vec<Event> {
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..count {
            let backlink = events.last().map(|event| event.id);
            events.push(event(key, seq, backlink, Vec::new(), "message"));
        }
        events
    }

    fn open() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path()).unwrap();
        (dir, store)
    }

    fn ingest(store: &mut EventStore, event: &Event) -> Ingest {
        store.ingest(&event.bytes, 0).unwrap()
    }

    /// Ingests every event in order, and checks each one was accepted.
    fn ingest_all(store: &mut EventStore, events: &[Event]) {
        for event in events {
            assert_eq!(
                ingest(store, event),
                Ingest::Accepted {
                    id: event.id,
                    released: vec![],
                }
            );
        }
    }

    fn sorted(a: &Hash, b: &Hash) -> [Hash; 2] {
        let mut ids = [*a, *b];
        ids.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
        ids
    }

    #[test]
    fn test_a_valid_chain_is_accepted_in_order() {
        let (_dir, mut store) = open();
        let signer = key(1);
        let events = chain_of(&signer, 3);
        ingest_all(&mut store, &events);
        let chain = store.chain(&author(&signer)).unwrap().unwrap();
        assert_eq!(
            chain,
            Chain {
                next_seq: 3,
                head: Some(events[2].id),
                frozen: false,
            }
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_duplicate_is_idempotent_and_changes_nothing() {
        let (_dir, mut store) = open();
        let signer = key(2);
        let events = chain_of(&signer, 2);
        ingest_all(&mut store, &events);
        let before = store.chain(&author(&signer)).unwrap();
        assert_eq!(
            ingest(&mut store, &events[0]),
            Ingest::Duplicate { id: events[0].id }
        );
        assert_eq!(
            ingest(&mut store, &events[1]),
            Ingest::Duplicate { id: events[1].id }
        );
        assert_eq!(store.chain(&author(&signer)).unwrap(), before);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_fork_freezes_the_chain_and_stores_both_headers() {
        let (_dir, mut store) = open();
        let signer = key(3);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        ingest_all(&mut store, &events);
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );
        assert_eq!(
            store.chain(&who).unwrap(),
            Some(Chain {
                next_seq: 1,
                head: Some(events[0].id),
                frozen: true,
            })
        );
        let proof = store.fork_proof(&who).unwrap().expect("a proof is stored");
        assert_eq!(proof.seq, 1);
        assert_eq!(
            [proof.first.id, proof.second.id],
            sorted(&events[1].id, &rival.id)
        );
        // The event that was applied stays stored. It just no longer applies.
        assert_eq!(store.event(&who, 1).unwrap(), Some(events[1].clone()));
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_frozen_chain_refuses_its_later_events_and_changes_nothing() {
        let (_dir, mut store) = open();
        let signer = key(4);
        let who = author(&signer);
        let events = chain_of(&signer, 3);
        ingest_all(&mut store, &events[..2]);
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        assert!(matches!(
            ingest(&mut store, &rival),
            Ingest::Fork { seq: 1, .. }
        ));
        let before = store.chain(&who).unwrap();
        assert_eq!(
            ingest(&mut store, &events[2]),
            Ingest::Refused(Refusal::Frozen)
        );
        assert!(store.event(&who, 2).unwrap().is_none());
        assert_eq!(store.chain(&who).unwrap(), before);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_fork_at_sequence_zero_leaves_no_applied_head() {
        let (_dir, mut store) = open();
        let signer = key(5);
        let who = author(&signer);
        let first = event(&signer, 0, None, Vec::new(), "first");
        let rival = event(&signer, 0, None, Vec::new(), "rival");
        ingest_all(&mut store, std::slice::from_ref(&first));
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: who,
                seq: 0
            }
        );
        assert_eq!(
            store.chain(&who).unwrap(),
            Some(Chain {
                next_seq: 0,
                head: None,
                frozen: true,
            })
        );
        assert_eq!(store.fork_proof(&who).unwrap().unwrap().seq, 0);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_conflict_below_the_fork_point_moves_the_fork_point_down() {
        let (_dir, mut store) = open();
        let signer = key(6);
        let who = author(&signer);
        let events = chain_of(&signer, 4);
        ingest_all(&mut store, &events);
        let high = event(&signer, 2, Some(events[1].id), Vec::new(), "high");
        assert_eq!(
            ingest(&mut store, &high),
            Ingest::Fork {
                author: who,
                seq: 2
            }
        );
        let low = event(&signer, 1, Some(events[0].id), Vec::new(), "low");
        assert_eq!(
            ingest(&mut store, &low),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );
        assert_eq!(
            store.chain(&who).unwrap(),
            Some(Chain {
                next_seq: 1,
                head: Some(events[0].id),
                frozen: true,
            })
        );
        let proof = store.fork_proof(&who).unwrap().unwrap();
        assert_eq!(proof.seq, 1);
        assert_eq!(
            [proof.first.id, proof.second.id],
            sorted(&events[1].id, &low.id)
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_the_fork_state_does_not_depend_on_which_header_arrived_first() {
        let signer = key(7);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        let (_first_dir, mut first) = open();
        ingest_all(&mut first, &events);
        ingest(&mut first, &rival);
        let (_second_dir, mut second) = open();
        ingest_all(&mut second, &events[..1]);
        ingest(&mut second, &rival);
        ingest(&mut second, &events[1]);
        assert_eq!(first.chain(&who).unwrap(), second.chain(&who).unwrap());
        assert_eq!(
            first.fork_proof(&who).unwrap(),
            second.fork_proof(&who).unwrap()
        );
    }

    #[test]
    fn test_an_event_ahead_of_the_chain_waits_and_its_predecessor_releases_it() {
        let (_dir, mut store) = open();
        let signer = key(8);
        let who = author(&signer);
        let events = chain_of(&signer, 3);
        assert_eq!(
            ingest(&mut store, &events[2]),
            Ingest::Waiting { id: events[2].id }
        );
        assert!(
            store.event(&who, 2).unwrap().is_none(),
            "a waiting event is not stored"
        );
        assert_eq!(store.waiting_len(), 1);
        assert_eq!(
            ingest(&mut store, &events[0]),
            Ingest::Accepted {
                id: events[0].id,
                released: vec![],
            }
        );
        assert_eq!(
            ingest(&mut store, &events[1]),
            Ingest::Accepted {
                id: events[1].id,
                released: vec![events[2].id],
            }
        );
        assert_eq!(store.chain(&who).unwrap().unwrap().next_seq, 3);
        assert_eq!(store.waiting_len(), 0);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_an_event_waiting_for_an_after_hash_is_released_by_that_event() {
        let (_dir, mut store) = open();
        let x = key(9);
        let y = key(10);
        let x0 = event(&x, 0, None, Vec::new(), "x");
        let y0 = event(&y, 0, None, vec![x0.id], "y");
        assert_eq!(ingest(&mut store, &y0), Ingest::Waiting { id: y0.id });
        assert_eq!(
            ingest(&mut store, &x0),
            Ingest::Accepted {
                id: x0.id,
                released: vec![y0.id],
            }
        );
        assert_eq!(store.chain(&author(&y)).unwrap().unwrap().next_seq, 1);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_an_after_hash_naming_an_unapplied_event_keeps_waiting() {
        let (_dir, mut store) = open();
        let x = key(11);
        let y = key(12);
        let events = chain_of(&x, 2);
        ingest_all(&mut store, &events);
        let rival = event(&x, 1, Some(events[0].id), Vec::new(), "rival");
        ingest(&mut store, &rival);
        // events[1] is stored, but the fork took it out of the applied prefix.
        let y0 = event(&y, 0, None, vec![events[1].id], "y");
        assert_eq!(ingest(&mut store, &y0), Ingest::Waiting { id: y0.id });
        assert!(store.chain(&author(&y)).unwrap().is_none());
    }

    #[test]
    fn test_a_backlink_that_is_not_the_applied_head_is_refused() {
        let (_dir, mut store) = open();
        let signer = key(13);
        let who = author(&signer);
        let events = chain_of(&signer, 1);
        ingest_all(&mut store, &events);
        let stray = event(
            &signer,
            1,
            Some(Hash::digest(b"not the head")),
            Vec::new(),
            "stray",
        );
        assert_eq!(
            ingest(&mut store, &stray),
            Ingest::Refused(Refusal::BacklinkMismatch)
        );
        assert!(store.event(&who, 1).unwrap().is_none());
        assert_eq!(store.chain(&who).unwrap().unwrap().next_seq, 1);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_bytes_that_do_not_verify_are_refused_and_change_nothing() {
        let (_dir, mut store) = open();
        let signer = key(14);
        let events = chain_of(&signer, 1);
        assert!(matches!(
            store.ingest(b"not cbor", 0).unwrap(),
            Ingest::Refused(Refusal::Malformed(_))
        ));
        // The signature is 64 bytes after the 0x58 0x40 header at offset 37. Flip one.
        let mut forged = events[0].bytes.clone();
        forged[48] ^= 0x01;
        let outcome = store.ingest(&forged, 0).unwrap();
        assert!(
            matches!(&outcome, Ingest::Refused(Refusal::Malformed(message)) if message.contains("signature")),
            "a forged signature was not refused: {outcome:?}"
        );
        assert!(store.chain(&author(&signer)).unwrap().is_none());
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_waiting_events_past_the_author_cap_are_refused() {
        let (_dir, store) = open();
        let mut store = store.with_waiting_limits(WaitingLimits::new(2, WEEK_MS));
        let signer = key(15);
        let who = author(&signer);
        let events = chain_of(&signer, 5);
        assert_eq!(
            ingest(&mut store, &events[2]),
            Ingest::Waiting { id: events[2].id }
        );
        assert_eq!(
            ingest(&mut store, &events[3]),
            Ingest::Waiting { id: events[3].id }
        );
        assert_eq!(
            ingest(&mut store, &events[4]),
            Ingest::Refused(Refusal::WaitingFull)
        );
        assert_eq!(store.waiting_count(&who), 2);
        assert!(store.event(&who, 4).unwrap().is_none());
    }

    #[test]
    fn test_a_waiting_event_expires_and_is_not_released_by_its_dependency() {
        let (_dir, store) = open();
        let mut store = store.with_waiting_limits(WaitingLimits::new(256, 1_000));
        let signer = key(16);
        let events = chain_of(&signer, 2);
        store.ingest(&events[1].bytes, 0).unwrap();
        assert_eq!(store.expire_waiting(1_000), 1);
        assert_eq!(
            store.ingest(&events[0].bytes, 1_000).unwrap(),
            Ingest::Accepted {
                id: events[0].id,
                released: vec![],
            }
        );
        assert!(store.event(&author(&signer), 1).unwrap().is_none());
        assert_eq!(store.waiting_len(), 0);
    }

    #[test]
    fn test_group_members_follow_applied_events_only() {
        let (_dir, mut store) = open();
        let signer = key(17);
        let who = author(&signer);
        let group = Hash::digest(b"group");
        let grouped = |seq: SeqNum, backlink: Option<Hash>| {
            let ext = ChainExt {
                kind: "message".to_owned(),
                group: Some(group),
                after: Vec::new(),
            };
            sign(&signer, seq, backlink, None, ext).unwrap()
        };
        let first = grouped(0, None);
        let second = grouped(1, Some(first.id));
        ingest(&mut store, &second);
        assert!(store.group_members(&group).unwrap().is_empty());
        ingest(&mut store, &first);
        assert_eq!(
            store.group_members(&group).unwrap(),
            vec![(who, 0), (who, 1)]
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_ingested_state_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(18);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        {
            let mut store = EventStore::open(dir.path()).unwrap();
            ingest_all(&mut store, &events);
            ingest(&mut store, &rival);
        }
        let store = EventStore::open(dir.path()).unwrap();
        assert_eq!(
            store.chain(&who).unwrap(),
            Some(Chain {
                next_seq: 1,
                head: Some(events[0].id),
                frozen: true,
            })
        );
        assert!(store.fork_proof(&who).unwrap().is_some());
        store.check_invariants().unwrap();
    }
}
