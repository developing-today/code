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
//!    held, the two are a fork, whatever their backlinks and dependencies. The proof for that
//!    sequence number holds the two lowest IDs there, and the proofs at higher points stay. The
//!    applied events from the lowest point on are voided, and no others. The proof headers
//!    become present, which releases the events that waited for them.
//! 5. **Accept.** The next sequence number, whose backlink is the applied head, is applied
//!    once every `after` hash is present.
//! 6. **Wait or park.** An event ahead of its chain, or with an `after` hash that is not
//!    applied yet, is held in the waiting pool and its row, under the caps, and decided again
//!    when its dependencies are present. An event whose backlink can never be the applied head
//!    is parked: held for good, so that a conflict with it is still seen.
//!
//! Each call runs in one redb write transaction. It holds every decision the call causes: the
//! event, each event it releases, the expiry of due events, and the waiting rows. The call
//! commits all of them or none. A refused event writes nothing. A duplicate writes nothing, except
//! that a stored event first received without its body takes the action of the body that arrives.
//!
//! # Applied state and forks
//!
//! The applied state depends on the set of events a store has received, not on their
//! arrival order. An event is applied when it sits below its author's fork point, its
//! backlink is the applied event before it, and every `after` hash is present. The fork
//! point is the lowest sequence number where the author has two distinct events, and the
//! proof holds the two lowest IDs there.
//!
//! An event that is stored but not applied is voided. A fork voids the author's events
//! from its fork point on, and nothing else. A voided event is kept, because it still counts
//! as the author's event at its sequence number, so a later conflict with it is still a fork.
//! The applied prefix of a chain is the events below its next sequence number, so the chain
//! row is what marks the voids.
//!
//! # Fork proofs are evidence
//!
//! A fork proof is kept. An author has one proof at each sequence number where it has forked,
//! and these rules hold for all of them:
//!
//! - A proof header is present and held for good. It is never applied, and an `after` hash that
//!   names it is satisfied, so an event that names a header is valid.
//! - A fork at a lower sequence number adds its own proof and does not remove the proofs above
//!   it. The lowest point is the fork point: it freezes the chain and voids the author's events
//!   from there on.
//! - A proof is replaced only by a conflict at its own sequence number. The replacement keeps
//!   the two lowest IDs there, and a header it drops stays present and held. It is still a node of
//!   the resolution, so an event that names it stays valid.
//!
//! An `after` hash is present when the store holds the event it names, applied or voided, or
//! when it is one of a fork's proof headers. A held event waits for each `after` hash that is
//! not present, so an event of another author that names a voided event still applies. This is
//! the Phase 1 rule. It has no membership or authority model to void by, so authority-based
//! voiding (decision 1) replaces it in Phase 2 (P2.3).
//!
//! Held events count too. Two held events at one sequence number are a fork, though
//! neither is applied. Held events are rows on disk, so a restart keeps them, and a conflict
//! with an event held before the restart is still seen. An event the caps or the age limit
//! drops is forgotten, and sync sends it again. A held event that a fork makes a proof header
//! stays held for good: it never applies, and the proof keeps its conflicts seen.
//!
//! # Known limit: a rival refused by a cap
//!
//! A conflict is found only against an event the store holds. A rival refused by its author's
//! cap is not held, so when the other event arrives, no conflict is seen. The fork is seen when
//! the refused rival is sent again, while the other event is still held. The behaviour is
//! pinned by `test_a_rival_refused_by_the_cap_is_seen_only_when_sent_again`.
//!
//! # Known limit: a rival refused as frozen
//!
//! An event above its author's fork point is refused as frozen when it arrives after the fork.
//! When it arrives before, it is kept as voided. So an `after` hash that names such an event is
//! present in some arrival orders and not in others. The batch property names rivals, but it
//! compares the batch path with per-event ingest in one arrival order, so it holds either way.
//!
//! The property tests in `tests/event_store_properties.rs` check the applied state against
//! this definition, for every arrival order.

use std::collections::{BTreeMap, HashMap, VecDeque};

use p2panda_auth::group::GroupAction;
use p2panda_core::{Hash, SeqNum};
use redb::{ReadableTable, ReadableTableMetadata, WriteTransaction};

use super::resolve::{self, decode_action, encode_action};
use super::waiting_rows::{Hold, Mirror};
use super::{
    ACTIONS, APPLIED, CHAINS, Chain, EVENTS, EventStore, FORKS, ForkProof, HASHES, PROOF_HEADERS,
    PROOF_IDS, StoreError, corrupt, decode_stored, lowest_proof, position_key, store_chain,
    store_event, store_fork,
};
use crate::chain::{self, Event};
use crate::documents::{is_document_kind, is_removal_kind};
use crate::membership::AuthId;
use crate::membership::body::decode_body;
use crate::rejection::Reason;
use crate::tag_claims::is_tag_kind;
use crate::waiting::{AuthorKey, WaitingPool};

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

impl Refusal {
    /// The reason this refusal gives. A malformed event has none: its bytes do not verify, so it is
    /// not an event the store judged.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Malformed(_) => None,
            Self::Frozen => Some(Reason::Frozen),
            Self::WaitingFull => Some(Reason::Waiting),
        }
    }
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
    /// fork point that changes the proof is reported the same way. The headers are present
    /// now, so events of other authors that waited for them may apply. Those are not listed
    /// here. Read them from the store.
    Fork {
        /// The author.
        author: AuthorKey,
        /// The fork point: the lowest sequence number where the author conflicts.
        seq: SeqNum,
    },
    /// The event is held, and not stored. It waits for a dependency, or it is parked because its
    /// backlink can never be the applied head. The hold is on disk, so a restart keeps it. Sync
    /// sends it again if the caps or the age limit drop it.
    Waiting {
        /// The event's ID.
        id: Hash,
    },
    /// The event was refused. Nothing changed.
    Refused(Refusal),
    /// The event is stored but does not apply. It is a group operation the membership layer
    /// voids, or content whose author lost authority at a voided event it names. It still counts
    /// as present, so events that name it and events after it in its author's chain are not held.
    /// `released` lists the waiting events that became applied because of it.
    Voided {
        /// The event's ID.
        id: Hash,
        /// The IDs of the waiting events it released that applied, in application order.
        released: Vec<Hash>,
    },
}

impl Ingest {
    /// The reason this result gives, when it is a refusal or a hold. A stored, applied, or duplicate
    /// event, and a fork that proves a conflict, give none.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Refused(refusal) => refusal.reason(),
            Self::Waiting { .. } => Some(Reason::Waiting),
            Self::Voided { .. } => Some(Reason::Voided),
            Self::Accepted { .. } | Self::Duplicate { .. } | Self::Fork { .. } => None,
        }
    }
}

/// What the store decides for one event, before the waiting pool sees it.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Stored and applied.
    Accept,
    /// Already stored, or a proof header. Nothing written.
    Duplicate,
    /// A conflict. The proof is written, and the fork point is `seq`. Its two headers are
    /// present now, so the events that waited for them are released.
    Fork { seq: SeqNum, proof: [Hash; 2] },
    /// Waits for these dependencies, none of which is applied. The caller holds the event.
    Wait(Vec<Hash>),
    /// Can never apply, but is held so that a conflict with it is seen. The caller holds it.
    Park,
    /// Refused. Nothing written.
    Refuse(Refusal),
}

impl EventStore {
    /// Verifies one received event and applies what it allows.
    ///
    /// `now_ms` is the clock. The events due at it expire first, so the caps and the rivals
    /// see only the events still held. It is a parameter so tests can run on a fake clock.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error. A refusal is not an error: it comes back as
    /// [`Ingest::Refused`]. After an error the waiting events are rebuilt from disk. If that
    /// fails too, the store refuses further calls until it is reopened.
    pub fn ingest(&mut self, bytes: &[u8], now_ms: u64) -> Result<Ingest, StoreError> {
        self.ingest_with_action(bytes, None, now_ms)
    }

    /// Verifies one received event with its body, and applies what it allows. This is the
    /// production path. The membership action comes from the body, by the body codec in
    /// [`crate::membership::body`]:
    ///
    /// - A body that is not a group operation, or no body at all, makes the event content.
    /// - A body that claims to be a group operation decodes to its action, or it is refused.
    /// - A body that is not the one the header commits to, by BLAKE3 hash and size, is refused.
    /// - A tag event (kind `tag.set` or `tag.clear`) takes no body. Its operation is in its
    ///   header (#182), so a payload on a tag event is refused. Its tag takes no membership
    ///   action, and the fold reads it from the stored header (see `claims`).
    /// - A document event (kind `document.put`) takes no body either. Its content is a blob that
    ///   the header names by hash, so a payload beside it is refused (P5.4).
    /// - A removal event (kind `document.remove`) takes no body. It names its document in its
    ///   header, so a payload beside it is refused (P5.6).
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error, as [`EventStore::ingest`] does.
    pub fn ingest_with_body(
        &mut self,
        bytes: &[u8],
        body: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError> {
        let event = match chain::decode(bytes) {
            Ok(event) => event,
            Err(err) => return Ok(Ingest::Refused(Refusal::Malformed(err.to_string()))),
        };
        let kind = &event.header.extensions.kind;
        let action = if is_tag_kind(kind) || is_document_kind(kind) || is_removal_kind(kind) {
            if body.is_some() {
                return Ok(Ingest::Refused(Refusal::Malformed(
                    "a tag, document or removal event carries its content in its header, not in a body"
                        .to_owned(),
                )));
            }
            None
        } else {
            match body_action(&event, body) {
                Ok(action) => action,
                Err(refusal) => return Ok(Ingest::Refused(refusal)),
            }
        };
        self.ingest_event(event, action.as_ref(), now_ms)
    }

    /// Verifies one received event, which may be a group operation, and applies what it allows.
    /// `action` is the membership change of a group operation, handed in by the caller. The store
    /// does not read bodies here. Other events take no action. The production path is
    /// [`EventStore::ingest_with_body`], and the scenario tests use this one.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error, as [`EventStore::ingest`] does.
    pub fn ingest_with_action(
        &mut self,
        bytes: &[u8],
        action: Option<&GroupAction<AuthId, ()>>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError> {
        let event = match chain::decode(bytes) {
            Ok(event) => event,
            Err(err) => return Ok(Ingest::Refused(Refusal::Malformed(err.to_string()))),
        };
        self.ingest_event(event, action, now_ms)
    }

    /// Runs a decoded event through the store's transaction, with its action.
    fn ingest_event(
        &mut self,
        event: Event,
        action: Option<&GroupAction<AuthId, ()>>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError> {
        self.transact(now_ms, |mirror| {
            let (staged, changed) = ingest_in(mirror, event, action, now_ms, Labels::AsStored)?;
            Ok((staged.finish(mirror.txn)?, changed))
        })
    }

    /// Decides already verified events, in order, in one write transaction. The outcomes are those
    /// that [`EventStore::ingest`] gives for each event in turn, and the stored state is the same.
    ///
    /// The batch resolves once, after its last event, unless a conflict is possible in it (see
    /// [`batch_may_fork`]). A fork is the one case where an event's label depends on the state
    /// after the events before it, so a batch that may fork resolves after each store.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error. Then nothing in the batch is committed.
    pub(crate) fn ingest_events(
        &mut self,
        events: Vec<Event>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {
        self.transact(now_ms, |mirror| {
            let labels = if batch_may_fork(mirror, &events)? {
                Labels::AsStored
            } else {
                Labels::AtEnd
            };
            // Batches come from sync, which carries no bodies. A group operation's action is
            // attached later, when its body arrives (`attach_action`).
            let mut staged = Vec::with_capacity(events.len());
            let mut changed = false;
            for event in events {
                let (outcome, did_change) = ingest_in(mirror, event, None, now_ms, labels)?;
                changed |= did_change;
                staged.push(outcome);
            }
            if staged
                .iter()
                .any(|item| matches!(item, Staged::Stored { .. }))
            {
                refresh(mirror.txn)?;
            }
            let outcomes = staged
                .into_iter()
                .map(|item| item.finish(mirror.txn))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((outcomes, changed))
        })
    }

    /// Decides a batch of received bytes, in order, in one write transaction. Bytes that do not
    /// verify are refused in their place and change nothing, as [`EventStore::ingest`] refuses
    /// them. The outcomes are those of calling [`EventStore::ingest`] on each entry in turn.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error. Then nothing in the batch is committed.
    pub fn ingest_batch(
        &mut self,
        batch: &[Vec<u8>],
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {
        let mut decoded = Vec::with_capacity(batch.len());
        // `Some` is a refusal for an entry that does not decode. `None` takes the next outcome of
        // the decoded events.
        let mut refusals: Vec<Option<Ingest>> = Vec::with_capacity(batch.len());
        for bytes in batch {
            match chain::decode(bytes) {
                Ok(event) => {
                    refusals.push(None);
                    decoded.push(event);
                }
                Err(err) => {
                    let refused = Ingest::Refused(Refusal::Malformed(err.to_string()));
                    refusals.push(Some(refused));
                }
            }
        }
        let mut stored = self.ingest_events(decoded, now_ms)?.into_iter();
        let mut outcomes = Vec::with_capacity(batch.len());
        for refusal in refusals {
            outcomes.push(match refusal {
                None => stored
                    .next()
                    .ok_or_else(|| corrupt("a batch lost an outcome"))?,
                Some(refused) => refused,
            });
        }
        Ok(outcomes)
    }

    /// Drops the waiting events due at `now_ms`, with their rows. Returns how many were dropped.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the write fails. The waiting events are then rebuilt from disk.
    pub fn expire_waiting(&mut self, now_ms: u64) -> Result<usize, StoreError> {
        self.transact(now_ms, |mirror| {
            let expired = mirror.expire(now_ms)?;
            Ok((expired, expired > 0))
        })
    }

    /// The number of events held, waiting or parked, across all authors.
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    /// The number of events `author` has held.
    pub fn waiting_count(&self, author: &AuthorKey) -> usize {
        self.waiting.waiting_count(*author)
    }

    /// Whether the event with this ID is held, waiting or parked.
    pub fn is_waiting(&self, id: &Hash) -> bool {
        self.waiting.contains(id)
    }
}

/// The membership action an event's body carries. `Ok(None)` makes the event content: it has no
/// body, or its body is not a group operation. A body that is not the one the header commits to is
/// refused, and so is a body that claims to be a group operation and does not decode.
fn body_action(
    event: &Event,
    body: Option<&[u8]>,
) -> Result<Option<GroupAction<AuthId, ()>>, Refusal> {
    let Some(body) = body else {
        return Ok(None);
    };
    committed(event, body)?;
    decode_body(body).map_err(|err| Refusal::Malformed(err.to_string()))
}

/// Refuses a body that is not the one the header commits to, by BLAKE3 hash and size.
fn committed(event: &Event, body: &[u8]) -> Result<(), Refusal> {
    let committed = event.header.payload_hash == Some(Hash::digest(body))
        && u32::try_from(body.len()).is_ok_and(|len| len == event.header.payload_size);
    if committed {
        Ok(())
    } else {
        Err(Refusal::Malformed(
            "the body is not the one the header commits to, by hash and size".to_owned(),
        ))
    }
}

/// How a call labels the events it stores: applied, or voided. The label is the resolution's answer
/// for the store at that point, so it can change as later events arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Labels {
    /// Resolve after each store, and read the label then. A single event is labelled this way.
    AsStored,
    /// Read the labels after the batch's one resolution. Only for a batch that cannot fork: a
    /// plain event stored later does not change the label of an event stored before it.
    AtEnd,
}

/// An event's outcome before its label is read. A call in [`Labels::AsStored`] has the label
/// already, so its outcome is ready. A stored event in [`Labels::AtEnd`] waits for the batch's
/// resolution. `released` are the events the store released with it, which are candidates for
/// `released` and are filtered by their labels at the end.
enum Staged {
    Ready(Ingest),
    Stored { id: Hash, released: Vec<Hash> },
}

impl Staged {
    /// Whether the event is kept: stored, or held. A refused event and a fork are not.
    fn keeps(&self) -> bool {
        matches!(
            self,
            Self::Stored { .. }
                | Self::Ready(
                    Ingest::Accepted { .. }
                        | Ingest::Voided { .. }
                        | Ingest::Duplicate { .. }
                        | Ingest::Waiting { .. },
                )
        )
    }

    /// The outcome, with its labels read from the store as it is now.
    fn finish(self, txn: &WriteTransaction) -> Result<Ingest, StoreError> {
        match self {
            Self::Ready(outcome) => Ok(outcome),
            Self::Stored { id, released } => {
                let mut applied_released = Vec::with_capacity(released.len());
                for hash in released {
                    if is_applied(txn, &hash)? {
                        applied_released.push(hash);
                    }
                }
                if is_applied(txn, &id)? {
                    Ok(Ingest::Accepted {
                        id,
                        released: applied_released,
                    })
                } else {
                    Ok(Ingest::Voided {
                        id,
                        released: applied_released,
                    })
                }
            }
        }
    }
}

/// Whether a batch can fork, so that one of its events could change the label of an event stored
/// before it. A fork needs two events at one position: a stored or held event and an arrival, two
/// arrivals, or an author who already has a fork proof. The check is conservative. It can call a
/// batch that does not fork a forking one, and never the other way round.
fn batch_may_fork(m: &Mirror<'_>, events: &[Event]) -> Result<bool, StoreError> {
    let mut positions: HashMap<(AuthorKey, SeqNum), Hash> = HashMap::new();
    for event in events {
        let author = author_of(event);
        let seq = event.header.seq_num;
        if read_proof(m.txn, &author)?.is_some() {
            return Ok(true);
        }
        if positions
            .insert((author, seq), event.id)
            .is_some_and(|prev| prev != event.id)
        {
            return Ok(true);
        }
        if stored_at(m.txn, &author, seq)?.is_some_and(|stored| stored.id != event.id) {
            return Ok(true);
        }
        if m.pool
            .held_by(&author)
            .any(|held| held.header.seq_num == seq && held.id != event.id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Decides a received event inside the call's transaction, and the events it releases. Returns
/// the outcome, and whether the call changed anything. A call that expired events has changed
/// something, even if its own event was refused.
fn ingest_in(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    now: u64,
    labels: Labels,
) -> Result<(Staged, bool), StoreError> {
    let expired = m.expire(now)?;
    let id = event.id;
    let author = author_of(&event);
    let rivals = held_rivals(m.pool, &event);
    let settled = settle(m, event, action, &rivals, now, false, labels)?;
    let attached = matches!(settled, Settled::Duplicate { attached: true });
    let changed = expired > 0
        || attached
        || !matches!(settled, Settled::Duplicate { .. } | Settled::Refused(_));
    let staged = match settled {
        Settled::Stored { applied, released } => {
            let released = drain(m, released.into(), now, labels)?;
            match applied {
                Some(true) => Staged::Ready(Ingest::Accepted { id, released }),
                Some(false) => Staged::Ready(Ingest::Voided { id, released }),
                None => Staged::Stored { id, released },
            }
        }
        Settled::Duplicate { .. } => Staged::Ready(Ingest::Duplicate { id }),
        Settled::Fork { seq, released } => {
            drain(m, released.into(), now, labels)?;
            Staged::Ready(Ingest::Fork { author, seq })
        }
        Settled::Held | Settled::Skipped => {
            drain(m, VecDeque::new(), now, labels)?;
            Staged::Ready(Ingest::Waiting { id })
        }
        Settled::Refused(refusal) => Staged::Ready(Ingest::Refused(refusal)),
    };
    Ok((staged, changed))
}

/// What one decision did, once the pool and the rows agree with it.
enum Settled {
    /// Stored. `applied` is the label when the call reads it now, and `None` when the batch reads
    /// it at its end. `released` are the events that waited only for it, each due a decision.
    Stored {
        applied: Option<bool>,
        released: Vec<Event>,
    },
    /// Already stored, or a fork proof header. Nothing changed, unless `attached`: a stored event
    /// that was first stored without its body takes the action of the body that arrived now.
    Duplicate { attached: bool },
    /// A conflict at `seq`. The proof headers are present now, and `released` are the events
    /// that waited only for them, each due a decision.
    Fork { seq: SeqNum, released: Vec<Event> },
    /// Held, waiting or parked.
    Held,
    /// The pool did not keep the event. See [`Hold::Skipped`].
    Skipped,
    /// Refused. Nothing is held.
    Refused(Refusal),
}

/// Decides `event` inside the transaction, and brings the pool and the rows into line with the
/// decision. `from_pool` is true for an event the pool released: it is no longer held, so a final
/// decision removes its row. Otherwise the event is new, or it is held and still is, so a
/// decision that leaves it held keeps its row.
fn settle(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    rivals: &[Event],
    now: u64,
    from_pool: bool,
    labels: Labels,
) -> Result<Settled, StoreError> {
    let id = event.id;
    let settled = match decide_in(m.txn, &event, rivals)? {
        Decision::Accept => {
            if let Some(action) = action {
                store_action(m.txn, &id, action)?;
            }
            // In a store with no group state every stored event applies, so the label is known
            // now, whatever the batch does later. Otherwise the label waits for the resolution.
            let applied = match labels {
                Labels::AsStored => {
                    if plain(m.txn)? {
                        apply_plain(m.txn, &event)?;
                    } else {
                        refresh(m.txn)?;
                    }
                    Some(is_applied(m.txn, &id)?)
                }
                Labels::AtEnd if plain(m.txn)? => {
                    apply_plain(m.txn, &event)?;
                    Some(true)
                }
                Labels::AtEnd => None,
            };
            Settled::Stored {
                applied,
                released: m.release(&id, now)?,
            }
        }
        Decision::Duplicate => {
            drop_released(m, &event, from_pool)?;
            Settled::Duplicate {
                attached: attach_action(m.txn, &id, action)?,
            }
        }
        Decision::Fork { seq, proof } => {
            if labels == Labels::AtEnd {
                // A batch that may not fork cannot reach a fork. Its labels were not read as they
                // stored, so the batch is refused rather than labelled wrongly.
                return Err(corrupt("a batch that cannot fork found a fork"));
            }
            drop_released(m, &event, from_pool)?;
            refresh(m.txn)?;
            let mut released = Vec::new();
            for header in proof {
                released.extend(m.present_proof(&header, now)?);
            }
            Settled::Fork { seq, released }
        }
        Decision::Refuse(refusal) => {
            drop_released(m, &event, from_pool)?;
            Settled::Refused(refusal)
        }
        Decision::Wait(deps) => {
            if let Some(action) = action {
                store_action(m.txn, &id, action)?;
            }
            held(m.hold(event, deps, false, now)?)
        }
        Decision::Park => {
            if let Some(action) = action {
                store_action(m.txn, &id, action)?;
            }
            held(m.hold(event, Vec::new(), true, now)?)
        }
    };
    Ok(settled)
}

/// The action of a held or stored group operation, from the actions table. A held event keeps
/// its action there from the moment it arrives, so a release does not need the caller again.
fn stored_action(
    txn: &WriteTransaction,
    id: &Hash,
) -> Result<Option<GroupAction<AuthId, ()>>, StoreError> {
    let actions = txn.open_table(ACTIONS)?;
    let row = actions.get(id.as_bytes().as_slice())?;
    row.map(|row| decode_action(row.value())).transpose()
}

/// Settles a released event that the decision does not apply. A fork proof header never applies,
/// so it is held for good again. Any other event is refused, and its row goes.
fn drop_released(m: &mut Mirror<'_>, event: &Event, from_pool: bool) -> Result<(), StoreError> {
    if !from_pool {
        return Ok(());
    }
    if is_proof_header(m.txn, &author_of(event), &event.id)? {
        m.park_proof(&event.id)
    } else {
        m.drop_row(&event.id)
    }
}

const fn held(hold: Hold) -> Settled {
    match hold {
        Hold::Held => Settled::Held,
        Hold::Full => Settled::Refused(Refusal::WaitingFull),
        Hold::Skipped => Settled::Skipped,
    }
}

/// Decides the events in `queue`, and each event they release, until none is left. Returns the
/// IDs of the events applied, in order.
fn drain(
    m: &mut Mirror<'_>,
    mut queue: VecDeque<Event>,
    now: u64,
    labels: Labels,
) -> Result<Vec<Hash>, StoreError> {
    let mut stored = Vec::new();
    queue.extend(m.pool.take_ready());
    while let Some(event) = queue.pop_front() {
        let id = event.id;
        let rivals = held_rivals(m.pool, &event);
        let action = stored_action(m.txn, &id)?;
        match settle(m, event, action.as_ref(), &rivals, now, true, labels)? {
            Settled::Stored { applied, released } => {
                // A label that is not read yet keeps the event as a candidate. Its label decides
                // whether it stays in the list, once the batch has resolved.
                if applied != Some(false) {
                    stored.push(id);
                }
                queue.extend(released);
            }
            Settled::Fork { released, .. } => queue.extend(released),
            Settled::Duplicate { .. } | Settled::Held | Settled::Skipped | Settled::Refused(_) => {}
        }
        queue.extend(m.pool.take_ready());
    }
    Ok(stored)
}

/// The held events that conflict with `event`: the author's other events at its sequence number.
fn held_rivals(pool: &WaitingPool<Hash, Event>, event: &Event) -> Vec<Event> {
    let author = author_of(event);
    let seq = event.header.seq_num;
    pool.held_by(&author)
        .filter(|held| held.header.seq_num == seq && held.id != event.id)
        .cloned()
        .collect()
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
                proof: [first.id, second.id],
            });
        }
    }
    if !conflicts.is_empty() {
        // The first conflict at this sequence number, or one below the fork point.
        conflicts.push(event.clone());
        let (first, second) = two_lowest(conflicts)?;
        store_fork(txn, &author, seq, &first, &second)?;
        return Ok(Decision::Fork {
            seq,
            proof: [first.id, second.id],
        });
    }

    // The backlink must be the stored event before this one. Voided events are stored, so a
    // backlink does not need to be applied. A backlink that is present at another position, or
    // that is missing where there is no backlink, can never link, so the event is parked. A fork
    // proof header is present but never stored, so it is parked too: waiting for it would wait on
    // a dependency that is already present.
    let mut waits = missing_deps(txn, &event.header.extensions.after)?;
    if seq > 0 {
        let linked = match event.header.backlink {
            Some(backlink) => {
                stored_at(txn, &author, seq - 1)?.is_some_and(|before| before.id == backlink)
            }
            None => false,
        };
        if !linked {
            match event.header.backlink {
                Some(backlink) if !is_present(txn, &backlink)? => waits.push(backlink),
                _ => return Ok(Decision::Park),
            }
        }
    }
    if waits.is_empty() {
        accept(txn, event)
    } else {
        Ok(Decision::Wait(waits))
    }
}

/// Stores `event`. Whether it applies is decided by [`refresh`], which the caller runs next.
fn accept(txn: &WriteTransaction, event: &Event) -> Result<Decision, StoreError> {
    store_event(txn, event)?;
    Ok(Decision::Accept)
}

/// Writes the action of a group operation, so the resolution can read it again.
fn store_action(
    txn: &WriteTransaction,
    id: &Hash,
    action: &GroupAction<AuthId, ()>,
) -> Result<(), StoreError> {
    let encoded = encode_action(action)?;
    let mut actions = txn.open_table(ACTIONS)?;
    actions.insert(id.as_bytes().as_slice(), encoded.as_slice())?;
    Ok(())
}

/// Attaches the action of a group operation to a stored event that has none. Such an event was
/// first stored without its body, so it applied as content. The header commits to the body by hash
/// and size, so the body that arrives later is the one the author signed. A stored event that has
/// an action keeps it. Returns whether an action was attached, and then the resolution is run again.
fn attach_action(
    txn: &WriteTransaction,
    id: &Hash,
    action: Option<&GroupAction<AuthId, ()>>,
) -> Result<bool, StoreError> {
    let Some(action) = action else {
        return Ok(false);
    };
    if !is_stored(txn, id)? || stored_action(txn, id)?.is_some() {
        return Ok(false);
    }
    store_action(txn, id, action)?;
    refresh(txn)?;
    Ok(true)
}

/// Recomputes which stored events apply, and writes the result: the applied table, and each
/// chain row as the leading run of applied events from sequence zero. The result is a function of
/// the stored events and the fork proofs, so every arrival order writes the same rows.
/// Whether the store holds no group state: no membership action and no fork. Then every stored
/// event applies (`resolve` has no membership layer to void one), so an accepted event is the
/// only change, and [`refresh`] would only rewrite what is already true. Reloading and re-resolving
/// every stored event on each accept made a session quadratic in its length.
fn plain(txn: &WriteTransaction) -> Result<bool, StoreError> {
    let actions = txn.open_table(ACTIONS)?;
    let forks = txn.open_table(FORKS)?;
    Ok(actions.is_empty()? && forks.is_empty()?)
}

/// Applies an accepted event in a store with no group state. This is what [`refresh`] would
/// write: the event is applied, and its author's applied prefix now runs through it.
fn apply_plain(txn: &WriteTransaction, event: &Event) -> Result<(), StoreError> {
    let author = author_of(event);
    txn.open_table(APPLIED)?
        .insert(event.id.as_bytes().as_slice(), [].as_slice())?;
    set_prefix(txn, &author, event.header.seq_num + 1, false)
}

pub(super) fn refresh(txn: &WriteTransaction) -> Result<(), StoreError> {
    let (stored, forks, headers) = {
        let events = txn.open_table(EVENTS)?;
        let actions = txn.open_table(ACTIONS)?;
        let forks = txn.open_table(FORKS)?;
        let proof_headers = txn.open_table(PROOF_HEADERS)?;
        resolve::load(&events, &actions, &forks, &proof_headers)?
    };
    let resolved = resolve::resolve(&stored, &forks, &headers)?;

    let mut applied_table = txn.open_table(APPLIED)?;
    let mut previous = Vec::new();
    for entry in applied_table.iter()? {
        let (key, _) = entry?;
        previous.push(key.value().to_vec());
    }
    for key in previous {
        applied_table.remove(key.as_slice())?;
    }
    for id in &resolved.applied {
        applied_table.insert(id.as_bytes().as_slice(), [].as_slice())?;
    }
    drop(applied_table);

    let mut order: Vec<(AuthorKey, SeqNum, bool)> = stored
        .iter()
        .map(|item| {
            let event = &item.event;
            (
                author_of(event),
                event.header.seq_num,
                resolved.applied.contains(&event.id),
            )
        })
        .collect();
    order.sort_unstable();
    let mut next: BTreeMap<AuthorKey, SeqNum> = forks.keys().map(|author| (*author, 0)).collect();
    for (author, seq, applied) in order {
        let run = next.entry(author).or_insert(0);
        if applied && seq == *run {
            *run += 1;
        }
    }
    for (author, prefix) in next {
        set_prefix(txn, &author, prefix, forks.contains_key(&author))?;
    }
    Ok(())
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

/// Whether `id` is a fork proof header of `author`. A header stays one after a later proof drops
/// it, so the check reads the presence index, not the current proof.
fn is_proof_header(
    txn: &WriteTransaction,
    author: &AuthorKey,
    id: &Hash,
) -> Result<bool, StoreError> {
    let proof_ids = txn.open_table(PROOF_IDS)?;
    let indexed = proof_ids.get(id.as_bytes().as_slice())?;
    Ok(indexed.is_some_and(|row| row.value() == author.as_slice()))
}

/// The proof at `author`'s lowest fork point, which freezes the chain.
fn read_proof(txn: &WriteTransaction, author: &AuthorKey) -> Result<Option<ForkProof>, StoreError> {
    lowest_proof(&txn.open_table(FORKS)?, author)
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

/// The `after` hashes that are not present yet.
fn missing_deps(txn: &WriteTransaction, after: &[Hash]) -> Result<Vec<Hash>, StoreError> {
    let mut missing = Vec::new();
    for hash in after {
        if !is_present(txn, hash)? {
            missing.push(*hash);
        }
    }
    Ok(missing)
}

/// Whether the event with this ID applies, in the resolved state.
pub(super) fn is_applied(txn: &WriteTransaction, hash: &Hash) -> Result<bool, StoreError> {
    let applied = txn.open_table(APPLIED)?;
    let found = applied.get(hash.as_bytes().as_slice())?.is_some();
    Ok(found)
}

/// Whether an `after` hash is present: the store holds the event it names, applied or voided, or
/// the event is one of a fork's proof headers. Presence never goes away, so an event that waited
/// for a present hash is never made to wait again.
pub(super) fn is_present(txn: &WriteTransaction, hash: &Hash) -> Result<bool, StoreError> {
    present_in(&txn.open_table(HASHES)?, &txn.open_table(PROOF_IDS)?, hash)
}

/// [`is_present`] over the `hashes` and `proof_ids` tables, whether they are read or written.
pub(super) fn present_in(
    hashes: &impl ReadableTable<&'static [u8], &'static [u8]>,
    proof_ids: &impl ReadableTable<&'static [u8], &'static [u8]>,
    hash: &Hash,
) -> Result<bool, StoreError> {
    if hashes.get(hash.as_bytes().as_slice())?.is_some() {
        return Ok(true);
    }
    Ok(proof_ids.get(hash.as_bytes().as_slice())?.is_some())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::chain::{ChainExt, sign};
    use crate::event_store::waiting_rows::{self, Row};
    use crate::rejection::Reason;
    use crate::waiting::WaitingLimits;
    use p2panda_core::SigningKey;
    use redb::ReadableDatabase;

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
            tag: None,
            document: None,
            removes: None,
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
        open_with(WaitingLimits::default())
    }

    fn open_with(limits: WaitingLimits) -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), limits, 0).unwrap();
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

    /// The applied rows and the chain rows, as bytes.
    fn rows(store: &EventStore) -> (Vec<Vec<u8>>, Vec<(Vec<u8>, Vec<u8>)>) {
        let txn = store.db.begin_read().unwrap();
        let applied = txn
            .open_table(APPLIED)
            .unwrap()
            .iter()
            .unwrap()
            .map(|row| row.unwrap().0.value().to_vec())
            .collect();
        let chains = txn
            .open_table(CHAINS)
            .unwrap()
            .iter()
            .unwrap()
            .map(|row| {
                let (key, value) = row.unwrap();
                (key.value().to_vec(), value.value().to_vec())
            })
            .collect();
        (applied, chains)
    }

    /// Re-resolves the whole store, as the store does on an accept that carries group state.
    fn refresh_all(store: &mut EventStore) {
        store
            .transact(0, |m| {
                refresh(m.txn)?;
                Ok(((), true))
            })
            .unwrap();
    }

    /// The fast path for a plain accept writes exactly the rows a full refresh writes.
    #[test]
    fn test_a_plain_accept_writes_what_a_full_refresh_would() {
        let (alice, bob) = (key(1), key(2));
        let (_dir, mut store) = open();
        let mut events = chain_of(&alice, 5);
        events.extend(chain_of(&bob, 3));
        for event in &events {
            ingest_all(&mut store, std::slice::from_ref(event));
            let fast = rows(&store);
            refresh_all(&mut store);
            assert_eq!(
                rows(&store),
                fast,
                "after sequence {}",
                event.header.seq_num
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
    fn test_a_fork_proof_backlink_releases_a_successor_as_parked() {
        let (_dir, mut store) = open();
        let fork_signer = key(27);
        let successor_signer = key(28);
        let missing = Hash::from_bytes([99; 32]);

        // Hold the sequence-zero event until its rival turns it into a proof header.
        let first = event(&fork_signer, 0, None, vec![missing], "waiting");
        assert_eq!(ingest(&mut store, &first), Ingest::Waiting { id: first.id });

        // This event waits for that header as its backlink while it is still missing.
        let successor = event(
            &successor_signer,
            1,
            Some(first.id),
            Vec::new(),
            "successor",
        );
        assert_eq!(
            ingest(&mut store, &successor),
            Ingest::Waiting { id: successor.id }
        );

        // The fork makes `first` present, but it can never become applied. The released
        // successor must remain held without waiting on the now-present proof header.
        let rival = event(&fork_signer, 0, None, Vec::new(), "rival");
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: author(&fork_signer),
                seq: 0,
            }
        );
        assert!(store.is_waiting(&successor.id));
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
    fn test_an_after_hash_naming_a_voided_event_is_applied() {
        let (_dir, mut store) = open();
        let x = key(11);
        let y = key(12);
        let events = chain_of(&x, 2);
        ingest_all(&mut store, &events);
        let rival = event(&x, 1, Some(events[0].id), Vec::new(), "rival");
        ingest(&mut store, &rival);
        // events[1] is stored, and the fork voided it. It is present, so y0 does not wait.
        let y0 = event(&y, 0, None, vec![events[1].id], "y");
        assert_eq!(
            ingest(&mut store, &y0),
            Ingest::Accepted {
                id: y0.id,
                released: vec![],
            }
        );
        assert_eq!(store.chain(&author(&y)).unwrap().unwrap().next_seq, 1);
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
    fn test_a_released_event_whose_backlink_is_a_fork_proof_header_is_parked() {
        // R, by another author, names H2 as its backlink and waits for it. H2 is a rival of the
        // applied H1 by the first author, so its arrival forks that chain, and H2 becomes a proof
        // header: present, but never stored. R is released by that. It cannot link to H2, so it
        // must park. Holding it to wait for H2 would wait on a dependency that is already present.
        // R is not the forking author's event: an event above the fork point is refused as frozen
        // before the backlink is read, so only another author's backlink reaches this path.
        let (_dir, mut store) = open();
        let forker = key(15);
        let other = key(16);
        let applied = event(&forker, 0, None, Vec::new(), "first");
        let rival = event(&forker, 0, None, Vec::new(), "rival");
        let after = event(&other, 1, Some(rival.id), Vec::new(), "after");
        assert!(matches!(
            ingest(&mut store, &applied),
            Ingest::Accepted { .. }
        ));
        assert_eq!(ingest(&mut store, &after), Ingest::Waiting { id: after.id });
        assert!(matches!(
            ingest(&mut store, &rival),
            Ingest::Fork { seq: 0, .. }
        ));
        store.check_invariants().unwrap();
        assert!(
            store.event(&author(&other), 1).unwrap().is_none(),
            "a parked event is not stored"
        );
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
    fn test_a_frozen_refusal_gives_the_frozen_reason() {
        let (_dir, mut store) = open();
        let signer = key(4);
        let events = chain_of(&signer, 3);
        ingest_all(&mut store, &events[..2]);
        let rival = event(&signer, 1, Some(events[0].id), Vec::new(), "rival");
        assert!(matches!(
            ingest(&mut store, &rival),
            Ingest::Fork { seq: 1, .. }
        ));
        let result = ingest(&mut store, &events[2]);
        assert_eq!(result, Ingest::Refused(Refusal::Frozen));
        assert_eq!(result.reason(), Some(Reason::Frozen));
    }

    #[test]
    fn test_a_held_event_and_a_capped_one_give_the_waiting_reason() {
        let (_dir, mut store) = open_with(WaitingLimits::new(2, WEEK_MS));
        let signer = key(15);
        let events = chain_of(&signer, 5);
        let held = ingest(&mut store, &events[2]);
        assert_eq!(held, Ingest::Waiting { id: events[2].id });
        assert_eq!(held.reason(), Some(Reason::Waiting));
        ingest(&mut store, &events[3]);
        let capped = ingest(&mut store, &events[4]);
        assert_eq!(capped, Ingest::Refused(Refusal::WaitingFull));
        assert_eq!(capped.reason(), Some(Reason::Waiting));
    }

    #[test]
    fn test_a_malformed_event_gives_no_reason() {
        let (_dir, mut store) = open();
        let result = store.ingest(&[0xff, 0x00], 0).unwrap();
        assert!(matches!(result, Ingest::Refused(Refusal::Malformed(_))));
        assert_eq!(
            result.reason(),
            None,
            "bytes that do not verify are not an event"
        );
    }

    #[test]
    fn test_waiting_events_past_the_author_cap_are_refused() {
        let (_dir, mut store) = open_with(WaitingLimits::new(2, WEEK_MS));
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
        let (_dir, mut store) = open_with(WaitingLimits::new(256, 1_000));
        let signer = key(16);
        let events = chain_of(&signer, 2);
        store.ingest(&events[1].bytes, 0).unwrap();
        assert_eq!(store.expire_waiting(1_000).unwrap(), 1);
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
                tag: None,
                document: None,
                removes: None,
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
            let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
            ingest_all(&mut store, &events);
            ingest(&mut store, &rival);
        }
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
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
    fn test_a_fork_at_sequence_zero_keeps_an_event_that_names_the_voided_one_applied() {
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
        assert_eq!(
            applied_ids(&store, &author(&y)),
            vec![y0.id],
            "y0 names x0, which the fork voided. x0 is still present, so y0 stays applied"
        );
        assert_eq!(
            store.chain(&author(&y)).unwrap().unwrap().next_seq,
            1,
            "y's chain keeps its applied event"
        );
        store.check_invariants().unwrap();
    }

    /// A fork voids only the forked author's own events from its fork point on. Honest events
    /// of other authors that name a voided event still apply, and so do their later events:
    /// the voided event is present, so it does not wait. Phase 1 has no membership or authority
    /// model, so the design's authority-based voiding (decision 1) cannot run yet. It replaces
    /// this rule in Phase 2 (P2.3).
    #[test]
    fn test_a_fork_voids_only_the_forked_chain_until_p2_3_authority_voiding() {
        let (_dir, mut store) = open();
        let (x, y, z) = (key(22), key(23), key(24));
        let x0 = event(&x, 0, None, Vec::new(), "message");
        let x1 = event(&x, 1, Some(x0.id), Vec::new(), "message");
        ingest_all(&mut store, &[x0.clone(), x1.clone()]);
        let rival = event(&x, 1, Some(x0.id), Vec::new(), "rival");
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Fork {
                author: author(&x),
                seq: 1
            }
        );
        // Y names X's voided event, and Y's next event names Y's first. Z names Y's second.
        let y0 = event(&y, 0, None, vec![x1.id], "message");
        let y1 = event(&y, 1, Some(y0.id), Vec::new(), "message");
        let z0 = event(&z, 0, None, vec![y1.id], "message");
        ingest_all(&mut store, &[y0.clone(), y1.clone(), z0.clone()]);
        assert_eq!(applied_ids(&store, &author(&x)), vec![x0.id]);
        assert_eq!(applied_ids(&store, &author(&y)), vec![y0.id, y1.id]);
        assert_eq!(applied_ids(&store, &author(&z)), vec![z0.id]);
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
            let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
        }
        let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
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
    fn test_a_restart_keeps_the_order_in_which_waiters_are_released() {
        let dir = tempfile::tempdir().unwrap();
        let root0 = event(&key(48), 0, None, Vec::new(), "message");
        // Eight authors each hold one event that waits for `root0`. The rows are keyed by hash,
        // so only the arrival number keeps them in this order across a restart.
        let waiters: Vec<Event> = (0..8)
            .map(|n| event(&key(50 + n), 0, None, vec![root0.id], "message"))
            .collect();
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            for waiter in &waiters {
                assert_eq!(
                    ingest(&mut store, waiter),
                    Ingest::Waiting { id: waiter.id }
                );
            }
        }
        let store = reopen(&dir, WaitingLimits::default(), 0);
        let mut store = store;
        assert_eq!(
            ingest(&mut store, &root0),
            Ingest::Accepted {
                id: root0.id,
                released: waiters.iter().map(|waiter| waiter.id).collect(),
            }
        );
    }

    #[test]
    fn test_a_fork_against_a_waiting_event_is_seen_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(32);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        {
            let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
        }
        let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
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
        let (_dir, mut store) = open_with(WaitingLimits::new(256, 1_000));
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

    #[test]
    fn test_a_released_event_that_waits_again_keeps_its_first_arrival_time() {
        let (_dir, mut store) = open();
        let signer = key(49);
        let events = chain_of(&signer, 3);
        // Ahead of its chain, and its backlink is the second event. It waits for that event.
        let stray = event(&signer, 5, Some(events[1].id), Vec::new(), "stray");
        assert_eq!(
            store.ingest(&stray.bytes, 0).unwrap(),
            Ingest::Waiting { id: stray.id }
        );
        assert_eq!(
            store.ingest(&events[0].bytes, 1).unwrap(),
            Ingest::Accepted {
                id: events[0].id,
                released: vec![],
            }
        );
        // The second event releases the stray. Its backlink is applied at an earlier position, so
        // it can never apply, and it is parked again: held for good, and waiting again.
        assert_eq!(
            store.ingest(&events[1].bytes, 2).unwrap(),
            Ingest::Accepted {
                id: events[1].id,
                released: vec![],
            }
        );
        assert!(
            store.is_waiting(&stray.id),
            "the released event waits again"
        );
        // It arrived at 0, so it is due at WEEK_MS. Counting from its second wait it would be due
        // at 2 + WEEK_MS.
        assert_eq!(store.expire_waiting(WEEK_MS).unwrap(), 1);
        assert_eq!(store.waiting_len(), 0);
        store.check_invariants().unwrap();
    }

    /// Opens the store in `dir` again, with `limits`, at clock `now`.
    fn reopen(dir: &tempfile::TempDir, limits: WaitingLimits, now: u64) -> EventStore {
        EventStore::open(dir.path(), limits, now).unwrap()
    }

    #[test]
    fn test_a_release_survives_a_restart_as_one_change() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(34);
        let who = author(&signer);
        let events = chain_of(&signer, 2);
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
            assert_eq!(
                ingest(&mut store, &events[0]),
                Ingest::Accepted {
                    id: events[0].id,
                    released: vec![events[1].id],
                }
            );
        }
        let store = reopen(&dir, WaitingLimits::default(), 0);
        assert!(
            !store.is_waiting(&events[1].id),
            "the released event applies"
        );
        assert_eq!(store.chain(&who).unwrap().unwrap().next_seq, 2);
        assert!(store.waiting_ids().unwrap().is_empty());
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_waiting_event_past_its_age_is_dropped_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let limits = WaitingLimits::new(256, 1_000);
        let signer = key(35);
        let events = chain_of(&signer, 2);
        {
            let mut store = reopen(&dir, limits, 0);
            assert_eq!(
                store.ingest(&events[1].bytes, 0).unwrap(),
                Ingest::Waiting { id: events[1].id }
            );
        }
        // Not due yet: the event has waited 999 ms.
        let store = reopen(&dir, limits, 999);
        assert!(store.is_waiting(&events[1].id));
        drop(store);
        // Due at 1_000 ms, so the open drops it and its row.
        let store = reopen(&dir, limits, 1_000);
        assert_eq!(store.waiting_len(), 0);
        assert!(store.waiting_ids().unwrap().is_empty());
        store.check_invariants().unwrap();
        drop(store);
        let store = reopen(&dir, limits, 1_000);
        assert_eq!(store.waiting_len(), 0, "a dropped event stays dropped");
    }

    #[test]
    fn test_an_expiry_made_by_expire_waiting_persists() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(36);
        let events = chain_of(&signer, 2);
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            store.ingest(&events[1].bytes, 0).unwrap();
            assert_eq!(store.expire_waiting(WEEK_MS).unwrap(), 1);
        }
        let store = reopen(&dir, WaitingLimits::default(), 0);
        assert_eq!(store.waiting_len(), 0);
        assert!(store.waiting_ids().unwrap().is_empty());
    }

    /// #2690, at the mirror itself: a release whose `arrived` expires another
    /// held event must delete that event's row in the releasing transaction --
    /// not leave it for a later, unrelated expire to count, or for a restart
    /// to drop silently. Driven through the mirror directly, because the
    /// ingest path's own expire-first composition would mask a mirror that
    /// forgot the flush.
    #[test]
    fn test_a_mirror_release_flushes_the_events_it_expires() {
        let signer = key(39);
        let events = chain_of(&signer, 3);
        let (_dir, mut store) = open();
        // events[2] is held at 0, waiting for events[1]. It is due at WEEK_MS.
        store
            .transact(0, |m| {
                m.hold(events[2].clone(), vec![events[1].id], false, 0)?;
                Ok(((), true))
            })
            .unwrap();
        // events[1] is held just before the cap, waiting for events[0]: fresh.
        store
            .transact(WEEK_MS - 1_000, |m| {
                m.hold(
                    events[1].clone(),
                    vec![events[0].id],
                    false,
                    WEEK_MS - 1_000,
                )?;
                Ok(((), true))
            })
            .unwrap();
        // Releasing events[0] at the cap releases events[1]; inside that
        // release, events[2] reaches the age cap and expires.
        let released = store
            .transact(WEEK_MS, |m| {
                let released = m.release(&events[0].id, WEEK_MS)?;
                Ok((released, true))
            })
            .unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].id, events[1].id);
        // The released event's row stays until its caller decides it; the
        // expired event's row is gone in the same transaction, and nothing
        // remains for a later expire to count.
        let rows = store.waiting_ids().unwrap();
        assert!(rows.contains(&events[1].id), "rows after release: {rows:?}");
        assert!(
            !rows.contains(&events[2].id),
            "the expired event's row survived the release: {rows:?}"
        );
        // Decide the released event the way its caller would, so the store
        // is back at a transaction boundary it accepts; then nothing remains
        // for a later expire to count.
        store
            .transact(WEEK_MS, |m| {
                m.drop_row(&events[1].id)?;
                Ok(((), true))
            })
            .unwrap();
        assert_eq!(store.expire_waiting(WEEK_MS).unwrap(), 0);
        assert!(store.waiting_ids().unwrap().is_empty());
        store.check_invariants().unwrap();
    }

    /// #2690: a release whose `arrived` expires another held event must delete
    /// that event's row in the releasing transaction -- not leave it for a
    /// later, unrelated expire to count, or for a restart to drop silently.
    #[test]
    fn test_a_release_flushes_the_events_it_expires() {
        let signer = key(38);
        let events = chain_of(&signer, 3);
        let (_dir, mut store) = open();
        // events[2] is held at 0, waiting for events[1]. It is due at WEEK_MS.
        assert_eq!(
            ingest(&mut store, &events[2]),
            Ingest::Waiting { id: events[2].id }
        );
        // events[1] is held just before the cap, waiting for events[0]: fresh.
        assert_eq!(
            store.ingest(&events[1].bytes, WEEK_MS - 1_000).unwrap(),
            Ingest::Waiting { id: events[1].id }
        );
        // events[0] arrives at the cap and is accepted, releasing events[1].
        // Inside that release, events[2] reaches the age cap and expires.
        assert_eq!(
            store.ingest(&events[0].bytes, WEEK_MS).unwrap(),
            Ingest::Accepted {
                id: events[0].id,
                released: vec![events[1].id],
            }
        );
        // The expiry belonged to the releasing call: nothing remains for a
        // later expire to count, the pool holds nothing, and no row for
        // events[2] (or any waiting row) survives.
        assert_eq!(store.expire_waiting(WEEK_MS).unwrap(), 0);
        assert_eq!(store.waiting_len(), 0);
        assert!(store.waiting_ids().unwrap().is_empty());
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_refused_waiting_event_leaves_no_row() {
        let dir = tempfile::tempdir().unwrap();
        let limits = WaitingLimits::new(1, WEEK_MS);
        let signer = key(37);
        let events = chain_of(&signer, 3);
        {
            let mut store = reopen(&dir, limits, 0);
            assert_eq!(
                ingest(&mut store, &events[1]),
                Ingest::Waiting { id: events[1].id }
            );
            assert_eq!(
                ingest(&mut store, &events[2]),
                Ingest::Refused(Refusal::WaitingFull)
            );
        }
        let store = reopen(&dir, limits, 0);
        assert_eq!(store.waiting_ids().unwrap(), vec![events[1].id]);
        assert!(!store.is_waiting(&events[2].id));
    }

    #[test]
    fn test_a_smaller_cap_on_reopen_drops_the_later_arrivals() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(38);
        let who = author(&signer);
        let events = chain_of(&signer, 4);
        {
            let mut store = reopen(&dir, WaitingLimits::new(3, WEEK_MS), 0);
            for event in &events[1..] {
                assert_eq!(ingest(&mut store, event), Ingest::Waiting { id: event.id });
            }
            assert_eq!(store.waiting_count(&who), 3);
        }
        // With a cap of two, the first two arrivals are kept, and the third row goes.
        let store = reopen(&dir, WaitingLimits::new(2, WEEK_MS), 0);
        assert_eq!(store.waiting_count(&who), 2);
        assert!(store.is_waiting(&events[1].id));
        assert!(store.is_waiting(&events[2].id));
        assert!(!store.is_waiting(&events[3].id));
        assert_eq!(store.waiting_ids().unwrap().len(), 2);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_smaller_total_on_reopen_drops_the_excess_and_still_opens() {
        // Three authors each hold one waiting event under the default total.
        // Reopening under a total of two must not brick the store: the excess
        // row is dropped, the store opens, and the drop is durable.
        let dir = tempfile::tempdir().unwrap();
        let mut ids = Vec::new();
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            for n in 41..=43 {
                let signer = key(n);
                let events = chain_of(&signer, 2);
                assert_eq!(
                    ingest(&mut store, &events[1]),
                    Ingest::Waiting { id: events[1].id }
                );
                ids.push(events[1].id);
            }
            assert_eq!(store.waiting_ids().unwrap().len(), 3);
        }
        let limits = WaitingLimits::default().with_total(2);
        let store = reopen(&dir, limits, 0);
        assert_eq!(store.waiting_ids().unwrap().len(), 2);
        assert_eq!(ids.iter().filter(|id| store.is_waiting(id)).count(), 2);
        store.check_invariants().unwrap();
        let store = reopen(&dir, limits, 0);
        assert_eq!(store.waiting_ids().unwrap().len(), 2);
    }

    #[test]
    fn test_a_parked_event_is_held_across_a_restart_and_still_sees_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key(39);
        let who = author(&signer);
        let events = chain_of(&signer, 1);
        let stray = event(
            &signer,
            1,
            Some(Hash::digest(b"not the head")),
            Vec::new(),
            "stray",
        );
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            ingest_all(&mut store, &events);
            assert_eq!(ingest(&mut store, &stray), Ingest::Waiting { id: stray.id });
        }
        let mut store = reopen(&dir, WaitingLimits::default(), 0);
        assert!(
            store.is_waiting(&stray.id),
            "the parked event is still held"
        );
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
    fn test_an_event_released_into_a_frozen_chain_leaves_no_row() {
        let dir = tempfile::tempdir().unwrap();
        let (x, y) = (key(40), key(41));
        let x_events = chain_of(&x, 2);
        let y0 = event(&y, 0, None, Vec::new(), "message");
        let waits = event(&x, 2, Some(x_events[1].id), vec![y0.id], "message");
        let rival = event(&x, 1, Some(x_events[0].id), Vec::new(), "rival");
        {
            let mut store = reopen(&dir, WaitingLimits::default(), 0);
            ingest_all(&mut store, &x_events);
            assert_eq!(ingest(&mut store, &waits), Ingest::Waiting { id: waits.id });
            assert!(matches!(
                ingest(&mut store, &rival),
                Ingest::Fork { seq: 1, .. }
            ));
        }
        let mut store = reopen(&dir, WaitingLimits::default(), 0);
        assert!(
            store.is_waiting(&waits.id),
            "the event waits across the restart"
        );
        // Its dependency arrives, so it is released. Its chain is frozen at 1, so it is refused.
        assert_eq!(
            ingest(&mut store, &y0),
            Ingest::Accepted {
                id: y0.id,
                released: vec![],
            }
        );
        assert!(!store.is_waiting(&waits.id));
        assert!(store.waiting_ids().unwrap().is_empty());
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_rival_refused_by_the_cap_is_seen_only_when_sent_again() {
        // The known limit in the module docs. The author's one slot holds `filler`, so the rival
        // is refused and not held. When the original arrives, nothing conflicts with it.
        let (_dir, mut store) = open_with(WaitingLimits::new(1, WEEK_MS));
        let signer = key(45);
        let who = author(&signer);
        let missing = Hash::digest(b"an event that is never delivered");
        let original = event(&signer, 1, Some(missing), Vec::new(), "message");
        let rival = event(&signer, 1, Some(missing), Vec::new(), "rival");
        let filler = event(&signer, 2, Some(missing), Vec::new(), "filler");
        assert_eq!(
            ingest(&mut store, &filler),
            Ingest::Waiting { id: filler.id }
        );
        assert_eq!(
            ingest(&mut store, &rival),
            Ingest::Refused(Refusal::WaitingFull)
        );
        // At WEEK_MS the filler is due, so the slot is free for the original.
        assert_eq!(
            store.ingest(&original.bytes, WEEK_MS).unwrap(),
            Ingest::Waiting { id: original.id }
        );
        assert!(
            store.fork_proof(&who).unwrap().is_none(),
            "the refused rival is not held, so no conflict is seen"
        );
        // Sent again, the rival finds the original, which is held.
        assert_eq!(
            store.ingest(&rival.bytes, WEEK_MS).unwrap(),
            Ingest::Fork {
                author: who,
                seq: 1
            }
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_held_event_that_a_fork_makes_a_proof_header_stays_held() {
        let (_dir, mut store) = open();
        let signer = key(50);
        let who = author(&signer);
        let missing = Hash::digest(b"an event that is never delivered");
        // Seq 1 waits for a backlink that never arrives, and its rival at seq 1 forks it.
        let held = event(&signer, 1, Some(missing), Vec::new(), "message");
        let rival = event(&signer, 1, Some(missing), Vec::new(), "rival");
        assert_eq!(ingest(&mut store, &held), Ingest::Waiting { id: held.id });
        assert!(matches!(
            ingest(&mut store, &rival),
            Ingest::Fork { seq: 1, .. }
        ));
        let proof = store.fork_proof(&who).unwrap().expect("a proof is stored");
        assert_eq!(
            [proof.first.id, proof.second.id],
            sorted(&held.id, &rival.id)
        );
        // A proof header never applies, so it stays held for good. It is neither stored nor held
        // if the fork drops it.
        assert!(store.is_waiting(&held.id), "a held proof header stays held");
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_write_whose_rebuild_fails_makes_the_store_refuse_calls() {
        let (_dir, mut store) = open_with(WaitingLimits::new(256, 1_000));
        let signer = key(47);
        let events = chain_of(&signer, 2);
        store.ingest(&events[1].bytes, 0).unwrap();
        // The row no longer decodes. The expiry that drops it fails, and so does the rebuild.
        store
            .write(|txn| {
                let mut rows = txn.open_table(crate::event_store::WAITING)?;
                rows.insert(events[1].id.as_bytes().as_slice(), b"garbage".as_slice())?;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            store.expire_waiting(1_000),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            store.expire_waiting(1_000),
            Err(StoreError::Unusable)
        ));
        assert!(matches!(
            store.ingest(&events[0].bytes, 1_000),
            Err(StoreError::Unusable)
        ));
    }

    #[test]
    fn test_invariants_refuse_a_waiting_event_whose_dependency_is_present() {
        let (_dir, mut store) = open();
        let (x, y) = (key(42), key(43));
        let x0 = event(&x, 0, None, Vec::new(), "message");
        ingest_all(&mut store, std::slice::from_ref(&x0));
        // The pool and the row both hold an event that waits for x0, which is already applied.
        let waits = event(&y, 0, None, vec![x0.id], "message");
        let row = Row {
            id: waits.id,
            author: author(&y),
            seq: 0,
            arrived_at: 0,
            arrival: 0,
            deps: vec![x0.id],
            bytes: waits.bytes.clone(),
        };
        store
            .waiting
            .restore(author(&y), waits.id, [x0.id], waits, 0)
            .unwrap();
        store.write(|txn| waiting_rows::put(txn, &row)).unwrap();
        let err = store.check_invariants().unwrap_err();
        assert!(err.to_string().contains("present"), "{err}");
    }

    #[test]
    fn test_an_event_ahead_of_a_voided_backlink_is_parked_and_survives_a_restart() {
        let (signer, other) = (key(32), key(33));
        let b0 = event(&other, 0, None, Vec::new(), "message");
        let b1 = event(&other, 1, Some(b0.id), Vec::new(), "message");
        let b1_rival = event(
            &other,
            1,
            Some(Hash::digest(b"not the head")),
            Vec::new(),
            "stray",
        );
        // The fork voids `b1`: it stays stored and present, and it is never applied again.
        let a1 = event(&signer, 1, Some(b1.id), Vec::new(), "message");
        let (dir, mut store) = open();
        ingest_all(&mut store, &[b0, b1]);
        assert_eq!(
            ingest(&mut store, &b1_rival),
            Ingest::Fork {
                author: author(&other),
                seq: 1
            }
        );
        // `a1` can never apply, so it is parked. It must not wait for `b1`, which is present.
        assert_eq!(ingest(&mut store, &a1), Ingest::Waiting { id: a1.id });
        store.check_invariants().unwrap();
        drop(store);
        let store = reopen(&dir, WaitingLimits::default(), 0);
        store.check_invariants().unwrap();
        assert!(store.is_waiting(&a1.id));
    }

    #[test]
    fn test_an_event_ahead_of_a_fork_proof_header_is_parked_and_survives_a_restart() {
        let (signer, other) = (key(34), key(35));
        let b0 = event(&other, 0, None, Vec::new(), "message");
        let b0_rival = event(&other, 0, None, Vec::new(), "rival");
        // `b0` is a fork proof header once `b0_rival` arrives. It is present and never applied.
        let a1 = event(&signer, 1, Some(b0.id), Vec::new(), "message");
        let (dir, mut store) = open();
        ingest_all(&mut store, std::slice::from_ref(&b0));
        assert_eq!(
            ingest(&mut store, &b0_rival),
            Ingest::Fork {
                author: author(&other),
                seq: 0
            }
        );
        assert_eq!(ingest(&mut store, &a1), Ingest::Waiting { id: a1.id });
        store.check_invariants().unwrap();
        drop(store);
        let store = reopen(&dir, WaitingLimits::default(), 0);
        store.check_invariants().unwrap();
        assert!(store.is_waiting(&a1.id));
    }

    /// A fork at a lower sequence number adds its own proof and keeps the one above. The header
    /// that a stored event names stays present, so the event stays valid, and the lower proof
    /// freezes the chain.
    #[test]
    fn test_a_lower_fork_keeps_the_proof_header_an_event_names() {
        let (dir, mut store) = open();
        let (a, c) = (key(20), key(22));
        let a0 = event(&a, 0, None, Vec::new(), "message");
        let a1 = event(&a, 1, Some(a0.id), Vec::new(), "message");
        let a1_rival = event(&a, 1, Some(a0.id), Vec::new(), "rival");
        let a0_rival = event(&a, 0, None, Vec::new(), "rival");
        let names_a1 = event(&c, 0, None, vec![a1.id], "message");
        for e in [&names_a1, &a1, &a1_rival, &a0] {
            ingest(&mut store, e);
        }
        assert_eq!(
            ingest(&mut store, &a0_rival),
            Ingest::Fork {
                author: author(&a),
                seq: 0
            }
        );
        let proof = store.fork_proof(&author(&a)).unwrap().unwrap();
        assert_eq!(proof.seq, 0);
        assert!(applied_ids(&store, &author(&c)).contains(&names_a1.id));
        assert_eq!(ingest(&mut store, &a1), Ingest::Duplicate { id: a1.id });
        store.check_invariants().unwrap();
        drop(store);
        let store = reopen(&dir, WaitingLimits::default(), 0);
        store.check_invariants().unwrap();
        assert_eq!(store.fork_proof(&author(&a)).unwrap().unwrap().seq, 0);
        assert!(applied_ids(&store, &author(&c)).contains(&names_a1.id));
    }

    /// A fork at a point replaces that point's proof, and the header the new proof drops stays
    /// present and in the resolution. An event that names the dropped header stays valid.
    #[test]
    fn test_a_header_dropped_from_its_point_stays_present_and_named() {
        let (dir, mut store) = open();
        let (a, c) = (key(20), key(22));
        let a0 = event(&a, 0, None, Vec::new(), "message");
        let a1 = event(&a, 1, Some(a0.id), Vec::new(), "message");
        let a1_rival = event(&a, 1, Some(a0.id), Vec::new(), "rival");
        // The proof keeps the two lowest IDs at sequence 1. The higher of the first two is the
        // header a lower-ID third event drops, so that is the one an event names.
        let dropped = if a1.id < a1_rival.id {
            a1_rival.clone()
        } else {
            a1.clone()
        };
        let third = (0..4096)
            .map(|i| event(&a, 1, Some(a0.id), Vec::new(), &format!("third {i}")))
            .find(|e| e.id < dropped.id)
            .expect("a third event with a lower ID");
        let names = event(&c, 0, None, vec![dropped.id], "message");
        // The fork proof is held while `a0` is absent, so no header is stored.
        for e in [&a1, &a1_rival, &names] {
            ingest(&mut store, e);
        }
        assert_eq!(
            ingest(&mut store, &third),
            Ingest::Fork {
                author: author(&a),
                seq: 1
            }
        );
        let proof = store.fork_proof(&author(&a)).unwrap().unwrap();
        assert!(![proof.first.id, proof.second.id].contains(&dropped.id));
        assert_eq!(
            ingest(&mut store, &dropped),
            Ingest::Duplicate { id: dropped.id }
        );
        ingest(&mut store, &a0);
        assert!(applied_ids(&store, &author(&c)).contains(&names.id));
        store.check_invariants().unwrap();
        drop(store);
        let store = reopen(&dir, WaitingLimits::default(), 0);
        store.check_invariants().unwrap();
        assert!(applied_ids(&store, &author(&c)).contains(&names.id));
    }

    /// `SplitMix64`, as the property tests under `tests/` use it. A scenario is a function of its
    /// seed.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) % n
        }
    }

    /// Three authors' chains with forks at random positions, duplicates, and an arrival order
    /// shuffled from `seed`. Their `after` hashes name the anchor chain, of `key(23)`, which has no
    /// fork. A fourth author's events name any event generated before them, rivals included, so an
    /// `after` hash can name a fork proof header, and a later fork at a lower point can supersede it.
    fn scenario(seed: u64) -> Vec<Event> {
        let mut rng = Rng(seed);
        let mut all: Vec<Event> = Vec::new();
        let anchor_key = key(23);
        let mut anchor: Vec<Event> = Vec::new();
        for seq in 0..=(rng.below(3) as SeqNum) {
            let backlink = anchor.last().map(|event| event.id);
            anchor.push(event(&anchor_key, seq, backlink, Vec::new(), "message"));
        }
        all.extend(anchor.iter().cloned());
        let mut chains: Vec<Vec<Event>> = Vec::new();
        for n in 0..3u8 {
            let who = key(20 + n);
            let mut chain: Vec<Event> = Vec::new();
            for seq in 0..=(rng.below(3) as SeqNum) {
                let backlink = chain.last().map(|event| event.id);
                let after = if rng.below(3) == 0 {
                    let named = rng.below(anchor.len() as u64) as usize;
                    vec![anchor[named].id]
                } else {
                    Vec::new()
                };
                let signed = event(&who, seq, backlink, after, "message");
                all.push(signed.clone());
                chain.push(signed);
            }
            chains.push(chain);
        }
        for _ in 0..rng.below(3) {
            let who = rng.below(3) as usize;
            let chain = &chains[who];
            let seq = rng.below(chain.len() as u64) as SeqNum;
            let backlink = seq.checked_sub(1).map(|prev| chain[prev as usize].id);
            let rival = event(&key(20 + who as u8), seq, backlink, Vec::new(), "rival");
            all.push(rival);
        }
        let fourth = key(24);
        let mut fourth_events: Vec<Event> = Vec::new();
        for seq in 0..=(rng.below(3) as SeqNum) {
            let backlink = fourth_events.last().map(|event| event.id);
            let target = rng.below(all.len() as u64) as usize;
            let signed = event(&fourth, seq, backlink, vec![all[target].id], "message");
            all.push(signed.clone());
            fourth_events.push(signed);
        }
        for _ in 0..rng.below(3) {
            let copy = all[rng.below(all.len() as u64) as usize].clone();
            all.push(copy);
        }
        for i in (1..all.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            all.swap(i, j);
        }
        all
    }

    /// Everything a caller can read from a store: the rows, the waiting events, the sync set, and
    /// each author's chain and fork proof.
    fn observed(store: &EventStore, authors: &[AuthorKey]) -> impl PartialEq + std::fmt::Debug {
        let per_author: Vec<(Option<Chain>, Option<ForkProof>)> = authors
            .iter()
            .map(|who| (store.chain(who).unwrap(), store.fork_proof(who).unwrap()))
            .collect();
        (
            rows(store),
            store.waiting_ids().unwrap(),
            store.sync_entries().unwrap(),
            per_author,
        )
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

        /// The batch path decides each page as per-event ingest decides its events, and it leaves
        /// the same state, for random arrival orders, forks, duplicates, `after` hashes, and page
        /// sizes.
        #[test]
        fn test_a_batch_decides_and_stores_what_per_event_ingest_does(
            seed in proptest::prelude::any::<u64>(),
            pages in proptest::prelude::any::<u64>(),
        ) {
            let arrivals = scenario(seed);
            let authors = [key(20), key(21), key(22), key(23), key(24)].map(|k| author(&k));
            let (_da, mut one) = open();
            let per_event: Vec<Ingest> = arrivals.iter().map(|e| ingest(&mut one, e)).collect();

            let (_db, mut paged) = open();
            let mut rng = Rng(pages);
            let mut rest = arrivals;
            let mut batched = Vec::new();
            while !rest.is_empty() {
                let take = (1 + rng.below(6) as usize).min(rest.len());
                let page: Vec<Event> = rest.drain(..take).collect();
                batched.extend(paged.ingest_events(page, 0).unwrap());
            }

            proptest::prop_assert_eq!(&batched, &per_event);
            proptest::prop_assert_eq!(observed(&paged, &authors), observed(&one, &authors));
            proptest::prop_assert!(paged.check_invariants().is_ok());
        }
    }
}
