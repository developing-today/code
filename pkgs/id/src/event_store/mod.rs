//! The event store: every verified event, the applied state of each author's
//! chain, and the fork proofs, in one redb file.
//!
//! The store keeps only what [`chain::decode`] has verified. Its tables:
//!
//! - `events`: an author's `(author, seq)` position to the event's bytes. Every
//!   accepted event is here, including events past a fork point, which are kept
//!   but not applied.
//! - `hashes`: an event's ID to its position.
//! - `chains`: an author to the applied prefix of their chain: the next sequence
//!   number, the hash of the last applied event, and the frozen flag.
//! - `forks`: an author to the fork proof, the two headers that conflict at the
//!   lowest sequence number where the author has forked.
//! - `groups`: a group hash to the positions of the events in that group.
//! - `meta`: the store's format.
//!
//! A position key is the author's 32 bytes followed by the sequence number in
//! big-endian, so one author's events sort in sequence order.
//!
//! # Creating the file
//!
//! A new store is built under [`TEMP_NAME`]: its tables are committed, the file
//! is synced, and only then is it renamed to [`FILE_NAME`], with the directory
//! synced before and after the rename. A crash leaves either no store file or a
//! complete one. Creating the file in place would let a crash during the first
//! commit leave a file that redb refuses to open, so the store could never start.
//! The crash model in `tests/crash/store.rs` checks every step of this sequence.
//!
//! # Where the files are
//!
//! [`EventStore::open`] takes a directory. [`Storage`] is the seam between the
//! store and its files: [`FileStorage`] is the disk, and the crash model in tests
//! implements the same trait over a simulated directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use p2panda_core::{Hash, SeqNum};
use redb::{
    Database, DatabaseError, MultimapTableDefinition, ReadableDatabase, ReadableTable,
    StorageError, TableDefinition, WriteTransaction,
};

use crate::chain::{self, Event};

mod ingest;

use crate::waiting::{AuthorKey, WaitingLimits, WaitingPool};
pub use ingest::{Ingest, Refusal};

/// The store's file name in its directory.
pub const FILE_NAME: &str = "events.redb";

/// Where a store is built before it is renamed to [`FILE_NAME`]. A file under
/// this name is never trusted: it is removed and built again.
pub const TEMP_NAME: &str = "events.redb.tmp";

/// The format row in `meta`. A store with another format is refused.
const FORMAT: &[u8] = b"id.events.store.v1";
const FORMAT_KEY: &[u8] = b"format";

const EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("events");
const HASHES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("hashes");
const CHAINS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("chains");
const FORKS: TableDefinition<'static, &'static [u8], &'static [u8]> = TableDefinition::new("forks");
const GROUPS: MultimapTableDefinition<'static, &'static [u8], &'static [u8]> =
    MultimapTableDefinition::new("groups");
const META: TableDefinition<'static, &'static [u8], &'static [u8]> = TableDefinition::new("meta");

/// Bytes in a position key: the author, then the sequence number.
const POSITION_LEN: usize = 40;

/// Bytes in a chain row: the next sequence number, the head hash, the frozen flag.
const CHAIN_LEN: usize = 8 + 32 + 1;

/// Why the event store failed. A failure is never a refusal of an event: refused
/// events are reported as outcomes, not errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The directory could not be read or written.
    #[error("event store file I/O: {0}")]
    Io(#[from] io::Error),
    /// redb could not open or create the database.
    #[error("event store database: {0}")]
    Database(#[from] DatabaseError),
    /// A redb transaction could not begin.
    #[error("event store transaction: {0}")]
    Transaction(#[from] redb::TransactionError),
    /// A table could not be opened, or it does not exist.
    #[error("event store table: {0}")]
    Table(#[from] redb::TableError),
    /// A read or write failed inside a transaction.
    #[error("event store storage: {0}")]
    Storage(#[from] StorageError),
    /// A commit failed.
    #[error("event store commit: {0}")]
    Commit(#[from] redb::CommitError),
    /// Another process holds the store open. redb allows one opener at a time.
    #[error("{FILE_NAME} is open in another process")]
    InUse,
    /// The file is there, but it is not an event store of this format.
    #[error("{FILE_NAME} is not an event store: {0}")]
    NotAStore(String),
    /// The store's rows contradict each other. The file was changed outside the store.
    #[error("event store is corrupt: {0}")]
    Corrupt(String),
    /// The position already holds an event, so a second one cannot be stored there.
    #[error("sequence {seq} of this author is already stored")]
    Occupied {
        /// The sequence number that is taken.
        seq: SeqNum,
    },
}

fn corrupt(message: impl Into<String>) -> StoreError {
    StoreError::Corrupt(message.into())
}

/// Where a store's files live. [`FileStorage`] is a directory on disk. The crash
/// model implements this trait over a simulated directory, so every step of
/// creation and ingest can be interrupted.
pub trait Storage {
    /// Creates the named database file, or opens it if it exists.
    ///
    /// # Errors
    ///
    /// Returns redb's error if the file cannot be created or opened.
    fn create_database(&self, name: &str) -> Result<Database, DatabaseError>;

    /// Opens the named database file, which must exist.
    ///
    /// # Errors
    ///
    /// Returns redb's error if the file is missing or is not a database.
    fn open_database(&self, name: &str) -> Result<Database, DatabaseError>;

    /// Whether the named file exists.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the directory cannot be read.
    fn exists(&self, name: &str) -> io::Result<bool>;

    /// Removes the named file. A file that does not exist is not an error.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the file exists and cannot be removed.
    fn remove(&self, name: &str) -> io::Result<()>;

    /// Renames `from` to `to`, replacing `to` if it exists.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the rename fails.
    fn rename(&self, from: &str, to: &str) -> io::Result<()>;

    /// Makes the named file's contents durable.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the sync fails.
    fn sync_file(&self, name: &str) -> io::Result<()>;

    /// Makes the directory's entries (created, renamed, or removed files) durable.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the sync fails.
    fn sync_dir(&self) -> io::Result<()>;
}

/// A store directory on disk.
#[derive(Clone, Debug)]
pub struct FileStorage {
    dir: PathBuf,
}

impl FileStorage {
    /// The store in `dir`. The directory must exist.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Storage for FileStorage {
    fn create_database(&self, name: &str) -> Result<Database, DatabaseError> {
        Database::builder().create(self.path(name))
    }

    fn open_database(&self, name: &str) -> Result<Database, DatabaseError> {
        Database::builder().open(self.path(name))
    }

    fn exists(&self, name: &str) -> io::Result<bool> {
        match fs::metadata(self.path(name)) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.path(name)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        fs::rename(self.path(from), self.path(to))
    }

    fn sync_file(&self, name: &str) -> io::Result<()> {
        fs::File::open(self.path(name))?.sync_all()
    }

    fn sync_dir(&self) -> io::Result<()> {
        fs::File::open(&self.dir)?.sync_all()
    }
}

/// The applied prefix of one author's chain, as the `chains` table holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chain {
    /// The sequence number the next applied event takes: the number of applied
    /// events. Zero for an author with none.
    pub next_seq: SeqNum,
    /// The hash of the last applied event, or `None` when none applies.
    pub head: Option<Hash>,
    /// Whether the author has forked. A frozen chain applies nothing from its
    /// fork point on.
    pub frozen: bool,
}

impl Chain {
    fn encode(self) -> [u8; CHAIN_LEN] {
        let mut row = [0u8; CHAIN_LEN];
        row[..8].copy_from_slice(&u64::from(self.next_seq).to_be_bytes());
        if let Some(head) = self.head {
            row[8..40].copy_from_slice(head.as_bytes());
        }
        row[40] = u8::from(self.frozen);
        row
    }

    fn decode(row: &[u8]) -> Result<Self, StoreError> {
        let row: &[u8; CHAIN_LEN] = row
            .try_into()
            .map_err(|_err| corrupt("a chain row has the wrong length"))?;
        let next_seq = seq_num(be_u64(&row[..8])?)?;
        let head = if next_seq == 0 {
            None
        } else {
            Some(hash_of(&row[8..40])?)
        };
        let frozen = match row[40] {
            0 => false,
            1 => true,
            _ => return Err(corrupt("a chain row has an unknown frozen flag")),
        };
        Ok(Self {
            next_seq,
            head,
            frozen,
        })
    }
}

/// Two conflicting headers from one author at one sequence number. Both are
/// signed by the author, so together they prove the author forked there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkProof {
    /// The sequence number both events name.
    pub seq: SeqNum,
    /// The event with the lower ID. The order does not depend on arrival.
    pub first: Event,
    /// The event with the higher ID.
    pub second: Event,
}

/// The event store. One redb file, opened by one process at a time.
#[derive(Debug)]
pub struct EventStore {
    db: Database,
    /// Events that wait for a dependency. They are held in memory, not stored.
    waiting: WaitingPool<Hash, Event>,
}

impl EventStore {
    /// Opens the store in `dir`, creating it if the directory has none.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InUse`] if another process has it open, and
    /// [`StoreError::NotAStore`] if the file is not an event store. Neither
    /// deletes or changes the file.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        Self::open_with(&FileStorage::new(dir))
    }

    /// Opens the store on `storage`, creating it if the storage has none. The crash model
    /// passes a simulated directory here.
    ///
    /// # Errors
    ///
    /// The same errors as [`EventStore::open`].
    pub fn open_with<S: Storage>(storage: &S) -> Result<Self, StoreError> {
        let db = if storage.exists(FILE_NAME)? {
            storage.open_database(FILE_NAME).map_err(open_error)?
        } else {
            create_fresh(storage)?
        };
        verify_schema(&db)?;
        Ok(Self {
            db,
            waiting: WaitingPool::new(WaitingLimits::default()),
        })
    }

    /// Sets the caps on waiting events: how many one author may hold, and how long
    /// one may wait. The pool starts empty, so set the caps before ingesting.
    #[must_use]
    pub fn with_waiting_limits(mut self, limits: WaitingLimits) -> Self {
        self.waiting = WaitingPool::new(limits);
        self
    }

    /// Runs `op` in one write transaction. The transaction commits if `op`
    /// succeeds, and is aborted if it fails.
    ///
    /// # Errors
    ///
    /// Returns `op`'s error, or a redb error from the commit.
    #[cfg(test)]
    pub(crate) fn write<T>(
        &self,
        op: impl FnOnce(&WriteTransaction) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let txn = self.db.begin_write()?;
        let value = op(&txn)?;
        txn.commit()?;
        Ok(value)
    }

    /// The applied state of `author`'s chain, or `None` if the author has no events.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn chain(&self, author: &AuthorKey) -> Result<Option<Chain>, StoreError> {
        let txn = self.db.begin_read()?;
        let chains = txn.open_table(CHAINS)?;
        let row = chains.get(author.as_slice())?;
        row.map(|row| Chain::decode(row.value())).transpose()
    }

    /// The stored event at `(author, seq)`, applied or not.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn event(&self, author: &AuthorKey, seq: SeqNum) -> Result<Option<Event>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let stored = events.get(position_key(author, seq).as_slice())?;
        stored.map(|bytes| decode_stored(bytes.value())).transpose()
    }

    /// The stored event with this ID, applied or not.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn event_by_id(&self, id: &Hash) -> Result<Option<Event>, StoreError> {
        let txn = self.db.begin_read()?;
        let hashes = txn.open_table(HASHES)?;
        let events = txn.open_table(EVENTS)?;
        let Some(position) = hashes.get(id.as_bytes().as_slice())? else {
            return Ok(None);
        };
        let stored = events.get(position.value())?;
        stored.map(|bytes| decode_stored(bytes.value())).transpose()
    }

    /// The fork proof for `author`, if the author has forked.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn fork_proof(&self, author: &AuthorKey) -> Result<Option<ForkProof>, StoreError> {
        let txn = self.db.begin_read()?;
        let forks = txn.open_table(FORKS)?;
        let row = forks.get(author.as_slice())?;
        row.map(|row| decode_proof(row.value())).transpose()
    }

    /// The positions, as `(author, seq)`, of the stored events in `group`.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn group_members(&self, group: &Hash) -> Result<Vec<(AuthorKey, SeqNum)>, StoreError> {
        let txn = self.db.begin_read()?;
        let groups = txn.open_multimap_table(GROUPS)?;
        let mut members = Vec::new();
        for position in groups.get(group.as_bytes().as_slice())? {
            members.push(split_position(position?.value())?);
        }
        Ok(members)
    }

    /// Checks the store's own rows against each other: every applied sequence is
    /// stored, every head matches its event, every index points at a stored
    /// event, and each frozen chain has a matching fork proof.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Corrupt`] for the first mismatch found, or a redb
    /// error if a read fails.
    pub fn check_invariants(&self) -> Result<(), StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let hashes = txn.open_table(HASHES)?;
        let chains = txn.open_table(CHAINS)?;
        let forks = txn.open_table(FORKS)?;
        let groups = txn.open_multimap_table(GROUPS)?;

        for row in events.iter()? {
            let (position, bytes) = row?;
            let (author, seq) = split_position(position.value())?;
            let event = decode_stored(bytes.value())?;
            if *event.header.verifying_key.as_bytes() != author || event.header.seq_num != seq {
                return Err(corrupt("an event is stored under another position"));
            }
            let indexed = hashes.get(event.id.as_bytes().as_slice())?;
            if indexed.map(|found| found.value().to_vec()) != Some(position.value().to_vec()) {
                return Err(corrupt("an event is not indexed under its ID"));
            }
            if let Some(group) = event.header.extensions.group {
                let mut found = false;
                for member in groups.get(group.as_bytes().as_slice())? {
                    found |= member?.value() == position.value();
                }
                if !found {
                    return Err(corrupt("an event is missing from its group index"));
                }
            }
        }

        for row in hashes.iter()? {
            let (id, position) = row?;
            let stored = events
                .get(position.value())?
                .ok_or_else(|| corrupt("an ID points at no stored event"))?;
            if Hash::digest(stored.value()) != hash_of(id.value())? {
                return Err(corrupt("an ID does not match the event it points at"));
            }
        }

        for row in chains.iter()? {
            let (author_bytes, chain_bytes) = row?;
            let author: AuthorKey = author_bytes
                .value()
                .try_into()
                .map_err(|_err| corrupt("a chain row has a short author key"))?;
            let chain = Chain::decode(chain_bytes.value())?;
            for seq in 0..chain.next_seq {
                if events.get(position_key(&author, seq).as_slice())?.is_none() {
                    return Err(corrupt(format!("applied sequence {seq} is not stored")));
                }
            }
            if let Some(head) = chain.head {
                let last = chain.next_seq.saturating_sub(1);
                let stored = events
                    .get(position_key(&author, last).as_slice())?
                    .ok_or_else(|| corrupt("the head of a chain is not stored"))?;
                if Hash::digest(stored.value()) != head {
                    return Err(corrupt("the head of a chain does not match its event"));
                }
            }
            let proof = forks
                .get(author_bytes.value())?
                .map(|row| decode_proof(row.value()))
                .transpose()?;
            match (chain.frozen, proof) {
                (false, None) => {}
                (true, Some(proof)) if proof.seq == chain.next_seq => {}
                _ => {
                    return Err(corrupt(
                        "a frozen chain has no fork proof at its fork point, or an open chain has one",
                    ));
                }
            }
        }

        for row in forks.iter()? {
            let (author_bytes, _) = row?;
            let chain = chains
                .get(author_bytes.value())?
                .map(|row| Chain::decode(row.value()))
                .transpose()?;
            if !chain.is_some_and(|chain| chain.frozen) {
                return Err(corrupt(
                    "a fork proof belongs to a chain that is not frozen",
                ));
            }
        }
        Ok(())
    }

    /// Runs redb's own integrity check on the file.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the check cannot run.
    pub fn check_integrity(&mut self) -> Result<bool, StoreError> {
        Ok(self.db.check_integrity()?)
    }
}

/// Builds a new store under [`TEMP_NAME`], then renames it into place.
fn create_fresh<S: Storage>(storage: &S) -> Result<Database, StoreError> {
    storage.remove(TEMP_NAME)?;
    let db = storage.create_database(TEMP_NAME)?;
    initialize(&db)?;
    drop(db);
    storage.sync_file(TEMP_NAME)?;
    storage.sync_dir()?;
    storage.rename(TEMP_NAME, FILE_NAME)?;
    storage.sync_dir()?;
    Ok(storage.open_database(FILE_NAME)?)
}

/// Commits every table and the format row in one transaction.
fn initialize(db: &Database) -> Result<(), StoreError> {
    let txn = db.begin_write()?;
    txn.open_table(EVENTS)?;
    txn.open_table(HASHES)?;
    txn.open_table(CHAINS)?;
    txn.open_table(FORKS)?;
    txn.open_multimap_table(GROUPS)?;
    let mut meta = txn.open_table(META)?;
    meta.insert(FORMAT_KEY, FORMAT)?;
    drop(meta);
    txn.commit()?;
    Ok(())
}

/// Checks that every table exists and that the format row is this store's.
fn verify_schema(db: &Database) -> Result<(), StoreError> {
    let txn = db.begin_read()?;
    let missing = |err: redb::TableError| StoreError::NotAStore(err.to_string());
    txn.open_table(EVENTS).map_err(missing)?;
    txn.open_table(HASHES).map_err(missing)?;
    txn.open_table(CHAINS).map_err(missing)?;
    txn.open_table(FORKS).map_err(missing)?;
    txn.open_multimap_table(GROUPS).map_err(missing)?;
    let meta = txn.open_table(META).map_err(missing)?;
    match meta.get(FORMAT_KEY)? {
        Some(found) if found.value() == FORMAT => Ok(()),
        _ => Err(StoreError::NotAStore(
            "the format row is missing or names another format".to_owned(),
        )),
    }
}

/// Maps redb's open errors. A second opener is `InUse`. redb reports a file that
/// is not a database as an `InvalidData` I/O error, so that is `NotAStore`, and
/// any other I/O failure is `Io`. Everything else means the file is not a store.
fn open_error(err: DatabaseError) -> StoreError {
    match err {
        DatabaseError::DatabaseAlreadyOpen => StoreError::InUse,
        DatabaseError::Storage(StorageError::Io(err))
            if err.kind() != io::ErrorKind::InvalidData =>
        {
            StoreError::Io(err)
        }
        other => StoreError::NotAStore(other.to_string()),
    }
}

/// The key of `(author, seq)`: the author's bytes, then the sequence number.
pub(crate) fn position_key(author: &AuthorKey, seq: SeqNum) -> [u8; POSITION_LEN] {
    let mut key = [0u8; POSITION_LEN];
    key[..32].copy_from_slice(author);
    key[32..].copy_from_slice(&u64::from(seq).to_be_bytes());
    key
}

/// Splits a position key into its author and sequence number.
fn split_position(key: &[u8]) -> Result<(AuthorKey, SeqNum), StoreError> {
    let key: &[u8; POSITION_LEN] = key
        .try_into()
        .map_err(|_err| corrupt("a position key has the wrong length"))?;
    let author: AuthorKey = key[..32]
        .try_into()
        .map_err(|_err| corrupt("a position key has a short author"))?;
    Ok((author, seq_num(be_u64(&key[32..])?)?))
}

fn be_u64(bytes: &[u8]) -> Result<u64, StoreError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_err| corrupt("a number is not 8 bytes"))?;
    Ok(u64::from_be_bytes(bytes))
}

/// A stored sequence number. Stored as eight bytes, and checked to fit `SeqNum`.
fn seq_num(value: u64) -> Result<SeqNum, StoreError> {
    SeqNum::try_from(value).map_err(|_err| corrupt("a sequence number is out of range"))
}

fn hash_of(bytes: &[u8]) -> Result<Hash, StoreError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_err| corrupt("a hash is not 32 bytes"))?;
    Ok(Hash::from_bytes(bytes))
}

/// Decodes a stored event. A stored event decoded when it was accepted, so a
/// failure here means the file changed underneath the store.
fn decode_stored(bytes: &[u8]) -> Result<Event, StoreError> {
    chain::decode(bytes).map_err(|err| corrupt(format!("a stored event does not decode: {err}")))
}

/// Encodes a fork proof: the sequence number, the length of the first header,
/// then the two headers, ordered by ID.
pub(crate) fn encode_proof(seq: SeqNum, a: &Event, b: &Event) -> Result<Vec<u8>, StoreError> {
    let (first, second) = if a.id.as_bytes() <= b.id.as_bytes() {
        (a, b)
    } else {
        (b, a)
    };
    let length = u32::try_from(first.bytes.len())
        .map_err(|_err| corrupt("an event is too long for a fork proof"))?;
    let mut out = Vec::with_capacity(12 + first.bytes.len() + second.bytes.len());
    out.extend_from_slice(&u64::from(seq).to_be_bytes());
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&first.bytes);
    out.extend_from_slice(&second.bytes);
    Ok(out)
}

fn decode_proof(bytes: &[u8]) -> Result<ForkProof, StoreError> {
    if bytes.len() < 12 {
        return Err(corrupt("a fork proof is shorter than its header"));
    }
    let seq = seq_num(be_u64(&bytes[..8])?)?;
    let length: [u8; 4] = bytes[8..12]
        .try_into()
        .map_err(|_err| corrupt("a fork proof has a short length"))?;
    let length = u32::from_be_bytes(length) as usize;
    let rest = &bytes[12..];
    if length > rest.len() {
        return Err(corrupt("a fork proof names more bytes than it holds"));
    }
    let (first, second) = rest.split_at(length);
    Ok(ForkProof {
        seq,
        first: decode_stored(first)?,
        second: decode_stored(second)?,
    })
}

/// Stores an accepted event at its position, and indexes it by ID and group.
/// A position that is already stored is refused, inside the same transaction.
pub(crate) fn store_event(txn: &WriteTransaction, event: &Event) -> Result<(), StoreError> {
    let author = *event.header.verifying_key.as_bytes();
    let seq = event.header.seq_num;
    let key = position_key(&author, seq);
    let mut events = txn.open_table(EVENTS)?;
    if events.get(key.as_slice())?.is_some() {
        return Err(StoreError::Occupied { seq });
    }
    events.insert(key.as_slice(), event.bytes.as_slice())?;
    drop(events);
    let mut hashes = txn.open_table(HASHES)?;
    hashes.insert(event.id.as_bytes().as_slice(), key.as_slice())?;
    drop(hashes);
    if let Some(group) = event.header.extensions.group {
        let mut groups = txn.open_multimap_table(GROUPS)?;
        groups.insert(group.as_bytes().as_slice(), key.as_slice())?;
    }
    Ok(())
}

/// Writes an author's chain row.
pub(crate) fn store_chain(
    txn: &WriteTransaction,
    author: &AuthorKey,
    chain: Chain,
) -> Result<(), StoreError> {
    let mut chains = txn.open_table(CHAINS)?;
    chains.insert(author.as_slice(), chain.encode().as_slice())?;
    Ok(())
}

/// Writes an author's fork proof, replacing any earlier one.
pub(crate) fn store_fork(
    txn: &WriteTransaction,
    author: &AuthorKey,
    seq: SeqNum,
    a: &Event,
    b: &Event,
) -> Result<(), StoreError> {
    let proof = encode_proof(seq, a, b)?;
    let mut forks = txn.open_table(FORKS)?;
    forks.insert(author.as_slice(), proof.as_slice())?;
    Ok(())
}

impl EventStore {
    /// The applied events of `author` with sequence numbers in `(after, until]`, in sequence
    /// order, at most `limit` of them. `after` is exclusive and `until` is inclusive. `None`
    /// leaves that side open. Events past a fork point are stored but not applied, so they are
    /// not returned.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn applied_events(
        &self,
        author: &AuthorKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
        limit: usize,
    ) -> Result<Vec<Event>, StoreError> {
        let txn = self.db.begin_read()?;
        let chains = txn.open_table(CHAINS)?;
        let Some(chain_row) = chains.get(author.as_slice())? else {
            return Ok(Vec::new());
        };
        let Some(last) = Chain::decode(chain_row.value())?.next_seq.checked_sub(1) else {
            return Ok(Vec::new());
        };
        let last = until.map_or(last, |until| until.min(last));
        let first = match after {
            Some(after) => after.checked_add(1),
            None => Some(0),
        };
        let Some(first) = first.filter(|first| *first <= last) else {
            return Ok(Vec::new());
        };
        let events = txn.open_table(EVENTS)?;
        let (start, end) = (position_key(author, first), position_key(author, last));
        let mut found = Vec::new();
        for row in events.range(start.as_slice()..=end.as_slice())?.take(limit) {
            let (_position, bytes) = row?;
            found.push(decode_stored(bytes.value())?);
        }
        Ok(found)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::chain::{ChainExt, sign};
    use p2panda_core::SigningKey;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    fn ext(kind: &str, group: Option<Hash>) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group,
            after: Vec::new(),
        }
    }

    /// The first `count` events of `key`'s chain, each naming the one before.
    fn chain_of(key: &SigningKey, count: SeqNum) -> Vec<Event> {
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..count {
            let backlink = events.last().map(|event| event.id);
            let event = sign(key, seq, backlink, None, ext("message", None)).unwrap();
            events.push(event);
        }
        events
    }

    fn open_temp() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn test_a_new_store_is_created_empty_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path()).unwrap();
        assert!(store.chain(&author(&key(1))).unwrap().is_none());
        store.check_invariants().unwrap();
        assert!(dir.path().join(FILE_NAME).exists());
        assert!(!dir.path().join(TEMP_NAME).exists(), "no temp file is left");
        drop(store);
        let reopened = EventStore::open(dir.path()).unwrap();
        reopened.check_invariants().unwrap();
    }

    #[test]
    fn test_a_stale_temp_file_is_replaced_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(TEMP_NAME), b"half a store from a crash").unwrap();
        let store = EventStore::open(dir.path()).unwrap();
        store.check_invariants().unwrap();
        assert!(!dir.path().join(TEMP_NAME).exists());
    }

    #[test]
    fn test_a_file_that_is_not_a_store_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let garbage = b"this is not a redb database at all".to_vec();
        fs::write(dir.path().join(FILE_NAME), &garbage).unwrap();
        let err = EventStore::open(dir.path()).unwrap_err();
        assert!(
            matches!(err, StoreError::NotAStore(_)),
            "unexpected error: {err}"
        );
        assert!(err.to_string().contains(FILE_NAME), "{err}");
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), garbage);
    }

    #[test]
    fn test_a_second_opener_is_refused_while_the_first_is_open() {
        let (dir, store) = open_temp();
        let err = EventStore::open(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::InUse), "unexpected error: {err}");
        drop(store);
        EventStore::open(dir.path()).unwrap();
    }

    #[test]
    fn test_a_second_event_at_one_position_is_refused_in_the_same_transaction() {
        let (_dir, store) = open_temp();
        let signer = key(2);
        let first = chain_of(&signer, 1).remove(0);
        let rival = sign(&signer, 0, None, None, ext("other", None)).unwrap();
        let result = store.write(|txn| {
            store_event(txn, &first)?;
            store_event(txn, &rival)
        });
        assert!(matches!(result, Err(StoreError::Occupied { seq: 0 })));
        // The transaction was aborted, so the first event was not stored either.
        assert!(store.event(&author(&signer), 0).unwrap().is_none());
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_stored_event_reads_back_by_position_id_and_group() {
        let (_dir, store) = open_temp();
        let signer = key(3);
        let group = Hash::digest(b"group");
        let event = sign(&signer, 0, None, None, ext("message", Some(group))).unwrap();
        store.write(|txn| store_event(txn, &event)).unwrap();
        let who = author(&signer);
        assert_eq!(store.event(&who, 0).unwrap(), Some(event.clone()));
        assert_eq!(store.event_by_id(&event.id).unwrap(), Some(event.clone()));
        assert_eq!(store.group_members(&group).unwrap(), vec![(who, 0)]);
        assert!(
            store
                .group_members(&Hash::digest(b"other"))
                .unwrap()
                .is_empty()
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_a_chain_row_round_trips_with_and_without_a_head() {
        let (_dir, store) = open_temp();
        let who = author(&key(4));
        let empty = Chain {
            next_seq: 0,
            head: None,
            frozen: false,
        };
        store.write(|txn| store_chain(txn, &who, empty)).unwrap();
        assert_eq!(store.chain(&who).unwrap(), Some(empty));
        let head = Chain {
            next_seq: 5,
            head: Some(Hash::digest(b"head")),
            frozen: false,
        };
        store.write(|txn| store_chain(txn, &who, head)).unwrap();
        assert_eq!(store.chain(&who).unwrap(), Some(head));
    }

    #[test]
    fn test_a_fork_proof_holds_both_headers_in_id_order() {
        let (_dir, store) = open_temp();
        let signer = key(5);
        let original = chain_of(&signer, 1).remove(0);
        let rival = sign(&signer, 0, None, None, ext("rival", None)).unwrap();
        let who = author(&signer);
        store
            .write(|txn| {
                store_event(txn, &original)?;
                store_fork(txn, &who, 0, &rival, &original)?;
                store_chain(
                    txn,
                    &who,
                    Chain {
                        next_seq: 0,
                        head: None,
                        frozen: true,
                    },
                )
            })
            .unwrap();
        let proof = store.fork_proof(&who).unwrap().expect("a proof is stored");
        assert_eq!(proof.seq, 0);
        assert!(proof.first.id.as_bytes() < proof.second.id.as_bytes());
        let mut ids = [original.id, rival.id];
        ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!([proof.first.id, proof.second.id], ids);
        store.check_invariants().unwrap();
    }

    #[test]
    fn test_invariants_refuse_a_head_with_no_stored_event() {
        let (_dir, store) = open_temp();
        let who = author(&key(6));
        let claims_two = Chain {
            next_seq: 2,
            head: Some(Hash::digest(b"second")),
            frozen: false,
        };
        store
            .write(|txn| store_chain(txn, &who, claims_two))
            .unwrap();
        assert!(matches!(
            store.check_invariants(),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn test_invariants_refuse_a_frozen_chain_without_a_proof() {
        let (_dir, store) = open_temp();
        let who = author(&key(7));
        let frozen = Chain {
            next_seq: 0,
            head: None,
            frozen: true,
        };
        store.write(|txn| store_chain(txn, &who, frozen)).unwrap();
        assert!(matches!(
            store.check_invariants(),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn test_applied_events_is_the_applied_prefix_in_range() {
        let (_dir, mut store) = open_temp();
        let signer = key(16);
        let who = author(&signer);
        let events = chain_of(&signer, 4);
        for event in &events {
            assert!(matches!(
                store.ingest(&event.bytes, 0).unwrap(),
                Ingest::Accepted { .. }
            ));
        }
        let seqs = |found: Vec<Event>| -> Vec<SeqNum> {
            found.iter().map(|event| event.header.seq_num).collect()
        };
        assert_eq!(
            seqs(store.applied_events(&who, None, None, 10).unwrap()),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            seqs(store.applied_events(&who, Some(0), Some(2), 10).unwrap()),
            vec![1, 2]
        );
        assert_eq!(
            seqs(store.applied_events(&who, None, None, 2).unwrap()),
            vec![0, 1]
        );
        assert!(
            store
                .applied_events(&who, Some(3), None, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .applied_events(&author(&key(17)), None, None, 10)
                .unwrap()
                .is_empty()
        );
        // A fork at seq 2 leaves seq 0 and 1 applied. Seq 3 stays stored, but it is not returned.
        let rival = sign(&signer, 2, Some(events[1].id), None, ext("rival", None)).unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, 0).unwrap(),
            Ingest::Fork { seq: 2, .. }
        ));
        assert_eq!(
            seqs(store.applied_events(&who, None, None, 10).unwrap()),
            vec![0, 1]
        );
        assert!(store.event(&who, 3).unwrap().is_some());
    }
}
