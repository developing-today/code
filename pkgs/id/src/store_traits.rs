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
use crate::event_store::{EventStore, Ingest, StoreError};
use crate::waiting::AuthorKey;

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
#[derive(Clone, Debug)]
pub struct SharedEventStore {
    inner: Arc<RwLock<EventStore>>,
}

impl SharedEventStore {
    /// Shares `store` between the clones of this handle.
    pub fn new(store: EventStore) -> Self {
        Self {
            inner: Arc::new(RwLock::new(store)),
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
    pub(crate) fn range_entries(
        &self,
    ) -> Result<
        Vec<(
            crate::event_store::range::Key,
            crate::event_store::range::Id,
        )>,
        TraitStoreError,
    > {
        self.read(|store| Ok(store.sync_entries()?))
    }

    /// The bytes of the event with this ID, under the read lock, taken for this one
    /// call. See [`EventStore::sync_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for a redb or corruption error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub(crate) fn range_bytes(
        &self,
        id: &crate::event_store::range::Id,
    ) -> Result<Option<Vec<u8>>, TraitStoreError> {
        self.read(|store| Ok(store.sync_bytes(id)?))
    }

    /// Decides a batch of received events in one write transaction, under the write
    /// lock, taken for this one call. See [`EventStore::ingest_events`].
    ///
    /// # Errors
    ///
    /// Returns [`TraitStoreError::Store`] for the first mismatch, or a redb error, and
    /// [`TraitStoreError::Poisoned`] if another thread panicked while it held the lock.
    pub(crate) fn range_ingest_events(
        &self,
        events: Vec<Event>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, TraitStoreError> {
        let mut store = self
            .inner
            .write()
            .map_err(|_err| TraitStoreError::Poisoned)?;
        Ok(store.ingest_events(events, now_ms)?)
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
        op: impl FnOnce(&EventStore) -> Result<T, TraitStoreError>,
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

/// The sequence number of the author's last applied event, or `None` when none applies.
fn applied_height(store: &EventStore, author: &AuthorKey) -> Result<Option<SeqNum>, StoreError> {
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
struct LogEntries {
    store: SharedEventStore,
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

impl LogEntries {
    const fn new(
        store: SharedEventStore,
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

impl Stream for LogEntries {
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

impl LogStore<AnyOperation, VerifyingKey, ChainLog, SeqNum, Hash> for SharedEventStore {
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

impl TopicStore<Hash, VerifyingKey, ChainLog> for SharedEventStore {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use futures_lite::StreamExt;
    use p2panda_core::SigningKey;

    use super::*;
    use crate::chain::{ChainExt, sign};
    use crate::waiting::WaitingLimits;

    /// One `[[package]]` entry of the lock: its name and the names it
    /// depends on. Dependency entries are `"name"`, `"name version"` or
    /// `"name version (source)"`; the name is the first word.
    fn lock_packages(lock: &str) -> Vec<(String, Vec<String>)> {
        let mut packages = Vec::new();
        let mut current: Option<(String, Vec<String>)> = None;
        let mut in_deps = false;
        for raw in lock.lines() {
            let line = raw.trim();
            if line.starts_with("[[") {
                in_deps = false;
                if let Some(package) = current.take() {
                    packages.push(package);
                }
            } else if current.is_none() {
                if let Some(rest) = line.strip_prefix("name = \"") {
                    current = Some((rest.trim_end_matches('"').to_owned(), Vec::new()));
                }
            } else if line == "dependencies = [" {
                in_deps = true;
            } else if in_deps && line == "]" {
                in_deps = false;
            } else if in_deps {
                if let Some(entry) = line.strip_suffix(',') {
                    let entry = entry.trim_matches('"');
                    if let Some(name) = entry.split(' ').next() {
                        current.as_mut().unwrap().1.push(name.to_owned());
                    }
                }
            }
        }
        if let Some(package) = current.take() {
            packages.push(package);
        }
        packages
    }

    /// Whether a manifest section header opens a dev-dependency section
    /// (`Some(true)`), a shipped one — normal or build (`Some(false)`) — or
    /// neither. Plain, target-specific and per-crate subtable spellings all
    /// count: `[dev-dependencies]`, `[target.'cfg(..)'.dependencies]`,
    /// `[dependencies.rusqlite]`.
    fn dep_section_dev(section: &str) -> Option<bool> {
        let tail = section.rsplit('.').next().unwrap_or(section);
        match tail {
            "dev-dependencies" => Some(true),
            "dependencies" | "build-dependencies" => Some(false),
            _ if section.starts_with("dev-dependencies.") => Some(true),
            _ if section.starts_with("dependencies.")
                || section.starts_with("build-dependencies.") =>
            {
                Some(false)
            }
            _ => None,
        }
    }

    /// The crate names a manifest declares in dependency sections of one
    /// class (`dev` selects dev-dependencies, otherwise the shipped ones).
    /// A `package = "..."` rename counts under the real name.
    fn manifest_dep_names(manifest: &str, dev: bool) -> Vec<String> {
        let mut names = Vec::new();
        let mut in_class = false;
        for raw in manifest.lines() {
            let line = raw.trim();
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section = line.trim_matches(|c| c == '[' || c == ']');
                in_class = dep_section_dev(section) == Some(dev);
                if in_class {
                    // A per-crate subtable names the crate in the header:
                    // `[dependencies.rusqlite]`.
                    if let Some((_, name)) = section.rsplit_once('.')
                        && !matches!(
                            name,
                            "dependencies" | "build-dependencies" | "dev-dependencies"
                        )
                    {
                        names.push(name.to_owned());
                    }
                }
                continue;
            }
            if !in_class {
                continue;
            }
            if let Some((key, _)) = line.split_once('=') {
                let key = key.trim().trim_matches('"');
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                {
                    names.push(key.to_owned());
                }
            }
            if let Some((_, rest)) = line.split_once("package = \"")
                && let Some(name) = rest.split('"').next()
            {
                names.push(name.to_owned());
            }
        }
        names
    }

    /// The `[package]` name of a manifest.
    fn manifest_package_name(manifest: &str) -> Option<String> {
        let mut in_package = false;
        for raw in manifest.lines() {
            let line = raw.trim();
            if line.starts_with('[') {
                in_package = line == "[package]";
                continue;
            }
            if in_package && let Some(rest) = line.strip_prefix("name = \"") {
                return Some(rest.trim_end_matches('"').to_owned());
            }
        }
        None
    }

    /// The shipped default build has no SQLite, with `LogSync` wired in. p2panda-sync's store
    /// dependency defaults to SQLite, so the vendored copy turns that default off
    /// (`vendor/README.md`).
    ///
    /// Cargo.lock records optional dependencies even when they are inactive, so its raw package
    /// edges cannot prove what ships. Keep checking for unexpected SQLite-family edges (allowing
    /// p2panda-store's optional `sqlx` edge), then ask Cargo for the shipped default-feature graph.
    /// The root manifest must still declare SQLite names only as dev-dependencies. This catches
    /// a shipped SQLite path, a root manifest regression, a missing p2panda-sync package, or the
    /// vendored line losing its `default-features = false`.
    #[test]
    fn test_the_lock_has_no_sqlite_with_logsync_wired() {
        const SQLITE: [&str; 4] = ["sqlx", "sqlite", "rusqlite", "libsqlite3-sys"];
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let lock = std::fs::read_to_string(format!("{manifest_dir}/Cargo.lock")).unwrap();
        let packages = lock_packages(&lock);
        assert!(
            packages.iter().any(|(name, _)| name == "p2panda-sync"),
            "Cargo.lock does not name p2panda-sync: LogSync is not wired in"
        );

        let manifest = std::fs::read_to_string(format!("{manifest_dir}/Cargo.toml")).unwrap();
        let root = manifest_package_name(&manifest).unwrap();
        // This package may declare SQLite names only as dev-dependencies.
        let shipped = manifest_dep_names(&manifest, false);
        for name in SQLITE {
            assert!(
                !shipped.iter().any(|declared| declared.as_str() == name),
                "Cargo.toml declares {name} outside dev-dependencies: SQLite is back in the \
                 shipped build (see vendor/README.md)"
            );
        }
        // Cargo.lock includes optional dependency edges. p2panda-store declares optional `sqlx`,
        // which is inactive in the shipped default-feature graph checked below.
        for (name, deps) in &packages {
            for dep in deps {
                if SQLITE.contains(&dep.as_str()) {
                    assert!(
                        SQLITE.contains(&name.as_str())
                            || name == &root
                            || (name == "p2panda-store" && dep == "sqlx"),
                        "{name} depends on {dep}: SQLite is back in the shipped build (see \
                         vendor/README.md)"
                    );
                }
            }
        }

        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let tree = std::process::Command::new(cargo)
            .current_dir(manifest_dir)
            .args([
                "tree",
                "--locked",
                "--package",
                "id",
                "--edges",
                "normal",
                "--no-default-features",
                "--features",
                "sandbox,world",
                "--prefix",
                "none",
                "--format",
                "{p}",
            ])
            .output()
            .expect("run cargo tree for the shipped default feature set");
        assert!(
            tree.status.success(),
            "cargo tree failed: {}",
            String::from_utf8_lossy(&tree.stderr)
        );
        let active_packages = String::from_utf8(tree.stdout).unwrap();
        for package in active_packages
            .lines()
            .filter_map(|line| line.split_whitespace().next())
        {
            assert!(
                !SQLITE.contains(&package),
                "{package} is active in the shipped default-feature graph (see vendor/README.md)"
            );
        }

        let vendored =
            std::fs::read_to_string(format!("{manifest_dir}/vendor/p2panda-sync/Cargo.toml"))
                .unwrap();
        let store_line = vendored
            .lines()
            .find(|line| line.starts_with("p2panda-store = {"))
            .unwrap();
        assert!(
            store_line.contains("default-features = false"),
            "the vendored p2panda-store keeps its SQLite default: {store_line}"
        );
    }

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author_of(key: &SigningKey) -> VerifyingKey {
        key.verifying_key()
    }

    fn group(n: u8) -> Hash {
        Hash::digest([n])
    }

    /// One signed event of `signer`'s chain, with an optional body.
    fn event(
        signer: &SigningKey,
        seq: SeqNum,
        backlink: Option<Hash>,
        group: Option<Hash>,
        kind: &str,
        body: Option<&[u8]>,
    ) -> Event {
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
        };
        sign(signer, seq, backlink, body, ext).unwrap()
    }

    /// The first `count` events of `signer`'s chain, all in `group`, each naming the one before.
    fn chain_of(signer: &SigningKey, count: SeqNum, group: Option<Hash>) -> Vec<Event> {
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..count {
            let backlink = events.last().map(|event| event.id);
            events.push(event(signer, seq, backlink, group, "message", None));
        }
        events
    }

    fn open() -> (tempfile::TempDir, SharedEventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, SharedEventStore::new(store))
    }

    fn ingest_all(store: &SharedEventStore, events: &[Event]) {
        for event in events {
            let outcome = store.ingest(&event.bytes, None, 0).unwrap();
            assert_eq!(
                outcome,
                Ingest::Accepted {
                    id: event.id,
                    released: Vec::new(),
                }
            );
        }
    }

    /// The `(count, bytes)` that the trait must report for `events`: header bytes plus the
    /// declared body sizes.
    fn sized(events: &[Event]) -> (u32, u32) {
        let count = u32::try_from(events.len()).unwrap();
        let bytes = events
            .iter()
            .map(|event| u32::try_from(event.bytes.len()).unwrap() + event.header.payload_size)
            .sum();
        (count, bytes)
    }

    async fn drain<S>(mut stream: S) -> Vec<LogEntry<AnyOperation, ChainLog>>
    where
        S: Stream<Item = Result<LogEntry<AnyOperation, ChainLog>, TraitStoreError>> + Unpin,
    {
        let mut entries = Vec::new();
        while let Some(entry) = stream.next().await {
            entries.push(entry.unwrap());
        }
        entries
    }

    async fn entries_of(
        store: &SharedEventStore,
        who: &VerifyingKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Vec<LogEntry<AnyOperation, ChainLog>> {
        drain(store.log_entries(who, &ChainLog, after, until).unwrap()).await
    }

    /// The streamed entry is the stored event, in the form the sync protocol sends.
    fn assert_is_event(entry: &LogEntry<AnyOperation, ChainLog>, event: &Event) {
        assert_eq!(entry.log_id, ChainLog);
        assert_eq!(entry.bytes, event.bytes);
        assert_eq!(entry.entry.hash, event.id);
        assert_eq!(entry.entry.header.seq_num, event.header.seq_num);
        assert_eq!(entry.entry.header.verifying_key, event.header.verifying_key);
        assert_eq!(entry.entry.header.backlink, event.header.backlink);
        assert!(
            entry.entry.body.is_none(),
            "bodies are not sent with events"
        );
    }

    #[tokio::test]
    async fn test_an_empty_store_has_no_log_for_any_author() {
        let (_dir, store) = open();
        let who = author_of(&key(1));
        assert_eq!(
            store.get_latest_entry_tx(&who, &ChainLog).await.unwrap(),
            None
        );
        assert_eq!(
            store.get_log_heights(&who, &[ChainLog]).await.unwrap(),
            None
        );
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, None)
                .await
                .unwrap(),
            None
        );
        assert!(entries_of(&store, &who, None, None).await.is_empty());
        assert_eq!(store.resolve(&group(1)).await.unwrap(), BTreeMap::new());
        store.check_invariants().unwrap();
    }

    #[tokio::test]
    async fn test_the_latest_entry_and_heights_follow_the_chain() {
        let (_dir, store) = open();
        let (alice, bob) = (key(2), key(3));
        let (who_alice, who_bob) = (author_of(&alice), author_of(&bob));
        let alice_events = chain_of(&alice, 5, Some(group(1)));
        let bob_events = chain_of(&bob, 2, Some(group(2)));
        ingest_all(&store, &alice_events);
        ingest_all(&store, &bob_events);

        let latest = store
            .get_latest_entry_tx(&who_alice, &ChainLog)
            .await
            .unwrap()
            .expect("alice has an entry");
        assert_eq!(latest.hash, alice_events[4].id);
        assert_eq!(latest.header.seq_num, 4);
        assert_eq!(latest.header.backlink, Some(alice_events[3].id));

        assert_eq!(
            store
                .get_log_heights(&who_alice, &[ChainLog])
                .await
                .unwrap(),
            Some(BTreeMap::from([(ChainLog, 4)]))
        );
        assert_eq!(
            store.get_log_heights(&who_bob, &[ChainLog]).await.unwrap(),
            Some(BTreeMap::from([(ChainLog, 1)]))
        );
        // Asking for no logs asks for nothing, and asking twice for one log names it once.
        assert_eq!(store.get_log_heights(&who_alice, &[]).await.unwrap(), None);
        assert_eq!(
            store
                .get_log_heights(&who_alice, &[ChainLog, ChainLog])
                .await
                .unwrap(),
            Some(BTreeMap::from([(ChainLog, 4)]))
        );

        assert_eq!(
            store.resolve(&group(1)).await.unwrap(),
            BTreeMap::from([(who_alice, vec![ChainLog])])
        );
        assert_eq!(
            store.resolve(&group(2)).await.unwrap(),
            BTreeMap::from([(who_bob, vec![ChainLog])])
        );
        store.check_invariants().unwrap();
    }

    #[tokio::test]
    async fn test_the_log_size_counts_headers_and_declared_bodies() {
        let (_dir, store) = open();
        let alice = key(4);
        let who = author_of(&alice);
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..5 {
            let backlink = events.last().map(|event| event.id);
            // Every body is non-empty, and the lengths differ, so the sum is not a count.
            let body = vec![7u8; 1 + 10 * usize::try_from(seq).unwrap()];
            events.push(event(
                &alice,
                seq,
                backlink,
                Some(group(1)),
                "message",
                Some(&body),
            ));
        }
        ingest_all(&store, &events);

        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, None)
                .await
                .unwrap(),
            Some(sized(&events))
        );
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, Some(0), Some(3))
                .await
                .unwrap(),
            Some(sized(&events[1..=3]))
        );
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, Some(100))
                .await
                .unwrap(),
            Some(sized(&events))
        );
        // An empty range of an author who has events is zero, not absent.
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, Some(4), None)
                .await
                .unwrap(),
            Some((0, 0))
        );
        assert_eq!(
            store
                .get_log_size(&author_of(&key(9)), &ChainLog, None, None)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn test_log_entries_stream_each_position_once_in_order() {
        let (_dir, store) = open();
        let alice = key(5);
        let who = author_of(&alice);
        let events = chain_of(&alice, 5, None);
        ingest_all(&store, &events);
        let stream =
            |after, until| LogEntries::new(store.clone(), *who.as_bytes(), after, until, 2);

        // A page of two makes the stream cross page boundaries.
        let all = drain(stream(None, None)).await;
        assert_eq!(all.len(), events.len());
        for (entry, event) in all.iter().zip(&events) {
            assert_is_event(entry, event);
        }
        let middle = drain(stream(Some(1), Some(3))).await;
        assert_eq!(middle.len(), 2, "after 1 is exclusive, until 3 inclusive");
        assert_is_event(&middle[0], &events[2]);
        assert_is_event(&middle[1], &events[3]);
        assert!(drain(stream(Some(4), None)).await.is_empty());
        let first = drain(stream(None, Some(0))).await;
        assert_eq!(first.len(), 1);
        assert_is_event(&first[0], &events[0]);
        assert_eq!(entries_of(&store, &who, Some(1), Some(3)).await.len(), 2);
    }

    #[tokio::test]
    async fn test_a_log_longer_than_one_page_is_streamed_whole() {
        let (_dir, store) = open();
        let alice = key(6);
        let who = author_of(&alice);
        // Three pages: 64, 64 and 3.
        let events = chain_of(&alice, 131, None);
        ingest_all(&store, &events);

        let entries = entries_of(&store, &who, None, None).await;
        assert_eq!(entries.len(), events.len());
        for (entry, event) in entries.iter().zip(&events) {
            assert_is_event(entry, event);
        }
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, None)
                .await
                .unwrap()
                .map(|(count, _bytes)| count),
            Some(131)
        );
        store.check_invariants().unwrap();
    }

    #[tokio::test]
    async fn test_a_topic_resolves_to_the_authors_applied_in_it() {
        let (_dir, store) = open();
        let (alice, bob, carol) = (key(7), key(8), key(9));
        let (topic, other_topic) = (group(1), group(2));
        ingest_all(&store, &chain_of(&alice, 3, Some(topic)));
        ingest_all(&store, &chain_of(&bob, 1, Some(topic)));
        ingest_all(&store, &chain_of(&carol, 2, Some(other_topic)));
        let (who_alice, who_bob, who_carol) =
            (author_of(&alice), author_of(&bob), author_of(&carol));

        assert_eq!(
            store.resolve(&topic).await.unwrap(),
            BTreeMap::from([(who_alice, vec![ChainLog]), (who_bob, vec![ChainLog])])
        );
        assert_eq!(
            store.resolve(&other_topic).await.unwrap(),
            BTreeMap::from([(who_carol, vec![ChainLog])])
        );
        assert!(store.resolve(&group(3)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_after_a_fork_only_the_applied_prefix_is_visible() {
        let (_dir, store) = open();
        let (alice, bob) = (key(10), key(11));
        let (topic, rival_topic) = (group(4), group(5));
        let who = author_of(&alice);
        // Alice: seq 0 outside any group, then seq 1 and seq 2 in `topic`.
        let first = event(&alice, 0, None, None, "message", None);
        let second = event(&alice, 1, Some(first.id), Some(topic), "message", None);
        let third = event(&alice, 2, Some(second.id), Some(topic), "message", None);
        ingest_all(&store, std::slice::from_ref(&first));
        ingest_all(&store, &[second.clone(), third.clone()]);
        // Alice's rival at seq 1: the same backlink as `second`, so it conflicts with it.
        let rival = event(&alice, 1, Some(first.id), Some(rival_topic), "rival", None);
        assert_eq!(
            store.ingest(&rival.bytes, None, 0).unwrap(),
            Ingest::Fork {
                author: *who.as_bytes(),
                seq: 1,
            }
        );
        ingest_all(&store, &chain_of(&bob, 1, Some(topic)));

        // Only seq 0 is applied. Seq 1 and seq 2 are stored, but they are not part of the log.
        let latest = store
            .get_latest_entry_tx(&who, &ChainLog)
            .await
            .unwrap()
            .expect("seq 0 is applied");
        assert_eq!(latest.hash, first.id);
        assert_eq!(
            store.get_log_heights(&who, &[ChainLog]).await.unwrap(),
            Some(BTreeMap::from([(ChainLog, 0)]))
        );
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, None)
                .await
                .unwrap(),
            Some(sized(std::slice::from_ref(&first)))
        );
        let entries = entries_of(&store, &who, None, None).await;
        assert_eq!(entries.len(), 1);
        assert_is_event(&entries[0], &first);
        // A bound past the applied prefix does not reveal the events stored beyond the fork.
        assert_eq!(
            store
                .get_log_size(&who, &ChainLog, None, Some(100))
                .await
                .unwrap(),
            Some(sized(std::slice::from_ref(&first)))
        );
        assert_eq!(entries_of(&store, &who, None, Some(100)).await.len(), 1);
        // Alice's events in `topic` are past the fork, so she is not in it. Bob still is.
        assert_eq!(
            store.resolve(&topic).await.unwrap(),
            BTreeMap::from([(author_of(&bob), vec![ChainLog])])
        );
        store.check_invariants().unwrap();
    }

    #[tokio::test]
    async fn test_the_methods_outside_the_sync_path_refuse_with_a_reason() {
        let (_dir, store) = open();
        let who = author_of(&key(12));
        let topic = group(6);
        // `get_latest_entry_tx` is the real read path now (no transaction permit on redb), so it
        // serves the empty store as `None`, not an error.
        assert_eq!(
            store.get_latest_entry_tx(&who, &ChainLog).await.unwrap(),
            None
        );
        assert!(matches!(
            store.prune_entries_tx(&who, &ChainLog, &0).await,
            Err(TraitStoreError::Unsupported(_))
        ));
        assert!(matches!(
            store.associate_tx(&topic, &who, &ChainLog).await,
            Err(TraitStoreError::Unsupported(_))
        ));
        assert!(matches!(
            store.remove_tx(&topic, &who, &ChainLog).await,
            Err(TraitStoreError::Unsupported(_))
        ));
        assert!(matches!(
            store.resolve_topics(&who, &ChainLog).await,
            Err(TraitStoreError::Unsupported(_))
        ));
        assert!(matches!(
            store.topics().await,
            Err(TraitStoreError::Unsupported(_))
        ));
    }

    #[tokio::test]
    async fn test_clones_share_one_store() {
        let (_dir, store) = open();
        let other = store.clone();
        let alice = key(13);
        ingest_all(&store, &chain_of(&alice, 2, None));
        assert_eq!(
            other
                .get_log_heights(&author_of(&alice), &[ChainLog])
                .await
                .unwrap(),
            Some(BTreeMap::from([(ChainLog, 1)]))
        );
    }

    #[tokio::test]
    #[allow(clippy::panic)]
    async fn test_a_poisoned_lock_is_reported_not_unwrapped() {
        let (_dir, store) = open();
        let poisoner = store.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = poisoner.inner.write().unwrap();
            panic!("poisoning the store lock on purpose");
        })
        .join();
        assert!(panicked.is_err());
        let who = author_of(&key(14));
        assert!(matches!(
            store.get_log_heights(&who, &[ChainLog]).await,
            Err(TraitStoreError::Poisoned)
        ));
        assert!(matches!(
            store.ingest(&[], None, 0),
            Err(TraitStoreError::Poisoned)
        ));
    }
}
