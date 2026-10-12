//! Ingest: how an event received from a peer is verified, decided, and stored.
//!
//! [`EventStore::ingest`] runs one event through these steps:
//!
//! 1. **Verify.** [`chain::decode`] checks the canonical encoding and the signature
//!    (through `AnyHeader::decode`, which uses `verify_strict`), then the extension's
//!    rules. Anything that fails is refused as [`Refusal::Malformed`].
//! 2. **Duplicate.** An event whose ID is stored, or is one of its author's fork proof
//!    headers, changes nothing.
//! 3. **Frozen.** An event above its author's fork point is refused as
//!    [`Refusal::Frozen`].
//! 4. **Fork.** If the author has another event at the same sequence number, stored or
//!    held, the two are a fork, whatever their backlinks and dependencies. The fork point
//!    moves down to that sequence number, and the proof holds the two lowest IDs there.
//!    The applied events from that point on are unapplied, and so is every applied event
//!    that rests on one of them.
//! 5. **Accept.** The next sequence number, whose backlink is the applied head, is applied
//!    once every `after` hash is applied.
//! 6. **Wait or park.** An event ahead of its chain, or with an `after` hash that is not
//!    applied yet, is held in the waiting pool under its caps, and decided again when its
//!    dependencies are applied. An event whose backlink can never be the applied head is
//!    parked: held for good, so that a conflict with it is still seen.
//!
//! Each decision that writes runs in one redb write transaction. A refused or held event
//! writes nothing, and an accepted one is committed before any event it released is
//! decided.
//!
//! # Applied state and forks
//!
//! The applied state depends on the set of events a store has received, not on their
//! arrival order. An event is applied when it sits below its author's fork point, its
//! backlink is the applied event before it, and every `after` hash is applied. The fork
//! point is the lowest sequence number where the author has two distinct events, and the
//! proof holds the two lowest IDs there.
//!
//! An event that is stored but not applied is voided. A fork voids the author's events
//! from its fork point on, and every event that rests on a voided one. A voided event is
//! kept, because it still counts as the author's event at its sequence number, so a later
//! conflict with it is still a fork. The applied prefix of a chain is the events below its
//! next sequence number, so the chain row is what marks the voids.
//!
//! Held events count too. Two held events at one sequence number are a fork, though
//! neither is applied. A restart forgets held events, and sync sends them again.
//!
//! The property tests in `tests/event_store_properties.rs` check the applied state against
//! this definition, for every arrival order.

use std::collections::VecDeque;

use p2panda_core::{Hash, SeqNum};
use redb::{ReadableMultimapTable, ReadableTable, WriteTransaction};

use super::{
    CHAINS, Chain, DEPENDENTS, EVENTS, EventStore, FORKS, ForkProof, HASHES, StoreError, corrupt,
    decode_proof, decode_stored, position_key, split_position, store_chain, store_event,
    store_fork,
};
use crate::chain::{self, Event};
use crate::waiting::{AuthorKey, WaitRefusal};

/// Why an event was refused. A refused event changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The bytes are not a valid event. The message names the rule that failed: a
    /// non-canonical encoding, a bad signature, or an extension that breaks its rules.
    Malformed(String),
    /// The event is above its author's fork point, so it never applies.
    Frozen,
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
    /// The author signed two different events at one sequence number. The chain is
    /// frozen there, and both headers are stored as the proof. A further event at the
    /// fork point that changes the proof is reported the same way.
    Fork {
        /// The author.
        author: AuthorKey,
        /// The fork point: the lowest sequence number where the author conflicts.
        seq: SeqNum,
    },
    /// The event is held in memory and not stored. It waits for a dependency, or it is
    /// parked because its backlink can never be the applied head. Sync sends it again if
    /// it is lost.
    Waiting {
        /// The event's ID.
        id: Hash,
    },
    /// The event was refused. Nothing changed.
    Refused(Refusal),
}

/// What the store decides for one event, before the waiting pool sees it.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Stored and applied.
    Accept,
    /// Already stored, or a proof header. Nothing written.
    Duplicate,
    /// A conflict. The proof is written, and the fork point is `seq`. `unapplied` are
    /// the events that left the applied prefix, which the waiting pool forgets.
    Fork { seq: SeqNum, unapplied: Vec<Hash> },
    /// Waits for these dependencies, none of which is applied. Nothing written.
    Wait(Vec<Hash>),
    /// Can never apply, but is held so that a conflict with it is seen. Nothing written.
    Park,
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
        let author = author_of(&event);
        let rivals = self.held_rivals(&event);
        match self.decide(&event, &rivals)? {
            Decision::Accept => {
                let arrived = self.waiting.arrived(id, now_ms);
                let released = self.drain(arrived.into(), now_ms)?;
                Ok(Ingest::Accepted { id, released })
            }
            Decision::Duplicate => Ok(Ingest::Duplicate { id }),
            Decision::Fork { seq, unapplied } => {
                self.forget(&unapplied);
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
            Decision::Park => match self.waiting.park(author, id, event, now_ms) {
                Ok(()) => Ok(Ingest::Waiting { id }),
                Err(WaitRefusal::AuthorFull) => Ok(Ingest::Refused(Refusal::WaitingFull)),
            },
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

    /// The held events that conflict with `event`: the author's other events at its
    /// sequence number.
    fn held_rivals(&self, event: &Event) -> Vec<Event> {
        let author = author_of(event);
        let seq = event.header.seq_num;
        self.waiting
            .held_by(&author)
            .filter(|held| held.header.seq_num == seq && held.id != event.id)
            .cloned()
            .collect()
    }

    /// Decides `event` in one write transaction. A decision that writes nothing aborts
    /// the transaction.
    fn decide(&self, event: &Event, rivals: &[Event]) -> Result<Decision, StoreError> {
        let txn = self.db.begin_write()?;
        let decision = decide_in(&txn, event, rivals)?;
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
            let author = author_of(&event);
            let rivals = self.held_rivals(&event);
            match self.decide(&event, &rivals)? {
                Decision::Accept => {
                    applied.push(id);
                    queue.extend(self.waiting.arrived(id, now_ms));
                }
                Decision::Fork { unapplied, .. } => self.forget(&unapplied),
                Decision::Wait(deps) => {
                    // A refused insert drops the event, as an event past the cap is dropped.
                    let _ = self.waiting.insert(author, id, deps, event, now_ms);
                }
                Decision::Park => {
                    let _ = self.waiting.park(author, id, event, now_ms);
                }
                Decision::Duplicate | Decision::Refuse(_) => {}
            }
            queue.extend(self.waiting.take_ready());
        }
        Ok(applied)
    }

    /// Tells the waiting pool that these events are no longer applied, so events that
    /// wait on them wait again.
    fn forget(&mut self, unapplied: &[Hash]) {
        for id in unapplied {
            self.waiting.forget(id);
        }
    }
}

/// Decides `event` inside `txn`, and writes what the decision needs. `rivals` are the
/// held events of the same author at the same sequence number.
fn decide_in(
    txn: &WriteTransaction,
    event: &Event,
    rivals: &[Event],
) -> Result<Decision, StoreError> {
    let author = author_of(event);
    let seq = event.header.seq_num;
    if is_stored(txn, &event.id)? || is_proof_header(txn, &author, &event.id)? {
        return Ok(Decision::Duplicate);
    }
    // Every other event the store knows of at this sequence number: stored, or held.
    let mut conflicts: Vec<Event> = rivals.to_vec();
    if let Some(stored) = stored_at(txn, &author, seq)? {
        conflicts.push(stored);
    }
    if let Some(proof) = read_proof(txn, &author)? {
        if seq > proof.seq {
            return Ok(Decision::Refuse(Refusal::Frozen));
        }
        if seq == proof.seq {
            // Another event at the fork point. The proof keeps the two lowest IDs there.
            conflicts.extend([proof.first.clone(), proof.second.clone(), event.clone()]);
            let (first, second) = two_lowest(conflicts)?;
            if first.id == proof.first.id && second.id == proof.second.id {
                return Ok(Decision::Refuse(Refusal::Frozen));
            }
            store_fork(txn, &author, seq, &first, &second)?;
            return Ok(Decision::Fork {
                seq,
                unapplied: Vec::new(),
            });
        }
    }
    if !conflicts.is_empty() {
        // The first conflict at this sequence number, or one below the fork point.
        conflicts.push(event.clone());
        let (first, second) = two_lowest(conflicts)?;
        store_fork(txn, &author, seq, &first, &second)?;
        let unapplied = freeze_and_unapply(txn, &author, seq)?;
        return Ok(Decision::Fork { seq, unapplied });
    }

    let chain = read_chain(txn, &author)?;
    if seq < chain.next_seq {
        // The applied event at this sequence number is stored, so it would be a conflict.
        return Err(corrupt("an applied sequence number has no stored event"));
    }
    let deps = unapplied_deps(txn, &event.header.extensions.after)?;
    if seq == chain.next_seq {
        if seq > 0 && event.header.backlink != chain.head {
            return Ok(Decision::Park);
        }
        if !deps.is_empty() {
            return Ok(Decision::Wait(deps));
        }
        return accept(txn, event, chain);
    }
    // Ahead of the chain. The backlink must be applied first, unless it already is, at
    // another position, and then this event can never apply.
    let mut waits = deps;
    let missing_backlink = match event.header.backlink {
        Some(backlink) => (!is_applied(txn, &backlink)?).then_some(backlink),
        None => None,
    };
    waits.extend(missing_backlink);
    if waits.is_empty() {
        Ok(Decision::Park)
    } else {
        Ok(Decision::Wait(waits))
    }
}

/// Stores `event` as the next applied event of its author, and records what it rests on.
fn accept(txn: &WriteTransaction, event: &Event, chain: Chain) -> Result<Decision, StoreError> {
    let author = author_of(event);
    let seq = event.header.seq_num;
    store_event(txn, event)?;
    index_dependents(txn, event)?;
    let next_seq = seq
        .checked_add(1)
        .ok_or_else(|| corrupt("a sequence number overflows"))?;
    store_chain(
        txn,
        &author,
        Chain {
            next_seq,
            head: Some(event.id),
            frozen: chain.frozen,
        },
    )?;
    Ok(Decision::Accept)
}

/// Records, for each `after` hash of `event`, that the event rests on it.
fn index_dependents(txn: &WriteTransaction, event: &Event) -> Result<(), StoreError> {
    let position = position_key(&author_of(event), event.header.seq_num);
    let mut dependents = txn.open_multimap_table(DEPENDENTS)?;
    for dep in &event.header.extensions.after {
        dependents.insert(dep.as_bytes().as_slice(), position.as_slice())?;
    }
    Ok(())
}

/// Freezes `author` at `seq`. The applied prefix ends there, and every applied event
/// from `seq` up is unapplied, with whatever rests on those. Returns the events that
/// left the applied prefix.
fn freeze_and_unapply(
    txn: &WriteTransaction,
    author: &AuthorKey,
    seq: SeqNum,
) -> Result<Vec<Hash>, StoreError> {
    let chain = read_chain(txn, author)?;
    set_prefix(txn, author, chain.next_seq, true)?;
    unapply_from(txn, *author, seq)
}

/// Unapplies `author`'s applied events from `seq` up, and every applied event that rests
/// on one of them. Each chain it touches is rewritten to its new applied prefix, with its
/// frozen flag kept. Returns the events that left the applied prefix.
fn unapply_from(
    txn: &WriteTransaction,
    author: AuthorKey,
    seq: SeqNum,
) -> Result<Vec<Hash>, StoreError> {
    let mut unapplied = Vec::new();
    let mut pending = vec![(author, seq)];
    while let Some((who, from)) = pending.pop() {
        let chain = read_chain(txn, &who)?;
        if from >= chain.next_seq {
            continue;
        }
        for position in from..chain.next_seq {
            let event = stored_event(txn, &who, position)?;
            for dependent in dependents_of(txn, &event.id)? {
                pending.push(split_position(&dependent)?);
            }
            unapplied.push(event.id);
        }
        set_prefix(txn, &who, from, chain.frozen)?;
    }
    Ok(unapplied)
}

/// Writes `author`'s chain row for an applied prefix of `next_seq` events. A chain with
/// no applied event and no fork has no row.
fn set_prefix(
    txn: &WriteTransaction,
    author: &AuthorKey,
    next_seq: SeqNum,
    frozen: bool,
) -> Result<(), StoreError> {
    if next_seq == 0 && !frozen {
        let mut chains = txn.open_table(CHAINS)?;
        chains.remove(author.as_slice())?;
        return Ok(());
    }
    let head = next_seq
        .checked_sub(1)
        .map(|last| stored_event(txn, author, last))
        .transpose()?
        .map(|event| event.id);
    store_chain(
        txn,
        author,
        Chain {
            next_seq,
            head,
            frozen,
        },
    )
}

/// The two events with the lowest IDs among `candidates`, in ID order. The candidates
/// may repeat an event.
fn two_lowest(mut candidates: Vec<Event>) -> Result<(Event, Event), StoreError> {
    candidates.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
    candidates.dedup_by(|a, b| a.id == b.id);
    let mut lowest = candidates.into_iter();
    match (lowest.next(), lowest.next()) {
        (Some(first), Some(second)) => Ok((first, second)),
        _ => Err(corrupt("a fork needs two distinct events")),
    }
}

fn author_of(event: &Event) -> AuthorKey {
    *event.header.verifying_key.as_bytes()
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
    let proof = read_proof(txn, author)?;
    Ok(proof.is_some_and(|proof| proof.first.id == *id || proof.second.id == *id))
}

fn read_proof(txn: &WriteTransaction, author: &AuthorKey) -> Result<Option<ForkProof>, StoreError> {
    let forks = txn.open_table(FORKS)?;
    let row = forks.get(author.as_slice())?;
    row.map(|row| decode_proof(row.value())).transpose()
}

fn read_chain(txn: &WriteTransaction, author: &AuthorKey) -> Result<Chain, StoreError> {
    let chains = txn.open_table(CHAINS)?;
    let row = chains.get(author.as_slice())?;
    match row {
        Some(row) => Chain::decode(row.value()),
        None => Ok(Chain::default()),
    }
}

/// The stored event at `author`'s `seq`, applied or voided, if there is one.
fn stored_at(
    txn: &WriteTransaction,
    author: &AuthorKey,
    seq: SeqNum,
) -> Result<Option<Event>, StoreError> {
    let events = txn.open_table(EVENTS)?;
    let row = events.get(position_key(author, seq).as_slice())?;
    row.map(|row| decode_stored(row.value())).transpose()
}

fn stored_event(
    txn: &WriteTransaction,
    author: &AuthorKey,
    seq: SeqNum,
) -> Result<Event, StoreError> {
    stored_at(txn, author, seq)?
        .ok_or_else(|| corrupt("an applied sequence number has no stored event"))
}

/// The positions of the stored events that name `id` in their `after` hashes.
fn dependents_of(txn: &WriteTransaction, id: &Hash) -> Result<Vec<Vec<u8>>, StoreError> {
    let dependents = txn.open_multimap_table(DEPENDENTS)?;
    let mut positions = Vec::new();
    for position in dependents.get(id.as_bytes().as_slice())? {
        positions.push(position?.value().to_vec());
    }
    Ok(positions)
}

/// The `after` hashes that are not applied yet.
fn unapplied_deps(txn: &WriteTransaction, after: &[Hash]) -> Result<Vec<Hash>, StoreError> {
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
    fn test_a_backlink_that_is_not_the_applied_head_is_parked_and_not_stored() {
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
        assert_eq!(ingest(&mut store, &stray), Ingest::Waiting { id: stray.id });
        assert!(
            store.event(&who, 1).unwrap().is_none(),
            "a parked event is not stored"
        );
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

    /// The IDs of `who`'s applied events, in sequence order.
    fn applied_ids(store: &EventStore, who: &AuthorKey) -> Vec<Hash> {
        let next = store.chain(who).unwrap().map_or(0, |chain| chain.next_seq);
        (0..next)
            .map(|seq| {
                store
                    .event(who, seq)
                    .unwrap()
                    .expect("an applied event is stored")
                    .id
            })
            .collect()
    }

    /// What a store reports for `who`: the chain row and the fork proof.
    fn state_of(store: &EventStore, who: &AuthorKey) -> (Option<Chain>, Option<ForkProof>) {
        (store.chain(who).unwrap(), store.fork_proof(who).unwrap())
    }

    #[test]
    fn test_a_fork_at_sequence_zero_unapplies_an_event_applied_on_top_of_it() {
        let (_dir, mut store) = open();
        let (x, y) = (key(20), key(21));
        let x0 = event(&x, 0, None, Vec::new(), "message");
        let y0 = event(&y, 0, None, vec![x0.id], "message");
        let rival = event(&x, 0, None, Vec::new(), "rival");
        ingest_all(&mut store, &[x0, y0.clone()]);
        assert_eq!(applied_ids(&store, &author(&y)), vec![y0.id]);
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: author(&x),
                seq: 0
            }
        );
        assert!(
            applied_ids(&store, &author(&y)).is_empty(),
            "y0 rests on x0, which the fork voided"
        );
        assert_eq!(
            store.event(&author(&y), 0).unwrap(),
            Some(y0),
            "y0 stays stored"
        );
        assert_eq!(store.chain(&author(&y)).unwrap(), None);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_fork_unapplies_every_event_that_rests_on_the_voided_ones() {
        let (_dir, mut store) = open();
        let (x, y, z) = (key(22), key(23), key(24));
        let x0 = event(&x, 0, None, Vec::new(), "message");
        let x1 = event(&x, 1, Some(x0.id), Vec::new(), "message");
        let y0 = event(&y, 0, None, vec![x1.id], "message");
        let y1 = event(&y, 1, Some(y0.id), Vec::new(), "message");
        let z0 = event(&z, 0, None, vec![y0.id], "message");
        ingest_all(&mut store, &[x0.clone(), x1, y0, y1, z0]);
        let rival = event(&x, 1, Some(x0.id), Vec::new(), "rival");
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: author(&x),
                seq: 1
            }
        );
        assert_eq!(applied_ids(&store, &author(&x)), vec![x0.id]);
        assert!(
            applied_ids(&store, &author(&y)).is_empty(),
            "y0 rests on x1, and y1 on y0"
        );
        assert!(
            applied_ids(&store, &author(&z)).is_empty(),
            "z0 rests on y0"
        );
        assert_eq!(store.chain(&author(&z)).unwrap(), None);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_conflict_with_an_unmatched_backlink_is_detected_in_either_order() {
        let signer = key(25);
        let who = author(&signer);
        let x0 = event(&signer, 0, None, Vec::new(), "message");
        let x1 = event(&signer, 1, Some(x0.id), Vec::new(), "message");
        let stray = event(
            &signer,
            1,
            Some(Hash::digest(b"not the head")),
            Vec::new(),
            "stray",
        );

        let (_first_dir, mut first) = open();
        ingest_all(&mut first, &[x0.clone(), x1.clone()]);
        assert_eq!(
            ingest(&mut first, &stray),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );

        let (_second_dir, mut second) = open();
        ingest_all(&mut second, std::slice::from_ref(&x0));
        ingest(&mut second, &stray);
        assert_eq!(
            ingest(&mut second, &x1),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );

        assert_eq!(state_of(&second, &who), state_of(&first, &who));
        first.check_invariants().unwrap();
        second.check_invariants().unwrap();
    }

    #[test]
    fn test_a_conflict_held_on_a_missing_dependency_is_detected_in_either_order() {
        let signer = key(26);
        let who = author(&signer);
        let x0 = event(&signer, 0, None, Vec::new(), "message");
        let missing = Hash::digest(b"an event that is never delivered");
        let held = event(&signer, 1, Some(x0.id), vec![missing], "message");
        let rival = event(&signer, 1, Some(x0.id), Vec::new(), "rival");

        let (_first_dir, mut first) = open();
        ingest_all(&mut first, std::slice::from_ref(&x0));
        assert_eq!(ingest(&mut first, &held), Ingest::Waiting { id: held.id });
        assert_eq!(
            ingest(&mut first, &rival),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );

        let (_second_dir, mut second) = open();
        ingest_all(&mut second, &[x0.clone(), rival.clone()]);
        assert_eq!(
            ingest(&mut second, &held),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );

        assert_eq!(state_of(&second, &who), state_of(&first, &who));
        first.check_invariants().unwrap();
        second.check_invariants().unwrap();
    }

    #[test]
    fn test_two_events_ahead_of_the_chain_at_one_position_are_a_fork_in_either_order() {
        let signer = key(27);
        let who = author(&signer);
        let x0 = event(&signer, 0, None, Vec::new(), "message");
        let x1 = event(&signer, 1, Some(x0.id), Vec::new(), "message");
        let x2 = event(&signer, 2, Some(x1.id), Vec::new(), "message");
        let stray = event(
            &signer,
            2,
            Some(Hash::digest(b"never delivered")),
            Vec::new(),
            "stray",
        );

        let (_first_dir, mut first) = open();
        assert_eq!(ingest(&mut first, &x2), Ingest::Waiting { id: x2.id });
        assert_eq!(
            ingest(&mut first, &stray),
            Ingest::Fork {
                author: who,
                seq: 2
            }
        );
        ingest_all(&mut first, &[x0.clone(), x1.clone()]);

        let (_second_dir, mut second) = open();
        ingest_all(&mut second, &[x0.clone(), x1.clone(), x2.clone()]);
        assert_eq!(
            ingest(&mut second, &stray),
            Ingest::Fork {
                author: who,
                seq: 2
            }
        );

        assert_eq!(state_of(&second, &who), state_of(&first, &who));
        assert_eq!(applied_ids(&first, &who), vec![x0.id, x1.id]);
        first.check_invariants().unwrap();
        second.check_invariants().unwrap();
    }

    #[test]
    fn test_an_event_ahead_of_its_chain_whose_backlink_is_applied_elsewhere_is_held() {
        let (signer, other) = (key(28), key(29));
        let other0 = event(&other, 0, None, Vec::new(), "message");
        let stray = event(&signer, 2, Some(other0.id), Vec::new(), "message");
        let (other0_bytes, stray_bytes) = (other0.bytes.clone(), stray.bytes.clone());
        let who = author(&signer);
        let (sent, received) = std::sync::mpsc::channel();
        // The ingest runs on its own thread, so a store that spins fails the test
        // instead of hanging it.
        std::thread::spawn(move || {
            let (_dir, mut store) = open();
            let first = store.ingest(&other0_bytes, 0).unwrap();
            let second = store.ingest(&stray_bytes, 0).unwrap();
            let stored = store.event(&who, 2).unwrap().is_some();
            let _ = sent.send((first, second, stored));
        });
        let (first, second, stored) = received
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("ingest did not return: the event made the store spin");
        assert_eq!(
            first,
            Ingest::Accepted {
                id: other0.id,
                released: vec![],
            }
        );
        assert_eq!(second, Ingest::Waiting { id: stray.id });
        assert!(!stored, "a held event is not stored");
    }

    #[test]
    fn test_the_fork_proof_holds_the_two_lowest_ids_whatever_the_order() {
        let signer = key(30);
        let who = author(&signer);
        let x0 = event(&signer, 0, None, Vec::new(), "message");
        let x1 = event(&signer, 1, Some(x0.id), Vec::new(), "message");
        // Two rivals below the original's ID, so the proof must leave the original out.
        let below: Vec<Event> = (0..16)
            .map(|n| event(&signer, 1, Some(x0.id), Vec::new(), &format!("rival {n}")))
            .filter(|rival| rival.id.as_bytes() < x1.id.as_bytes())
            .collect();
        assert!(
            below.len() >= 2,
            "the fixture needs two rivals below the original"
        );
        let (rival_a, rival_b) = (below[0].clone(), below[1].clone());
        let lowest_two = sorted(&rival_a.id, &rival_b.id);

        let orders = [
            vec![x0.clone(), x1.clone(), rival_a.clone(), rival_b.clone()],
            vec![x0.clone(), rival_a.clone(), rival_b.clone(), x1.clone()],
            vec![x0, rival_b, x1, rival_a],
        ];
        let mut states = Vec::new();
        for order in &orders {
            let (_dir, mut store) = open();
            for event in order {
                store.ingest(&event.bytes, 0).unwrap();
            }
            let proof = store
                .fork_proof(&who)
                .unwrap()
                .expect("a fork proof is stored");
            assert_eq!(
                [proof.first.id, proof.second.id],
                lowest_two,
                "the proof holds the two lowest IDs at the fork point"
            );
            store.check_invariants().unwrap();
            states.push(state_of(&store, &who));
        }
        assert!(
            states.windows(2).all(|pair| pair[0] == pair[1]),
            "the orders reach different states: {states:?}"
        );
    }

    #[test]
    fn test_a_waiting_event_is_held_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(31);
        let events = chain_of(&signer, 2);
        {
            let mut store = EventStore::open(dir.path()).unwrap();
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
        }
        let mut store = EventStore::open(dir.path()).unwrap();
        assert_eq!(
            store.waiting_len(),
            1,
            "the waiting event is still held after the restart"
        );
        assert_eq!(
            ingest(&mut store, &events[0]),
            Ingest::Accepted {
                id: events[0].id,
                released: vec![events[1].id],
            }
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_fork_against_a_waiting_event_is_seen_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(32);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        {
            let mut store = EventStore::open(dir.path()).unwrap();
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
        }
        let mut store = EventStore::open(dir.path()).unwrap();
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_an_expired_waiting_event_is_not_a_rival() {
        let (_dir, store) = open();
        let mut store = store.with_waiting_limits(WaitingLimits::new(256, 1_000));
        let signer = key(33);
        let events = chain_of(&signer, 2);
        assert_eq!(
            store.ingest(&events[1].bytes, 0).unwrap(),
            Ingest::Waiting { id: events[1].id }
        );
        // The waiting event is due at 1_000, so it has expired before the rival arrives.
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        assert_eq!(
            store.ingest(&rival.bytes, 1_000).unwrap(),
            Ingest::Waiting { id: rival.id }
        );
        assert_eq!(store.waiting_len(), 1, "only the rival is held");
    }
}
