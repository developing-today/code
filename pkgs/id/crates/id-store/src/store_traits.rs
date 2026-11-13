//! The event store behind p2panda-store's traits.
//!
//! [`SharedEventStore`] wraps an [`EventStore`] so that it can be cloned and shared, and
//! implements the two traits that p2panda-sync's `LogSync` and `TopicLogSync` read through:
//! `LogStore` and `TopicStore`.
//!
//! - A **log** is one author's chain. [`ChainLog`] is the only log id, because each device has
//!   one chain and the group is an attribute of its events, not a log (decision N4).
//! - A **topic** is a group hash. `TopicStore::resolve` lists the authors with an applied event
//!   in that group, each with its one log.
//! - A store's log is its **applied prefix**. Events past a fork point are stored but not
//!   applied, so heights, sizes, entries and the latest entry stop at the fork point. Two stores
//!   with the same applied state report the same logs.
//!
//! The methods that the sync path does not use return [`TraitStoreError::Unsupported`], with the
//! reason, rather than a guess. Auth state, fork proofs and waiting events are not part of these
//! traits, and stay in the event store's own tables.
//!
//! # `LogSync`
//!
//! `LogSync` runs over these traits in [`crate::log_sync`]. It uses a vendored copy of p2panda-sync
//! (`vendor/`) whose store dependency has no SQLite default, so the build has no SQLite.
//! `test_the_lock_has_no_sqlite_with_logsync_wired` fails if SQLite returns.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use futures_lite::Stream;
use p2panda_core::{
    AnyOperation, Hash, HeaderError, IdentityError, RawOperation, SeqNum, VerifyingKey,
};
use p2panda_store::logs::{LogEntry, LogStore, LogStream};
use p2panda_store::topics::TopicStore;
use serde::{Deserialize, Serialize};

use crate::chain::Event;
use crate::event_types::{
    BlockKind, BlockList, BlockLookup, BlockLookupFailure, Ingest, StoreError,
};
use id_core::waiting::AuthorKey;

/// The event-store surface [`SharedEventStore`] wraps.
///
/// Seam 3 of the crate split (design doc, #4405 step 2): the traits crate
/// holds traits only, so the shared handle is generic over this trait
/// instead of naming the concrete `EventStore`. The event store implements
/// it (in the facade until step 2b, in `id-events` after), and the facade
/// pins the parameter: `id::store_traits::SharedEventStore` is
/// `SharedEventStore<id::event_store::EventStore>`, so every existing call
/// site keeps its types. The methods are exactly the ones the trait
/// implementations and the store search in this crate call; each delegates
/// to the event store's inherent method of the same name (the block check
/// is `is_blocked_at` here because the inherent `is_blocked` takes the
/// clock as a parameter).
pub trait EventStoreBackend: std::fmt::Debug + Send + Sync + 'static {
    /// See `EventStore::ingest_with_body`.
    fn ingest_with_body(
        &mut self,
        bytes: &[u8],
        body: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError>;
    /// See `EventStore::ingest_batch`.
    fn ingest_batch(&mut self, batch: &[Vec<u8>], now_ms: u64) -> Result<Vec<Ingest>, StoreError>;
    /// See `EventStore::ingest_events`.
    fn ingest_events(&mut self, events: Vec<Event>, now_ms: u64)
    -> Result<Vec<Ingest>, StoreError>;
    /// See `EventStore::ingest_events_with_bodies`.
    fn ingest_events_with_bodies(
        &mut self,
        events: Vec<(Event, Option<Vec<u8>>)>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError>;
    /// See `EventStore::stored_body`.
    fn stored_body(&self, id: &Hash) -> Result<Option<Vec<u8>>, StoreError>;
    /// See `EventStore::sync_entries`.
    fn sync_entries(&self) -> Result<Vec<(crate::range::Key, crate::range::Id)>, StoreError>;
    /// See `EventStore::sync_bytes`.
    fn sync_bytes(&self, id: &crate::range::Id) -> Result<Option<Vec<u8>>, StoreError>;
    /// See `EventStore::check_invariants`.
    fn check_invariants(&self) -> Result<(), StoreError>;
    /// See `EventStore::event`.
    fn event(&self, author: &AuthorKey, seq: SeqNum) -> Result<Option<Event>, StoreError>;
    /// See `EventStore::applied_events`.
    fn applied_events(
        &self,
        author: &AuthorKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
        limit: usize,
    ) -> Result<Vec<Event>, StoreError>;
    /// See `EventStore::group_members`.
    fn group_members(&self, group: &Hash) -> Result<Vec<(AuthorKey, SeqNum)>, StoreError>;
    /// See `EventStore::chain`.
    fn chain(&self, author: &AuthorKey) -> Result<Option<crate::event_types::Chain>, StoreError>;
    /// See `EventStore::is_blocked` (the inherent method, which takes the clock).
    fn is_blocked_at(&self, kind: BlockKind, value: &[u8], now_ms: u64)
    -> Result<bool, StoreError>;
    /// See `EventStore::tag_claims`.
    fn tag_claims(&self) -> Result<Vec<crate::tag_claims::Claim>, StoreError>;
    /// See `EventStore::event_by_id`.
    fn event_by_id(&self, id: &Hash) -> Result<Option<Event>, StoreError>;
}

/// Entries read per page. A page takes the read lock once, so a stream never holds the lock
/// between polls, and a long log is never held in memory at once.
const PAGE: usize = 64;

/// The one log an author has: their chain. Decision N4: one log per device, with the group carried
/// on each event, rather than one log per group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChainLog;

/// Why a store trait method failed.
#[derive(Debug, thiserror::Error)]
pub enum TraitStoreError {
    /// The event store failed. See [`StoreError`].
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A stored event does not decode as an operation. Events are verified when they are
    /// accepted, so this means the file changed outside the store.
    #[error("stored event is not a valid operation: {0}")]
    Operation(#[from] HeaderError),
    /// A stored author key is not a valid public key. As with an invalid event, the file changed
    /// outside the store.
    #[error("stored author key is not a valid public key: {0}")]
    Author(#[from] IdentityError),
    /// A count or a byte total does not fit the `u32` that the trait returns.
    #[error("log size does not fit in a u32")]
    Overflow,
    /// Another thread panicked while it held the store's lock.
    #[error("event store lock is poisoned")]
    Poisoned,
    /// The trait method is not implemented. The message says why.
    #[error("not implemented: {0}")]
    Unsupported(&'static str),
    /// The store returned a page that did not start after the stream's cursor, so the stream
    /// would repeat it forever.
    #[error("log stream did not advance past its cursor")]
    Stalled,
}

/// A clonable handle to one event store that implements p2panda-store's traits.
///
/// Trait reads share the store's read lock. [`SharedEventStore::ingest`] takes the write lock.
#[derive(Debug)]
pub struct SharedEventStore<S> {
    inner: Arc<RwLock<S>>,
}

impl<S> Clone for SharedEventStore<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S: EventStoreBackend> SharedEventStore<S> {
    /// Shares `store` between the clones of this handle.
    pub fn new(store: S) -> Self {
        Self {
            inner: Arc::new(RwLock::new(store)),
        }
    }

    /// The write guard on the shared store. Exposed, hidden, for this
    /// crate's integration tests: the poisoned-lock test holds it across a
    /// panic, which a field access did when these tests were unit tests.
    #[doc(hidden)]
    pub fn write_guard_for_test(&self) -> std::sync::RwLockWriteGuard<'_, S> {
        match self.inner.write() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        }
    }

    /// Verifies and applies one received event, with its body. `body` is the bytes the header
    /// commits to, or `None` when the event arrived without one. The action of a group operation
    /// comes from the body. See [`EventStore::ingest_with_body`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn ingest(
        &self,
        bytes: &[u8],
        body: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<Ingest, TraitStoreError> {
        let mut store = self
            .inner
            .write()
            .map_err(|_err| TraitStoreError::Poisoned)?;
        Ok(store.ingest_with_body(bytes, body, now_ms)?)
    }

    /// Verifies and applies a batch of received events, in order, in one write transaction. The
    /// outcomes are those of [`SharedEventStore::ingest`] on each entry with no body. See
    /// [`EventStore::ingest_batch`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn ingest_batch(
        &self,
        batch: &[Vec<u8>],
        now_ms: u64,
    ) -> Result<Vec<Ingest>, TraitStoreError> {
        let mut store = self
            .inner
            .write()
            .map_err(|_err| TraitStoreError::Poisoned)?;
        Ok(store.ingest_batch(batch, now_ms)?)
    }

    /// The range sync set of the store, under the read lock, taken for this one call.
    /// See [`EventStore::sync_entries`]. This is one step of the range session seam
    /// (`event_store::sync::RangeStore`, #179): the lock is never held across an await.
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn range_entries(
        &self,
    ) -> Result<Vec<(crate::range::Key, crate::range::Id)>, TraitStoreError> {
        self.read(|store| Ok(store.sync_entries()?))
    }

    /// The bytes of the event with this ID, under the read lock, taken for this one
    /// call. See [`EventStore::sync_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn range_bytes(&self, id: &crate::range::Id) -> Result<Option<Vec<u8>>, TraitStoreError> {
        self.read(|store| Ok(store.sync_bytes(id)?))
    }

    /// The stored body of the event with this ID, under the read lock, taken for this
    /// one call. See [`EventStore::stored_body`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn range_body(&self, id: &crate::range::Id) -> Result<Option<Vec<u8>>, TraitStoreError> {
        self.read(|store| Ok(store.stored_body(&Hash::from_bytes(*id))?))
    }

    /// Decides a batch of received events with their bodies in one write transaction,
    /// under the write lock, taken for this one call. See
    /// [`EventStore::ingest_events_with_bodies`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb error or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub fn range_ingest_with_bodies(
        &self,
        events: Vec<(Event, Option<Vec<u8>>)>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, TraitStoreError> {
        let mut store = self
            .inner
            .write()
            .map_err(|_err| TraitStoreError::Poisoned)?;
        Ok(store.ingest_events_with_bodies(events, now_ms)?)
    }

    /// Checks the store's own rows. See [`EventStore::check_invariants`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for the first mismatch, or a redb error.
    pub fn check_invariants(&self) -> Result<(), TraitStoreError> {
        self.read(|store| Ok(store.check_invariants()?))
    }

    /// Runs `op` under the read lock. It answers the queries the traits do not cover, such as the
    /// applied state and the roster of a group.
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock, or
    /// whatever `op` returns.
    pub fn read<T>(
        &self,
        op: impl FnOnce(&S) -> Result<T, TraitStoreError>,
    ) -> Result<T, TraitStoreError> {
        let store = self
            .inner
            .read()
            .map_err(|_err| TraitStoreError::Poisoned)?;
        op(&store)
    }

    /// The newest applied entry of `author`.
    fn latest_entry(&self, author: &VerifyingKey) -> Result<Option<AnyOperation>, TraitStoreError> {
        let author = *author.as_bytes();
        let latest = self.read(|store| {
            let Some(height) = applied_height(store, &author)? else {
                return Ok(None);
            };
            Ok(store.event(&author, height)?)
        })?;
        latest.as_ref().map(operation_of).transpose()
    }

    /// The applied height of `author` in each requested log.
    fn heights(
        &self,
        author: &VerifyingKey,
        logs: &[ChainLog],
    ) -> Result<Option<BTreeMap<ChainLog, SeqNum>>, TraitStoreError> {
        let author = *author.as_bytes();
        let height = self.read(|store| Ok(applied_height(store, &author)?))?;
        Ok(match height {
            Some(height) if !logs.is_empty() => {
                Some(logs.iter().map(|log| (*log, height)).collect())
            }
            _ => None,
        })
    }

    /// The count and byte total of `author`'s applied events in `(after, until]`.
    fn log_size(
        &self,
        author: &VerifyingKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Result<Option<(u32, u32)>, TraitStoreError> {
        let author = *author.as_bytes();
        // One read lock for the whole sum, so the count and the bytes come from one state.
        self.read(|store| {
            if applied_height(store, &author)?.is_none() {
                return Ok(None);
            }
            let (mut count, mut bytes) = (0u32, 0u32);
            let mut cursor = after;
            loop {
                let page = store.applied_events(&author, cursor, until, PAGE)?;
                let Some(last) = page.last() else {
                    break;
                };
                for event in &page {
                    count = count.checked_add(1).ok_or(TraitStoreError::Overflow)?;
                    let size = u32::try_from(event.bytes.len())
                        .ok()
                        .and_then(|len| len.checked_add(event.header.payload_size))
                        .ok_or(TraitStoreError::Overflow)?;
                    bytes = bytes.checked_add(size).ok_or(TraitStoreError::Overflow)?;
                }
                cursor = Some(last.header.seq_num);
                if page.len() < PAGE {
                    break;
                }
            }
            Ok(Some((count, bytes)))
        })
    }

    /// The authors with an applied event in `topic`, each with its one log. An event past a fork
    /// point does not count.
    fn authors_in(
        &self,
        topic: &Hash,
    ) -> Result<BTreeMap<VerifyingKey, Vec<ChainLog>>, TraitStoreError> {
        let authors = self.read(|store| {
            let mut authors = BTreeSet::new();
            for (author, seq) in store.group_members(topic)? {
                let applied = applied_height(store, &author)?;
                if applied.is_some_and(|height| seq <= height) {
                    authors.insert(author);
                }
            }
            Ok(authors)
        })?;
        let mut resolved = BTreeMap::new();
        for author in authors {
            resolved.insert(VerifyingKey::from_bytes(&author)?, vec![ChainLog]);
        }
        Ok(resolved)
    }
}

impl<S: EventStoreBackend> BlockList for SharedEventStore<S> {
    /// Whether (`kind`, `value`) is on the store's operator block list
    /// and still enforced, under the read lock, taken for this one call.
    /// The wall clock judges expiry, and a read error or a poisoned lock
    /// invents no block.
    fn is_blocked(&self, kind: BlockKind, value: &[u8]) -> bool {
        // The bool seam fails open (see the trait docs): a read error
        // or a poisoned lock invents no block. `lookup` preserves the
        // failure for the seams that distinguish it (#8750).
        self.lookup(kind, value).is_blocked()
    }

    fn lookup(&self, kind: BlockKind, value: &[u8]) -> BlockLookup {
        let answered =
            self.read(|store| Ok(store.is_blocked_at(kind, value, id_core::time::unix_ms())?));
        match answered {
            Ok(true) => BlockLookup::Blocked,
            Ok(false) => BlockLookup::Unblocked,
            Err(TraitStoreError::Store(err)) => {
                let failure = if matches!(err, StoreError::Corrupt(_)) {
                    BlockLookupFailure::Corrupt
                } else {
                    BlockLookupFailure::Storage
                };
                tracing::warn!("block-list lookup failed ({}): {err}", failure.as_str());
                BlockLookup::Indeterminate(failure)
            }
            Err(err) => {
                let failure = BlockLookupFailure::Unavailable;
                tracing::warn!("block-list lookup failed ({}): {err}", failure.as_str());
                BlockLookup::Indeterminate(failure)
            }
        }
    }
}

/// The sequence number of the author's last applied event, or `None` when none applies.
fn applied_height<S: EventStoreBackend>(
    store: &S,
    author: &AuthorKey,
) -> Result<Option<SeqNum>, StoreError> {
    let Some(chain) = store.chain(author)? else {
        return Ok(None);
    };
    Ok(chain.next_seq.checked_sub(1))
}

/// The operation the sync protocol sends for a stored event: its header bytes, with no body.
/// The body is not part of an event, and only its size is declared in the header.
fn operation_of(event: &Event) -> Result<AnyOperation, TraitStoreError> {
    let raw: RawOperation = (event.bytes.clone(), None);
    Ok(AnyOperation::try_from(raw)?)
}

/// One author's log, streamed a page at a time.
pub struct LogEntries<S> {
    store: SharedEventStore<S>,
    author: AuthorKey,
    /// Entries at or below this sequence number have been read. `None` means none have.
    after: Option<SeqNum>,
    until: Option<SeqNum>,
    page_size: usize,
    /// The current page, not yet streamed.
    page: VecDeque<Event>,
    /// Set after the last entry or after an error, so the stream ends.
    done: bool,
}

impl<S: EventStoreBackend> LogEntries<S> {
    pub const fn new(
        store: SharedEventStore<S>,
        author: AuthorKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
        page_size: usize,
    ) -> Self {
        Self {
            store,
            author,
            after,
            until,
            page_size,
            page: VecDeque::new(),
            done: false,
        }
    }

    /// Reads the page after the cursor and moves the cursor past it. An empty page is the end.
    fn next_page(&mut self) -> Result<VecDeque<Event>, TraitStoreError> {
        let (author, after, until, page_size) =
            (self.author, self.after, self.until, self.page_size);
        let page = self
            .store
            .read(|store| Ok(store.applied_events(&author, after, until, page_size)?))?;
        if let Some(first) = page.first() {
            // A page must start after the cursor. Otherwise the same page would come back forever.
            if after.is_some_and(|after| first.header.seq_num <= after) {
                return Err(TraitStoreError::Stalled);
            }
        }
        if let Some(last) = page.last() {
            self.after = Some(last.header.seq_num);
        }
        Ok(page.into())
    }
}

impl<S: EventStoreBackend> Stream for LogEntries<S> {
    type Item = Result<LogEntry<AnyOperation, ChainLog>, TraitStoreError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if this.page.is_empty() {
            match this.next_page() {
                Ok(page) => this.page = page,
                Err(err) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(err)));
                }
            }
        }
        let Some(event) = this.page.pop_front() else {
            this.done = true;
            return Poll::Ready(None);
        };
        match operation_of(&event) {
            Ok(entry) => Poll::Ready(Some(Ok(LogEntry {
                entry,
                log_id: ChainLog,
                bytes: event.bytes,
            }))),
            Err(err) => {
                this.done = true;
                Poll::Ready(Some(Err(err)))
            }
        }
    }
}

impl<S: EventStoreBackend> LogStore<AnyOperation, VerifyingKey, ChainLog, SeqNum, Hash>
    for SharedEventStore<S>
{
    type Error = TraitStoreError;

    /// The newest applied entry. The log id is ignored: an author has one log.
    ///
    /// The `_tx` name is the upstream `LogStore` requirement; this redb store has no separate
    /// transaction permit, so the read is served directly by `latest_entry`.
    fn get_latest_entry_tx(
        &self,
        author: &VerifyingKey,
        _log_id: &ChainLog,
    ) -> impl Future<Output = Result<Option<AnyOperation>, TraitStoreError>> {
        ready(self.latest_entry(author))
    }

    /// The applied height of each requested log. `None` when no log was requested, or when the
    /// author has no applied event.
    fn get_log_heights(
        &self,
        author: &VerifyingKey,
        logs: &[ChainLog],
    ) -> impl Future<Output = Result<Option<BTreeMap<ChainLog, SeqNum>>, TraitStoreError>> {
        ready(self.heights(author, logs))
    }

    /// The count and byte total of the applied events in `(after, until]`. The bytes are the
    /// header bytes plus the body size each header declares, as the SQLite store counts them.
    /// `None` when the author has no applied event. An empty range is `Some((0, 0))`.
    fn get_log_size(
        &self,
        author: &VerifyingKey,
        _log_id: &ChainLog,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> impl Future<Output = Result<Option<(u32, u32)>, TraitStoreError>> {
        ready(self.log_size(author, after, until))
    }

    /// The applied entries in `(after, until]`, in sequence order. Each is streamed a page at a
    /// time, and each page takes the read lock once.
    fn log_entries(
        &self,
        author: &VerifyingKey,
        _log_id: &ChainLog,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Result<LogStream<AnyOperation, ChainLog, TraitStoreError>, TraitStoreError> {
        let entries = LogEntries::new(self.clone(), *author.as_bytes(), after, until, PAGE);
        Ok(Box::pin(entries))
    }

    /// Not implemented. Pruning has an open design question: whether it removes whole events or
    /// only their bodies (task 10).
    fn prune_entries_tx(
        &self,
        _author: &VerifyingKey,
        _log_id: &ChainLog,
        _until: &SeqNum,
    ) -> impl Future<Output = Result<u64, TraitStoreError>> {
        ready(Err(TraitStoreError::Unsupported(
            "pruning is an open design question (task 10): headers or only bodies",
        )))
    }
}

impl<S: EventStoreBackend> TopicStore<Hash, VerifyingKey, ChainLog> for SharedEventStore<S> {
    type Error = TraitStoreError;

    /// Not implemented. A topic's membership is each event's group, which ingest writes, so there
    /// is no association table to add to.
    fn associate_tx(
        &self,
        _topic: &Hash,
        _author: &VerifyingKey,
        _data_id: &ChainLog,
    ) -> impl Future<Output = Result<bool, TraitStoreError>> {
        ready(Err(TraitStoreError::Unsupported(
            "a topic's membership is each event's group, written by ingest",
        )))
    }

    /// Not implemented, for the same reason as `associate_tx`.
    fn remove_tx(
        &self,
        _topic: &Hash,
        _author: &VerifyingKey,
        _data_id: &ChainLog,
    ) -> impl Future<Output = Result<bool, TraitStoreError>> {
        ready(Err(TraitStoreError::Unsupported(
            "a topic's membership is each event's group, written by ingest",
        )))
    }

    /// The authors with an applied event in the group, each with its one log. An event past a
    /// fork point does not count.
    fn resolve(
        &self,
        topic: &Hash,
    ) -> impl Future<Output = Result<BTreeMap<VerifyingKey, Vec<ChainLog>>, TraitStoreError>> {
        ready(self.authors_in(topic))
    }

    /// Not implemented. `TopicLogSync` resolves from a topic, not to topics.
    fn resolve_topics(
        &self,
        _author: &VerifyingKey,
        _data_id: &ChainLog,
    ) -> impl Future<Output = Result<Vec<Hash>, TraitStoreError>> {
        ready(Err(TraitStoreError::Unsupported(
            "reverse topic lookup is not needed by `TopicLogSync`",
        )))
    }

    /// Not implemented, for the same reason as `resolve_topics`.
    fn topics(&self) -> impl Future<Output = Result<Vec<Hash>, TraitStoreError>> {
        ready(Err(TraitStoreError::Unsupported(
            "listing all topics is not needed by `TopicLogSync`",
        )))
    }
}
